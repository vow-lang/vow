//! Synchronous verification driver.
//!
//! Runs ESBMC over every vowed function in a module and reduces the
//! per-function verdicts to a single [`VerifyOutcome`] plus the list of
//! [`SkippedFunction`]s. The concurrency core — worker pool, early-halt, and
//! lowest-index deterministic reporting — lives in [`run_pool`], which takes the
//! per-function verdict as an injected closure. That seam lets the pool logic be
//! tested without invoking ESBMC; [`verify_one_function`] is the real verdict.
//!
//! Terminal outcomes are turned into a `BuildOutput` by `verify_outcome`, the
//! sibling module that owns that translation.

use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use vow_verify::{
    ConstantValue, Encoding, Solver, SolverConfig, VerificationResult, VerifyLimits, VerifyRequest,
    detect_constant_functions, emit_verify_c_source, find_esbmc, non_modelable_reason,
    run_with_fallback, verify,
};

use crate::cache::VerifyCache;
use crate::verify_outcome::{SkippedFunction, VerifyOutcome};
use crate::{counterexample, perfetto};

/// Per-function verdict: continue, skip-with-warning, or halt.
enum PerFuncResult {
    Ok,
    Skipped(SkippedFunction),
    Halt(VerifyOutcome),
}

/// A raw verifier result either needs counterexample enrichment or is already
/// a complete per-function verdict.
enum VerificationDisposition {
    Counterexample(vow_verify::Counterexample),
    Complete(PerFuncResult),
}

fn classify_verification_result(
    function: &str,
    result: VerificationResult,
) -> VerificationDisposition {
    let complete = |result| VerificationDisposition::Complete(result);
    match result {
        VerificationResult::Failed(ce) => VerificationDisposition::Counterexample(ce),
        VerificationResult::ToolError(message) => {
            complete(PerFuncResult::Halt(VerifyOutcome::Error {
                function: function.to_string(),
                message,
            }))
        }
        VerificationResult::Timeout => complete(PerFuncResult::Halt(VerifyOutcome::Timeout {
            function: function.to_string(),
        })),
        VerificationResult::Unknown { reason } => {
            complete(PerFuncResult::Halt(VerifyOutcome::Unknown {
                function: function.to_string(),
                reason,
            }))
        }
        VerificationResult::Proven | VerificationResult::ProvenIr => complete(PerFuncResult::Ok),
        VerificationResult::ToolNotFound => {
            complete(PerFuncResult::Halt(VerifyOutcome::ToolNotFound))
        }
        VerificationResult::Skipped { reason } => {
            complete(PerFuncResult::Skipped(SkippedFunction {
                function: function.to_string(),
                reason,
            }))
        }
    }
}

/// Verify one vowed function with ESBMC, folding the raw
/// [`VerificationResult`] into a [`PerFuncResult`].
///
/// Thread-safe: `verify_cache` writes are content-addressed.
#[allow(clippy::too_many_arguments)]
fn verify_one_function(
    func: &vow_ir::Function,
    ir_module: &vow_ir::Module,
    const_fns: &std::collections::HashMap<vow_ir::FuncId, ConstantValue>,
    file: &str,
    call_site_index: &std::collections::HashMap<String, Vec<counterexample::CallSiteInfo>>,
    verify_cache: Option<&VerifyCache>,
    limits: &VerifyLimits,
    config: &SolverConfig,
) -> PerFuncResult {
    // Non-modelable vowed functions must be skipped here; the C emitter would emit __ESBMC_assert(0) traps for them.
    if let Some(reason) = non_modelable_reason(func, ir_module, const_fns) {
        return PerFuncResult::Skipped(SkippedFunction {
            function: func.name.clone(),
            reason,
        });
    }

    // Resolve Auto solver via heuristic (Phase B).
    // Skip heuristic when encoding is Ir — that forces Z3 via resolve().
    let func_config = if config.solver == Solver::Auto && config.encoding != Encoding::Ir {
        let heuristic = vow_verify::classify_function(func);
        SolverConfig {
            solver: heuristic.solver,
            encoding: config.encoding,
            timeout_secs: config.timeout_secs,
            memlimit_mb: config.memlimit_mb,
        }
    } else {
        *config
    };

    let result = if let Some(vc) = verify_cache {
        let c_src = emit_verify_c_source(func, ir_module, const_fns, limits);

        // Lookup under the pre-fallback `func_config`; only FAILED entries come
        // back (a forged PROVEN file is discarded inside the cache, so it can
        // never bypass ESBMC).
        if let Some(ce) = vc.lookup_failure(&c_src, limits.max_k_step, &func_config) {
            VerificationResult::Failed(ce)
        } else {
            let esbmc = match find_esbmc() {
                Some(p) => p,
                None => return PerFuncResult::Halt(VerifyOutcome::ToolNotFound),
            };
            let (res, resolved_config) =
                run_with_fallback(&esbmc, &c_src, limits.max_k_step, &func.name, &func_config);
            // Store under the config that actually produced the result. Only a
            // Counterexample can be handed to `store_failure`, so a PROVEN
            // result is structurally uncacheable and can never bypass ESBMC on a
            // later run.
            if let VerificationResult::Failed(ce) = &res {
                vc.store_failure(&c_src, limits.max_k_step, &resolved_config, ce);
            }
            res
        }
    } else {
        verify(&VerifyRequest {
            const_fns: Some(const_fns),
            config: Some(&func_config),
            ..VerifyRequest::new(func, ir_module, limits)
        })
    };

    match classify_verification_result(&func.name, result) {
        VerificationDisposition::Counterexample(ce) => {
            let sce = counterexample::build_structured_counterexample_with_module(
                func,
                Some(ir_module),
                &ce,
                file,
                call_site_index,
            );
            PerFuncResult::Halt(VerifyOutcome::Failed {
                function: func.name.clone(),
                description: ce.description.clone(),
                counterexamples: vec![sce],
            })
        }
        VerificationDisposition::Complete(result) => result,
    }
}

