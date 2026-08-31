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

fn insert_hashmap(values: HashMap<i64, u128>) {
    values.insert(0, 340282366920938463463374607431768211437);
}

fn insert_btreemap(values: BTreeMap<i64, u128>) {
    values.insert(0, 340282366920938463463374607431768211436);
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

/// A 128-bit value handed to an i64-only builtin (the `Vec` element helpers)
/// must fail closed. Before the guard it compiled and silently returned the
/// low limb — worse than the hard Cranelift panic it replaced.
#[test]
fn wide_values_in_aggregates_fail_closed() {
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_vec.vow");
    let output_path = dir.path().join("wide_vec");
    fs::write(
        &source_path,
        "module WideVec\n\
         fn main() -> i32 {\n\
         let v: Vec<i128> = Vec::new();\n\
         v.push(3154393236604333326345);\n\
         v[0];\n\
         0\n\
         }\n",
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
        Some(1),
        "128-bit Vec elements must fail closed\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(json["status"], "CompileFailed");
    assert!(
        json["message"]
            .as_str()
            .is_some_and(|message| message.contains("silently drop the high 64 bits")),
        "build must explain why it refused: {json}"
    );
    assert!(
        !output_path.exists(),
        "refused wide aggregate must not leave an executable"
    );
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

/// The divisor-zero trap on the routed path must abort rather than reach the
/// runtime helper, matching what `i64 / 0` does today.
///
/// The two operator families abort differently, and deliberately so. A
/// *checked* operator's abort is specified behaviour, so it is reported:
/// `__vow_arithmetic_overflow` prints the `ArithmeticOverflow` envelope and
/// exits the reserved status `134` (`cli.md`), which is a normal exit and so
/// carries an exit code. The *unchecked* `/` and `%` reach the backend's own
/// divisor trap, which is a `ud2`: the process dies on `SIGILL` and has no
/// exit code at all. Asserting only "does not return" would not distinguish
/// them, and would pass again if a checked operator regressed to a bare trap.
#[test]
fn wide_division_by_zero_traps() {
    ensure_runtime_archive();
    for (name, expression, diagnosed) in [
        ("div", "x / y", false),
        ("rem", "x % y", false),
        ("checked_div", "x /! y", true),
        ("checked_rem", "x %! y", true),
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
        if diagnosed {
            assert_eq!(
                run.status.code(),
                Some(134),
                "{name}: a checked operator's divisor-zero abort must exit 134,                  not die on a bare trap; stderr: {stderr:?}"
            );
            assert!(
                stderr.contains(r#"{"error":"ArithmeticOverflow"}"#),
                "{name}: a checked operator's abort must be diagnosed; stderr: {stderr:?}"
            );
        } else {
            assert_eq!(
                run.status.code(),
                None,
                "{name}: division by zero must abort, not return"
            );
        }
    }
}
