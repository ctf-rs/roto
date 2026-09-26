use roto_syntax::{
    DiagnosticSeverity, LexedToken, ParseErrorKind, Parser, Span, Spans,
    Token,
    ast::{Expr, Literal},
    lex, parse_file,
};

#[test]
fn regex_literals_preserve_raw_bodies_and_exact_spans_at_eof() {
    for (hash_count, pattern) in [
        (0, ""),
        (1, ""),
        (2, ""),
        (0, r"\d"),
        (0, r"\\d"),
        (1, r#"say "hello""#),
        (2, "one\"#two"),
        (3, "one\"##two"),
        (1, "one\"###two"),
        (0, "one\"#two"),
        (0, "λ雪🦀"),
        (0, "first\nsecond\r\nthird"),
        (2, "λ\n\"#🦀\nend\\"),
    ] {
        let hashes = "#".repeat(hash_count);
        let literal = format!("r{hashes}\"{pattern}\"{hashes}");
        let source = format!(" \n{literal}");
        assert_eq!(
            lex(&source),
            [LexedToken {
                token: Some(Token::Regex(&literal)),
                span: 2..source.len(),
            }],
            "{source:?}",
        );

        let mut spans = Spans::default();
        let expression =
            Parser::run_parser(Parser::expr, 7, &mut spans, &source).unwrap();
        let Expr::Literal(parsed) = &expression.node else {
            panic!("expected a literal: {expression:?}");
        };
        let Literal::Regex(body) = &parsed.node else {
            panic!("expected a regex literal: {parsed:?}");
        };
        assert_eq!(body, pattern);
        assert_eq!(spans.get(parsed), Span::new(7, 2..source.len()),);
        assert_eq!(spans.get(&expression), spans.get(parsed));
        assert_eq!(Token::Regex(&literal).to_string(), literal);
    }
}

#[test]
fn regex_delimiters_allow_arbitrary_matching_hash_counts() {
    for count in [1, 2, 3, 8, 255, 256, 1024] {
        let hashes = "#".repeat(count);
        let insufficient = "#".repeat(count - 1);
        let pattern = format!("before\"{insufficient}after");
        let literal = format!("r{hashes}\"{pattern}\"{hashes}");
        let mut spans = Spans::default();
        let expression =
            Parser::run_parser(Parser::expr, 0, &mut spans, &literal)
                .unwrap();
        let Expr::Literal(parsed) = expression.node else {
            panic!("expected a literal");
        };
        assert!(
            matches!(parsed.node, Literal::Regex(body) if body == pattern)
        );
        assert_eq!(spans.get(parsed.id).range(), 0..literal.len());
    }
}

#[test]
fn unterminated_regex_is_one_token_and_has_a_full_literal_diagnostic() {
    for literal in [
        "r\"",
        "r\"\\d",
        "r#\"",
        "r#\"unclosed\"",
        "r##\"λ\nunclosed\"#",
        "r#\"excess\"##",
        "r\"excess\"#",
    ] {
        let prefix = "// 雪\nconst PATTERN: Pattern = ";
        let source = format!("{prefix}{literal}");
        let tokens = lex(&source);
        let last = tokens.last().unwrap();
        assert_eq!(last.token, Some(Token::UnterminatedRegex(literal)));
        assert_eq!(last.span, prefix.len()..source.len());

        let error = parse_file(9, "pkg", &source).unwrap_err();
        assert_eq!(error.kind, ParseErrorKind::UnterminatedRegexLiteral);
        let diagnostic = error.diagnostic();
        assert_eq!(diagnostic.message, "unterminated regex literal");
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostic.span, Span::new(9, prefix.len()..source.len()));
        assert_eq!(diagnostic.labels[0].span, diagnostic.span);
    }
}

#[test]
fn regex_prefixes_do_not_change_identifiers_or_normal_strings() {
    let tokens: Vec<_> = lex("r regex r_name r1 r# \"text\";")
        .into_iter()
        .map(|token| token.token.unwrap())
        .collect();
    assert_eq!(
        tokens,
        [
            Token::Ident("r"),
            Token::Ident("regex"),
            Token::Ident("r_name"),
            Token::Ident("r1"),
            Token::Ident("r"),
            Token::Hash,
            Token::String("\"text\""),
            Token::SemiColon,
        ]
    );
}
