//! The `vow contracts` command: lower a source file to IR, list every contract
//! clause as a [`ContractEntryJson`], optionally verify each clause with ESBMC,
//! and emit a single JSON [`ContractsResultJson`] with a rollup summary.
//!
//! The public surface is the single [`run_contracts_command`] entry point.
//! Entry assembly ([`build_contract_entries`]), per-clause verification
//! ([`update_contract_statuses`]), summary rollup ([`build_contracts_summary`]),
//! and the fail-closed exit predicate ([`contracts_summary_has_failure`]) are
//! internal helpers, each unit-tested through this module. The JSON DTOs live in
//! [`crate::report`]; this module owns the logic that produces them.

use std::path::Path;

use vow_verify::{
    ReachVerdict, SolverConfig, VerificationResult, VerifyLimits, detect_constant_functions,
    emit_bodyreplace_c_source, emit_reach_c_source, emit_verify_c_source, find_esbmc,
    non_modelable_reason, run_esbmc_bodyreplace, run_esbmc_multi_property, run_esbmc_reach,
};

use crate::compile_frontend_with_root;
use crate::contract_quality;
use crate::report::{
    ContractEntryJson, ContractSourceJson, ContractsQualityJson, ContractsResultJson,
    ContractsSummaryJson,
};

fn build_contracts_summary(entries: &[ContractEntryJson]) -> ContractsSummaryJson {
    let mut summary = ContractsSummaryJson {
        total: entries.len() as u32,
        proven: 0,
        failed: 0,
        unknown: 0,
        timeout: 0,
        error: 0,
        not_verified: 0,
        skipped: 0,
        vacuous: 0,
        trivially_satisfiable: 0,
        quality: ContractsQualityJson {
            weak: 0,
            tautological: 0,
            substantive: 0,
        },
    };
    for e in entries {
        if e.trivially_satisfiable {
            summary.trivially_satisfiable += 1;
        }
        match e.status.as_str() {
            "proven" | "proven-ir" => summary.proven += 1,
            "failed" => summary.failed += 1,
            "unknown" => summary.unknown += 1,
            "timeout" => summary.timeout += 1,
            "error" => summary.error += 1,
            "skipped" => summary.skipped += 1,
            "vacuous" => summary.vacuous += 1,
            _ => summary.not_verified += 1,
        }
        match e.quality.as_str() {
            "weak" => summary.quality.weak += 1,
            "tautological" => summary.quality.tautological += 1,
            _ => summary.quality.substantive += 1,
        }
    }
    summary
}

