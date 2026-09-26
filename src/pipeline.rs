//! Compiler pipeline that executes multiple compiler stages in sequence

use std::{
    collections::{HashMap, VecDeque},
    fmt,
};

use crate::{
    codegen::{
        self, Module, TypedFunc,
        check::{FunctionRetrievalError, RotoFunc},
    },
    file_tree::SourceFile,
    label::LabelStore,
    lir::{
        self,
        eval::{self, Memory},
        value::IrValue,
    },
    mir,
    module::{ModuleTree, Parsed},
    parser::{
        ParseError,
        meta::{Span, Spans},
    },
    runtime::{
        Ctx, NoCtx, OptCtx, Runtime, RuntimeFunctionRef, context::Context,
    },
    typechecker::{
        error::{Level, TypeError},
        info::TypeInfo,
        scope::ResolvedName,
    },
};

/// Severity of a structured Roto compiler diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotoDiagnosticSeverity {
    /// An error that prevents successful compilation.
    Error,

    /// A warning or secondary suggestion.
    Warning,

    /// Related explanatory information.
    Info,
}

/// Alternative name for diagnostic label severity.
pub type RotoDiagnosticLevel = RotoDiagnosticSeverity;

/// One source label attached to a structured Roto diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotoDiagnosticLabel {
    /// Index into [`RotoReport::files`].
    pub file: usize,

    /// Exact byte range within that source file.
    pub range: std::ops::Range<usize>,

    /// Label severity.
    pub level: RotoDiagnosticSeverity,

    /// Human-readable label text.
    pub message: String,
}

/// A compiler diagnostic independent of its rendered terminal format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotoDiagnostic {
    /// Diagnostic severity.
    pub severity: RotoDiagnosticSeverity,

    /// Human-readable primary message.
    pub message: String,

    /// Exact primary source span, when this diagnostic is source-related.
    pub span: Option<Span>,

    /// Primary and related source labels.
    pub labels: Vec<RotoDiagnosticLabel>,

    /// Additional explanatory notes.
    pub notes: Vec<String>,

    /// Suggested ways to resolve the diagnostic.
    pub help: Vec<String>,
}

#[cfg(feature = "logger")]
use crate::ir_printer::{IrPrinter, Printable};

#[cfg(feature = "logger")]
use log::info;

/// An error from a compilation of a Roto script.
#[derive(Debug)]
pub(crate) enum RotoError {
    Read(String, std::io::Error),
    Parse(ParseError),
    Type(TypeError),
    TestsFailed(),
    CouldNotRetrieveFunction(FunctionRetrievalError),
    Custom(String),
}

/// An error report containing a set of Roto errors.
///
/// The report can be printed with the regular [`std::fmt::Display`].
#[derive(Default)]
pub struct RotoReport {
    /// Files that were used during compilation.
    ///
    /// These are used to print diagnostics.
    pub files: Vec<SourceFile>,
    pub(crate) errors: Vec<RotoError>,

    /// Spans of AST nodes.
    pub spans: Spans,
}

/// Compiler stage: loaded, parsed and type checked
pub struct TypeChecked<'r, Ctx: OptCtx> {
    module_tree: ModuleTree,
    type_info: TypeInfo,
    order: Vec<ResolvedName>,
    runtime: &'r Runtime<Ctx>,
}

/// Compiler stage: MIR
pub struct LoweredToMir<'r, Ctx: OptCtx> {
    runtime: &'r Runtime<Ctx>,
    ir: mir::Mir,
    label_store: LabelStore,
    type_info: TypeInfo,
}

/// Compiler stage: LIR
pub struct LoweredToLir<'r, Ctx: OptCtx> {
    runtime: &'r Runtime<Ctx>,
    ir: lir::Lir,
    runtime_functions: HashMap<RuntimeFunctionRef, lir::Signature>,
    label_store: LabelStore,
    type_info: TypeInfo,
}

/// The final compiled package of script.
///
/// Functions can be extracted from this package using [`Package::get_function`].
pub struct Package<Ctx: OptCtx> {
    module: Module<Ctx>,
}

