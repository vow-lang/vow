//! `vow build --perfetto <file>` writes a gzipped Chrome Trace Event Format
//! trace of the compilation, and the build's stdout JSON is unaffected by the
//! flag (the trace is a pure side artifact).

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

fn vow_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vow"))
}

const PROGRAM: &str = "module M\nfn main() -> i32 [io] { 0 }\n";

fn decode_trace(path: &std::path::Path) -> serde_json::Value {
    let f = std::fs::File::open(path).expect("trace file should exist");
    let mut gz = flate2::read::GzDecoder::new(f);
    let mut s = String::new();
    gz.read_to_string(&mut s)
        .expect("trace must be valid gzip data");
    serde_json::from_str(&s).expect("trace must be valid JSON")
}

#[test]
fn build_with_perfetto_writes_loadable_trace() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("m.vow");
    std::fs::write(&src, PROGRAM).unwrap();
    let out = dir.path().join("out");
    let trace = dir.path().join("trace.json.gz");

    let result = Command::new(vow_bin())
        .args([
            "build",
            "--no-verify",
            src.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--perfetto",
            trace.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run vow");

    assert!(trace.exists(), "perfetto trace file was not written");
    let v = decode_trace(&trace);
    let events = v["traceEvents"]
        .as_array()
        .expect("traceEvents must be an array");
    assert!(!events.is_empty(), "trace has no events");

    let span_names: Vec<&str> = events
        .iter()
        .filter(|e| e["ph"] == "X")
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        span_names.contains(&"parse"),
        "expected a frontend 'parse' phase span, got {span_names:?}"
    );
    assert!(
        span_names.contains(&"codegen"),
        "expected a 'codegen' span, got {span_names:?}"
    );

    // Sanity: the run itself didn't crash.
    assert!(result.status.code().is_some(), "vow terminated abnormally");
}

#[test]
fn perfetto_flag_does_not_change_build_json() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("m.vow");
    std::fs::write(&src, PROGRAM).unwrap();
    let out = dir.path().join("out");
    let trace = dir.path().join("trace.json.gz");

    let run = |with_perfetto: bool| {
        let mut args = vec![
            "build".to_string(),
            "--no-verify".to_string(),
            src.to_str().unwrap().to_string(),
            "-o".to_string(),
            out.to_str().unwrap().to_string(),
        ];
        if with_perfetto {
            args.push("--perfetto".to_string());
            args.push(trace.to_str().unwrap().to_string());
        }
        let o = Command::new(vow_bin())
            .args(&args)
            .output()
            .expect("failed to run vow");
        String::from_utf8_lossy(&o.stdout).into_owned()
    };

    let without = run(false);
    let with = run(true);
    assert_eq!(
        without, with,
        "build stdout JSON must be identical with and without --perfetto"
    );
}
