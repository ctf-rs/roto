#![cfg(not(miri))]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use roto::{
    FileTree, List, Module, NoCtx, RegexLiteral, RotoReport, Runtime, Type,
    Val, location, syntax,
};

#[derive(Clone, Debug)]
struct CompiledPattern(Arc<PatternData>);

#[derive(Debug)]
struct PatternData {
    pattern: String,
    build: usize,
    drops: Arc<AtomicUsize>,
}

impl PartialEq for CompiledPattern {
    fn eq(&self, other: &Self) -> bool {
        self.0.pattern == other.0.pattern
    }
}

impl Drop for PatternData {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Counts {
    builds: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl Counts {
    fn provider(&self) -> RegexLiteral {
        let builds = self.builds.clone();
        let drops = self.drops.clone();
        RegexLiteral::new(
            move |pattern| {
                let build = builds.fetch_add(1, Ordering::SeqCst) + 1;
                if pattern.starts_with('[') {
                    return Err("host rejected the pattern".into());
                }
                Ok(Val(CompiledPattern(Arc::new(PatternData {
                    pattern: pattern.to_owned(),
                    build,
                    drops: drops.clone(),
                }))))
            },
            location!(),
        )
    }

    fn runtime(&self) -> Runtime<NoCtx> {
        let mut runtime = Runtime::new();
        runtime.add(pattern_type("HostPattern")).unwrap();
        runtime.add(self.provider()).unwrap();
        runtime
    }

    fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }

