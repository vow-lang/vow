use vow_ir::{PatternAggregateMap, StringExprSet, lower_module_with_pattern_aggregates};

const FORMS_FIXTURE: &str = include_str!("../../tests/fixtures/contracts/contract_text_forms.vow");
const FORMS_EXPECTED: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_forms.expected");
const BLOCKS_FIXTURE: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_blocks.vow");
const BLOCKS_EXPECTED: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_blocks.expected");

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

/// The `.expected` files hold one line per clause with `\` written as `\\` and a
/// newline as `\n`, so a multi-line description stays on one line.
fn escape(description: &str) -> String {
    description.replace('\\', "\\\\").replace('\n', "\\n")
}

fn escaped_descriptions(source: &str, file: &str) -> Vec<String> {
    descriptions(source, file)
        .iter()
        .map(|d| escape(d))
        .collect()
}

fn expected_lines(expected: &str) -> Vec<String> {
    expected.lines().map(str::to_string).collect()
}

#[test]
fn contract_descriptions_match_the_shared_expected_text() {
    let got = escaped_descriptions(FORMS_FIXTURE, "contract_text_forms.vow");
    assert_eq!(got, expected_lines(FORMS_EXPECTED));
}

#[test]
fn compound_expression_descriptions_match_the_shared_expected_text() {
    let got = escaped_descriptions(BLOCKS_FIXTURE, "contract_text_blocks.vow");
    assert_eq!(got, expected_lines(BLOCKS_EXPECTED));
}

#[test]
fn u64_literal_above_i64_max_prints_unsigned() {
    let got = descriptions(
        "module U\nfn f(x: u64) -> u64 vow {\n  ensures: result == 18446744073709551614\n} { x }\n",
        "u.vow",
    );
    assert_eq!(got, ["ensures result == 18446744073709551614"]);
}
