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

/// Map a test binary's outcome to its per-file test status. `Some(0)` passed,
/// any other exit code failed, and so did a binary killed by a signal (no exit
/// code); only a process killed at the timeout deadline is `timeout`.
fn classify_execution_outcome(exit_code: Option<i32>, timed_out: bool) -> &'static str {
    match (timed_out, exit_code) {
        (true, _) => "timeout",
        (false, Some(0)) => "passed",
        (false, _) => "failed",
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
    let module_root = module_root_override.map(Path::to_path_buf).or_else(|| {
        if scan_path.is_dir() {
            Some(scan_path.to_path_buf())
        } else {
            infer_single_file_root(scan_path)
        }
    });
    TestSelection { files, module_root }
}

fn infer_single_file_root(file: &Path) -> Option<PathBuf> {
    let src = std::fs::read_to_string(file).ok()?;
    let (module, _) = vow_syntax::parser::parse_module(&src, &file.to_string_lossy());
    let uses: Vec<Vec<String>> = module.uses.into_iter().map(|u| u.path).collect();
    crate::module_loader::infer_module_root(file, &uses, |p| p.exists())
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
    test_workers: usize,
) {
    if !path.exists() {
        let result = missing_path_result(path);
        println!("{}", serde_json::to_string(&result).unwrap());
        eprintln!("error: test path '{}' does not exist", path.display());
        std::process::exit(1);
    }

    let test_result = run_selected(
        path,
        filter,
        module_root_override,
        &RunSettings::new(verify, mode, timeout_ms, limits, jobs),
        test_workers,
        &machine_under_pressure,
    );

    let json = serde_json::to_string(&test_result).expect("TestResult must be serializable");
    println!("{json}");

    if test_result.failed > 0 {
        std::process::exit(1);
    }
}

/// The result for a test path that does not exist: one synthetic `failed`
/// entry (naming the path) so the run is never mistaken for a pass.
fn missing_path_result(path: &Path) -> TestResult {
    let file = path.to_string_lossy().into_owned();
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.clone());
    let entry = TestEntry {
        file,
        name,
        status: "failed".to_string(),
        exit_code: None,
        stdout: String::new(),
        stderr: format!("test path '{}' does not exist", path.display()),
        duration_ms: 0,
        diagnostics: vec![],
        counterexamples: vec![],
    };
    build_test_result(
        vec![entry],
        ContractDensity {
            functions_total: 0,
            functions_with_vows: 0,
            density_pct: 0.0,
        },
    )
}

struct RunSettings<'a> {
    verify: bool,
    mode: BuildMode,
    timeout_ms: u64,
    limits: &'a VerifyLimits,
    verify_jobs: usize,
}

impl<'a> RunSettings<'a> {
    fn new(
        verify: bool,
        mode: BuildMode,
        timeout_ms: u64,
        limits: &'a VerifyLimits,
        verify_jobs: usize,
    ) -> Self {
        RunSettings {
            verify,
            mode,
            timeout_ms,
            limits,
            verify_jobs,
        }
    }
}

/// Select, run and aggregate the tests under `path` into one [`TestResult`].
/// Pure with respect to the process: printing and the exit code are the
/// caller's job.
fn run_selected(
    path: &Path,
    filter: Option<&str>,
    module_root_override: Option<&Path>,
    settings: &RunSettings,
    test_workers: usize,
    under_pressure: &(dyn Fn() -> bool + Sync),
) -> TestResult {
    let selection = select_tests(path, filter, module_root_override);
    let module_root = selection.module_root.as_deref();

    let mut total_density = ContractDensity {
        functions_total: 0,
        functions_with_vows: 0,
        density_pct: 0.0,
    };

    let cfg = RunConfig {
        module_root,
        verify: settings.verify,
        mode: settings.mode,
        timeout_ms: settings.timeout_ms,
        limits: settings.limits,
        verify_jobs: settings.verify_jobs,
    };
    let workers = test_workers.max(1).min(selection.files.len().max(1));
    let results = run_all(&selection.files, &cfg, workers, under_pressure);
    let mut entries = Vec::with_capacity(results.len());
    for (entry, density) in results {
        total_density.functions_total += density.functions_total;
        total_density.functions_with_vows += density.functions_with_vows;
        entries.push(entry);
    }

    build_test_result(entries, total_density)
}

struct RunConfig<'a> {
    module_root: Option<&'a Path>,
    verify: bool,
    mode: BuildMode,
    timeout_ms: u64,
    limits: &'a VerifyLimits,
    verify_jobs: usize,
}

