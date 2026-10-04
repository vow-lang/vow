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

fn error_messages(name: &str) -> Vec<(String, String, Vec<String>)> {
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
            let hints = diagnostic["hints"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|hint| hint.as_str().unwrap_or("").to_string())
                .collect();
            (
                diagnostic["error_code"].as_str().unwrap_or("").to_string(),
                diagnostic["message"].as_str().unwrap_or("").to_string(),
                hints,
            )
        })
        .collect()
}

const SLICE_MESSAGE: &str = "slice types (`[T]`) are not supported in Vow";
const SLICE_HINT: &str = "use `Vec<T>` to hold a sequence of values";
const RESERVED_HINT: &str = "choose a different name for this type";

fn assert_all_slice_diagnostics(name: &str, count: usize) {
    let diagnostics = error_messages(name);
    assert_eq!(diagnostics.len(), count, "{name}: {diagnostics:?}");
    for (code, message, hints) in diagnostics {
        assert_eq!(code, "UnsupportedFeature", "{name}");
        assert_eq!(message, SLICE_MESSAGE, "{name}");
        assert_eq!(hints, vec![SLICE_HINT.to_string()], "{name}");
    }
}

fn assert_reserved_diagnostics(name: &str, reserved: &[&str]) {
    let diagnostics = error_messages(name);
    assert_eq!(diagnostics.len(), reserved.len(), "{name}: {diagnostics:?}");
    for ((code, message, hints), builtin) in diagnostics.into_iter().zip(reserved) {
        assert_eq!(code, "UnsupportedFeature", "{name}");
        assert_eq!(
            message,
            format!("`{builtin}` is a builtin type name and cannot be declared as a user type"),
            "{name}"
        );
        assert_eq!(hints, vec![RESERVED_HINT.to_string()], "{name}");
    }
}

#[test]
fn slice_types_are_rejected_once_per_written_slice() {
    assert_all_slice_diagnostics("slice_type_every_position.vow", 11);
}

#[test]
fn slice_cast_targets_are_rejected_once_per_bracket_pair() {
    let slices = error_messages("slice_type_cast_target.vow")
        .into_iter()
        .filter(|(_, message, _)| message == SLICE_MESSAGE)
        .count();
    assert_eq!(slices, 3);
}

#[test]
fn builtin_type_names_cannot_be_declared() {
    assert_reserved_diagnostics(
        "reserved_type_name_every_builtin.vow",
        &["String", "BTreeMap", "Result", "u8", "bool"],
    );
}
