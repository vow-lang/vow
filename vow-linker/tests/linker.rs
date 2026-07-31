use std::process::Command;

use tempfile::TempDir;
use vow_linker::link_executable;
use vow_linker::platform_link_args_for;

#[test]
fn links_native_input_into_executable() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("main.c");
    let object = dir.path().join("main.o");
    let executable = dir.path().join("main");
    std::fs::write(&source, "int main(void) { return 23; }\n").unwrap();

    let compile_status = Command::new("cc")
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .status()
        .unwrap();
    assert!(compile_status.success());

    link_executable([object.as_path()], &executable).unwrap();

    let status = Command::new(executable).status().unwrap();
    assert_eq!(status.code(), Some(23));
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
