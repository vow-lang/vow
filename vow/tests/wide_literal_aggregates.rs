use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

/// Codegen tests below link real executables, which needs `libvow_runtime.a`.
/// `cargo test` builds only the crates under test, so on a clean checkout with
/// no prior `cargo build --all` that archive does not exist and every such
/// test fails on a link error rather than on the behavior it means to check.
///
/// Build it on demand once per test binary and point the compiler at it via
/// `VOW_RUNTIME_PATH`, so these tests are self-contained instead of silently
/// depending on the order the developer happened to run cargo in. Building
/// (rather than tolerating the link failure, as `effect_gating.rs` does for
/// its frontend-only assertions) is what keeps the runtime behavior these
/// tests exist to verify actually under test.
fn ensure_runtime_archive() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Honor an archive the caller already provisioned.
        if std::env::var_os("VOW_RUNTIME_PATH").is_some_and(|p| PathBuf::from(p).exists()) {
            return;
        }
        let target_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../target"));
        for profile in ["release", "debug"] {
            let candidate = target_dir.join(profile).join("libvow_runtime.a");
            if candidate.exists() {
                return; // the linker's own search finds this unaided
            }
        }
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "vow-runtime"])
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
            .status()
            .expect("spawn cargo to build vow-runtime");
        assert!(status.success(), "failed to build vow-runtime staticlib");
        let built = target_dir.join("debug").join("libvow_runtime.a");
        assert!(
            built.exists(),
            "cargo build -p vow-runtime did not produce {}",
            built.display()
        );
        // SAFETY: single-threaded `Once` initializer, before any test spawns a
        // child process that reads the environment.
        unsafe { std::env::set_var("VOW_RUNTIME_PATH", &built) };
    });
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

struct WrappedWide {
    value: Option<u128>,
}

struct WideVec {
    values: Vec<u128>,
}

enum WideEnum {
    Value(u128),
}

enum NestedWideEnum {
    Value(Option<u128>),
}

fn make_struct() -> WideStruct {
    WideStruct { value: 18446744073709551616 }
}

fn make_enum() -> WideEnum {
    WideEnum::Value(18446744073709551617)
}

fn choose_wide(flag: bool) -> u128 {
    if flag { 170141183460469231731687303715884105728 } else { 0 }
}

fn compare_wide(x: u128, flag: bool) -> bool {
    x == if flag { 18446744073709551618 } else { 0 }
}

fn compare_wide_literal_first(x: u128) -> bool {
    340282366920938463463374607431768211450 == x
}

fn assign_wide(target: WideStruct) {
    target.value = 18446744073709551619;
}

fn suffixed_negative_zero() -> u128 {
    -0u128
}

fn match_wide(value: Option<i64>) -> u128 {
    match value {
        Option::Some(_) => { 340282366920938463463374607431768211455 },
        Option::None => { 0 },
    }
}

fn make_option() -> Option<u128> {
    let value: Option<u128> = Option::Some(340282366920938463463374607431768211454);
    value
}

fn make_ok() -> Result<u128, ()> {
    let value: Result<u128, ()> = Result::Ok(340282366920938463463374607431768211453);
    value
}

fn make_err() -> Result<(), u128> {
    let value: Result<(), u128> = Result::Err(340282366920938463463374607431768211452);
    value
}

fn consume_option(value: Option<u128>) {
}

fn pass_option() {
    consume_option(Option::Some(340282366920938463463374607431768211451));
}

fn loop_wide() -> u128 {
    loop {
        break 340282366920938463463374607431768211449;
    }
}

fn make_wrapped() -> WrappedWide {
    WrappedWide {
        value: Option::Some(340282366920938463463374607431768211448),
    }
}

fn choose_option(flag: bool) -> Option<u128> {
    if flag {
        Option::Some(340282366920938463463374607431768211447)
    } else {
        Option::None
    }
}

fn assign_wide_vec(values: Vec<u128>) {
    values[0] = 340282366920938463463374607431768211446;
}

fn match_option(value: Option<i64>) -> Option<u128> {
    match value {
        Option::Some(_) => { Option::Some(340282366920938463463374607431768211445) },
        Option::None => { Option::None },
    }
}

fn loop_option() -> Option<u128> {
    loop {
        break Option::Some(340282366920938463463374607431768211444);
    }
}

fn assign_wide_vec_field(target: WideVec) {
    target.values[0] = 340282366920938463463374607431768211443;
}

fn cast_wide_marker() -> u128 {
    (340282366920938463463374607431768211442 + 0) as u128
}

