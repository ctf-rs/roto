//! Module tree of a Roto script

use std::collections::BTreeMap;

use crate::{
    FileTree, RotoError, RotoReport,
    ast::{self, Identifier},
    parser::{
        Parser,
        meta::{Meta, Span, Spans},
    },
};

pub struct Parsed {
    pub module_tree: ModuleTree,
    pub file_tree: FileTree,
    pub spans: Spans,
}

#[derive(Default)]
pub struct ModuleTree {
    pub modules: Vec<Module>,
}

#[derive(Clone, Copy)]
pub struct ModuleRef(pub usize);

pub struct Module {
    pub ident: Meta<Identifier>,
    pub ast: ast::SyntaxTree,
    pub children: BTreeMap<Identifier, ModuleRef>,
    pub parent: Option<ModuleRef>,
}

impl FileTree {
    /// Parse the files in the [`FileTree`] returning the AST.
    pub fn parse(self) -> Result<Parsed, RotoReport> {
        Parsed::from_files(self)
    }
}

impl Parsed {
    pub(crate) fn from_single_parsed(
        file_tree: FileTree,
        parsed: roto_syntax::ParsedSource,
    ) -> Result<Self, RotoReport> {
        let (parsed_file, module, tree, spans, source) = parsed.into_parts();
        let matches_file = match file_tree.files.as_slice() {
            [file] => {
                file.module_name == module.as_str()
                    && file.contents == source
                    && file.children.is_empty()
            }
            _ => false,
        };

        if parsed_file != 0 || !matches_file {
            return Err(RotoReport {
                files: file_tree.files,
                errors: vec![RotoError::Custom(
                    "parsed source does not match the single-file FileTree"
                        .into(),
                )],
                spans,
            });
        }

        let module = Module {
            ident: module,
            ast: tree,
            children: BTreeMap::new(),
            parent: None,
        };
        Ok(Self {
            module_tree: ModuleTree {
                modules: vec![module],
            },
            file_tree,
            spans,
        })
    }

    fn from_files(file_tree: FileTree) -> Result<Self, RotoReport> {
        let mut file_to_mod = BTreeMap::new();
        let mut modules = Vec::new();
        let mut spans = Spans::default();
        let mut errors = Vec::new();

        // First add all modules to the tree
        for (i, file) in file_tree.files.iter().enumerate() {
            let ident: Identifier = (&file.module_name).into();
            let ident = spans.add(
                Span {
                    file: i,
                    start: 0,
                    end: 1,
                },
                ident,
            );

            let ast = match Parser::parse(i, &mut spans, &file.contents) {
                Ok(ast) => ast,
                Err(err) => {
                    errors.push(RotoError::Parse(*err));
                    continue;
                }
            };

            file_to_mod.insert(i, modules.len());

            modules.push(Module {
                ident,
                children: BTreeMap::new(),
                parent: None,
                ast,
            })
        }

        if !errors.is_empty() {
            return Err(RotoReport {
                files: file_tree.files,
                errors,
                spans,
            });
        }

        // Then wire up all the relations between the modules
        for (parent, file) in file_tree.files.iter().enumerate() {
            for child in &file.children {
                let child_module = &mut modules[*child];
                child_module.parent = Some(ModuleRef(parent));
                let child_ident = *child_module.ident;
                modules[parent]
                    .children
                    .insert(child_ident, ModuleRef(*child));
            }
        }

        Ok(Self {
            module_tree: ModuleTree { modules },
            file_tree,
            spans,
        })
    }
}
