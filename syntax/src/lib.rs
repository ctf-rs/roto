//! Roto's authoritative, target-independent syntax frontend.
//!
//! This crate contains the lexer, parser, public AST, exact source metadata,
//! and structured parse diagnostics shared by the native compiler and editor
//! tooling. It intentionally has no runtime or code-generation dependencies
//! and supports `wasm32-unknown-unknown`.

use std::ops::Range;

pub mod ast;
mod diagnostic;
pub mod parser;

pub use ast::SyntaxTree;
pub use diagnostic::{Diagnostic, DiagnosticLabel, DiagnosticSeverity};
pub use parser::error::{Hint, ParseError, ParseErrorKind};
pub use parser::meta::{Meta, MetaId, Span, Spans};
pub use parser::token::{FStringToken, Keyword, Token};
pub use parser::{ParseResult, Parser};

/// One item emitted by Roto's authoritative lexer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexedToken<'source> {
    /// The token, or `None` when the lexer encountered invalid input.
    pub token: Option<Token<'source>>,

    /// The exact byte range of the token in the original source.
    pub span: Range<usize>,
}

/// Tokenize `source` with Roto's authoritative lexer.
#[must_use]
pub fn lex(source: &str) -> Vec<LexedToken<'_>> {
    let mut lexer = parser::lexer::Lexer::new(source);
    let mut tokens = Vec::new();
    while let Some((token, span)) = lexer.next() {
        let token = token.ok();
        tokens.push(LexedToken { token, span });
        if token == Some(Token::FStringStart) {
            lex_f_string(&mut lexer, &mut tokens);
        }
    }
    tokens
}

fn lex_f_string<'source>(
    lexer: &mut parser::lexer::Lexer<'source>,
    tokens: &mut Vec<LexedToken<'source>>,
) {
    loop {
        let Some((part, span)) = lexer.f_string_part() else {
            return;
        };
        match part {
            FStringToken::StringEnd(text) => {
                tokens.push(LexedToken {
                    token: Some(Token::FStringEnd(text)),
                    span,
                });
                return;
            }
            FStringToken::StringIntermediate(text) => {
                if !text.is_empty() {
                    tokens.push(LexedToken {
                        token: Some(Token::FStringText(text)),
                        span,
                    });
                }
            }
        }

        let Some((token, span)) = lexer.next() else {
            return;
        };
        let token = token.ok();
        tokens.push(LexedToken { token, span });
        if token != Some(Token::CurlyLeft) {
            return;
        }

        let mut brace_depth = 0usize;
        while let Some((token, span)) = lexer.next() {
            let token = token.ok();
            tokens.push(LexedToken { token, span });
            match token {
                Some(Token::FStringStart) => {
                    lex_f_string(lexer, tokens);
                }
                Some(Token::CurlyLeft) => brace_depth += 1,
                Some(Token::CurlyRight) if brace_depth == 0 => break,
                Some(Token::CurlyRight) => brace_depth -= 1,
                _ => {}
            }
        }
    }
}

/// A parsed Roto source file and the exact spans of its AST nodes.
#[derive(Clone, Debug)]
pub struct ParsedSource {
    /// File identifier stored in every source span.
    file: usize,

    /// Module identifier used by Roto's module tree.
    module: Meta<ast::Identifier>,

    /// Parsed syntax tree.
    tree: SyntaxTree,

    /// Byte spans for all metadata-wrapped AST nodes.
    spans: Spans,

    source: String,
}

impl ParsedSource {
    /// Return the file identifier stored in this source's spans.
    #[must_use]
    pub fn file(&self) -> usize {
        self.file
    }

    /// Return the metadata-wrapped module identifier.
    #[must_use]
    pub fn module(&self) -> &Meta<ast::Identifier> {
        &self.module
    }

    /// Return the source text from which this syntax tree was parsed.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Return the parsed syntax tree.
    #[must_use]
    pub fn tree(&self) -> &SyntaxTree {
        &self.tree
    }

    /// Return the byte spans for this syntax tree's metadata.
    #[must_use]
    pub fn spans(&self) -> &Spans {
        &self.spans
    }

    /// Return the parsed module name.
    #[must_use]
    pub fn module_name(&self) -> &str {
        self.module.as_str()
    }

    /// Consume this parsed source and return its coherent components.
    ///
    /// Parser integrations can use this to transfer ownership without
    /// cloning. The components cannot be assembled back into a
    /// [`ParsedSource`] through the safe public API.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (usize, Meta<ast::Identifier>, SyntaxTree, Spans, String) {
        (self.file, self.module, self.tree, self.spans, self.source)
    }
}

/// Parse one source file as the root `pkg` module.
///
/// # Errors
///
/// Returns Roto's structured parse error, including exact byte spans, labels,
/// notes, and help text.
pub fn parse(source: &str) -> ParseResult<ParsedSource> {
    parse_module("pkg", source)
}

/// Parse one named source module using file identifier zero.
///
/// # Errors
///
/// Returns Roto's structured parse error, including exact byte spans, labels,
/// notes, and help text.
pub fn parse_module(module: &str, source: &str) -> ParseResult<ParsedSource> {
    parse_file(0, module, source)
}

