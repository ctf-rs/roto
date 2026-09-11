use roto::{
    FileTree, NoCtx, RotoDiagnosticSeverity, RotoReport, RotoString, Runtime,
    syntax,
};

fn compile_error(source: &str) -> RotoReport {
    let runtime = Runtime::<NoCtx>::new();
    match FileTree::test_file("diagnostic.roto", source, 0).compile(&runtime)
    {
        Ok(_) => panic!("source unexpectedly compiled"),
        Err(report) => report,
    }
}

fn assert_rendered_contains_structured(report: &RotoReport) {
    let mut rendered = String::new();
    report.write(&mut rendered, false).unwrap();

    for diagnostic in report.diagnostics() {
        assert!(
            rendered.contains(&diagnostic.message),
            "rendered report omitted {:?}",
            diagnostic.message
        );
        for label in diagnostic.labels {
            assert!(
                rendered.contains(&label.message),
                "rendered report omitted {:?}",
                label.message
            );
        }
        for note in diagnostic.notes {
            assert!(rendered.contains(&note));
        }
        for help in diagnostic.help {
            assert!(rendered.contains(&help));
        }
    }
}

#[test]
fn native_parser_reports_the_same_structured_failure() {
    let source = "/* outer /* nested */ body */\nfn main() {}";
    let syntax_error = syntax::parse(source).unwrap_err().diagnostic();
    let report = compile_error(source);
    let diagnostics = report.diagnostics();
    let diagnostic = &diagnostics[0];

    assert_eq!(diagnostic.severity, RotoDiagnosticSeverity::Error);
    assert_eq!(diagnostic.span, Some(syntax_error.span));
    assert_eq!(
        diagnostic.message,
        format!("Parse error: {}", syntax_error.message)
    );
    assert_eq!(
        diagnostic.labels[0].range,
        syntax_error.labels[0].span.range()
    );
    assert_rendered_contains_structured(&report);
}

#[test]
fn type_diagnostics_keep_exact_identifier_spans() {
    let source = "fn main() {\n    missing_value\n}\n";
    let report = compile_error(source);
    let diagnostics = report.diagnostics();
    let diagnostic = &diagnostics[0];
    let start = source.find("missing_value").unwrap();

    assert!(diagnostic.message.contains("missing_value"));
    assert_eq!(diagnostic.span.unwrap().range(), start..start + 13);
    assert_eq!(diagnostic.labels[0].range, start..start + 13);
    assert_rendered_contains_structured(&report);
}

#[test]
fn pre_parsed_question_mark_source_compiles_without_reparsing() {
    let source = "\
fn propagate(value: Result[i32, String]) -> Result[i32, String] {
    Ok(value?)
}";
    let parsed = syntax::parse(source).unwrap();
    let runtime = Runtime::<NoCtx>::new();
    let mut package = FileTree::test_file("question-mark.roto", source, 0)
        .compile_parsed(parsed, &runtime)
        .unwrap();
    let function = package
        .get_function::<
            fn(Result<i32, RotoString>) -> Result<i32, RotoString>,
        >("propagate")
        .unwrap();

    assert_eq!(function.call(Ok(42)), Ok(42));
    assert_eq!(function.call(Err("failure".into())), Err("failure".into()));
}

#[test]
fn pre_parsed_source_must_match_the_compiler_input() {
    let parsed = syntax::parse("fn main() {}").unwrap();
    let runtime = Runtime::<NoCtx>::new();
    let report =
        match FileTree::test_file("mismatch.roto", "fn other() {}", 0)
            .compile_parsed(parsed, &runtime)
        {
            Ok(_) => panic!("mismatched parsed source unexpectedly compiled"),
            Err(report) => report,
        };

    assert_eq!(
        report.diagnostics()[0].message,
        "parsed source does not match the single-file FileTree"
    );
}