fn make_nested_enum() -> NestedWideEnum {
    NestedWideEnum::Value(Option::Some(340282366920938463463374607431768211441))
}

fn push_wide_vec(values: Vec<u128>) {
    values.push(340282366920938463463374607431768211440);
}

fn return_option() -> Option<u128> {
    return Option::Some(340282366920938463463374607431768211439);
}

fn assign_wrapped(target: WrappedWide) {
    target.value = Option::Some(340282366920938463463374607431768211438);
}

fn insert_hashmap(values: HashMap<i64, Option<u128>>) {
    values.insert(0, Option::Some(340282366920938463463374607431768211437));
}

fn insert_btreemap(values: BTreeMap<i64, Option<u128>>) {
    values.insert(0, Option::Some(340282366920938463463374607431768211436));
}

fn assign_option_vec(values: Vec<Option<u128>>) {
    values[0] = Option::Some(340282366920938463463374607431768211435);
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
    assert!(
        stdout.contains("ConstU128[170141183460469231731687303715884105728u128]"),
        "conditional branch lost its high limb before the Phi:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[18446744073709551618u128]"),
        "binary conditional operand lost its high limb:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211450u128]"),
        "left-hand binary literal lost its high limb:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[18446744073709551619u128]"),
        "field assignment lost its high limb:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211455u128]"),
        "match arm lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211454u128]"),
        "Option payload lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211453u128]"),
        "Result::Ok payload lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211452u128]"),
        "Result::Err payload lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211451u128]"),
        "generic function argument lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211449u128]"),
        "loop break value lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211448u128]"),
        "nested generic struct field lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211447u128]"),
        "generic conditional payload lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211446u128]"),
        "indexed Vec assignment lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211445u128]"),
        "generic match payload lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211444u128]"),
        "generic loop payload lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211443u128]"),
        "Vec-valued struct field assignment lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211442u128]"),
        "explicit wide cast lost its compound marker limbs:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211441u128]"),
        "nested user-enum payload lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211440u128]"),
        "Vec::push argument lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211439u128]"),
        "explicit generic return lost its contextual wide type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211438u128]"),
        "field assignment lost its complete declared type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211437u128]"),
        "HashMap insertion lost its declared value type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211436u128]"),
        "BTreeMap insertion lost its declared value type:\n{stdout}"
    );
    assert!(
        stdout.contains("ConstU128[340282366920938463463374607431768211435u128]"),
        "index assignment lost its complete declared element type:\n{stdout}"
    );
}

#[test]
fn wide_codegen_produces_an_executable() {
    ensure_runtime_archive();
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_codegen.vow");
    let output_path = dir.path().join("wide_codegen");
    fs::write(
        &source_path,
        "module WideCodegen\nfn value() -> u128 { 42u128 }\nfn main() -> i32 { 0 }\n",
    )
    .unwrap();

    let output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            source_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("invalid JSON from build: {error}\nstdout: {stdout}"));

    assert_eq!(
        output.status.code(),
        Some(0),
        "wide constant codegen must succeed\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(json["status"], "Unverified");
    assert!(
        output_path.exists(),
        "successful wide codegen must produce an executable"
    );
}

