//! The `vow test` command: discover `test_*.vow` / `*_test.vow` files, compile
//! and execute each, and assemble a single JSON [`TestResult`].
//!
//! The public surface is the single [`run_test_command`] entry point; discovery,
//! filtering, contract-density counting, per-file status classification, and
//! result assembly are internal helpers, each unit-tested through this module.

use std::path::{Path, PathBuf};

use vow_codegen::{BuildMode, TraceMode};
use vow_verify::{SolverConfig, VerifyLimits};

use crate::report::{ContractDensity, CounterexampleJson, DiagnosticJson, TestEntry, TestResult};
use crate::{BuildStatus, compile_frontend_with_root, run_pipeline_from_frontend};

/// Map a completed build pipeline's [`BuildStatus`] to the per-file test status
/// for statuses that terminate the test *before* the binary is executed.
///
/// Returns `Some(status)` for the fail-closed terminal outcomes (`compile_error`
/// / `verify_failed` / `contract_skipped`) and `None` when the pipeline produced
/// a runnable binary — in which case the per-file status is decided by the
/// process exit code (see [`classify_execution_outcome`]). `Skipped` (≥1 vowed
/// function non-modelable, ESBMC never run) is reported as `contract_skipped` so
/// consumers can tell it apart from `verify_failed` (ESBMC proved a violation);
/// both are fail-closed (#386).
fn classify_pipeline_status(status: &BuildStatus) -> Option<&'static str> {
    match status {
        BuildStatus::CompileFailed { .. } => Some("compile_error"),
        BuildStatus::VerifyFailed { .. } => Some("verify_failed"),
        BuildStatus::Skipped => Some("contract_skipped"),
        BuildStatus::Verified | BuildStatus::Unverified => None,
    }
}

/// Map a test binary's process exit code to its per-file test status. `Some(0)`
/// passed, any other exit code failed, and `None` (the process was killed at the
/// timeout deadline) is `timeout`.
fn classify_execution_outcome(exit_code: Option<i32>) -> &'static str {
    match exit_code {
        Some(0) => "passed",
        Some(_) => "failed",
        None => "timeout",
    }
}

/// Ordered inputs and module-resolution policy for one `vow test` run.
///
/// The execution loop consumes this plan without knowing discovery, naming,
/// filtering, symlink, ordering, or module-root precedence rules.
struct TestSelection {
    files: Vec<PathBuf>,
    module_root: Option<PathBuf>,
}

fn select_tests(
    scan_path: &Path,
    filter: Option<&str>,
    module_root_override: Option<&Path>,
) -> TestSelection {
    let mut files = discover_test_files(scan_path);
    if let Some(pattern) = filter {
        files.retain(|file| {
            file.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.contains(pattern))
        });
    }
    let module_root = module_root_override
        .map(Path::to_path_buf)
        .or_else(|| scan_path.is_dir().then(|| scan_path.to_path_buf()));
    TestSelection { files, module_root }
}

fn discover_test_files(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files: Vec<PathBuf> = Vec::new();
    collect_test_files(path, &mut files);
    files.sort();
    files
}

fn collect_test_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            collect_test_files(&entry_path, out);
        } else if file_type.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".vow") && (name.starts_with("test_") || name.ends_with("_test.vow"))
            {
                out.push(entry_path);
            }
        }
    }
}

