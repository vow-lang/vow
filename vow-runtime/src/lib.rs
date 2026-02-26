use std::ffi::CStr;
use std::io::Write as _;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_violation(vow_id: u32, blame: u8, desc_ptr: *const i8) {
    let blame_str = if blame == 0 { "Caller" } else { "Callee" };
    let desc = if desc_ptr.is_null() {
        std::borrow::Cow::Borrowed("")
    } else {
        unsafe { CStr::from_ptr(desc_ptr) }.to_string_lossy()
    };
    let json = format!(
        r#"{{"error":"VowViolation","vow_id":{vow_id},"blame":"{blame_str}","description":"{desc}"}}"#
    );
    let human = format!("vow violation: {desc}, blame={blame_str}");
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "{human}");
    std::process::exit(1);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vow_print_str(ptr: *const i8) {
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
    print!("{s}");
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_print_i64(v: i64) {
    print!("{v}");
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn __vow_arithmetic_overflow() {
    let json = r#"{"error":"ArithmeticOverflow"}"#;
    let _ = writeln!(std::io::stderr(), "{json}");
    let _ = writeln!(std::io::stderr(), "arithmetic overflow");
    std::process::exit(1);
}