impl RotoReport {
    /// Return structured compiler diagnostics with exact byte ranges.
    ///
    /// These diagnostics are independent of the terminal-oriented
    /// [`Display`](std::fmt::Display) rendering and are suitable for language
    /// servers and other editor tooling.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<RotoDiagnostic> {
        self.errors
            .iter()
            .map(|error| match error {
                RotoError::Read(name, io) => RotoDiagnostic {
                    severity: RotoDiagnosticSeverity::Error,
                    message: format!("Could not read file `{name}`: {io}"),
                    span: None,
                    labels: Vec::new(),
                    notes: Vec::new(),
                    help: Vec::new(),
                },
                RotoError::Parse(error) => {
                    let diagnostic = error.diagnostic();
                    RotoDiagnostic {
                        severity: RotoDiagnosticSeverity::Error,
                        message: format!(
                            "Parse error: {}",
                            diagnostic.message
                        ),
                        span: Some(diagnostic.span),
                        labels: diagnostic
                            .labels
                            .into_iter()
                            .map(|label| RotoDiagnosticLabel {
                                file: label.span.file,
                                range: label.span.range(),
                                level: match label.severity {
                                    roto_syntax::DiagnosticSeverity::Error => {
                                        RotoDiagnosticSeverity::Error
                                    }
                                    roto_syntax::DiagnosticSeverity::Warning => {
                                        RotoDiagnosticSeverity::Warning
                                    }
                                    roto_syntax::DiagnosticSeverity::Info => {
                                        RotoDiagnosticSeverity::Info
                                    }
                                },
                                message: label.message,
                            })
                            .collect(),
                        notes: diagnostic.notes,
                        help: diagnostic.help,
                    }
                }
                RotoError::Type(error) => {
                    let span = self.spans.get(error.location);
                    RotoDiagnostic {
                        severity: RotoDiagnosticSeverity::Error,
                        message: format!(
                            "Type error: {}",
                            error.description
                        ),
                        span: Some(span),
                        labels: error
                            .labels
                            .iter()
                            .map(|label| {
                                let span = self.spans.get(label.id);
                                RotoDiagnosticLabel {
                                    file: span.file,
                                    range: span.range(),
                                    level: match label.level {
                                        Level::Error => {
                                            RotoDiagnosticSeverity::Error
                                        }
                                        Level::Info => {
                                            RotoDiagnosticSeverity::Info
                                        }
                                    },
                                    message: label.message.clone(),
                                }
                            })
                            .collect(),
                        notes: error.notes.clone(),
                        help: Vec::new(),
                    }
                }
                RotoError::TestsFailed() => RotoDiagnostic {
                    severity: RotoDiagnosticSeverity::Error,
                    message: "Tests failed".into(),
                    span: None,
                    labels: Vec::new(),
                    notes: Vec::new(),
                    help: Vec::new(),
                },
                RotoError::CouldNotRetrieveFunction(error) => {
                    RotoDiagnostic {
                        severity: RotoDiagnosticSeverity::Error,
                        message: format!(
                            "Could not retrieve function: {error}"
                        ),
                        span: None,
                        labels: Vec::new(),
                        notes: Vec::new(),
                        help: Vec::new(),
                    }
                }
                RotoError::Custom(message) => RotoDiagnostic {
                    severity: RotoDiagnosticSeverity::Error,
                    message: message.clone(),
                    span: None,
                    labels: Vec::new(),
                    notes: Vec::new(),
                    help: Vec::new(),
                },
            })
            .collect()
    }

    /// Write this report to a type implementing [`fmt::Write`].
    pub fn write(&self, mut f: impl fmt::Write, color: bool) -> fmt::Result {
        use ariadne::{Color, Label, Report, ReportKind};

        let sources = self
            .files
            .iter()
            .map(|s| {
                (
                    s.name(),
                    ariadne::Source::from(s.contents.clone())
                        .with_display_line_offset(s.location_offset),
                )
            })
            .collect();

        let mut file_cache = ariadne::FnCache::new(
            (move |id| {
                Err(Box::new(format!("Failed to fetch source '{}'", id)))
            })
                as fn(&_) -> Result<_, Box<dyn std::fmt::Debug + 'static>>,
        )
        .with_sources(sources);

        let config = ariadne::Config::new().with_color(color);

        for diagnostic in self.diagnostics() {
            let Some(span) = diagnostic.span else {
                write!(f, "{}", diagnostic.message)?;
                continue;
            };

            let file = self.filename(span);
            let file_text = &self.files[span.file].contents;
            let labels = diagnostic.labels.iter().map(|label| {
                let span = Span::new(label.file, label.range.clone());
                Label::new((
                    self.filename(span),
                    span.character_range(&self.files[span.file].contents),
                ))
                .with_message(&label.message)
                .with_color(match label.level {
                    RotoDiagnosticSeverity::Error => Color::Red,
                    RotoDiagnosticSeverity::Warning => Color::Yellow,
                    RotoDiagnosticSeverity::Info => Color::Blue,
                })
            });

            let mut report = Report::build(
                ReportKind::Error,
                (file, span.character_range(file_text)),
            )
            .with_config(config)
            .with_message(&diagnostic.message)
            .with_labels(labels);

            report.with_notes(&diagnostic.notes);
            for help in diagnostic.help {
                report = report.with_help(help);
            }

            let mut output = Vec::new();
            report.finish().write(&mut file_cache, &mut output).unwrap();
            write!(f, "{}", String::from_utf8_lossy(&output))?;
        }

        Ok(())
    }
}

impl std::fmt::Display for RotoReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write(f, true)
    }
}

impl std::fmt::Debug for RotoReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, true)
    }
}

impl RotoReport {
    fn filename(&self, s: Span) -> String {
        self.files[s.file].name()
    }
}

impl std::error::Error for RotoReport {}

#[cfg(all(test, not(miri)))]
macro_rules! src {
    ($code:expr) => {
        $crate::FileTree::test_file(file!(), $code, line!() as usize - 1)
    };
}

#[cfg(all(test, not(miri)))]
pub(crate) use src;

