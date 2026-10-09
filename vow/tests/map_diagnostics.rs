use std::path::{Path, PathBuf};
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/error")
        .join(name)
}

fn error_diagnostics(name: &str) -> Vec<(String, String)> {
    let dir = tempfile::TempDir::new().unwrap();
    let output = dir.path().join("out");
    let command_output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            fixture(name).to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&command_output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("invalid JSON from build: {error}\nstdout: {stdout}"));
    json["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|diagnostic| diagnostic["severity"] == "error")
        .map(|diagnostic| {
            (
                diagnostic["error_code"].as_str().unwrap_or("").to_string(),
                diagnostic["message"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

fn assert_diagnostics(name: &str, expected: &[(&str, &str)]) {
    let actual = error_diagnostics(name);
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(code, message)| (code.to_string(), message.to_string()))
        .collect();
    assert_eq!(actual, expected, "{name}");
}

// Only `tests/run_tests.sh` reads the `// TEST: error-code` and `error-count`
// directives, and no workflow runs it; CI's parity run compares the two compilers'
// error-code multisets but not absolute counts. This test is the CI check that the
// Rust compiler reports exactly the pinned code and count for every map fixture,
// which together with parity pins the self-hosted compiler too.
const DIRECTIVE_FIXTURES: &[&str] = &[
    "map_bad_type_once_per_site.vow",
    "hashmap_key_string_reported_once.vow",
    "btreemap_key_string_reported_once.vow",
    "hashmap_key_unsupported.vow",
    "map_value_unsupported.vow",
    "hashmap_value_linear.vow",
    "map_value_linear_forward_ref.vow",
    "hashmap_new_unannotated.vow",
    "btreemap_new_unannotated.vow",
    "vec_new_unannotated.vow",
    "index_non_indexable.vow",
    "index_hashmap_vec_key.vow",
    "vec_element_linear.vow",
    "vec_element_linear_nested.vow",
    "vec_element_linear_forward_ref.vow",
    "unit_param.vow",
    "unit_mismatch.vow",
];

fn directive(source: &str, key: &str) -> Option<String> {
    let prefix = format!("// TEST: {key} ");
    source
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(str::to_string)
}

#[test]
fn map_fixtures_report_their_directive_code_and_count() {
    for name in DIRECTIVE_FIXTURES {
        let source = std::fs::read_to_string(fixture(name)).unwrap();
        let diagnostics = error_diagnostics(name);
        let count: usize = directive(&source, "error-count")
            .unwrap_or_else(|| panic!("{name} has no error-count directive"))
            .parse()
            .unwrap();
        assert_eq!(diagnostics.len(), count, "{name}: {diagnostics:?}");
        if let Some(code) = directive(&source, "error-code") {
            assert!(
                diagnostics.iter().any(|(actual, _)| *actual == code),
                "{name}: expected {code} in {diagnostics:?}"
            );
        }
    }
}

// The directives also pin a stderr substring. These tests pin what they cannot: the
// full message text, which the self-hosted checker's unit tests assert verbatim, and
// the order the diagnostics come out in.

const KEY_STRING: &str = "HashMap key type `String` is not supported: keys are compared by value as a single machine word";

fn not_indexable(ty: &str) -> String {
    format!("index operation on non-indexable type `{ty}`")
}

#[test]
fn an_unannotated_collection_error_names_the_method_and_the_fix() {
    let message = |method: &str| {
        format!(
            "cannot infer the collection type of the receiver of `{method}`: annotate its binding with a full type"
        )
    };
    assert_diagnostics(
        "hashmap_new_unannotated.vow",
        &[
            ("TypeMismatch", &message("insert")),
            ("TypeMismatch", &message("get")),
        ],
    );
}

#[test]
fn a_bad_map_key_message_prints_the_full_type() {
    assert_diagnostics(
        "hashmap_key_string_reported_once.vow",
        &[("UnsupportedFeature", KEY_STRING)],
    );
    let keys: Vec<String> = error_diagnostics("hashmap_key_unsupported.vow")
        .into_iter()
        .map(|(_, message)| message)
        .collect();
    assert!(
        keys.iter().any(|message| message.contains("`(i64, i64)`")),
        "{keys:?}"
    );
}

#[test]
fn non_indexable_messages_print_the_full_type() {
    assert_diagnostics(
        "index_non_indexable.vow",
        &[
            ("TypeMismatch", &not_indexable("HashMap<i64, i64>")),
            ("TypeMismatch", &not_indexable("HashMap<i64, i64>")),
            ("TypeMismatch", &not_indexable("BTreeMap<i64, i64>")),
            ("TypeMismatch", &not_indexable("String")),
        ],
    );
}

#[test]
fn a_bad_key_type_is_reported_before_the_index_that_uses_it() {
    assert_diagnostics(
        "index_hashmap_vec_key.vow",
        &[
            (
                "UnsupportedFeature",
                "HashMap key type `Vec<i64>` is not supported: keys are compared by value as a single machine word",
            ),
            ("TypeMismatch", &not_indexable("HashMap<Vec<i64>, i64>")),
        ],
    );
}
