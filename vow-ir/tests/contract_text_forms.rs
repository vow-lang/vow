use vow_ir::{PatternAggregateMap, StringExprSet, lower_module_with_pattern_aggregates};

const FIXTURE: &str = include_str!("../../tests/fixtures/contracts/contract_text_forms.vow");
const EXPECTED: &str = include_str!("../../tests/fixtures/contracts/contract_text_forms.expected");

fn descriptions(source: &str, file: &str) -> Vec<String> {
    let (ast, diagnostics) = vow_syntax::parser::parse_module(source, file);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let item_files = vec![file.to_string(); ast.items.len()];
    let module = lower_module_with_pattern_aggregates(
        &ast,
        &item_files,
        &StringExprSet::new(),
        PatternAggregateMap::new(),
    );
    module
        .functions
        .iter()
        .flat_map(|f| f.vows.iter().map(|v| v.description.clone()))
        .collect()
}

#[test]
fn contract_descriptions_match_the_shared_expected_text() {
    let got = descriptions(FIXTURE, "contract_text_forms.vow");
    let want: Vec<String> = EXPECTED.lines().map(str::to_string).collect();
    assert_eq!(got, want);
}

#[test]
fn u64_literal_above_i64_max_prints_unsigned() {
    let got = descriptions(
        "module U\nfn f(x: u64) -> u64 vow {\n  ensures: result == 18446744073709551614\n} { x }\n",
        "u.vow",
    );
    assert_eq!(got, ["ensures result == 18446744073709551614"]);
}
