#[test]
fn roto_enum_rejects_custom_drop() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/roto_enum_custom_drop.rs");
}

#[test]
fn fresh_function_rejects_non_unit_returns() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fresh_function_non_unit.rs");
}
