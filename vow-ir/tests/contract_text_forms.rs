use vow_ir::{PatternAggregateMap, StringExprSet, lower_module_with_pattern_aggregates};

const FORMS_FIXTURE: &str = include_str!("../../tests/fixtures/contracts/contract_text_forms.vow");
const FORMS_EXPECTED: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_forms.expected");
const BLOCKS_FIXTURE: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_blocks.vow");
const BLOCKS_EXPECTED: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_blocks.expected");
const POSTFIX_FIXTURE: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_postfix.vow");
const POSTFIX_CANONICAL_FIXTURE: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_postfix_canonical.vow");
const POSTFIX_EXPECTED: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_postfix.expected");
const ATOMS_FIXTURE: &str = include_str!("../../tests/fixtures/contracts/contract_text_atoms.vow");
const ATOMS_EXPECTED: &str =
    include_str!("../../tests/fixtures/contracts/contract_text_atoms.expected");

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
fn leaf_and_binding_descriptions_match_the_shared_expected_text() {
    let got = escaped_descriptions(ATOMS_FIXTURE, "contract_text_atoms.vow");
    assert_eq!(got, expected_lines(ATOMS_EXPECTED));
}

#[test]
fn postfix_and_cast_descriptions_match_the_shared_expected_text() {
    let got = escaped_descriptions(POSTFIX_FIXTURE, "contract_text_postfix.vow");
    assert_eq!(got, expected_lines(POSTFIX_EXPECTED));
}

#[test]
fn canonical_postfix_spelling_prints_back_to_itself() {
    let got = escaped_descriptions(
        POSTFIX_CANONICAL_FIXTURE,
        "contract_text_postfix_canonical.vow",
    );
    assert_eq!(got, expected_lines(POSTFIX_EXPECTED));
}

/// The canonical twin is derived data: its clauses are the expected text with
/// the escapes undone. Fail when it drifts from the tricky-spelling fixture.
#[test]
fn canonical_postfix_fixture_is_derived_from_the_expected_file() {
    const MARKER: &str = "module ContractTextPostfix";
    let after_marker = |text: &str| text[text.find(MARKER).expect("module line")..].to_string();
    let mut canonical_clauses = POSTFIX_EXPECTED.lines().map(|line| {
        let mut text = String::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match (c, c == '\\') {
                (_, true) => match chars.next() {
                    Some('n') => text.push('\n'),
                    Some(other) => text.push(other),
                    None => text.push('\\'),
                },
                _ => text.push(c),
            }
        }
        let (kind, expr) = text.split_once(' ').expect("clause kind");
        format!("  {kind}: {expr}")
    });
    let mut derived = String::new();
    for line in after_marker(POSTFIX_FIXTURE).split_inclusive('\n') {
        if line.starts_with("  requires: ") {
            derived.push_str(&canonical_clauses.next().expect("clause count"));
            derived.push('\n');
        } else {
            derived.push_str(line);
        }
    }
    assert!(
        canonical_clauses.next().is_none(),
        "unused expected clauses"
    );
    assert_eq!(derived, after_marker(POSTFIX_CANONICAL_FIXTURE));
}