/// Record one per-function ESBMC proof as a span on its worker thread track,
/// plus a flow arrow from the verification handoff origin to the proof start.
/// No-op when profiling is off.
fn record_proof_span(
    prof: Option<&perfetto::Profiler>,
    verify_start: u64,
    idx: usize,
    name: &str,
    tid: u64,
    start_us: u64,
) {
    let Some(p) = prof else { return };
    p.flow(
        "verify->esbmc",
        perfetto::PID_COMPILER,
        perfetto::TID_VERIFY_DRIVER,
        verify_start,
        idx as u64,
        perfetto::FlowEdge::Start,
    );
    p.flow(
        "verify->esbmc",
        perfetto::PID_COMPILER,
        tid,
        start_us,
        idx as u64,
        perfetto::FlowEdge::End,
    );
    p.span(
        &format!("esbmc:{name}"),
        perfetto::PID_COMPILER,
        tid,
        start_us,
        p.now_us().saturating_sub(start_us),
        vec![("function".to_string(), name.to_string())],
    );
}

/// Drive verification across a module's vowed functions.
///
/// Collects the vowed functions, then hands the index space to [`run_pool`]
/// with [`verify_one_function`] as the per-function verdict.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_verification_sync(
    ir_module: &vow_ir::Module,
    file: &str,
    call_site_index: &std::collections::HashMap<String, Vec<counterexample::CallSiteInfo>>,
    verify_cache: Option<&VerifyCache>,
    limits: &VerifyLimits,
    jobs: usize,
    config: &SolverConfig,
    prof: Option<&perfetto::Profiler>,
) -> (VerifyOutcome, Vec<SkippedFunction>) {
    let const_fns = detect_constant_functions(ir_module);

    let vowed: Vec<&vow_ir::Function> = ir_module
        .functions
        .iter()
        .filter(|f| !f.vows.is_empty())
        .collect();

    if vowed.is_empty() {
        return (VerifyOutcome::Proven, Vec::new());
    }

    // Handoff origin: a flow arrow runs from here to each per-function proof.
    let verify_start = prof.map(|p| p.now_us()).unwrap_or(0);

    let names: Vec<&str> = vowed.iter().map(|f| f.name.as_str()).collect();
    run_pool(jobs, prof, verify_start, &names, |idx| {
        verify_one_function(
            vowed[idx],
            ir_module,
            &const_fns,
            file,
            call_site_index,
            verify_cache,
            limits,
            config,
        )
    })
}