    fn drops(&self) -> usize {
        self.drops.load(Ordering::SeqCst)
    }
}

fn pattern_type(name: &str) -> Type {
    Type::clone::<Val<CompiledPattern>>(name, "", location!()).unwrap()
}

fn source_file(source: &str) -> FileTree {
    FileTree::test_file("regex.roto", source, 0)
}

fn compile_error(source: &str, runtime: &Runtime<NoCtx>) -> RotoReport {
    match source_file(source).compile(runtime) {
        Ok(_) => panic!("source unexpectedly compiled: {source}"),
        Err(error) => error,
    }
}

fn assert_literal_error(
    report: &RotoReport,
    source: &str,
    literal: &str,
    message: &str,
) {
    let diagnostics = report.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert!(
        diagnostic.message.contains(message),
        "{}",
        diagnostic.message
    );
    let start = source.find(literal).unwrap();
    let expected = start..start + literal.len();
    assert_eq!(diagnostic.span.unwrap().range(), expected);
    assert_eq!(diagnostic.labels[0].range, expected);
    assert_eq!(diagnostic.labels[0].file, 0);
    let mut rendered = String::new();
    report.write(&mut rendered, false).unwrap();
    assert!(rendered.contains(&diagnostic.message));
    assert!(rendered.contains(&diagnostic.labels[0].message));
}

#[test]
fn regex_literals_require_a_host_provider() {
    let mut runtime = Runtime::new();
    runtime.add(pattern_type("HostPattern")).unwrap();
    let literal = r##"r#"a"b"#"##;
    let source = format!("const FLAG: HostPattern = {literal};");
    let report = compile_error(&source, &runtime);
    assert_literal_error(
        &report,
        &source,
        literal,
        "no regex literal provider is registered",
    );
}

#[test]
fn regex_callback_errors_span_the_full_raw_literal() {
    for literal in ["r\"[bad\"", "r###\"[λ\\d\n\"##🦀\"###"] {
        let counts = Counts::default();
        let runtime = counts.runtime();
        let source = format!("// 雪\nconst FLAG: HostPattern = {literal};");
        let report = compile_error(&source, &runtime);
        assert_literal_error(
            &report,
            &source,
            literal,
            "invalid regex literal: host rejected the pattern",
        );
        assert_eq!(
            report.diagnostics()[0].labels[0].message,
            "host rejected the pattern",
        );
        assert_eq!(counts.builds(), 1);
        assert_eq!(counts.drops(), 0);
    }
}

#[test]
fn regex_literals_are_rejected_in_all_executable_bodies() {
    let counts = Counts::default();
    let runtime = counts.runtime();
    let literal = r##"r#"\d+"#"##;
    for source in [
        format!("fn main() -> HostPattern {{ {literal} }}"),
        format!("filter main() {{ let p = {literal}; accept }}"),
        format!("filtermap main() {{ let p = {literal}; accept }}"),
        format!("test inline_regex {{ let p = {literal}; accept }}"),
        format!("fn main() {{ let x = {{ let p = {literal}; p }}; }}"),
        format!(
            "const FLAG: HostPattern = make();
             fn make() -> HostPattern {{ {literal} }}"
        ),
    ] {
        let report = compile_error(&source, &runtime);
        assert_literal_error(
            &report,
            &source,
            literal,
            "only allowed in module-level constant initializers",
        );
    }
    assert_eq!(counts.builds(), 0);
}

#[test]
fn regex_constants_support_aliases_records_options_lists_and_inference() {
    let counts = Counts::default();
    let runtime = counts.runtime();
    let source = r###"
        const FLAG: HostPattern = r"\d";
        const ALIAS: HostPattern = FLAG;
        record Wrapper { pattern: HostPattern }
        const NESTED: Wrapper = {
            let inferred = r#"a"b"#;
            Wrapper { pattern: inferred }
        };
        const OPTIONAL: Option[HostPattern] = Some(r"\\d");
        const PATTERNS: List[HostPattern] = [r"first", r"second"];

        fn direct() -> HostPattern { FLAG }
        fn alias() -> HostPattern { ALIAS }
        fn nested() -> HostPattern { NESTED.pattern }
        fn optional() -> Option[HostPattern] { OPTIONAL }
        fn patterns() -> List[HostPattern] { PATTERNS }
    "###;
    let parsed = syntax::parse(source).unwrap();
    let mut package = source_file(source)
        .compile_parsed(parsed, &runtime)
        .unwrap();
    assert_eq!(counts.builds(), 5);

    let direct = package
        .get_function::<fn() -> Val<CompiledPattern>>("direct")
        .unwrap();
    let alias = package
        .get_function::<fn() -> Val<CompiledPattern>>("alias")
        .unwrap();
    let nested = package
        .get_function::<fn() -> Val<CompiledPattern>>("nested")
        .unwrap();
    let optional = package
        .get_function::<fn() -> Option<Val<CompiledPattern>>>("optional")
        .unwrap();
    let patterns = package
        .get_function::<fn() -> List<Val<CompiledPattern>>>("patterns")
        .unwrap();
    let first = direct.call();
    assert_eq!(first.0.0.pattern, r"\d");
    assert!(Arc::ptr_eq(&first.0.0, &alias.call().0.0));
    assert_eq!(nested.call().0.0.pattern, "a\"b");
    assert_eq!(optional.call().unwrap().0.0.pattern, r"\\d");
    assert_eq!(patterns.call().len(), 2);

    drop(first);
    drop(package);
    drop(runtime);
    assert_eq!(counts.drops(), 0);
    assert_eq!(nested.call().0.0.pattern, "a\"b");
    drop((direct, alias, nested, optional, patterns));
    assert_eq!(counts.drops(), 5);
}

#[test]
fn regex_values_outlive_runtime_and_package_without_recompilation() {
    let counts = Counts::default();
    let function = {
        let runtime = counts.runtime();
        let mut package = source_file(
            r#"const FLAG: HostPattern = r"\d+";
               fn main() -> HostPattern { FLAG }"#,
        )
        .compile(&runtime)
        .unwrap();
        let function = package
            .get_function::<fn() -> Val<CompiledPattern>>("main")
            .unwrap();
        drop(package);
        drop(runtime);
        function
    };
    assert_eq!(counts.builds(), 1);
    assert_eq!(counts.drops(), 0);

    let value = function.call();
    for _ in 0..5_000 {
        let cloned_function = function.clone();
        let cloned_value = cloned_function.call();
        assert!(Arc::ptr_eq(&value.0.0, &cloned_value.0.0));
        assert_eq!(cloned_value.0.0.pattern, r"\d+");
    }
    assert_eq!(counts.builds(), 1);
    assert_eq!(counts.drops(), 0);
    drop(function);
    assert_eq!(counts.drops(), 0);
    drop(value);
    assert_eq!(counts.drops(), 1);
}

#[test]
fn regex_values_are_compiled_independently_for_each_package() {
    let counts = Counts::default();
    let runtime = counts.runtime();
    let source = r#"const FLAG: HostPattern = r"same";
                    fn main() -> HostPattern { FLAG }"#;
    let mut first = source_file(source).compile(&runtime).unwrap();
    let mut second = source_file(source).compile(&runtime).unwrap();
    assert_eq!(counts.builds(), 2);
    let a = first
        .get_function::<fn() -> Val<CompiledPattern>>("main")
        .unwrap()
        .call();
    let b = second
        .get_function::<fn() -> Val<CompiledPattern>>("main")
        .unwrap()
        .call();
    assert!(!Arc::ptr_eq(&a.0.0, &b.0.0));
    assert_eq!((a.0.0.build, b.0.0.build), (1, 2));
    drop((a, b, first));
    assert_eq!(counts.drops(), 1);
    drop(second);
    assert_eq!(counts.drops(), 2);
}

#[test]
fn failed_compilations_drop_successfully_built_regex_values() {
    for failure in [
        r#"const BAD: HostPattern = r"[invalid";"#,
        "fn bad() { unknown_value }",
    ] {
        let counts = Counts::default();
        let runtime = counts.runtime();
        let source =
            format!(r#"const GOOD: HostPattern = r"good"; {failure}"#);
        let report = compile_error(&source, &runtime);
        assert_eq!(report.diagnostics().len(), 1);
        assert_eq!(counts.drops(), 1);
        drop(report);
        drop(runtime);
        assert_eq!(counts.drops(), 1);
    }
}

#[test]
fn regex_compilation_visits_every_literal_even_in_dead_constant_branches() {
    let counts = Counts::default();
    let runtime = counts.runtime();
    let mut package = source_file(
        r#"const FLAG: HostPattern = if false { r"dead" } else { r"live" };
           fn main() -> HostPattern { FLAG }"#,
    )
    .compile(&runtime)
    .unwrap();
    assert_eq!(counts.builds(), 2);
    let function = package
        .get_function::<fn() -> Val<CompiledPattern>>("main")
        .unwrap();
    assert_eq!(function.call().0.0.pattern, "live");
    drop((function, package));
    assert_eq!(counts.drops(), 2);
}

#[test]
fn regex_provider_registration_rejects_unregistered_types_and_duplicates() {
    let counts = Counts::default();
    let mut runtime = Runtime::new();
    let before = runtime.functions().len();
    let error = runtime.add(counts.provider()).unwrap_err();
    assert!(error.to_string().contains("unregistered type"));
    runtime.add(pattern_type("HostPattern")).unwrap();
    runtime.add(counts.provider()).unwrap();

    let other = Counts::default();
    let error = runtime.add(other.provider()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Only one regex literal provider")
    );
    assert_eq!(runtime.functions().len(), before);
    assert_eq!(counts.builds(), 0);
    assert_eq!(other.builds(), 0);

    let package = source_file(r#"const FLAG: HostPattern = r"one";"#)
        .compile(&runtime)
        .unwrap();
    assert_eq!(counts.builds(), 1);
    assert_eq!(other.builds(), 0);
    drop(package);
    assert_eq!(counts.drops(), 1);
}

#[test]
fn regex_provider_uses_the_actual_namespaced_registered_type() {
    let counts = Counts::default();
    let mut host = Module::new("host", "", location!()).unwrap();
    host.add(counts.provider());
    host.add(pattern_type("Matcher"));
    let runtime = Runtime::from_lib(host).unwrap();
    let mut package = source_file(
        r#"const FLAG: host.Matcher = r"namespace";
           fn main() -> host.Matcher { FLAG }"#,
    )
    .compile(&runtime)
    .unwrap();
    let function = package
        .get_function::<fn() -> Val<CompiledPattern>>("main")
        .unwrap();
    assert_eq!(function.call().0.0.pattern, "namespace");
    assert_eq!(counts.builds(), 1);
    drop((function, package));
    assert_eq!(counts.drops(), 1);
}

#[test]
fn regex_provider_transforms_values_instead_of_assuming_a_host_layout() {
    let mut runtime = Runtime::new();
    runtime
        .add(RegexLiteral::new(
            |pattern| Ok(Some(pattern.len() as i32)),
            location!(),
        ))
        .unwrap();
    let mut package = source_file(
        r#"const FLAG: Option[i32] = r"abcd";
           fn main() -> Option[i32] { FLAG }"#,
    )
    .compile(&runtime)
    .unwrap();
    let function =
        package.get_function::<fn() -> Option<i32>>("main").unwrap();
    drop((runtime, package));
    assert_eq!(function.call(), Some(4));
}

#[test]
fn regex_type_mismatch_is_diagnosed_before_compiling() {
    let counts = Counts::default();
    let runtime = counts.runtime();
    let literal = r#"r"wrong type""#;
    let source = format!("const FLAG: String = {literal};");
    let report = compile_error(&source, &runtime);
    let diagnostic = &report.diagnostics()[0];
    assert_eq!(diagnostic.message, "Type error: mismatched types");
    assert!(diagnostic.labels[0].message.contains("HostPattern"));
    let start = source.find(literal).unwrap();
    assert_eq!(
        diagnostic.span.unwrap().range(),
        start..start + literal.len()
    );
    assert_eq!(counts.builds(), 0);
}

#[test]
fn unterminated_regex_reports_the_same_native_and_syntax_span() {
    let source = "// 雪\nconst FLAG: i32 = r##\"λ\nunclosed\"#";
    let syntax_error = syntax::parse(source).unwrap_err().diagnostic();
    let runtime = Runtime::new();
    let report = compile_error(source, &runtime);
    let diagnostic = &report.diagnostics()[0];
    assert_eq!(diagnostic.span, Some(syntax_error.span));
    assert_eq!(
        diagnostic.message,
        "Parse error: unterminated regex literal"
    );
}
