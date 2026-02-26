use std::io::Write as _;

#[unsafe(no_mangle)]
pub extern "C" fn __vow_violation(vow_id: u32, blame: u8) {
    let blame_str = if blame == 0 { "Caller" } else { "Callee" };
    let json = format!(r#"{{"error":"VowViolation","vow_id":{vow_id},"blame":"{blame_str}"}}"#);
    let human = format!("vow violation: vow_id={vow_id}, blame={blame_str}");
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "{human}");
    std::process::exit(1);
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_arithmetic_overflow() {
    let json = r#"{"error":"ArithmeticOverflow"}"#;
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "arithmetic overflow");
    std::process::exit(1);
}