/// Run `files` with up to `workers` concurrent workers, returning the per-file
/// results in `files` order whatever the completion order. Extra workers
/// (beyond the first) only claim a file while the machine is not under memory
/// or IO stall pressure; worker 0 always makes progress.
fn run_all(
    files: &[PathBuf],
    cfg: &RunConfig,
    workers: usize,
    under_pressure: &(dyn Fn() -> bool + Sync),
) -> Vec<(TestEntry, ContractDensity)> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<(TestEntry, ContractDensity)>>> =
        files.iter().map(|_| Mutex::new(None)).collect();
    let work = |worker: usize| loop {
        if worker > 0 {
            while under_pressure() && next.load(Ordering::SeqCst) < files.len() {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        let i = next.fetch_add(1, Ordering::SeqCst);
        let Some(file) = files.get(i) else { break };
        let result = run_one_test(file, i, cfg);
        *slots[i].lock().unwrap() = Some(result);
    };
    if workers <= 1 {
        work(0);
    } else {
        std::thread::scope(|scope| {
            for worker in 0..workers {
                let work = &work;
                scope.spawn(move || work(worker));
            }
        });
    }
    slots
        .into_iter()
        .map(|slot| slot.into_inner().unwrap().expect("every file was run"))
        .collect()
}

/// The `avg10` figure on the PSI line starting with `kind` (`some` or `full`)
/// in `text`; `None` when absent (no `/proc/pressure`).
fn psi_avg10(text: &str, kind: &str) -> Option<f64> {
    let line = text.lines().find(|l| l.split(' ').next() == Some(kind))?;
    line.split(' ')
        .find_map(|f| f.strip_prefix("avg10="))?
        .parse()
        .ok()
}

/// Whether memory (`some`) or IO (`full`) stall pressure is high enough that
/// another concurrent worker would make things worse.
fn pressure_is_high(memory_psi: &str, io_psi: &str) -> bool {
    let mem = psi_avg10(memory_psi, "some").unwrap_or(0.0);
    let io = psi_avg10(io_psi, "full").unwrap_or(0.0);
    mem >= 20.0 || io >= 20.0
}

fn machine_under_pressure() -> bool {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    pressure_is_high(&read("/proc/pressure/memory"), &read("/proc/pressure/io"))
}

/// The entry for a file whose binary never ran (it failed to compile, verify,
/// or produce an executable): no exit code and no captured output.
fn unexecuted_entry(
    file: String,
    name: String,
    status: &str,
    start: std::time::Instant,
    diagnostics: Vec<DiagnosticJson>,
    counterexamples: Vec<CounterexampleJson>,
) -> TestEntry {
    TestEntry {
        file,
        name,
        status: status.to_string(),
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        duration_ms: start.elapsed().as_millis() as u64,
        diagnostics,
        counterexamples,
    }
}

fn run_one_test(test_file: &Path, index: usize, cfg: &RunConfig) -> (TestEntry, ContractDensity) {
    let module_root = cfg.module_root;
    let no_density = ContractDensity {
        functions_total: 0,
        functions_with_vows: 0,
        density_pct: 0.0,
    };
    let start = std::time::Instant::now();
    let _ = std::fs::create_dir_all("build");
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
            let entry =
                unexecuted_entry(file_str, name, "compile_error", start, diagnostics, vec![]);
            return (entry, no_density);
        }
    };

    let density = count_contract_density(
        frontend
            .ir()
            .expect("LoweredIr goal must produce IR for test density"),
    );

    let tmp_out =
        Path::new("build").join(format!("vow_test_{name}_{}_{index}", std::process::id()));
    let result = run_pipeline_from_frontend(
        frontend,
        test_file,
        Some(&tmp_out),
        cfg.mode,
        !cfg.verify,
        false,
        TraceMode::Off,
        true,
        cfg.limits,
        cfg.verify_jobs,
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
    // A successful pipeline without an executable cannot be run either.
    let exe_path = match (classify_pipeline_status(&result.status), &result.executable) {
        (None, Some(exe)) => exe.clone(),
        (terminal, _) => {
            let status = terminal.unwrap_or("compile_error");
            let entry =
                unexecuted_entry(file_str, name, status, start, diagnostics, counterexamples);
            return (entry, density);
        }
    };

    // Execute with a timeout
    let exe_abs = std::fs::canonicalize(&exe_path).unwrap_or(exe_path.clone());
    let child = std::process::Command::new(&exe_abs)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let (exit_code, timed_out, stdout_str, stderr_str) = match child {
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

            let timeout = std::time::Duration::from_millis(cfg.timeout_ms);
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
                Some(code) => (code, false, stdout, stderr),
                None => (None, true, String::new(), "timeout".to_string()),
            }
        }
        Err(e) => (Some(-1), false, String::new(), e.to_string()),
    };

    // Clean up the produced binary
    let _ = std::fs::remove_file(&exe_path);

    let status = classify_execution_outcome(exit_code, timed_out);

    let entry = TestEntry {
        file: file_str,
        name,
        status: status.to_string(),
        exit_code,
        stdout: stdout_str,
        stderr: stderr_str,
        duration_ms: start.elapsed().as_millis() as u64,
        diagnostics,
        counterexamples,
    };
    (entry, density)
}