fn update_contract_statuses(
    entries: &mut [ContractEntryJson],
    ir_module: &vow_ir::Module,
    limits: &VerifyLimits,
    config: &SolverConfig,
) {
    let const_fns = detect_constant_functions(ir_module);
    for func in &ir_module.functions {
        if func.vows.is_empty() {
            continue;
        }

        if non_modelable_reason(func, ir_module, &const_fns).is_some() {
            for entry in entries.iter_mut() {
                if entry.function_id == func.id.0 {
                    entry.status = "skipped".to_string();
                }
            }
            continue;
        }

        let esbmc = match find_esbmc() {
            Some(p) => p,
            None => {
                for entry in entries.iter_mut() {
                    if entry.function_id == func.id.0 {
                        entry.status = "error".to_string();
                    }
                }
                continue;
            }
        };
        let c_src = emit_verify_c_source(func, ir_module, &const_fns, limits);
        // Per-clause status via ESBMC `--multi-property`: every `vow:N` claim is
        // reported individually, so each contract clause gets a precise verdict
        // instead of the siblings of a failed clause collapsing to `unknown`
        // (#81 PR-A). The single-counterexample verify cache is bypassed on this
        // path; precise per-clause status, not throughput, is the goal here.
        let (overall, verdicts) =
            run_esbmc_multi_property(&esbmc, &c_src, limits.max_k_step, &func.name, config);

        for entry in entries.iter_mut() {
            if entry.function_id != func.id.0 {
                continue;
            }
            entry.status = match verdicts.get(&entry.vow_id) {
                Some(true) => "proven",
                Some(false) => "failed",
                None => match &overall {
                    VerificationResult::Proven | VerificationResult::ProvenIr => "proven",
                    VerificationResult::Timeout => "timeout",
                    VerificationResult::Unknown { .. } => "unknown",
                    VerificationResult::ToolError(_) | VerificationResult::ToolNotFound => "error",
                    VerificationResult::Skipped { .. } => "skipped",
                    // Overall verification failed but ESBMC did not report this
                    // specific clause individually — genuinely undecided.
                    VerificationResult::Failed(_) => "unknown",
                },
            }
            .to_string();
        }

        // Vacuity probe (#81 PR-B): if the function's `requires` are
        // contradictory, the `ensures` above all passed vacuously. Re-run with a
        // `vow_reach` label planted after the requires; if that point is
        // unreachable (ESBMC SUCCESSFUL), the whole contract is vacuous —
        // override the misleading `proven`s with `vacuous`. Only functions with
        // a `requires` are eligible (emit_reach_c_source returns None otherwise),
        // so this adds at most one extra ESBMC run per such function.
        if let Some(reach_src) = emit_reach_c_source(func, ir_module, &const_fns, limits)
            && run_esbmc_reach(&esbmc, &reach_src, limits.max_k_step, &func.name, config)
                == ReachVerdict::Vacuous
        {
            for entry in entries.iter_mut() {
                if entry.function_id == func.id.0 {
                    entry.status = "vacuous".to_string();
                }
            }
        }

        // Weakness probe (#81 PR-C): re-verify a body-replaced model where the
        // returned value is forced to the type-default. If the `ensures` still
        // holds, a trivial `return <default>` body satisfies the contract — it
        // is too weak to pin down the implementation. Marks each `ensures` clause
        // `trivially_satisfiable` (informational; never changes the exit code).
        // Skipped for non-scalar returns / returned-parameter results, which
        // keeps the signal one-sided — it never claims weakness it cannot show.
        if let Some(br_src) = emit_bodyreplace_c_source(func, ir_module, &const_fns, limits)
            && run_esbmc_bodyreplace(&esbmc, &br_src, limits.max_k_step, &func.name, config)
        {
            for entry in entries.iter_mut() {
                if entry.function_id == func.id.0 && entry.kind == "ensures" {
                    entry.trivially_satisfiable = true;
                }
            }
        }
    }
}

/// `vow contracts --verify` fails closed when any contract is not proven —
/// the same fail-closed set as `vow build --verify` / `vow verify` (#479).
/// `proven` / `proven-ir` / `not_verified` pass; failed/timeout/unknown/error/
/// skipped fail.
fn contracts_summary_has_failure(summary: &ContractsSummaryJson) -> bool {
    summary.failed > 0
        || summary.timeout > 0
        || summary.unknown > 0
        || summary.error > 0
        || summary.skipped > 0
        || summary.vacuous > 0
}

/// Assemble the per-clause `ContractEntryJson` records for `vow contracts` from
/// a lowered IR module: one entry per vow on each function, in function-then-vow
/// order (unlike contract *density*, this includes `main`). Each entry starts in
/// its pre-verification state — `status` is `"not_verified"` and
/// `trivially_satisfiable` is `false` — which the optional `--verify` pass
/// (`update_contract_statuses`) later mutates in place. `kind` and `quality` are
/// the static, no-ESBMC classifications; `blame` is the capitalized
/// `Caller`/`Callee`/`None` form the JSON exposes.
fn build_contract_entries(ir_module: &vow_ir::Module) -> Vec<ContractEntryJson> {
    let mut entries: Vec<ContractEntryJson> = Vec::new();
    for func in &ir_module.functions {
        for vow in &func.vows {
            let analysis = contract_quality::analyze(&vow.description);
            let blame = match vow.blame {
                vow_diag::Blame::Caller => "Caller",
                vow_diag::Blame::Callee => "Callee",
                vow_diag::Blame::None => "None",
            };
            entries.push(ContractEntryJson {
                vow_id: vow.id.0,
                function: func.name.clone(),
                function_id: func.id.0,
                kind: analysis.kind.as_str().to_string(),
                description: vow.description.clone(),
                blame: blame.to_string(),
                source: ContractSourceJson {
                    file: vow.file.clone(),
                    offset: vow.offset,
                },
                status: "not_verified".to_string(),
                quality: analysis.quality.as_str().to_string(),
                trivially_satisfiable: false,
            });
        }
    }
    entries
}

