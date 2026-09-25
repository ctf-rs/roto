#[test]
fn roto_enum_rejects_custom_drop() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/roto_enum_custom_drop.rs");
}
