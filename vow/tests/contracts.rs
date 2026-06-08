use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples")
}

fn run_contracts(file: &str) -> serde_json::Value {
    let out = Command::new(vow_bin())
        .args(["contracts", examples_dir().join(file).to_str().unwrap()])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(0), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from contracts: {e}\nstdout: {stdout}"))
}

#[test]
fn contracts_help() {
    let out = Command::new(vow_bin())
        .args(["contracts", "--help"])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from contracts --help: {e}\nstdout: {stdout}"));
    assert!(json.get("tool").is_some());
}

#[test]
fn contracts_no_contracts() {
    let json = run_contracts("hello.vow");
    let contracts = json["contracts"].as_array().unwrap();
    assert!(contracts.is_empty());
    assert_eq!(json["summary"]["total"], 0);
    assert_eq!(json["summary"]["not_verified"], 0);
}

#[test]
fn contracts_divide_requires() {
    let json = run_contracts("divide.vow");
    let contracts = json["contracts"].as_array().unwrap();
    assert_eq!(contracts.len(), 1);
    let c = &contracts[0];
    assert_eq!(c["function"], "divide");
    assert_eq!(c["kind"], "requires");
    assert_eq!(c["blame"], "Caller");
    assert_eq!(c["status"], "not_verified");
    assert_eq!(c["quality"], "substantive");
    assert!(c["description"].as_str().unwrap().contains("y != 0"));
    assert!(c["source"]["file"].as_str().unwrap().contains("divide.vow"));
    assert_eq!(json["summary"]["total"], 1);
    assert_eq!(json["summary"]["not_verified"], 1);
}

#[test]
fn contracts_verify_missing_esbmc_keeps_contracts_schema() {
    let empty_path = tempfile::TempDir::new().unwrap();
    let out = Command::new(vow_bin())
        .args([
            "contracts",
            "--verify",
            examples_dir().join("divide.vow").to_str().unwrap(),
        ])
        .env("PATH", empty_path.path())
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(1), "exit code should be 1");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("invalid JSON from contracts --verify without ESBMC: {e}\nstdout: {stdout}")
    });
    let contracts = json["contracts"].as_array().unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0]["status"], "error");
    assert!(json["summary"].is_object());
    assert_eq!(json["summary"]["total"], 1);
    assert_eq!(json["summary"]["error"], 1);
    assert_eq!(json["summary"]["not_verified"], 0);
}

#[test]
fn contracts_bisect_requires_and_invariant() {
    let json = run_contracts("bisect.vow");
    let contracts = json["contracts"].as_array().unwrap();
    assert_eq!(contracts.len(), 2);
    let kinds: Vec<&str> = contracts
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"requires"));
    assert!(kinds.contains(&"invariant"));
    assert_eq!(json["summary"]["total"], 2);
}

#[test]
fn contracts_ensures() {
    let json = run_contracts("callee_blame.vow");
    let contracts = json["contracts"].as_array().unwrap();
    assert_eq!(contracts.len(), 2);
    for c in contracts {
        assert_eq!(c["kind"], "ensures");
        assert_eq!(c["blame"], "Callee");
    }
}

#[test]
fn contracts_where_clause() {
    let json = run_contracts("where_divide.vow");
    let contracts = json["contracts"].as_array().unwrap();
    assert_eq!(contracts.len(), 2);
    for c in contracts {
        assert_eq!(c["kind"], "requires");
        assert_eq!(c["blame"], "Caller");
        assert!(c["description"]
            .as_str()
            .unwrap()
            .contains("where on parameter"));
    }
}

#[test]
fn contracts_multiple_functions() {
    let json = run_contracts("cegis_fixed.vow");
    let contracts = json["contracts"].as_array().unwrap();
    assert_eq!(contracts.len(), 4);
    let funcs: Vec<&str> = contracts
        .iter()
        .map(|c| c["function"].as_str().unwrap())
        .collect();
    assert!(funcs.contains(&"safe_sub"));
    assert_eq!(json["summary"]["total"], 4);
}

#[test]
fn contracts_compile_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let bad = dir.path().join("bad.vow");
    std::fs::write(&bad, "fn main() -> i32 [io] { let x: bool = 42; 0 }").unwrap();
    let out = Command::new(vow_bin())
        .args(["contracts", bad.to_str().unwrap()])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn contracts_json_has_all_summary_fields() {
    let json = run_contracts("divide.vow");
    let summary = &json["summary"];
    assert!(summary.get("total").is_some());
    assert!(summary.get("proven").is_some());
    assert!(summary.get("failed").is_some());
    assert!(summary.get("unknown").is_some());
    assert!(summary.get("timeout").is_some());
    assert!(summary.get("error").is_some());
    assert!(summary.get("not_verified").is_some());
    let quality = &summary["quality"];
    assert!(quality.get("weak").is_some());
    assert!(quality.get("tautological").is_some());
    assert!(quality.get("substantive").is_some());
}

#[test]
fn contracts_json_has_all_entry_fields() {
    let json = run_contracts("divide.vow");
    let c = &json["contracts"][0];
    assert!(c.get("vow_id").is_some());
    assert!(c.get("function").is_some());
    assert!(c.get("kind").is_some());
    assert!(c.get("description").is_some());
    assert!(c.get("blame").is_some());
    assert!(c.get("source").is_some());
    assert!(c.get("status").is_some());
    assert!(c.get("quality").is_some());
    assert!(c["source"].get("file").is_some());
    assert!(c["source"].get("offset").is_some());
}

#[test]
fn contracts_quality_classifies_clause_shapes() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("quality.vow");
    std::fs::write(
        &src,
        "module Quality\n\
         fn weak_one(x: i64) -> i64 vow {\n  ensures: result >= 0\n} { 0 }\n\
         fn strong_one(x: i64) -> i64 vow {\n  requires: x >= 0,\n  ensures: result == x + 1\n} { x + 1 }\n\
         fn taut_one() -> i64 vow {\n  ensures: true\n} { 0 }\n",
    )
    .unwrap();
    let out = Command::new(vow_bin())
        .args(["contracts", src.to_str().unwrap()])
        .output()
        .expect("failed to run vow");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON from contracts: {e}\nstdout: {stdout}"));
    let quality_of = |func: &str, kind: &str| -> String {
        json["contracts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["function"] == func && c["kind"] == kind)
            .unwrap_or_else(|| panic!("missing {func}/{kind}"))["quality"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // A postcondition that only bounds result by a constant is weak; an equality
    // postcondition and a domain precondition are substantive; `ensures true` is
    // tautological. See docs/spec/contracts-methodology.md.
    assert_eq!(quality_of("weak_one", "ensures"), "weak");
    assert_eq!(quality_of("strong_one", "requires"), "substantive");
    assert_eq!(quality_of("strong_one", "ensures"), "substantive");
    assert_eq!(quality_of("taut_one", "ensures"), "tautological");
    let q = &json["summary"]["quality"];
    assert_eq!(q["weak"], 1);
    assert_eq!(q["substantive"], 2);
    assert_eq!(q["tautological"], 1);
}
