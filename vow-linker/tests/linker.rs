use std::error::Error;
use std::process::Command;

use tempfile::TempDir;
use vow_linker::LinkError;
use vow_linker::link_executable;
use vow_linker::link_reproducible_executable;
use vow_linker::platform_link_args_for;

fn compile_returning(dir: &TempDir, exit_code: i32) -> std::path::PathBuf {
    let source = dir.path().join("main.c");
    let object = dir.path().join("main.o");
    std::fs::write(
        &source,
        format!("int main(void) {{ return {exit_code}; }}\n"),
    )
    .unwrap();

    let compile_status = Command::new("cc")
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .status()
        .unwrap();
    assert!(compile_status.success());
    object
}

#[test]
fn links_native_input_into_executable() {
    let dir = TempDir::new().unwrap();
    let object = compile_returning(&dir, 23);
    let executable = dir.path().join("main");

    link_executable([object.as_path()], &executable).unwrap();

    let status = Command::new(executable).status().unwrap();
    assert_eq!(status.code(), Some(23));
}

#[test]
fn links_reproducible_native_input_into_executable() {
    let dir = TempDir::new().unwrap();
    let object = compile_returning(&dir, 29);
    let executable = dir.path().join("main");

    link_reproducible_executable([object.as_path()], &executable).unwrap();

    let status = Command::new(executable).status().unwrap();
    assert_eq!(status.code(), Some(29));
}

#[test]
fn reports_unsuccessful_linker_status() {
    let dir = TempDir::new().unwrap();
    let invalid_object = dir.path().join("invalid.o");
    let executable = dir.path().join("main");
    std::fs::write(&invalid_object, b"not an object file").unwrap();

    let error = link_executable([invalid_object.as_path()], &executable).unwrap_err();

    assert!(matches!(&error, LinkError::Failed(status) if !status.success()));
    assert!(error.to_string().starts_with("cc exited with status "));
    assert!(error.source().is_none());
}

#[test]
fn invocation_error_reports_its_source() {
    let error = LinkError::Invoke(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "test linker is missing",
    ));

    assert_eq!(
        error.to_string(),
        "failed to invoke cc: test linker is missing"
    );
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("test linker is missing")
    );
}

#[test]
fn linux_link_args_include_dl() {
    assert_eq!(
        platform_link_args_for("linux"),
        ["-lpthread", "-ldl", "-lm"]
    );
}

#[test]
fn macos_link_args_omit_dl() {
    assert_eq!(platform_link_args_for("macos"), ["-lpthread", "-lm"]);
}

#[test]
fn other_link_args_omit_dl() {
    assert_eq!(platform_link_args_for("freebsd"), ["-lpthread", "-lm"]);
}
