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

#[test]
fn an_unannotated_collection_is_one_clear_error_per_call() {
    let message = |method: &str| {
        format!(
            "cannot infer the collection type of the receiver of `{method}`: annotate its binding with a full type"
        )
    };
    let insert = message("insert");
    let get = message("get");
    let push = message("push");
    assert_diagnostics(
        "hashmap_new_unannotated.vow",
        &[("TypeMismatch", &insert), ("TypeMismatch", &get)],
    );
    assert_diagnostics("btreemap_new_unannotated.vow", &[("TypeMismatch", &insert)]);
    assert_diagnostics("vec_new_unannotated.vow", &[("TypeMismatch", &push)]);
}