/// 128-bit `Vec` elements are 16 bytes wide, so both limbs must survive a
/// push/index/assign round trip, neighbouring elements must not overlap, and a
/// buffer that regrows must keep every element intact.
#[test]
fn wide_vec_elements_round_trip_both_limbs() {
    ensure_runtime_archive();
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_vec.vow");
    let output_path = dir.path().join("wide_vec");
    fs::write(
        &source_path,
        r#"module WideVec
fn main() -> () [io] {
    let v: Vec<u128> = Vec::new();
    let mut i: u64 = 0;
    while i < 40 {
        v.push(340282366920938463463374607431768211454 - (i as u128));
        i = i + 1;
    }
    v[3] = 7;
    let mut total: u128 = 0;
    for x in v {
        total = total + x;
    }
    let first: u128 = v[0];
    let fourth: u128 = v[3];
    let last: u128 = v[39];
    print_i64(u128_to_u8_wrap(first >> 120) as i64);
    print_str(" ");
    print_i64(u128_to_u8_wrap(first) as i64);
    print_str(" ");
    print_i64(u128_to_u8_wrap(fourth) as i64);
    print_str(" ");
    print_i64(u128_to_u8_wrap(last) as i64);
    print_str(" ");
    print_i64(u128_to_u8_wrap(total) as i64);
}
"#,
    )
    .unwrap();

    let output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            source_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(
        output.status.code(),
        Some(0),
        "build failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(&output_path)
        .output()
        .expect("failed to run compiled program");
    assert_eq!(run.status.code(), Some(0), "program aborted");
    // Element `i` is 2^128 - 2 - i, except element 3, which was set to 7. The
    // sum of all 40 wraps modulo 2^128 to a value whose low byte is 176.
    assert_eq!(String::from_utf8_lossy(&run.stdout), "255 254 7 215 176");
}

/// Text of one function in a `--dump-ir` listing.
fn ir_function<'a>(dump: &'a str, name: &str) -> &'a str {
    let header = format!("fn {name}(");
    let start = dump
        .find(&header)
        .unwrap_or_else(|| panic!("missing function `{name}` in:\n{dump}"));
    let rest = &dump[start..];
    let end = rest[1..].find("\nfn ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

/// A `Vec<u128>` element is two slots wide, so `push`, index read, index
/// write, and `for` go through the element-address helpers and the two-slot
/// `WideSlot` access instead of the 8-byte `*_val` helpers. The element width
/// comes from the checker, so a parameter-typed `Vec` works, and a narrow
/// `Vec` keeps the 8-byte helper.
#[test]
fn wide_vec_accesses_use_element_address_helpers() {
    ensure_runtime_archive();
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_vec_layout.vow");
    let output_path = dir.path().join("wide_vec_layout");
    fs::write(
        &source_path,
        fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/wide_vec_layout.vow"),
        )
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            "--no-cache",
            "--dump-ir",
            source_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    let dump = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);

    let expect = |name: &str, present: &[&str], absent: &[&str]| {
        let body = ir_function(&dump, name);
        for needle in present {
            assert!(
                body.contains(needle),
                "`{name}` must contain {needle}:\n{body}"
            );
        }
        for needle in absent {
            assert!(
                !body.contains(needle),
                "`{name}` must not contain {needle}:\n{body}"
            );
        }
    };
    expect(
        "push_one",
        &["extern:__vow_vec_push_wide_ptr", "FieldSet[wide_slot_0]"],
        &["__vow_vec_push_val"],
    );
    expect(
        "read_one",
        &["extern:__vow_vec_get_wide_ptr", "FieldGet[wide_slot_0]"],
        &["__vow_vec_get_val"],
    );
    expect(
        "write_one",
        &["extern:__vow_vec_set_wide_ptr", "FieldSet[wide_slot_0]"],
        &["__vow_vec_set_val"],
    );
    expect(
        "sum",
        &["extern:__vow_vec_get_wide_ptr", "FieldGet[wide_slot_0]"],
        &["__vow_vec_get_val"],
    );
    expect(
        "narrow",
        &["extern:__vow_vec_get_val"],
        &["__vow_vec_get_wide_ptr", "wide_slot"],
    );
}

/// 128-bit struct fields occupy two consecutive slots, so both limbs must
/// survive a store/load round trip and the narrow fields around the wide one
/// must not overlap it.
#[test]
fn wide_struct_fields_round_trip_both_limbs() {
    ensure_runtime_archive();
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_struct.vow");
    let output_path = dir.path().join("wide_struct");
    fs::write(
        &source_path,
        r#"module WideStruct
struct S { a: i64, w: u128, c: i64 }
fn main() -> () [io] {
    let s: S = S { a: 1, w: 340282366920938463463374607431768211454, c: 2 };
    s.a = 3;
    s.c = 4;
    let r: u128 = s.w - (s.a as u128) - (s.c as u128);
    print_i64(u128_to_u8_wrap(r >> 120) as i64);
    print_str(" ");
    print_i64(u128_to_u8_wrap(r) as i64);
}
"#,
    )
    .unwrap();

    let output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            source_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(
        output.status.code(),
        Some(0),
        "build failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(&output_path)
        .output()
        .expect("failed to run compiled program");
    assert_eq!(run.status.code(), Some(0), "program aborted");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "255 247");
}

/// 128-bit enum payloads occupy two consecutive slots, so the high limb must
/// survive a store/load round trip and the narrow payload after the wide one
/// must not overlap it.
#[test]
fn wide_enum_payloads_round_trip_both_limbs() {
    ensure_runtime_archive();
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_enum.vow");
    let output_path = dir.path().join("wide_enum");
    fs::write(
        &source_path,
        r#"module WideEnum
enum E { V(i64, u128, i64), Empty }
fn pick(e: E) -> u128 {
    match e {
        E::V(a, w, c) => { w + (a as u128) + (c as u128) },
        E::Empty => { 0 },
    }
}
fn main() -> () [io] {
    let r: u128 = pick(E::V(1, 340282366920938463463374607431768211454, 0));
    print_i64(u128_to_u8_wrap(r >> 120) as i64);
    print_str(" ");
    print_i64(u128_to_u8_wrap(r) as i64);
}
"#,
    )
    .unwrap();

    let output = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            source_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");
    assert_eq!(
        output.status.code(),
        Some(0),
        "build failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(&output_path)
        .output()
        .expect("failed to run compiled program");
    assert_eq!(run.status.code(), Some(0), "program aborted");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "255 255");
}