/// The pool concurrency core, independent of ESBMC.
///
/// `verdict(idx)` produces the [`PerFuncResult`] for function `idx`; in
/// production it is [`verify_one_function`], but any closure works, which is how
/// the determinism guarantees below are tested without a solver.
///
/// Returns the lowest-indexed halt-class outcome (deterministic regardless of
/// which worker finishes first), or `Proven`/`SkippedNonModelable` when nothing
/// halts. Skipped functions never halt — they are aggregated and reported as
/// warnings.
fn run_pool(
    jobs: usize,
    prof: Option<&perfetto::Profiler>,
    verify_start: u64,
    names: &[&str],
    verdict: impl Fn(usize) -> PerFuncResult + Sync,
) -> (VerifyOutcome, Vec<SkippedFunction>) {
    let count = names.len();
    let jobs = jobs.max(1).min(count.max(1));
    if jobs == 1 {
        // Serial path records proof spans on TID_VERIFY_DRIVER; the worker path
        // below uses TID_WORKER_BASE + w. This track divergence is deliberate —
        // do not unify the two paths.
        let mut skipped = Vec::new();
        for (idx, &name) in names.iter().enumerate() {
            let proof_start = prof.map(|p| p.now_us()).unwrap_or(0);
            let result = verdict(idx);
            record_proof_span(
                prof,
                verify_start,
                idx,
                name,
                perfetto::TID_VERIFY_DRIVER,
                proof_start,
            );
            match result {
                PerFuncResult::Ok => {}
                PerFuncResult::Skipped(s) => skipped.push(s),
                PerFuncResult::Halt(out) => return (out, skipped),
            }
        }
        if skipped.is_empty() {
            return (VerifyOutcome::Proven, skipped);
        }
        return (VerifyOutcome::SkippedNonModelable, skipped);
    }

    // Stop after first halt-class outcome (Failed/Error/Timeout/ToolNotFound);
    // return lowest-indexed halt for deterministic reporting. Skipped
    // functions never halt — they're aggregated and reported as warnings.
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let halts: StdMutex<Vec<Option<VerifyOutcome>>> =
        StdMutex::new((0..count).map(|_| None).collect());
    let skipped_acc: StdMutex<Vec<Option<SkippedFunction>>> =
        StdMutex::new((0..count).map(|_| None).collect());

    thread::scope(|scope| {
        for w in 0..jobs {
            let next = &next;
            let stop = &stop;
            let halts = &halts;
            let skipped_acc = &skipped_acc;
            let verdict = &verdict;
            let worker_tid = perfetto::TID_WORKER_BASE + w as u64;
            scope.spawn(move || {
                loop {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let idx = next.fetch_add(1, Ordering::AcqRel);
                    if idx >= count {
                        break;
                    }
                    // Always finish what we've claimed so `halts[idx]` reflects
                    // its true verdict — otherwise lowest-index halt reporting
                    // becomes timing-dependent. The pre-check already avoids claims
                    // in the common post-halt case.
                    let proof_start = prof.map(|p| p.now_us()).unwrap_or(0);
                    let result = verdict(idx);
                    record_proof_span(prof, verify_start, idx, names[idx], worker_tid, proof_start);
                    match result {
                        PerFuncResult::Ok => {}
                        PerFuncResult::Skipped(s) => {
                            let mut guard =
                                skipped_acc.lock().expect("verify skipped mutex poisoned");
                            guard[idx] = Some(s);
                        }
                        PerFuncResult::Halt(out) => {
                            let mut guard = halts.lock().expect("verify halts mutex poisoned");
                            guard[idx] = Some(out);
                            drop(guard);
                            // Release pairs with sibling threads' stop.load(Acquire) to propagate early-exit.
                            stop.store(true, Ordering::Release);
                        }
                    }
                }
            });
        }
    });

    let halts = halts.into_inner().expect("verify halts mutex poisoned");
    let outcome = halts.into_iter().flatten().next().unwrap_or_else(|| {
        if skipped_acc
            .lock()
            .expect("verify skipped mutex poisoned")
            .iter()
            .any(Option::is_some)
        {
            VerifyOutcome::SkippedNonModelable
        } else {
            VerifyOutcome::Proven
        }
    });
    let skipped: Vec<SkippedFunction> = skipped_acc
        .into_inner()
        .expect("verify skipped mutex poisoned")
        .into_iter()
        .flatten()
        .collect();
    (outcome, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, Condvar};

    fn halt(function: &str) -> PerFuncResult {
        PerFuncResult::Halt(VerifyOutcome::Failed {
            function: function.to_string(),
            description: String::new(),
            counterexamples: Vec::new(),
        })
    }

    fn skip(function: &str) -> PerFuncResult {
        PerFuncResult::Skipped(SkippedFunction {
            function: function.to_string(),
            reason: "non-modelable".to_string(),
        })
    }

    // Every function verifies: outcome is Proven, nothing skipped. jobs=4
    // exercises the worker pool without invoking ESBMC.
    #[test]
    fn run_pool_all_ok_is_proven() {
        let names = ["f0", "f1", "f2", "f3"];
        let (outcome, skipped) = run_pool(4, None, 0, &names, |_| PerFuncResult::Ok);
        assert!(matches!(outcome, VerifyOutcome::Proven));
        assert!(skipped.is_empty());
    }

    // Two functions halt; force f5's fake verdict to complete before f2's while
    // both are in flight. The pool must still report the lower index.
    #[test]
    fn run_pool_reports_lowest_index_halt() {
        let names = ["f0", "f1", "f2", "f3", "f4", "f5"];
        let both_halts_started = Barrier::new(2);
        let higher_halt_completed = Condvar::new();
        let completion = StdMutex::new((false, Vec::new()));
        let (outcome, _) = run_pool(6, None, 0, &names, |idx| match idx {
            2 => {
                let mut completion = completion.lock().unwrap();
                both_halts_started.wait();
                while !completion.0 {
                    completion = higher_halt_completed.wait(completion).unwrap();
                }
                completion.1.push(2);
                drop(completion);
                halt("f2")
            }
            5 => {
                let result = halt("f5");
                both_halts_started.wait();
                let mut completion = completion.lock().unwrap();
                completion.1.push(5);
                completion.0 = true;
                higher_halt_completed.notify_one();
                result
            }
            _ => PerFuncResult::Ok,
        });
        assert_eq!(completion.lock().unwrap().1, [5, 2]);
        assert!(matches!(
            outcome,
            VerifyOutcome::Failed { function, .. } if function == "f2"
        ));
    }

    // No halt: every skip is aggregated, in ascending index order regardless of
    // completion order (jobs=3 over 5 functions).
    #[test]
    fn run_pool_aggregates_skips_in_index_order() {
        let names = ["f0", "f1", "f2", "f3", "f4"];
        let (outcome, skipped) = run_pool(3, None, 0, &names, |idx| {
            if idx == 1 || idx == 3 {
                skip(&format!("f{idx}"))
            } else {
                PerFuncResult::Ok
            }
        });
        assert!(matches!(outcome, VerifyOutcome::SkippedNonModelable));
        let got: Vec<&str> = skipped.iter().map(|s| s.function.as_str()).collect();
        assert_eq!(got, ["f1", "f3"]);
    }

    // Serial path (jobs=1) returns on the first halt: later functions are never
    // evaluated. The call counter pins the short-circuit.
    #[test]
    fn run_pool_serial_halts_without_evaluating_later() {
        let names = ["f0", "f1", "f2", "f3"];
        let calls = AtomicUsize::new(0);
        let (outcome, _) = run_pool(1, None, 0, &names, |idx| {
            calls.fetch_add(1, Ordering::SeqCst);
            if idx == 1 {
                halt(&format!("f{idx}"))
            } else {
                PerFuncResult::Ok
            }
        });
        assert!(matches!(
            outcome,
            VerifyOutcome::Failed { function, .. } if function == "f1"
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "f2/f3 must not be evaluated"
        );
    }

    // Serial path: a skip recorded before the halt is returned alongside it.
    #[test]
    fn run_pool_serial_returns_skips_before_halt() {
        let names = ["f0", "f1"];
        let (outcome, skipped) = run_pool(1, None, 0, &names, |idx| {
            if idx == 0 { skip("f0") } else { halt("f1") }
        });
        assert!(matches!(outcome, VerifyOutcome::Failed { .. }));
        let got: Vec<&str> = skipped.iter().map(|s| s.function.as_str()).collect();
        assert_eq!(got, ["f0"]);
    }

    #[test]
    fn classify_verification_result_maps_every_terminal_variant() {
        let failed = classify_verification_result(
            "f",
            VerificationResult::Failed(vow_verify::Counterexample {
                description: "bad result".to_string(),
                vow_id: None,
                callee_precondition: None,
                values: Vec::new(),
                block_visits: Vec::new(),
                raw_output: String::new(),
            }),
        );
        assert!(matches!(
            failed,
            VerificationDisposition::Counterexample(ce) if ce.description == "bad result"
        ));

        let tool_error = classify_verification_result(
            "f",
            VerificationResult::ToolError("solver crashed".to_string()),
        );
        assert!(matches!(
            tool_error,
            VerificationDisposition::Complete(PerFuncResult::Halt(VerifyOutcome::Error {
                function,
                message,
            })) if function == "f" && message == "solver crashed"
        ));

        let timeout = classify_verification_result("f", VerificationResult::Timeout);
        assert!(matches!(
            timeout,
            VerificationDisposition::Complete(PerFuncResult::Halt(VerifyOutcome::Timeout {
                function,
            })) if function == "f"
        ));

        let unknown = classify_verification_result(
            "f",
            VerificationResult::Unknown {
                reason: "forward condition".to_string(),
            },
        );
        assert!(matches!(
            unknown,
            VerificationDisposition::Complete(PerFuncResult::Halt(VerifyOutcome::Unknown {
                function,
                reason,
            })) if function == "f" && reason == "forward condition"
        ));

        assert!(matches!(
            classify_verification_result("f", VerificationResult::Proven),
            VerificationDisposition::Complete(PerFuncResult::Ok)
        ));
        assert!(matches!(
            classify_verification_result("f", VerificationResult::ProvenIr),
            VerificationDisposition::Complete(PerFuncResult::Ok)
        ));
        assert!(matches!(
            classify_verification_result("f", VerificationResult::ToolNotFound),
            VerificationDisposition::Complete(PerFuncResult::Halt(VerifyOutcome::ToolNotFound))
        ));

        let skipped = classify_verification_result(
            "f",
            VerificationResult::Skipped {
                reason: "unsupported opcode".to_string(),
            },
        );
        assert!(matches!(
            skipped,
            VerificationDisposition::Complete(PerFuncResult::Skipped(SkippedFunction {
                function,
                reason,
            })) if function == "f" && reason == "unsupported opcode"
        ));
    }

    #[test]
    fn record_proof_span_emits_matching_flows_and_named_span() {
        let prof = perfetto::Profiler::new();
        record_proof_span(Some(&prof), 10, 3, "checked", 101, 20);

        let events = prof.snapshot();
        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0].kind,
            perfetto::EventKind::Flow {
                id: 3,
                edge: perfetto::FlowEdge::Start,
            }
        ));
        assert_eq!(events[0].tid, perfetto::TID_VERIFY_DRIVER);
        assert_eq!(events[0].ts_us, 10);
        assert!(matches!(
            &events[1].kind,
            perfetto::EventKind::Flow {
                id: 3,
                edge: perfetto::FlowEdge::End,
            }
        ));
        assert_eq!(events[1].tid, 101);
        assert_eq!(events[1].ts_us, 20);
        assert!(matches!(&events[2].kind, perfetto::EventKind::Span { .. }));
        assert_eq!(events[2].name, "esbmc:checked");
        assert_eq!(
            events[2].args,
            [("function".to_string(), "checked".to_string())]
        );
    }

    fn module_with_functions(functions: Vec<vow_ir::Function>) -> vow_ir::Module {
        vow_ir::Module {
            name: "test".to_string(),
            functions,
            strings: Vec::new(),
            struct_layouts: Vec::new(),
            enum_layouts: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn run_verification_sync_empty_module_is_proven() {
        let module = module_with_functions(Vec::new());
        let (outcome, skipped) = run_verification_sync(
            &module,
            "test.vow",
            &std::collections::HashMap::new(),
            None,
            &VerifyLimits::default(),
            4,
            &SolverConfig::default_config(),
            None,
        );
        assert!(matches!(outcome, VerifyOutcome::Proven));
        assert!(skipped.is_empty());
    }

    #[test]
    fn run_verification_sync_reports_effectful_vow_as_skipped() {
        let function = vow_ir::Function {
            id: vow_ir::FuncId(0),
            name: "effectful".to_string(),
            params: Vec::new(),
            param_names: Vec::new(),
            return_ty: vow_ir::Ty::Unit,
            effects: vec![vow_syntax::ast::Effect::IO],
            vows: vec![vow_ir::VowEntry {
                id: vow_ir::VowId(0),
                description: "ensures: true".to_string(),
                blame: vow_diag::Blame::Callee,
                bindings: Vec::new(),
                file: "test.vow".to_string(),
                offset: 0,
            }],
            blocks: Vec::new(),
            local_names: std::collections::HashMap::new(),
            summary: vow_ir::RegionSummary::default(),
            source_file: "test.vow".to_string(),
        };
        let module = module_with_functions(vec![function]);

        let (outcome, skipped) = run_verification_sync(
            &module,
            "test.vow",
            &std::collections::HashMap::new(),
            None,
            &VerifyLimits::default(),
            1,
            &SolverConfig::default_config(),
            None,
        );
        assert!(matches!(outcome, VerifyOutcome::SkippedNonModelable));
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].function, "effectful");
        assert!(skipped[0].reason.contains("has effects"));
    }
}
