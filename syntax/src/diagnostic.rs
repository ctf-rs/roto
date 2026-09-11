//! Structured syntax diagnostics.

use crate::{Hint, ParseError, Span};

/// Severity of a syntax diagnostic or one of its labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    /// An error that prevents parsing.
    Error,

    /// A secondary suggestion associated with an error.
    Warning,

    /// Related explanatory information.
    Info,
}

/// A message attached to an exact source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticLabel {
    /// Label severity.
    pub severity: DiagnosticSeverity,

    /// Exact byte span in the source file.
    pub span: Span,

    /// Human-readable label text.
    pub message: String,
}

/// A structured parse diagnostic independent of terminal rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Diagnostic severity.
    pub severity: DiagnosticSeverity,

    /// Human-readable primary message.
    pub message: String,

    /// Exact primary byte span in the source file.
    pub span: Span,

    /// Primary and related source labels.
    pub labels: Vec<DiagnosticLabel>,

    /// Additional explanatory notes.
    pub notes: Vec<String>,

    /// Suggested ways to resolve the diagnostic.
    pub help: Vec<String>,
}

impl ParseError {
    /// Convert this parser error into a presentation-independent diagnostic.
    #[must_use]
    pub fn diagnostic(&self) -> Diagnostic {
        let mut labels = vec![DiagnosticLabel {
            severity: DiagnosticSeverity::Error,
            span: self.location,
            message: self.kind.label(),
        }];
        labels.extend(self.hints.iter().map(|Hint { location, text }| {
            DiagnosticLabel {
                severity: DiagnosticSeverity::Warning,
                span: *location,
                message: text.clone(),
            }
        }));

        Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: self.to_string(),
            span: self.location,
            labels,
            notes: self.note.iter().cloned().collect(),
            help: self.kind.hint().into_iter().collect(),
        }
    }
}
