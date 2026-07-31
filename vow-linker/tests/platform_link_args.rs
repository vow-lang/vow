use vow_linker::platform_link_args_for;

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
