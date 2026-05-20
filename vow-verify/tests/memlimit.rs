use std::path::Path;

use vow_verify::{
    Encoding, Solver, SolverConfig, VerificationResult, find_esbmc, run_esbmc_with_max_k_step,
};

fn sh_quote(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\'', "'\\''");
    format!("'{s}'")
}

fn synthetic_solver_pressure_c_source(vars: usize) -> String {
    let mut c = String::from(
        "extern _Bool __VERIFIER_nondet_bool(void);\n\
         int main(void) {\n",
    );
    for i in 0..vars {
        c.push_str(&format!("  _Bool b{i} = __VERIFIER_nondet_bool();\n"));
    }
    c.push_str("  __ESBMC_assert((");
    for i in 0..vars {
        if i > 0 {
            c.push_str(") && (");
        }
        c.push_str(&format!("b{i} || !b{i}"));
    }
    c.push_str("), \"tautology\");\n  return 0;\n}\n");
    c
}

#[cfg(unix)]
#[test]
fn memlimit_probe_bounds_real_esbmc_rss_when_enabled() {
    if std::env::var("VOW_VERIFY_RUN_MEMLIMIT_RSS").as_deref() != Ok("1") {
        eprintln!("SKIP: set VOW_VERIFY_RUN_MEMLIMIT_RSS=1 to run the real ESBMC RSS probe");
        return;
    }

    let real_esbmc = match find_esbmc() {
        Some(path) => path,
        None => {
            eprintln!("SKIP: esbmc not found");
            return;
        }
    };
    let time = Path::new("/usr/bin/time");
    if !time.is_file() {
        eprintln!("SKIP: /usr/bin/time not found");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let rss_path = dir.path().join("max-rss-kb.txt");
    let wrapper = dir.path().join("esbmc-with-rss");
    let script = format!(
        "#!/bin/sh\nexec {} -f '%M' -o {} {} \"$@\"\n",
        sh_quote(time),
        sh_quote(&rss_path),
        sh_quote(&real_esbmc)
    );
    std::fs::write(&wrapper, script).expect("write wrapper");

    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&wrapper).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&wrapper, perms).expect("chmod wrapper");

    let memlimit_mb = 128;
    let config = SolverConfig {
        solver: Solver::Boolector,
        encoding: Encoding::Bv,
        timeout_secs: Some(60),
        memlimit_mb: Some(memlimit_mb),
    };
    let result = run_esbmc_with_max_k_step(
        &wrapper,
        &synthetic_solver_pressure_c_source(4096),
        5,
        "memlimit_probe",
        &config,
    );

    match &result {
        VerificationResult::Unknown { reason } => {
            if reason == "memory limit exceeded" {
                // This is the expected result when the synthetic case hits the cap.
            }
        }
        VerificationResult::Proven
        | VerificationResult::Failed(_)
        | VerificationResult::ProvenIr => {
            // Accept successful structured outcomes: this env-gated probe is an
            // RSS sanity check, not the deterministic memory-limit classifier.
        }
        other => panic!("expected a structured verifier result, got {other:?}"),
    }

    let rss_kb: u64 = std::fs::read_to_string(&rss_path)
        .expect("rss file")
        .trim()
        .parse()
        .expect("rss kb");
    // Sanity bound, not proof that this specific synthetic case always hits
    // the cap: some ESBMC/solver versions discharge the tautology cheaply. The
    // deterministic unit tests cover flag wiring and OOM classification; this
    // env-gated probe checks that a capped real invocation returns a structured
    // result and does not escape by many hundreds of MiB.
    let allowed_kb = u64::from(memlimit_mb + 512) * 1024;
    assert!(
        rss_kb <= allowed_kb,
        "ESBMC max RSS {rss_kb} KiB exceeded capped run allowance {allowed_kb} KiB"
    );
}