/// Assemble the final [`TestResult`] from the collected per-file `entries` and
/// the accumulated contract `density`. Pure: no IO, no process exit — the
/// orchestration in [`run_test_command`] handles printing and the exit code.
///
/// Finalizes `density_pct` (integer math matching the self-hosted compiler),
/// tallies pass/fail/skip, and gates the overall status. A file counts as
/// `failed` when its status is any of `failed`, `compile_error`,
/// `verify_failed`, `contract_skipped`, or `timeout` — all fail-closed (#386,
/// #415).
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
                "failed" | "compile_error" | "verify_failed" | "contract_skipped" | "timeout"
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
    fn single_file_selection_infers_the_ancestor_module_root() {
        let dir = TempDir::new().unwrap();
        write_source(&dir, "dep.vow", "module Dep fn d() -> i64 { 1 }");
        let tests_dir = dir.path().join("tests");
        std::fs::create_dir(&tests_dir).unwrap();
        let file = tests_dir.join("test_a.vow");
        std::fs::write(&file, "module A use dep fn main() -> i32 { 0 }").unwrap();

        assert_eq!(
            select_tests(&file, None, None).module_root,
            Some(dir.path().to_path_buf())
        );

        let explicit = dir.path().join("elsewhere");
        assert_eq!(
            select_tests(&file, None, Some(&explicit)).module_root,
            Some(explicit)
        );

        let local = tests_dir.join("test_b.vow");
        std::fs::write(&local, "module B fn main() -> i32 { 0 }").unwrap();
        assert_eq!(select_tests(&local, None, None).module_root, None);
    }

    #[test]
    fn psi_avg10_reads_the_requested_line() {
        let psi = "some avg10=12.50 avg60=3.00 avg300=1.00 total=42\n\
                   full avg10=0.75 avg60=0.00 avg300=0.00 total=7\n";
        assert_eq!(psi_avg10(psi, "some"), Some(12.5));
        assert_eq!(psi_avg10(psi, "full"), Some(0.75));
        assert_eq!(psi_avg10("", "some"), None);
        assert_eq!(psi_avg10("some avg60=1.00\n", "some"), None);
    }

    #[test]
    fn run_all_preserves_file_order_for_any_worker_count() {
        let dir = TempDir::new().unwrap();
        let files: Vec<PathBuf> = ["test_c", "test_a", "test_b"]
            .iter()
            .map(|n| write_source(&dir, &format!("{n}.vow"), "module M fn main() -> i32 { 0 }"))
            .collect();
        let limits = VerifyLimits::default();
        let cfg = RunConfig {
            module_root: None,
            verify: false,
            mode: BuildMode::Debug,
            timeout_ms: 30_000,
            limits: &limits,
            verify_jobs: 1,
        };
        for workers in [1, 3] {
            let results = run_all(&files, &cfg, workers, &|| false);
            let names: Vec<&str> = results.iter().map(|(e, _)| e.name.as_str()).collect();
            assert_eq!(names, ["test_c", "test_a", "test_b"], "workers={workers}");
            assert!(results.iter().all(|(e, _)| e.status == "passed"));
        }
    }

    fn run_single(path: &Path, verify: bool, timeout_ms: u64) -> (TestEntry, ContractDensity) {
        let limits = VerifyLimits::default();
        let cfg = RunConfig {
            module_root: None,
            verify,
            mode: BuildMode::Debug,
            timeout_ms,
            limits: &limits,
            verify_jobs: 1,
        };
        run_one_test(path, 0, &cfg)
    }

    #[test]
    fn run_one_test_reports_every_execution_outcome() {
        let dir = TempDir::new().unwrap();
        let broken = write_source(&dir, "test_broken.vow", "module B use nothing");
        let (entry, density) = run_single(&broken, false, 30_000);
        assert_eq!(entry.status, "compile_error");
        assert!(!entry.diagnostics.is_empty());
        assert_eq!(density.functions_total, 0);

        let failing = write_source(&dir, "test_failing.vow", "module F fn main() -> i32 { 3 }");
        let (entry, _) = run_single(&failing, false, 30_000);
        assert_eq!(entry.status, "failed");
        assert_eq!(entry.exit_code, Some(3));

        let hanging = write_source(
            &dir,
            "test_hanging.vow",
            "module H fn main() -> i32 { let mut i: i64 = 0; while true { i = i + 1; } 0 }",
        );
        let (entry, _) = run_single(&hanging, false, 300);
        assert_eq!(entry.status, "timeout");
        assert_eq!(entry.exit_code, None);
        assert_eq!(entry.stderr, "timeout");
    }

    #[test]
    fn run_one_test_with_verify_stops_on_a_proved_violation() {
        if vow_verify::find_esbmc().is_none() {
            return;
        }
        let dir = TempDir::new().unwrap();
        let wrong = write_source(
            &dir,
            "test_wrong.vow",
            "module W fn f(x: i64) -> i64 vow { ensures: result > x } { x } \
             fn main() -> i32 { 0 }",
        );
        let (entry, density) = run_single(&wrong, true, 30_000);
        assert_eq!(entry.status, "verify_failed");
        assert_eq!(entry.exit_code, None);
        assert_eq!(density.functions_with_vows, 1);
    }

    #[test]
    fn pressure_is_high_reads_memory_some_and_io_full() {
        let low =
            "some avg10=1.00 avg60=0 avg300=0 total=0\nfull avg10=0.00 avg60=0 avg300=0 total=0";
        let high_mem =
            "some avg10=20.00 avg60=0 avg300=0 total=0\nfull avg10=0.00 avg60=0 avg300=0 total=0";
        let high_io =
            "some avg10=0.00 avg60=0 avg300=0 total=0\nfull avg10=35.5 avg60=0 avg300=0 total=0";
        assert!(!pressure_is_high(low, low));
        assert!(pressure_is_high(high_mem, low));
        assert!(pressure_is_high(low, high_io));
        assert!(!pressure_is_high("", ""));
        // Smoke: reading the real /proc/pressure files never panics.
        let _ = machine_under_pressure();
    }

    #[test]
    fn run_selected_aggregates_a_directory_scan() {
        let dir = TempDir::new().unwrap();
        write_source(&dir, "test_ok.vow", "module O fn main() -> i32 { 0 }");
        write_source(&dir, "test_bad.vow", "module B fn main() -> i32 { 1 }");
        let limits = VerifyLimits::default();
        let settings = RunSettings::new(false, BuildMode::Debug, 30_000, &limits, 1);
        let result = run_selected(dir.path(), None, None, &settings, 4, &|| false);
        assert_eq!(result.status, "TestsFailed");
        assert_eq!((result.total, result.passed, result.failed), (2, 1, 1));
        let names: Vec<&str> = result.tests.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["test_bad", "test_ok"]);

        let filtered = run_selected(dir.path(), Some("ok"), None, &settings, 1, &|| false);
        assert_eq!(filtered.status, "TestsPassed");
        assert_eq!(filtered.total, 1);
    }

    #[test]
    fn run_all_backs_off_while_the_machine_is_under_pressure() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = TempDir::new().unwrap();
        let files: Vec<PathBuf> = ["test_a", "test_b", "test_c"]
            .iter()
            .map(|n| write_source(&dir, &format!("{n}.vow"), "module M fn main() -> i32 { 0 }"))
            .collect();
        let limits = VerifyLimits::default();
        let cfg = RunConfig {
            module_root: None,
            verify: false,
            mode: BuildMode::Debug,
            timeout_ms: 30_000,
            limits: &limits,
            verify_jobs: 1,
        };
        let polls = AtomicUsize::new(0);
        let pressure = || polls.fetch_add(1, Ordering::SeqCst) < 2;
        let results = run_all(&files, &cfg, 2, &pressure);
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(e, _)| e.status == "passed"));
        assert!(polls.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn classify_execution_outcome_maps_exit_codes() {
        assert_eq!(classify_execution_outcome(Some(0), false), "passed");
        assert_eq!(classify_execution_outcome(Some(1), false), "failed");
        assert_eq!(classify_execution_outcome(Some(-1), false), "failed");
        assert_eq!(classify_execution_outcome(None, true), "timeout");
    }

    #[cfg(unix)]
    #[test]
    fn a_signal_death_is_a_failure_not_a_timeout() {
        use std::os::unix::process::ExitStatusExt;

        // Raw wait status for death by SIGABRT: no exit code at all.
        let aborted = std::process::ExitStatus::from_raw(6);
        assert_eq!(aborted.code(), None);
        assert_eq!(classify_execution_outcome(aborted.code(), false), "failed");
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
            "timeout",
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
    fn missing_path_result_is_one_failed_entry_naming_the_path() {
        let result = missing_path_result(Path::new("no/such/dir/t.vow"));
        assert_eq!(result.status, "TestsFailed");
        assert_eq!(result.total, 1);
        assert_eq!(result.tests.len(), 1);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 1);
        assert_eq!(result.skipped, 0);
        let entry = &result.tests[0];
        assert_eq!(entry.status, "failed");
        assert_eq!(entry.file, "no/such/dir/t.vow");
        assert_eq!(entry.name, "t");
        assert_eq!(entry.exit_code, None);
        assert!(
            entry.stderr.contains("no/such/dir/t.vow"),
            "stderr={}",
            entry.stderr
        );
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
