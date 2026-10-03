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

// The `// TEST:` directives in tests/error fixtures pin the error code, the error
// count and a stderr substring. These tests pin what they cannot: the full message
// text, which the self-hosted checker's unit tests assert verbatim, and the order
// the diagnostics come out in.

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