pub(crate) fn run_contracts_command(
    source: &Path,
    verify: bool,
    no_cache: bool,
    limits: &VerifyLimits,
    config: &SolverConfig,
) {
    let frontend = match compile_frontend_with_root(source, None, None) {
        Ok(f) => f,
        Err(output) => {
            output.emit_json();
            std::process::exit(1);
        }
    };
    let ir_module = frontend
        .ir()
        .expect("LoweredIr goal must produce IR for contracts");

    let mut entries = build_contract_entries(ir_module);

    let mut exit_code = 0;
    if verify {
        if find_esbmc().is_none() {
            for entry in &mut entries {
                entry.status = "error".to_string();
            }
            exit_code = 1;
        } else {
            // The per-clause `--multi-property` path runs ESBMC fresh and never
            // consults the verify cache, so don't construct it — `VerifyCache::new()`
            // would create the on-disk cache dir on every `vow contracts --verify`.
            // `--no-cache` is therefore already the de facto behavior on this path.
            let _ = no_cache;
            update_contract_statuses(&mut entries, ir_module, limits, config);
        }
    }

    let summary = build_contracts_summary(&entries);
    // Fail closed: `vow contracts --verify` exits non-zero when any contract is
    // not proven, matching `vow build --verify` / `vow verify` (#479).
    if contracts_summary_has_failure(&summary) {
        exit_code = 1;
    }
    let result = ContractsResultJson {
        contracts: entries,
        summary,
    };
    let json = serde_json::to_string(&result).expect("ContractsResult must be serializable");
    println!("{json}");
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{
        ContractEntryJson, ContractSourceJson, ContractsQualityJson, ContractsSummaryJson,
    };

    #[test]
    fn build_contracts_summary_counts_each_status_bucket() {
        let source = ContractSourceJson {
            file: "test.vow".to_string(),
            offset: 0,
        };
        let entry = |status: &str| ContractEntryJson {
            vow_id: 0,
            function: "f".to_string(),
            function_id: 0,
            kind: "ensures".to_string(),
            // Description must agree with the hard-coded `quality` below.
            description: "ensures: result == x".to_string(),
            blame: "callee".to_string(),
            source: source.clone(),
            status: status.to_string(),
            quality: "substantive".to_string(),
            trivially_satisfiable: false,
        };
        let summary = build_contracts_summary(&[
            entry("proven"),
            entry("proven-ir"),
            entry("failed"),
            entry("unknown"),
            entry("timeout"),
            entry("error"),
            entry("skipped"),
            entry("not-run"),
        ]);

        assert_eq!(summary.total, 8);
        assert_eq!(summary.proven, 2);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.unknown, 1);
        assert_eq!(summary.timeout, 1);
        assert_eq!(summary.error, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.not_verified, 1);
        assert_eq!(summary.quality.substantive, 8);
        assert_eq!(summary.quality.weak, 0);
        assert_eq!(summary.quality.tautological, 0);
    }

    #[test]
    fn build_contracts_summary_counts_vacuous_and_trivially_satisfiable() {
        // Gap-fill: the `vacuous` status bucket and the `trivially_satisfiable`
        // tally are both wired through `build_contracts_summary` but were not
        // exercised by any prior test.
        let source = ContractSourceJson {
            file: "test.vow".to_string(),
            offset: 0,
        };
        let entry = |status: &str, trivially: bool| ContractEntryJson {
            vow_id: 0,
            function: "f".to_string(),
            function_id: 0,
            kind: "ensures".to_string(),
            description: "ensures: result == x".to_string(),
            blame: "callee".to_string(),
            source: source.clone(),
            status: status.to_string(),
            quality: "substantive".to_string(),
            trivially_satisfiable: trivially,
        };
        let summary = build_contracts_summary(&[
            entry("vacuous", false),
            entry("vacuous", true),
            entry("proven", true),
        ]);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.vacuous, 2);
        assert_eq!(summary.proven, 1);
        // Independent of status: every entry with the flag set is tallied.
        assert_eq!(summary.trivially_satisfiable, 2);
    }

    #[test]
    fn build_contracts_summary_tallies_quality_buckets() {
        // Gap-fill: the weak / tautological quality buckets were never counted
        // by a prior test (all prior entries were `substantive`).
        let source = ContractSourceJson {
            file: "test.vow".to_string(),
            offset: 0,
        };
        let entry = |quality: &str| ContractEntryJson {
            vow_id: 0,
            function: "f".to_string(),
            function_id: 0,
            kind: "ensures".to_string(),
            description: "ensures: result == x".to_string(),
            blame: "callee".to_string(),
            source: source.clone(),
            status: "not_verified".to_string(),
            quality: quality.to_string(),
            trivially_satisfiable: false,
        };
        let summary = build_contracts_summary(&[
            entry("weak"),
            entry("weak"),
            entry("tautological"),
            entry("substantive"),
            // Any unrecognized quality falls through to `substantive`.
            entry("mystery"),
        ]);

        assert_eq!(summary.quality.weak, 2);
        assert_eq!(summary.quality.tautological, 1);
        assert_eq!(summary.quality.substantive, 2);
    }

    #[test]
    fn contracts_fail_closed_on_unproven_statuses() {
        let all_proven = ContractsSummaryJson {
            total: 1,
            proven: 1,
            failed: 0,
            unknown: 0,
            timeout: 0,
            error: 0,
            not_verified: 0,
            skipped: 0,
            vacuous: 0,
            trivially_satisfiable: 0,
            quality: ContractsQualityJson {
                weak: 0,
                tautological: 0,
                substantive: 1,
            },
        };
        // All proven, or unverified (no --verify) → pass (exit 0).
        assert!(!contracts_summary_has_failure(&all_proven));
        assert!(!contracts_summary_has_failure(&ContractsSummaryJson {
            proven: 0,
            not_verified: 1,
            ..all_proven.clone()
        }));
        // Every not-proven status fails closed (matches `vow build --verify`).
        for failing in [
            ContractsSummaryJson {
                failed: 1,
                ..all_proven.clone()
            },
            ContractsSummaryJson {
                timeout: 1,
                ..all_proven.clone()
            },
            ContractsSummaryJson {
                unknown: 1,
                ..all_proven.clone()
            },
            ContractsSummaryJson {
                error: 1,
                ..all_proven.clone()
            },
            ContractsSummaryJson {
                skipped: 1,
                ..all_proven.clone()
            },
            ContractsSummaryJson {
                vacuous: 1,
                ..all_proven.clone()
            },
        ] {
            assert!(contracts_summary_has_failure(&failing));
        }
    }

    // ---- Contract-entry assembly (build_contract_entries) ----

    fn contract_vow(
        id: u32,
        description: &str,
        blame: vow_diag::Blame,
        offset: u32,
    ) -> vow_ir::VowEntry {
        vow_ir::VowEntry {
            id: vow_ir::VowId(id),
            description: description.to_string(),
            blame,
            bindings: vec![],
            file: "c.vow".to_string(),
            offset,
        }
    }

    fn contract_fn(id: u32, name: &str, vows: Vec<vow_ir::VowEntry>) -> vow_ir::Function {
        vow_ir::Function {
            id: vow_ir::FuncId(id),
            name: name.to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: vow_ir::Ty::Unit,
            effects: vec![],
            vows,
            blocks: vec![vow_ir::BasicBlock {
                id: vow_ir::BlockId(0),
                insts: vec![],
            }],
            local_names: std::collections::HashMap::new(),
            summary: vow_ir::RegionSummary::default(),
            source_file: String::new(),
        }
    }

    fn contract_module(functions: Vec<vow_ir::Function>) -> vow_ir::Module {
        vow_ir::Module {
            name: "Contracts".to_string(),
            functions,
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn build_contract_entries_maps_kind_blame_source_and_defaults() {
        // One function carrying a Caller-blamed `requires` and a Callee-blamed
        // `ensures`. Pins the per-vow field mapping independently of the loop:
        // kind from the description keyword, the *capitalized* blame the JSON
        // exposes, source file/offset threading, and the pre-verification
        // defaults (`not_verified`, `trivially_satisfiable == false`).
        let module = contract_module(vec![contract_fn(
            7,
            "safe_div",
            vec![
                contract_vow(3, "requires: y != 0", vow_diag::Blame::Caller, 11),
                contract_vow(4, "ensures: result == x", vow_diag::Blame::Callee, 42),
            ],
        )]);

        let entries = build_contract_entries(&module);

        assert_eq!(entries.len(), 2);

        let req = &entries[0];
        assert_eq!(req.vow_id, 3);
        assert_eq!(req.function, "safe_div");
        assert_eq!(req.function_id, 7);
        assert_eq!(req.kind, "requires");
        assert_eq!(req.blame, "Caller");
        assert_eq!(req.source.file, "c.vow");
        assert_eq!(req.source.offset, 11);
        assert_eq!(req.status, "not_verified");
        assert!(!req.trivially_satisfiable);

        let ens = &entries[1];
        assert_eq!(ens.vow_id, 4);
        assert_eq!(ens.kind, "ensures");
        assert_eq!(ens.blame, "Callee");
        assert_eq!(ens.source.offset, 42);
        assert_eq!(ens.status, "not_verified");
    }

    #[test]
    fn build_contract_entries_flattens_functions_and_vows_in_order() {
        // Two contracted functions plus one with no vows. Entries appear in
        // function-then-vow order, the vow-less function contributes nothing,
        // and — unlike contract *density* — `main`'s vows are included.
        let module = contract_module(vec![
            contract_fn(
                0,
                "main",
                vec![contract_vow(
                    0,
                    "requires: n >= 0",
                    vow_diag::Blame::Caller,
                    0,
                )],
            ),
            contract_fn(1, "helper", vec![]),
            contract_fn(
                2,
                "clamp",
                vec![
                    contract_vow(1, "ensures: result <= hi", vow_diag::Blame::Callee, 0),
                    contract_vow(2, "invariant: lo <= hi", vow_diag::Blame::None, 0),
                ],
            ),
        ]);

        let entries = build_contract_entries(&module);

        let trace: Vec<(&str, &str, &str)> = entries
            .iter()
            .map(|e| (e.function.as_str(), e.kind.as_str(), e.blame.as_str()))
            .collect();
        assert_eq!(
            trace,
            vec![
                ("main", "requires", "Caller"),
                ("clamp", "ensures", "Callee"),
                ("clamp", "invariant", "None"),
            ]
        );
    }

    #[test]
    fn build_contract_entries_classifies_clause_quality() {
        // The static, no-ESBMC quality classifier is wired through per entry:
        // a bare `result` bound is weak, an equality is substantive, and a
        // constant clause is tautological.
        let module = contract_module(vec![contract_fn(
            0,
            "f",
            vec![
                contract_vow(0, "ensures: result >= 0", vow_diag::Blame::Callee, 0),
                contract_vow(1, "ensures: result == x", vow_diag::Blame::Callee, 0),
                contract_vow(2, "invariant: true", vow_diag::Blame::Callee, 0),
            ],
        )]);

        let entries = build_contract_entries(&module);
        let quality: Vec<&str> = entries.iter().map(|e| e.quality.as_str()).collect();
        assert_eq!(quality, vec!["weak", "substantive", "tautological"]);
    }
}
