//! Agent-facing JSON output model for the `vow` CLI.
//!
//! Owns the serde DTOs emitted on stdout by `build`, `verify`, `test`, and
//! `contracts`, plus the pure conversions from the driver's internal domain
//! model (`BuildOutput`, `StructuredCounterexample`, `vow_diag::Diagnostic`)
//! into that stable wire schema. Kept separate from `main.rs` so the JSON
//! contract lives in one place and its mapping logic is exercised directly.

use std::collections::BTreeMap;

use serde::Serialize;
use vow_diag::{Diagnostic, Severity};

use crate::{BuildOutput, BuildStatus, StructuredCounterexample};

#[derive(Debug, Clone, Serialize)]
pub struct SpanJson {
    pub file: String,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticJson {
    pub error_code: String,
    pub message: String,
    pub severity: String,
    pub span: SpanJson,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secondary: Vec<SpanJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blame: Option<String>,
}

impl DiagnosticJson {
    pub(crate) fn from_diagnostic(d: &Diagnostic) -> Self {
        let blame = match d.blame {
            vow_diag::Blame::Caller => Some("caller".to_string()),
            vow_diag::Blame::Callee => Some("callee".to_string()),
            vow_diag::Blame::None => None,
        };
        let secondary = d
            .secondary
            .iter()
            .map(|s| SpanJson {
                file: s.file.clone(),
                offset: s.byte_offset,
                length: s.byte_len,
            })
            .collect();
        Self {
            error_code: format!("{:?}", d.code),
            message: d.message.clone(),
            severity: match d.severity {
                Severity::Error => "error".to_string(),
                Severity::Warning => "warning".to_string(),
                Severity::Note => "note".to_string(),
            },
            span: SpanJson {
                file: d.primary.file.clone(),
                offset: d.primary.byte_offset,
                length: d.primary.byte_len,
            },
            hints: d.hints.clone(),
            secondary,
            blame,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CeCallSiteJson {
    pub caller_function: String,
    pub file: String,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CeViolatingArgJson {
    pub param: String,
    pub value: String,
    pub arg_offset: u32,
    pub arg_length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CePathStepJson {
    pub block_id: u32,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CeBranchDecisionJson {
    pub condition_offset: u32,
    pub condition_length: u32,
    pub taken: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CounterexampleJson {
    pub function: String,
    pub values: BTreeMap<String, String>,
    pub violation: String,
    pub vow_id: u32,
    pub source: Option<SpanJson>,
    pub blame: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub call_sites: Vec<CeCallSiteJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub violating_args: Vec<CeViolatingArgJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub execution_path: Vec<CePathStepJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub branch_decisions: Vec<CeBranchDecisionJson>,
    /// `--replay-cex` differential-test outcome (issue #335): `"confirmed"`,
    /// `"diverged"`, or `"skipped"`. Absent unless `--replay-cex` was passed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildResult {
    pub status: String,
    pub executable: Option<String>,
    pub diagnostics: Vec<DiagnosticJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<String>,
    pub counterexamples: Vec<CounterexampleJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractEntryJson {
    pub vow_id: u32,
    pub function: String,
    #[serde(skip)]
    pub function_id: u32,
    pub kind: String,
    pub description: String,
    pub blame: String,
    pub source: ContractSourceJson,
    pub status: String,
    /// Static quality classification of the clause shape (no ESBMC): one of
    /// `weak` (an `ensures` that only bounds `result` by a constant),
    /// `tautological` (constant clause that says nothing about the program),
    /// or `substantive` (equality / relational / inverse / call). See
    /// docs/spec/contracts-methodology.md.
    pub quality: String,
    /// Verification-based weakness signal (#81 PR-C): true when a trivial
    /// `return <default>` body still satisfies this `ensures` — the contract is
    /// too weak to pin down the implementation. Only computed for `ensures`
    /// clauses under `--verify`; always false for `requires`/`invariant` and
    /// when `--verify` is off. Informational: it does not affect the exit code.
    pub trivially_satisfiable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractSourceJson {
    pub file: String,
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractsSummaryJson {
    pub total: u32,
    pub proven: u32,
    pub failed: u32,
    pub unknown: u32,
    pub timeout: u32,
    pub error: u32,
    pub not_verified: u32,
    pub skipped: u32,
    pub vacuous: u32,
    /// Count of `ensures` clauses a trivial `return <default>` body satisfies
    /// (#81 PR-C). Informational, like `quality`; does not affect the exit code.
    pub trivially_satisfiable: u32,
    pub quality: ContractsQualityJson,
}

/// Static contract-quality tallies (independent of verification status).
#[derive(Debug, Clone, Serialize)]
pub struct ContractsQualityJson {
    pub weak: u32,
    pub tautological: u32,
    pub substantive: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractsResultJson {
    pub contracts: Vec<ContractEntryJson>,
    pub summary: ContractsSummaryJson,
}

// ---------------------------------------------------------------------------
// Test output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    pub status: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub tests: Vec<TestEntry>,
    pub contract_density: ContractDensity,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestEntry {
    pub file: String,
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub diagnostics: Vec<DiagnosticJson>,
    pub counterexamples: Vec<CounterexampleJson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractDensity {
    pub functions_total: usize,
    pub functions_with_vows: usize,
    pub density_pct: f64,
}
impl CounterexampleJson {
    pub(crate) fn from_structured(ce: &StructuredCounterexample) -> Self {
        Self {
            function: ce.function.clone(),
            // Drop internal user-struct heap-model storage (`__vow_heap[]` / bump
            // pointer): a verifier artifact, not a source free variable, so
            // struct counterexamples don't dump the whole slot array to agents.
            values: ce
                .values
                .iter()
                .filter(|(name, _)| !name.contains("__vow_heap"))
                .cloned()
                .collect(),
            violation: ce.violation.clone(),
            vow_id: ce.vow_id,
            source: ce.source.as_ref().map(|s| SpanJson {
                file: s.file.clone(),
                offset: s.offset,
                length: s.length,
            }),
            blame: ce.blame.clone(),
            call_sites: ce
                .call_sites
                .iter()
                .map(|cs| CeCallSiteJson {
                    caller_function: cs.caller_function.clone(),
                    file: cs.file.clone(),
                    offset: cs.offset,
                    length: cs.length,
                })
                .collect(),
            violating_args: ce
                .violating_args
                .iter()
                .map(|va| CeViolatingArgJson {
                    param: va.param.clone(),
                    value: va.value.clone(),
                    arg_offset: va.arg_offset,
                    arg_length: va.arg_length,
                })
                .collect(),
            execution_path: ce
                .execution_path
                .iter()
                .map(|ps| CePathStepJson {
                    block_id: ps.block_id,
                    offset: ps.offset,
                    length: ps.length,
                })
                .collect(),
            branch_decisions: ce
                .branch_decisions
                .iter()
                .map(|bd| CeBranchDecisionJson {
                    condition_offset: bd.condition_offset,
                    condition_length: bd.condition_length,
                    taken: bd.taken.clone(),
                })
                .collect(),
            replay: ce.replay.clone(),
            replay_reason: ce.replay_reason.clone(),
        }
    }
}

impl BuildOutput {
    pub fn to_build_result(&self) -> BuildResult {
        let status = match &self.status {
            BuildStatus::Verified => "Verified",
            BuildStatus::Unverified => "Unverified",
            BuildStatus::Skipped => "Skipped",
            BuildStatus::CompileFailed { .. } => "CompileFailed",
            BuildStatus::VerifyFailed { .. } => "VerifyFailed",
        }
        .to_string();

        let (message, function, counterexample) = match &self.status {
            BuildStatus::CompileFailed { message } => (Some(message.clone()), None, None),
            BuildStatus::VerifyFailed {
                function,
                description,
            } => (None, Some(function.clone()), Some(description.clone())),
            _ => (None, None, None),
        };

        BuildResult {
            status,
            executable: self.executable.as_ref().map(|p| p.display().to_string()),
            diagnostics: self
                .diagnostics
                .iter()
                .map(DiagnosticJson::from_diagnostic)
                .collect(),
            message,
            function,
            counterexample,
            counterexamples: self
                .counterexamples
                .iter()
                .map(CounterexampleJson::from_structured)
                .collect(),
            verify_status: self.verify_status.clone(),
            verify_message: self.verify_message.clone(),
        }
    }

    pub fn emit_json(&self) {
        let result = self.to_build_result();
        let json = serde_json::to_string(&result).expect("BuildResult must be serializable");
        println!("{json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vow_diag::{Blame, ErrorCode, SourceLocation};

    fn loc(file: &str, offset: u32, len: u32) -> SourceLocation {
        SourceLocation {
            file: file.to_string(),
            byte_offset: offset,
            byte_len: len,
        }
    }

    fn diag(code: ErrorCode, severity: Severity, blame: Blame, message: &str) -> Diagnostic {
        Diagnostic {
            severity,
            code,
            message: message.to_string(),
            primary: loc("divide.vow", 10, 5),
            secondary: Vec::new(),
            blame,
            hints: Vec::new(),
        }
    }

    #[test]
    fn from_diagnostic_maps_blame_severity_and_span() {
        let mut d = diag(
            ErrorCode::VowRequiresViolated,
            Severity::Error,
            Blame::Caller,
            "y must be non-zero",
        );
        d.hints = vec!["pass a non-zero divisor".to_string()];

        let json = DiagnosticJson::from_diagnostic(&d);

        assert_eq!(json.blame.as_deref(), Some("caller"));
        assert_eq!(json.severity, "error");
        assert_eq!(json.message, "y must be non-zero");
        assert_eq!(json.span.file, "divide.vow");
        assert_eq!(json.span.offset, 10);
        assert_eq!(json.span.length, 5);
        assert_eq!(json.hints, vec!["pass a non-zero divisor".to_string()]);
    }

    #[test]
    fn from_diagnostic_callee_blame_and_none_blame() {
        let callee = diag(
            ErrorCode::VowEnsuresViolated,
            Severity::Error,
            Blame::Callee,
            "postcondition",
        );
        assert_eq!(
            DiagnosticJson::from_diagnostic(&callee).blame.as_deref(),
            Some("callee")
        );

        let plain = diag(
            ErrorCode::TypeMismatch,
            Severity::Warning,
            Blame::None,
            "type error",
        );
        let pj = DiagnosticJson::from_diagnostic(&plain);
        assert_eq!(pj.blame, None);
        assert_eq!(pj.severity, "warning");
    }

    fn minimal_ce(function: &str, values: Vec<(&str, &str)>) -> StructuredCounterexample {
        StructuredCounterexample {
            function: function.to_string(),
            values: values
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            violation: "requires violated".to_string(),
            vow_id: 7,
            source: None,
            blame: "caller".to_string(),
            call_sites: Vec::new(),
            violating_args: Vec::new(),
            execution_path: Vec::new(),
            branch_decisions: Vec::new(),
            replay: None,
            replay_reason: None,
            replay_raw_values: Vec::new(),
            replay_raw_output: String::new(),
        }
    }

    #[test]
    fn from_structured_drops_vow_heap_storage_but_keeps_source_vars() {
        // A struct counterexample carries both real source free variables and
        // the verifier's internal `__vow_heap` bump-slot storage. Only the
        // source variables should reach agents.
        let ce = minimal_ce(
            "push",
            vec![("val", "5"), ("__vow_heap[0]", "999"), ("h.len", "2")],
        );

        let json = CounterexampleJson::from_structured(&ce);

        assert_eq!(json.function, "push");
        assert_eq!(json.vow_id, 7);
        assert_eq!(json.blame, "caller");
        let keys: Vec<&str> = json.values.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["h.len", "val"]); // BTreeMap-sorted; no __vow_heap
        assert_eq!(json.values.get("val").map(String::as_str), Some("5"));
        assert!(!json.values.keys().any(|k| k.contains("__vow_heap")));
    }

    fn build_output(status: BuildStatus) -> BuildOutput {
        BuildOutput {
            status,
            executable: None,
            diagnostics: Vec::new(),
            counterexamples: Vec::new(),
            verify_status: None,
            verify_message: None,
        }
    }

    #[test]
    fn to_build_result_routes_verify_failed_fields() {
        let mut out = build_output(BuildStatus::VerifyFailed {
            function: "divide".to_string(),
            description: "y != 0 violated".to_string(),
        });
        out.verify_status = Some("failed".to_string());
        out.verify_message = Some("counterexample found".to_string());

        let r = out.to_build_result();

        assert_eq!(r.status, "VerifyFailed");
        assert_eq!(r.function.as_deref(), Some("divide"));
        assert_eq!(r.counterexample.as_deref(), Some("y != 0 violated"));
        assert_eq!(r.message, None);
        assert_eq!(r.verify_status.as_deref(), Some("failed"));
        assert_eq!(r.verify_message.as_deref(), Some("counterexample found"));
    }

    #[test]
    fn to_build_result_routes_compile_failed_message() {
        let out = build_output(BuildStatus::CompileFailed {
            message: "parse error".to_string(),
        });

        let r = out.to_build_result();

        assert_eq!(r.status, "CompileFailed");
        assert_eq!(r.message.as_deref(), Some("parse error"));
        assert_eq!(r.function, None);
        assert_eq!(r.counterexample, None);
    }

    #[test]
    fn to_build_result_maps_plain_statuses() {
        assert_eq!(
            build_output(BuildStatus::Verified).to_build_result().status,
            "Verified"
        );
        assert_eq!(
            build_output(BuildStatus::Unverified)
                .to_build_result()
                .status,
            "Unverified"
        );
        assert_eq!(
            build_output(BuildStatus::Skipped).to_build_result().status,
            "Skipped"
        );
    }
}
