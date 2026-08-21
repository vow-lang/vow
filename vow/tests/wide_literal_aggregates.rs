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
fn deferred_wide_codegen_fails_closed_without_panicking() {
    let dir = tempfile::TempDir::new().unwrap();
    let source_path = dir.path().join("wide_codegen.vow");
    let output_path = dir.path().join("wide_codegen");
    fs::write(
        &source_path,
        "module WideCodegen\nfn value() -> u128 { 42u128 }\n",
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
        "deferred wide codegen must fail closed\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(json["status"], "CompileFailed");
    assert!(
        json["message"]
            .as_str()
            .is_some_and(|message| message.contains("wide constant codegen")),
        "build must explain the deferred backend seam: {json}"
    );
    assert!(
        !output_path.exists(),
        "failed wide codegen must not leave an executable"
    );
}