/// Division, remainder, and checked multiply on 128-bit operands have no
/// native Cranelift lowering, so they route through `vow-runtime` helpers
/// (epic #526 seam 3b). End to end they must build and produce the same
/// answers native `u128` arithmetic does.
#[test]
fn wide_division_and_checked_multiply_run_through_runtime_helpers() {
    ensure_runtime_archive();
    for (name, expression, x, y, expected) in [
        (
            "div",
            "x / y",
            340282366920938463463374607431768211455u128,
            5,
            68056473384187692692674921486353642291u128,
        ),
        (
            "rem",
            "x % y",
            340282366920938463463374607431768211455,
            7,
            3,
        ),
        (
            "checked_div",
            "x /! y",
            3781582535110458081280,
            5,
            756316507022091616256,
        ),
        ("checked_rem", "x %! y", 3781582535110458081280, 7, 4),
        (
            "checked_mul",
            "x *! y",
            3781582535110458081280,
            2,
            7563165070220916162560,
        ),
    ] {
        let dir = tempfile::TempDir::new().unwrap();
        let source_path = dir.path().join("wide_div.vow");
        let output_path = dir.path().join("wide_div");
        // The result is printed one byte at a time, low limb then high, so a
        // helper ABI that truncated to 64 bits would fail on the high column
        // rather than pass by correlated truncation.
        fs::write(
            &source_path,
            format!(
                "module WideDiv\n\
                 fn f(x: u128, y: u128) -> u128 {{ {expression} }}\n\
                 fn main() -> () [io] {{\n\
                 let r: u128 = f({x}, {y});\n\
                 let mut i: u128 = 0;\n\
                 while i < 16 {{\n\
                 print_i64(u128_to_u8_wrap(r >> (i * 8)) as i64);\n\
                 print_str(\" \");\n\
                 i = i + 1;\n\
                 }}\n\
                 }}\n"
            ),
        )
        .unwrap();

        let output = Command::new(vow_bin())
            .args([
                "build",
                "--no-verify",
                source_path.to_str().unwrap(),
                "-o",
                output_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run vow");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: build failed\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(output_path.exists(), "{name}: no executable produced");

        let run = Command::new(&output_path)
            .output()
            .expect("failed to run compiled program");
        assert_eq!(run.status.code(), Some(0), "{name}: program aborted");
        let want: String = (0..16)
            .map(|i| format!("{} ", (expected >> (i * 8)) as u8))
            .collect();
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            want,
            "{name}: wrong 128-bit result"
        );
    }
}

/// Every divisor-zero abort on the routed path must use the runtime reporter,
/// regardless of whether the source operator is checked or unchecked. This
/// keeps the abort machine-readable and gives it the reserved exit status 134.
#[test]
fn wide_division_by_zero_traps() {
    ensure_runtime_archive();
    for (name, expression) in [
        ("div", "x / y"),
        ("rem", "x % y"),
        ("checked_div", "x /! y"),
        ("checked_rem", "x %! y"),
    ] {
        let dir = tempfile::TempDir::new().unwrap();
        let source_path = dir.path().join("wide_div_zero.vow");
        let output_path = dir.path().join("wide_div_zero");
        fs::write(
            &source_path,
            format!(
                "module WideDivZero\nfn f(x: u128, y: u128) -> u128 {{ {expression} }}\nfn main() -> i32 {{ f(10, 0); 0 }}\n"
            ),
        )
        .unwrap();

        let output = Command::new(vow_bin())
            .args([
                "build",
                "--no-verify",
                source_path.to_str().unwrap(),
                "-o",
                output_path.to_str().unwrap(),
            ])
            .output()
            .expect("failed to run vow");
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: build failed\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );

        let run = Command::new(&output_path)
            .output()
            .expect("failed to run compiled program");
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(
            run.status.code(),
            Some(134),
            "{name}: a divisor-zero abort must exit 134, not die on a bare trap; stderr: {stderr:?}"
        );
        assert!(
            stderr.contains(r#"{"error":"ArithmeticOverflow"}"#),
            "{name}: a divisor-zero abort must be diagnosed; stderr: {stderr:?}"
        );
    }
}