/// Parse one named source module using a caller-provided file identifier.
///
/// # Errors
///
/// Returns Roto's structured parse error, including exact byte spans, labels,
/// notes, and help text.
pub fn parse_file(
    file: usize,
    module: &str,
    source: &str,
) -> ParseResult<ParsedSource> {
    let mut spans = Spans::default();
    let module = spans.add(
        Span {
            file,
            start: 0,
            end: usize::from(!source.is_empty()),
        },
        ast::Identifier::from(module),
    );
    Parser::parse(file, &mut spans, source).map(|tree| ParsedSource {
        file,
        module,
        tree,
        spans,
        source: source.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Declaration, Expr};

    #[test]
    fn token_spans_are_exact_and_non_code_regions_are_opaque() {
        let block_comment = "/* outer / /* nested % */ still comment */";
        let source = format!(
            "{block_comment} // ignored / %\n\"string / %\" '/' value / 2"
        );
        let tokens = lex(&source);

        assert_eq!(
            tokens.first(),
            Some(&LexedToken {
                token: Some(Token::BlockComment(block_comment)),
                span: 0..block_comment.len(),
            })
        );

        let string_start = source.find("\"string / %\"").unwrap();
        assert!(tokens.iter().any(|token| {
            token.span
                == (string_start..string_start + "\"string / %\"".len())
                && matches!(token.token, Some(Token::String(_)))
        }));

        let char_start = source.find("'/'").unwrap();
        assert!(tokens.iter().any(|token| {
            token.span == (char_start..char_start + "'/'".len())
                && matches!(token.token, Some(Token::Char(_)))
        }));

        let operators = tokens
            .iter()
            .filter_map(|token| match token.token {
                Some(Token::Slash) => Some("/"),
                Some(Token::Percent) => Some("%"),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(operators, ["/"]);
    }

    #[test]
    fn invalid_unicode_advances_by_one_codepoint() {
        let tokens = lex("☃ ok");
        assert_eq!(tokens[0].token, None);
        assert_eq!(tokens[0].span, 0..'☃'.len_utf8());
        assert_eq!(tokens[1].token, Some(Token::Ident("ok")));
        assert_eq!(tokens[1].span, 4..6);
    }

    #[test]
    fn f_string_text_is_opaque_but_interpolations_are_tokenized() {
        let source = "f\"λ / % {value / 2}\" / 4";
        let tokens = lex(source);
        let operators = tokens
            .iter()
            .filter_map(|token| match token.token {
                Some(Token::Slash) => Some("/"),
                Some(Token::Percent) => Some("%"),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(operators, ["/", "/"]);
        assert!(tokens.iter().any(|token| {
            matches!(token.token, Some(Token::FStringText("λ / % ")))
                && &source[token.span.clone()] == "λ / % "
        }));
        assert!(tokens.iter().any(|token| {
            matches!(token.token, Some(Token::FStringEnd("\"")))
                && &source[token.span.clone()] == "\""
        }));
    }

    #[test]
    fn parsed_ast_exposes_question_mark_and_exact_spans() {
        let source = "\
fn propagate(value: Result[i32, String]) -> Result[i32, String] {
    Ok(value?)
}";
        let parsed = parse_file(7, "editor", source).unwrap();
        assert_eq!(parsed.file(), 7);
        assert_eq!(parsed.module().as_str(), "editor");
        assert_eq!(parsed.module_name(), "editor");
        assert_eq!(parsed.source(), source);

        let Declaration::Function(function) = &parsed.tree().declarations[0]
        else {
            panic!("expected a function declaration");
        };
        let expression = function
            .body
            .last
            .as_deref()
            .expect("function has a trailing expression");
        let Expr::FunctionCall(_, arguments) = &expression.node else {
            panic!("expected Ok(...)");
        };
        let Expr::QuestionMark(inner) = &arguments[0].node else {
            panic!("expected the question-mark expression");
        };

        let question = source.find("value?").unwrap();
        assert_eq!(
            parsed.spans().get(&arguments[0]).range(),
            question..question + "value?".len()
        );
        assert_eq!(
            parsed.spans().get(inner.as_ref()).range(),
            question..question + "value".len()
        );
        assert!(parsed.spans().iter().all(|(_, span)| span.file == 7));

        let (file, module, tree, spans, owned_source) = parsed.into_parts();
        assert_eq!(file, 7);
        assert_eq!(module.as_str(), "editor");
        assert_eq!(tree.declarations.len(), 1);
        assert_eq!(spans.get(&module).file, file);
        assert_eq!(owned_source, source);
    }

    #[test]
    fn parse_errors_have_structured_spans_and_labels() {
        let comment = "/* nested /* comment */ body */";
        let error = parse(&format!("{comment}\nfn main() {{}}"))
            .expect_err("block comments are not valid Roto syntax");
        let diagnostic = error.diagnostic();

        assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostic.span.range(), 0..comment.len());
        assert_eq!(diagnostic.labels[0].span, diagnostic.span);
        assert_eq!(diagnostic.labels[0].severity, DiagnosticSeverity::Error);
        assert!(diagnostic.message.contains("expected"));
    }
}
