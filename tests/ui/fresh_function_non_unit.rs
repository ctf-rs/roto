use roto::{FileTree, List, Runtime};

fn main() {
    let runtime = Runtime::new();
    let package = FileTree::test_file(
        "non-unit.roto",
        r#"
            const VALUES: List[String] = ["retained"];
            fn main() -> List[String] { VALUES }
        "#,
        0,
    )
    .compile(&runtime)
    .unwrap();
    let _function = package
        .into_fresh_function::<fn() -> List<roto::RotoString>>("main")
        .unwrap();
}
