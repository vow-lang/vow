use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

#[test]
fn aggregate_contexts_lower_wide_literal_magnitudes_at_the_declared_width() {
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_aggregates.vow");
    fs::write(
        &source_path,
        r#"module WideAggregates

struct WideStruct {
    value: u128,
}

enum WideEnum {
    Value(u128),
}

fn make_struct() -> WideStruct {
    WideStruct { value: 18446744073709551616 }
}

fn make_enum() -> WideEnum {
    WideEnum::Value(18446744073709551617)
}
"#,
    )
    .unwrap();

    let output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--dump-ir",
            source_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "dump-ir failed\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("ConstU128[18446744073709551616u128]"),
        "struct field lost its high limb:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[18446744073709551617u128]"),
        "enum payload lost its high limb:\n{stdout}"
    );
}