#[cfg(all(test, not(miri)))]
macro_rules! source_file {
    ($module_name:literal, $code:literal) => {
        $crate::SourceFile {
            name: file!().into(),
            module_name: $module_name.into(),
            contents: $code.into(),
            location_offset: line!() as usize - 1,
            children: Vec::new(),
        }
    };
}

#[cfg(all(test, not(miri)))]
pub(crate) use source_file;

impl Parsed {
    pub fn typecheck<'r, Ctx: OptCtx>(
        self,
        runtime: &'r Runtime<Ctx>,
    ) -> Result<TypeChecked<'r, Ctx>, RotoReport> {
        let Parsed {
            file_tree,
            module_tree,
            spans,
        } = self;

        let result = crate::typechecker::typecheck(&runtime.rt, &module_tree);

        let (type_info, order) = match result {
            Ok(type_info) => type_info,
            Err(error) => {
                return Err(RotoReport {
                    files: file_tree.files,
                    errors: vec![RotoError::Type(error)],
                    spans,
                });
            }
        };

        Ok(TypeChecked {
            module_tree,
            type_info,
            order,
            runtime,
        })
    }
}

impl<'r, Ctx: OptCtx> TypeChecked<'r, Ctx> {
    pub fn lower_to_mir(&self) -> LoweredToMir<'r, Ctx> {
        let TypeChecked {
            module_tree,
            type_info,
            order,
            runtime,
        } = self;

        let mut type_info = type_info.clone();
        let mut label_store = LabelStore::default();
        let ir = mir::lower_to_mir(
            module_tree,
            &runtime.rt,
            &mut type_info,
            &mut label_store,
            order,
        );

        #[cfg(feature = "logger")]
        {
            if log::log_enabled!(log::Level::Info) {
                let printer = IrPrinter {
                    type_info: &type_info,
                    label_store: &label_store,
                    scope: None,
                };
                let s = ir.print(&printer);
                info!("\n{s}");
            }
        }

        LoweredToMir {
            ir,
            runtime,
            label_store,
            type_info,
        }
    }
}

impl<'r, Ctx: OptCtx> LoweredToMir<'r, Ctx> {
    pub fn lower_to_lir(self) -> LoweredToLir<'r, Ctx> {
        let LoweredToMir {
            runtime,
            ir,
            mut label_store,
            mut type_info,
        } = self;

        let mut runtime_functions = HashMap::new();
        let mut ctx = lir::lower::LowerCtx {
            runtime: &runtime.rt,
            type_info: &mut type_info,
            label_store: &mut label_store,
            runtime_functions: &mut runtime_functions,
            drops_to_generate: VecDeque::new(),
            clones_to_generate: VecDeque::new(),
            eq_to_generate: VecDeque::new(),
        };
        let ir = lir::lower_to_lir(&mut ctx, ir);

        #[cfg(feature = "logger")]
        {
            if log::log_enabled!(log::Level::Info) {
                let printer = IrPrinter {
                    type_info: &type_info,
                    label_store: &label_store,
                    scope: None,
                };
                let s = ir.print(&printer);
                info!("\n{s}");
            }
        }

        LoweredToLir {
            runtime,
            ir,
            label_store,
            type_info,
            runtime_functions,
        }
    }
}

impl<Ctx: OptCtx> LoweredToLir<'_, Ctx> {
    pub fn eval(
        &self,
        mem: &mut Memory,
        ctx: IrValue,
        args: Vec<IrValue>,
    ) -> Option<IrValue> {
        eval::eval(
            &self.runtime.rt,
            &self.ir.functions,
            &self.type_info.compiled_constants,
            "main",
            mem,
            ctx,
            args,
        )
    }

    pub fn codegen(self) -> Package<Ctx> {
        let module = codegen::codegen(
            self.runtime,
            &self.ir.functions,
            &self.runtime_functions,
            self.label_store,
            self.type_info,
        );
        Package { module }
    }
}

impl<Ctx: OptCtx> Package<Ctx> {
    /// Return an iterator with all the tests in this [`Package`].
    pub fn get_tests(
        &mut self,
    ) -> impl Iterator<Item = codegen::testing::TestCase<Ctx>> + use<'_, Ctx>
    {
        codegen::testing::get_tests(&mut self.module)
    }

    /// Get a function from this [`Package`].
    pub fn get_function<F: RotoFunc>(
        &mut self,
        name: &str,
    ) -> Result<TypedFunc<Ctx, F>, FunctionRetrievalError> {
        self.module.get_function(name)
    }
}

impl Package<NoCtx> {
    /// Run all tests in this [`Package`] without context.
    #[allow(clippy::result_unit_err)]
    pub fn run_tests(&mut self) -> Result<(), ()> {
        codegen::testing::run_tests(&mut self.module, NoCtx)
    }
}

impl<C: Context> Package<Ctx<C>> {
    /// Run all tests in this [`Package`] with context.
    #[allow(clippy::result_unit_err)]
    pub fn run_tests(&mut self, ctx: C) -> Result<(), ()> {
        codegen::testing::run_tests(&mut self.module, Ctx(ctx))
    }
}

#[cfg(all(test, not(miri)))]
mod regex_tests;
