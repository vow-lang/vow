/// Returns the native libraries needed to link the Vow runtime on `os`.
pub fn platform_link_args_for(os: &str) -> &'static [&'static str] {
    match os {
        "linux" => &["-lpthread", "-ldl", "-lm"],
        "macos" => &["-lpthread", "-lm"],
        _ => &["-lpthread", "-lm"],
    }
}
