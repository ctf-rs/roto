use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::{
    FileTree, Function, RegexLiteral, RotoString, Type, Val,
    ast::Literal,
    ir_printer::{IrPrinter, Printable},
    location,
    parser::Parser,
};

#[test]
fn regex_literals_lower_to_constants_without_runtime_constructors() {
    let builds = Arc::new(AtomicUsize::new(0));
    let counter = builds.clone();
    let mut runtime = Runtime::new();
    let functions_before = runtime.functions().len();
    runtime
        .add(RegexLiteral::new(
            move |pattern| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(RotoString::from(pattern))
            },
            location!(),
        ))
        .unwrap();
    assert_eq!(runtime.functions().len(), functions_before);

    let checked = FileTree::test_file(
        "regex.roto",
        r###"
            const DIRECT: String = r"\d";
            const ALIAS: String = DIRECT;
            const NESTED: Option[String] = Some(r#"a"b"#);
            fn direct() -> String { ALIAS }
            fn nested() -> Option[String] { NESTED }
        "###,
        0,
    )
    .parse()
    .unwrap()
    .typecheck(&runtime)
    .unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert_eq!(checked.type_info.compiled_constants.len(), 2);
    assert_eq!(checked.type_info.regex_literals.len(), 2);
    let mir = checked.lower_to_mir();
    assert!(!mir.ir.items.iter().any(|item| {
        matches!(item.ty, mir::ItemKind::Constant { .. })
            && item.name.as_str() == "pkg.DIRECT"
    }));
    let instructions: Vec<_> = mir
        .ir
        .items
        .iter()
        .flat_map(|item| &item.blocks)
        .flat_map(|block| &block.instructions)
        .collect();
    assert!(instructions.iter().any(|instruction| matches!(
        instruction,
        mir::Instruction::Assign {
            value: mir::Value::Constant(..),
            ..
        }
    )));
    assert!(!instructions.iter().any(|instruction| matches!(
        instruction,
        mir::Instruction::Assign {
            value: mir::Value::Const(
                Literal::Regex(_) | Literal::String(_),
                _
            ) | mir::Value::CallRuntime { .. },
            ..
        }
    )));

    let lir = mir.lower_to_lir();
    assert!(lir.runtime_functions.is_empty());
    let instructions: Vec<_> = lir
        .ir
        .functions
        .iter()
        .flat_map(|item| &item.blocks)
        .flat_map(|block| &block.instructions)
        .collect();
    assert!(instructions.iter().any(|instruction| matches!(
        instruction,
        lir::Instruction::ConstantAddress { .. }
    )));
    assert!(!instructions.iter().any(|instruction| matches!(
        instruction,
        lir::Instruction::InitString { .. }
            | lir::Instruction::CallRuntime { .. }
    )));

    let mut package = lir.codegen();
    let direct = package
        .get_function::<fn() -> RotoString>("direct")
        .unwrap();
    let nested = package
        .get_function::<fn() -> Option<RotoString>>("nested")
        .unwrap();
    for _ in 0..1_000 {
        assert_eq!(direct.clone().call(), RotoString::from(r"\d"));
        assert_eq!(nested.clone().call(), Some(RotoString::from("a\"b")));
    }
    assert_eq!(builds.load(Ordering::SeqCst), 2);
}

#[test]
fn regex_interpreter_loads_direct_and_nested_compiled_constants() {
    let builds = Arc::new(AtomicUsize::new(0));
    let counter = builds.clone();
    let mut runtime = Runtime::new();
    runtime
        .add(RegexLiteral::new(
            move |pattern| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(pattern.len() as i32)
            },
            location!(),
        ))
        .unwrap();
    let lir = FileTree::test_file(
        "regex.roto",
        r#"const DIRECT: i32 = r"abc";
           const ALIAS: i32 = DIRECT;
           const NESTED: Option[i32] = Some(r"abcde");
           fn main() -> i32 {
               match NESTED {
                   Some(value) => ALIAS + value,
                   None => 0,
               }
           }"#,
        0,
    )
    .parse()
    .unwrap()
    .typecheck(&runtime)
    .unwrap()
    .lower_to_mir()
    .lower_to_lir();
    for _ in 0..3 {
        let mut memory = Memory::new();
        let context = IrValue::Pointer(memory.allocate(1));
        assert_eq!(
            lir.eval(&mut memory, context, vec![]),
            Some(IrValue::I32(8))
        );
    }
    assert_eq!(builds.load(Ordering::SeqCst), 2);
}

#[derive(Clone, Debug)]
struct TrackedPattern(Arc<TrackedData>);

#[derive(Debug)]
struct TrackedData {
    len: u64,
    drops: Arc<AtomicUsize>,
}

impl PartialEq for TrackedPattern {
    fn eq(&self, other: &Self) -> bool {
        self.0.len == other.0.len
    }
}

impl Drop for TrackedData {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn regex_interpreter_drops_owned_values_and_keeps_the_compiled_pool() {
    let builds = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let mut runtime = Runtime::new();
    runtime
        .add(
            Type::clone::<Val<TrackedPattern>>("Pattern", "", location!())
                .unwrap(),
        )
        .unwrap();
    let counter = builds.clone();
    let dropped = drops.clone();
    runtime
        .add(RegexLiteral::new(
            move |pattern| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Val(TrackedPattern(Arc::new(TrackedData {
                    len: pattern.len() as u64,
                    drops: dropped.clone(),
                }))))
            },
            location!(),
        ))
        .unwrap();
    runtime
        .add(
            Function::new(
                "pattern_len",
                "",
                vec!["pattern"],
                |pattern: Val<TrackedPattern>| pattern.0.0.len,
                location!(),
            )
            .unwrap(),
        )
        .unwrap();
    let lir = FileTree::test_file(
        "regex.roto",
        r#"const DIRECT: Pattern = r"abc";
           const ALIAS: Pattern = DIRECT;
           const NESTED: Option[Pattern] = Some(r"abcde");
           fn main() -> u64 {
               match NESTED {
                   Some(pattern) => pattern_len(pattern) + pattern_len(ALIAS),
                   None => 0,
               }
           }"#,
        0,
    )
    .parse()
    .unwrap()
    .typecheck(&runtime)
    .unwrap()
    .lower_to_mir()
    .lower_to_lir();
    for _ in 0..10 {
        let mut memory = Memory::new();
        let context = IrValue::Pointer(memory.allocate(1));
        let result = lir.eval(&mut memory, context, vec![]);
        assert!(matches!(result, Some(IrValue::U64(8))), "{result:?}");
        assert_eq!(drops.load(Ordering::SeqCst), 0);
    }
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    drop(lir);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn regex_literal_printing_roundtrips_raw_patterns() {
    let type_info = TypeInfo::new();
    let labels = LabelStore::default();
    let printer = IrPrinter {
        type_info: &type_info,
        label_store: &labels,
        scope: None,
    };
    for pattern in ["", r"\d", r"\\d", "a\"b", "a\"###b", "λ\n雪"] {
        let printed = Literal::Regex(pattern.into()).print(&printer);
        let parsed = Parser::run_parser(
            Parser::expr,
            0,
            &mut Spans::default(),
            &printed,
        )
        .unwrap();
        let crate::ast::Expr::Literal(literal) = parsed.node else {
            panic!("expected a literal");
        };
        assert!(
            matches!(literal.node, Literal::Regex(body) if body == pattern)
        );
    }
}