fn count_contract_density(ir_module: &vow_ir::Module) -> ContractDensity {
    let mut total = 0usize;
    let mut with_vows = 0usize;
    for func in &ir_module.functions {
        if func.name == "main" {
            continue;
        }
        total += 1;
        if !func.vows.is_empty() {
            with_vows += 1;
        }
    }
    // Integer math matching self-hosted: (n * 1000) / total gives tenths of a percent
    let tenths = ((with_vows * 1000).checked_div(total)).unwrap_or(0);
    ContractDensity {
        functions_total: total,
        functions_with_vows: with_vows,
        density_pct: (tenths / 10) as f64 + (tenths % 10) as f64 / 10.0,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_test_command(
    path: &Path,
    verify: bool,
    filter: Option<&str>,
    module_root_override: Option<&Path>,
    mode: BuildMode,
    timeout_ms: u64,
    limits: &VerifyLimits,
    jobs: usize,
) {
    if !path.exists() {
        let result = TestResult {
            status: "CompileFailed".to_string(),
            total: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            tests: vec![],
            contract_density: ContractDensity {
                functions_total: 0,
                functions_with_vows: 0,
                density_pct: 0.0,
            },
        };
        println!("{}", serde_json::to_string(&result).unwrap());
        eprintln!("error: test path '{}' does not exist", path.display());
        std::process::exit(1);
    }

    let selection = select_tests(path, filter, module_root_override);
    let module_root = selection.module_root.as_deref();

    let mut entries = Vec::new();
    let mut total_density = ContractDensity {
        functions_total: 0,
        functions_with_vows: 0,
        density_pct: 0.0,
    };

    let _ = std::fs::create_dir_all("build");

    for test_file in &selection.files {
        let start = std::time::Instant::now();
        let file_str = test_file.to_string_lossy().to_string();
        let name = test_file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Compile frontend once — extract density before codegen
        let frontend = match compile_frontend_with_root(test_file, module_root, None) {
            Ok(f) => f,
            Err(output) => {
                let diagnostics: Vec<DiagnosticJson> = output
                    .diagnostics
                    .iter()
                    .map(DiagnosticJson::from_diagnostic)
                    .collect();
                entries.push(TestEntry {
                    file: file_str,
                    name,
                    status: "compile_error".to_string(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    diagnostics,
                    counterexamples: vec![],
                });
                continue;
            }
        };

        let density = count_contract_density(
            frontend
                .ir()
                .expect("LoweredIr goal must produce IR for test density"),
        );
        total_density.functions_total += density.functions_total;
        total_density.functions_with_vows += density.functions_with_vows;

        let tmp_out = Path::new("build").join(format!("vow_test_{name}_{}", std::process::id()));
        let result = run_pipeline_from_frontend(
            frontend,
            test_file,
            Some(&tmp_out),
            mode,
            !verify,
            false,
            TraceMode::Off,
            true,
            limits,
            jobs,
            &SolverConfig::default_config(),
            None,
        );

        let diagnostics: Vec<DiagnosticJson> = result
            .diagnostics
            .iter()
            .map(DiagnosticJson::from_diagnostic)
            .collect();
        let counterexamples: Vec<CounterexampleJson> = result
            .counterexamples
            .iter()
            .map(CounterexampleJson::from_structured)
            .collect();

        // Terminal pipeline outcomes (compile_error / verify_failed /
        // contract_skipped) never produce a runnable binary — record and skip
        // execution. A `None` classification means the pipeline succeeded and the
        // per-file status is decided by the process exit code below.
        if let Some(status) = classify_pipeline_status(&result.status) {
            entries.push(TestEntry {
                file: file_str,
                name,
                status: status.to_string(),
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: start.elapsed().as_millis() as u64,
                diagnostics,
                counterexamples,
            });
            continue;
        }

        let exe_path = match &result.executable {
            Some(p) => p.clone(),
            None => {
                entries.push(TestEntry {
                    file: file_str,
                    name,
                    status: "compile_error".to_string(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    diagnostics,
                    counterexamples,
                });
                continue;
            }
        };

        // Execute with ulimit wrapper and timeout
        let exe_abs = std::fs::canonicalize(&exe_path).unwrap_or(exe_path.clone());
        let child = std::process::Command::new("sh")
            .args([
                "-c",
                "ulimit -v 2000000; \"$0\"",
                &exe_abs.display().to_string(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let (exit_code, stdout_str, stderr_str) = match child {
            Ok(mut child) => {
                // Take stdout/stderr handles and drain in background threads to
                // prevent pipe buffer deadlock when tests produce >64KB output.
                use std::io::Read;
                let stdout_handle = child.stdout.take();
                let stderr_handle = child.stderr.take();
                let stdout_thread = std::thread::spawn(move || {
                    let mut buf = String::new();
                    if let Some(mut r) = stdout_handle {
                        let _ = r.read_to_string(&mut buf);
                    }
                    buf
                });
                let stderr_thread = std::thread::spawn(move || {
                    let mut buf = String::new();
                    if let Some(mut r) = stderr_handle {
                        let _ = r.read_to_string(&mut buf);
                    }
                    buf
                });

                let timeout = std::time::Duration::from_millis(timeout_ms);
                let deadline = std::time::Instant::now() + timeout;
                let exit = loop {
                    match child.try_wait() {
                        Ok(Some(status)) => break Some(status.code()),
                        Ok(None) => {
                            if std::time::Instant::now() >= deadline {
                                let _ = child.kill();
                                let _ = child.wait();
                                break None;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(_) => break Some(Some(-1)),
                    }
                };

                let stdout = stdout_thread.join().unwrap_or_default();
                let stderr = stderr_thread.join().unwrap_or_default();
                match exit {
                    Some(code) => (code, stdout, stderr),
                    None => (None, String::new(), "timeout".to_string()),
                }
            }
            Err(e) => (Some(-1), String::new(), e.to_string()),
        };

        // Clean up the produced binary
        let _ = std::fs::remove_file(&exe_path);

        let status = classify_execution_outcome(exit_code);

        entries.push(TestEntry {
            file: file_str,
            name,
            status: status.to_string(),
            exit_code,
            stdout: stdout_str,
            stderr: stderr_str,
            duration_ms: start.elapsed().as_millis() as u64,
            diagnostics,
            counterexamples,
        });
    }

    let test_result = build_test_result(entries, total_density);

    let json = serde_json::to_string(&test_result).expect("TestResult must be serializable");
    println!("{json}");

    if test_result.failed > 0 {
        std::process::exit(1);
    }
}

/// Assemble the final [`TestResult`] from the collected per-file `entries` and
/// the accumulated contract `density`. Pure: no IO, no process exit — the
/// orchestration in [`run_test_command`] handles printing and the exit code.
///
/// Finalizes `density_pct` (integer math matching the self-hosted compiler),
/// tallies pass/fail/skip, and gates the overall status. A file counts as
/// `failed` when its status is any of `failed`, `compile_error`,
/// `verify_failed`, or `contract_skipped` — all fail-closed (#386).
fn build_test_result(entries: Vec<TestEntry>, mut density: ContractDensity) -> TestResult {
    // Compute final density (integer math matching self-hosted compiler)
    if let Some(tenths) = (density.functions_with_vows * 1000).checked_div(density.functions_total)
    {
        density.density_pct = (tenths / 10) as f64 + (tenths % 10) as f64 / 10.0;
    }

    let passed = entries.iter().filter(|e| e.status == "passed").count();
    let failed = entries
        .iter()
        .filter(|e| {
            matches!(
                e.status.as_str(),
                "failed" | "compile_error" | "verify_failed" | "contract_skipped"
            )
        })
        .count();
    let skipped = entries.iter().filter(|e| e.status == "skipped").count();

    let status = if failed > 0 {
        "TestsFailed"
    } else {
        "TestsPassed"
    };

    TestResult {
        status: status.to_string(),
        total: entries.len(),
        passed,
        failed,
        skipped,
        tests: entries,
        contract_density: density,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_source(dir: &TempDir, name: &str, src: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, src).unwrap();
        path
    }

    #[test]
    fn select_tests_returns_ordered_files_and_resolved_root() {
        let dir = TempDir::new().unwrap();
        let beta = write_source(&dir, "beta_test.vow", "module Beta");
        let alpha = write_source(&dir, "test_alpha.vow", "module Alpha");
        write_source(&dir, "notes.vow", "module Notes");
        let nested_dir = dir.path().join("nested");
        std::fs::create_dir(&nested_dir).unwrap();
        let nested = nested_dir.join("test_nested.vow");
        std::fs::write(&nested, "module Nested").unwrap();

        let all = select_tests(dir.path(), None, None);
        assert_eq!(all.files, vec![beta, nested, alpha.clone()]);
        assert_eq!(all.module_root, Some(dir.path().to_path_buf()));

        let override_root = dir.path().join("module-root");
        let filtered = select_tests(dir.path(), Some("alpha"), Some(&override_root));
        assert_eq!(filtered.files, vec![alpha]);
        assert_eq!(filtered.module_root, Some(override_root));

        let single = write_source(&dir, "plain.vow", "module Plain");
        let unmatched = select_tests(&single, Some("missing"), None);
        assert!(unmatched.files.is_empty());
        assert_eq!(unmatched.module_root, None);
    }

    #[test]
    fn classify_execution_outcome_maps_exit_codes() {
        assert_eq!(classify_execution_outcome(Some(0)), "passed");
        assert_eq!(classify_execution_outcome(Some(1)), "failed");
        assert_eq!(classify_execution_outcome(Some(-1)), "failed");
        assert_eq!(classify_execution_outcome(None), "timeout");
    }

    #[test]
    fn classify_pipeline_status_maps_terminal_statuses() {
        assert_eq!(
            classify_pipeline_status(&BuildStatus::CompileFailed {
                message: String::new()
            }),
            Some("compile_error")
        );
        assert_eq!(
            classify_pipeline_status(&BuildStatus::VerifyFailed {
                function: String::new(),
                description: String::new()
            }),
            Some("verify_failed")
        );
        assert_eq!(
            classify_pipeline_status(&BuildStatus::Skipped),
            Some("contract_skipped")
        );
    }

    #[test]
    fn classify_pipeline_status_returns_none_when_executable() {
        // Verified / Unverified pipelines produced a runnable binary, so the
        // per-file status is decided by the process exit code, not the pipeline.
        assert_eq!(classify_pipeline_status(&BuildStatus::Verified), None);
        assert_eq!(classify_pipeline_status(&BuildStatus::Unverified), None);
    }

    #[test]
    fn select_tests_accepts_file_and_sorted_test_names() {
        let dir = TempDir::new().unwrap();
        let single = write_source(&dir, "plain.vow", "module Plain fn main() -> i32 { 0 }");
        assert_eq!(
            select_tests(&single, None, None).files,
            vec![single.clone()]
        );

        write_source(&dir, "notes.vow", "module Notes");
        let beta = write_source(&dir, "beta_test.vow", "module Beta");
        let alpha = write_source(&dir, "test_alpha.vow", "module Alpha");
        assert_eq!(
            select_tests(dir.path(), None, None).files,
            vec![beta, alpha]
        );
    }

    #[test]
    fn select_tests_recurses_into_subdirectories() {
        let dir = TempDir::new().unwrap();
        let top = write_source(&dir, "test_top.vow", "module Top");
        let nested_dir = dir.path().join("tests");
        std::fs::create_dir(&nested_dir).unwrap();
        let nested = nested_dir.join("test_nested.vow");
        std::fs::write(&nested, "module Nested").unwrap();
        // Non-test files in the subdir must be skipped, like at top level.
        std::fs::write(nested_dir.join("helper.vow"), "module Helper").unwrap();

        let files = select_tests(dir.path(), None, None).files;
        // Lexicographic sort on the full path: "test_top.vow" < "tests/test_nested.vow"
        // because '_' (0x5F) sorts before 's' (0x73). Tests rely on stable ordering,
        // so anchor the expected sequence to the observed lexicographic rule.
        assert_eq!(files, vec![top, nested]);
    }

    #[test]
    fn select_tests_skips_symlink_entries() {
        // DirEntry::file_type() does not follow symlinks, so both symlinked
        // files and symlinked dirs are silently skipped. The self-hosted side
        // matches via __vow_fs_is_symlink. Verify the Rust behaviour stays
        // pinned so the two compilers can't drift.
        let dir = TempDir::new().unwrap();
        let real_test = write_source(&dir, "test_real.vow", "module Real");

        // Symlink to a regular .vow file outside the scan tree — must be skipped.
        let external = TempDir::new().unwrap();
        let external_target = external.path().join("test_external.vow");
        std::fs::write(&external_target, "module External").unwrap();
        let symlinked_file = dir.path().join("test_symlink.vow");
        std::os::unix::fs::symlink(&external_target, &symlinked_file).unwrap();

        // Symlink to a directory — its contents must not be recursed into.
        let external_dir = external.path().join("nested");
        std::fs::create_dir(&external_dir).unwrap();
        std::fs::write(
            external_dir.join("test_inside_symlink.vow"),
            "module Inside",
        )
        .unwrap();
        let symlinked_dir = dir.path().join("subdir_symlink");
        std::os::unix::fs::symlink(&external_dir, &symlinked_dir).unwrap();

        let files = select_tests(dir.path(), None, None).files;
        assert_eq!(files, vec![real_test]);
    }

    #[test]
    fn select_tests_filter_keeps_stem_substring_matches() {
        let dir = TempDir::new().unwrap();
        let lexer = write_source(&dir, "test_lexer.vow", "module Lexer");
        write_source(&dir, "test_parser.vow", "module Parser");
        let lexer_extra = write_source(&dir, "test_lexer_extra.vow", "module LexerExtra");

        assert_eq!(
            select_tests(dir.path(), Some("lex"), None).files,
            vec![lexer, lexer_extra]
        );
    }

    #[test]
    fn select_tests_filter_ignores_extension_and_parent_directory() {
        let dir = TempDir::new().unwrap();
        let suite = dir.path().join("suite");
        std::fs::create_dir(&suite).unwrap();
        let alpha = suite.join("test_alpha.vow");
        std::fs::write(&alpha, "module Alpha").unwrap();
        std::fs::write(suite.join("test_beta.vow"), "module Beta").unwrap();

        assert!(select_tests(dir.path(), Some("vow"), None).files.is_empty());
        assert!(
            select_tests(dir.path(), Some("suite"), None)
                .files
                .is_empty()
        );
        assert_eq!(
            select_tests(dir.path(), Some("alpha"), None).files,
            vec![alpha]
        );
    }

    #[test]
    fn select_tests_empty_filter_keeps_everything() {
        let dir = TempDir::new().unwrap();
        let a = write_source(&dir, "test_a.vow", "module A");
        let keep = write_source(&dir, "keep_test.vow", "module Keep");
        assert_eq!(
            select_tests(dir.path(), Some(""), None).files,
            vec![keep, a]
        );
    }

    #[test]
    fn count_contract_density_ignores_main_and_reports_tenths() {
        use vow_ir::{BasicBlock, BlockId, FuncId, RegionSummary, Ty, VowEntry, VowId};

        let make_func = |id, name: &str, vows| vow_ir::Function {
            id: FuncId(id),
            name: name.to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Unit,
            effects: vec![],
            vows,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let vowed = VowEntry {
            id: VowId(0),
            description: "ensures: true".to_string(),
            blame: vow_diag::Blame::Callee,
            bindings: vec![],
            file: "test.vow".to_string(),
            offset: 0,
        };
        let module = vow_ir::Module {
            name: "Density".to_string(),
            functions: vec![
                make_func(0, "main", vec![vowed.clone()]),
                make_func(1, "with_vow", vec![vowed]),
                make_func(2, "without_vow", vec![]),
            ],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };

        let density = count_contract_density(&module);
        assert_eq!(density.functions_total, 2);
        assert_eq!(density.functions_with_vows, 1);
        assert_eq!(density.density_pct, 50.0);
    }

    // ---- Test-run result assembly (build_test_result) ----

    fn test_entry(status: &str) -> TestEntry {
        TestEntry {
            file: "t.vow".to_string(),
            name: "t".to_string(),
            status: status.to_string(),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 0,
            diagnostics: vec![],
            counterexamples: vec![],
        }
    }

    fn density(functions_total: usize, functions_with_vows: usize) -> ContractDensity {
        ContractDensity {
            functions_total,
            functions_with_vows,
            density_pct: 0.0,
        }
    }

    #[test]
    fn build_test_result_tallies_passed_and_skipped() {
        let result = build_test_result(
            vec![
                test_entry("passed"),
                test_entry("passed"),
                test_entry("skipped"),
            ],
            density(0, 0),
        );
        assert_eq!(result.status, "TestsPassed");
        assert_eq!(result.total, 3);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn build_test_result_fails_closed_on_each_failure_status() {
        // Every one of these per-file statuses must count as a failure and flip
        // the overall status to TestsFailed (fail-closed, #386). `skipped` and
        // `passed` must NOT count toward `failed`.
        for status in [
            "failed",
            "compile_error",
            "verify_failed",
            "contract_skipped",
        ] {
            let result = build_test_result(
                vec![test_entry("passed"), test_entry(status)],
                density(0, 0),
            );
            assert_eq!(result.status, "TestsFailed", "status={status}");
            assert_eq!(result.failed, 1, "status={status}");
            assert_eq!(result.passed, 1, "status={status}");
            assert_eq!(result.skipped, 0, "status={status}");
        }
    }

    #[test]
    fn build_test_result_finalizes_density_pct() {
        // Percentage truncated to one decimal via integer math. Expected values
        // are hand-derived from the ratio, independent of the implementation.
        let close =
            |got: f64, want: f64| assert!((got - want).abs() < 1e-9, "got {got}, want {want}");
        // 1/3 = 33.33..% -> 33.3
        close(
            build_test_result(vec![], density(3, 1))
                .contract_density
                .density_pct,
            33.3,
        );
        // 2/3 = 66.66..% -> 66.6
        close(
            build_test_result(vec![], density(3, 2))
                .contract_density
                .density_pct,
            66.6,
        );
        // 1/2 = 50.0%
        close(
            build_test_result(vec![], density(2, 1))
                .contract_density
                .density_pct,
            50.0,
        );
        // All vowed -> 100.0%
        close(
            build_test_result(vec![], density(4, 4))
                .contract_density
                .density_pct,
            100.0,
        );
        // No functions -> 0.0 (no divide-by-zero).
        close(
            build_test_result(vec![], density(0, 0))
                .contract_density
                .density_pct,
            0.0,
        );
    }
}
