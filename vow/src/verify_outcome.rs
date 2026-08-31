//! Translation layer from a terminal build/verify outcome into the external
//! [`BuildOutput`] result shape.
//!
//! The verification driver (`run_verification_sync` / `verify_one_function` in
//! `verification.rs`) produces a [`VerifyOutcome`] verdict plus a list of
//! [`SkippedFunction`]s. This module owns that vocabulary and is the single
//! place that turns any terminal outcome — a proof, a counterexample, a
//! compile failure, or a panicked verifier — into a [`BuildOutput`] with the
//! matching diagnostics. `BuildOutput`/`BuildStatus`/`StructuredCounterexample`
//! remain the crate's shared output vocabulary and live in `main.rs`.

use std::path::PathBuf;

use vow_codegen::CodegenError;
use vow_diag::{Blame, Diagnostic, Severity, SourceLocation};

use crate::{BuildOutput, BuildStatus, StructuredCounterexample};

/// The verifier's verdict for a whole module. Produced by the verification
/// driver in `verification.rs` and consumed by [`to_output`] / [`to_output_with_warnings`].
pub(crate) enum VerifyOutcome {
    /// ESBMC not invoked (`--no-verify`); maps to `BuildStatus::Unverified` (exit 0).
    /// Named `NotRun` (not `Skipped`) to avoid colliding with `SkippedNonModelable`,
    /// which has the opposite exit code.
    NotRun,
    /// ESBMC ran but ≥1 vowed function non-modelable; maps to `BuildStatus::Skipped` (exit 1).
    SkippedNonModelable,
    Proven,
    Failed {
        function: String,
        description: String,
        counterexamples: Vec<StructuredCounterexample>,
    },
    Timeout {
        function: String,
    },
    /// ESBMC finished but returned `VERIFICATION UNKNOWN` — neither proof
    /// nor counterexample. Distinct from Timeout (no wall-clock cutoff) and
    /// from Error (no parser failure / process crash).
    Unknown {
        function: String,
        reason: String,
    },
    Error {
        function: String,
        message: String,
    },
    ToolNotFound,
}

/// A vowed function the verifier skipped; surfaces as a Warning in `BuildOutput.diagnostics`.
#[derive(Debug, Clone)]
pub(crate) struct SkippedFunction {
    pub(crate) function: String,
    pub(crate) reason: String,
}

/// A checked operator whose `ArithmeticOverflow` abort the verifier proved
/// reachable (#585). Surfaces as an `ArithOverflowReachable` Warning.
#[derive(Debug, Clone)]
pub(crate) struct ArithOverflowWarning {
    pub(crate) function: String,
    /// Why it aborts, from `ArithAbort::description()`.
    pub(crate) cause: &'static str,
    pub(crate) file: String,
    pub(crate) offset: u32,
    pub(crate) length: u32,
}

/// A non-fatal finding from verifying one function. The two kinds differ in
/// consequence, not just wording: a skip means a contract went **unproved** and
/// fails the run closed, whereas a reachable checked-arithmetic abort leaves
/// every contract proved and only reports a runtime behaviour. Keeping them in
/// one enum is what lets [`crate::verification::run_pool`] aggregate both while
/// still asking `is_skip` before it lifts the build status.
#[derive(Debug, Clone)]
pub(crate) enum VerifyWarning {
    Skipped(SkippedFunction),
    ArithOverflow(ArithOverflowWarning),
}

impl VerifyWarning {
    /// True for the kind that means something went unproved, and so must fail
    /// the run closed.
    pub(crate) fn is_skip(&self) -> bool {
        matches!(self, Self::Skipped(_))
    }

    /// Identity of the diagnostic this warning renders to, for deduplication.
    /// Deliberately the *rendered* identity — same function, same place, same
    /// cause — not the warning's provenance, since the point is to collapse the
    /// several verify targets that can report one shared site.
    fn dedup_key(&self) -> (u8, &str, &str, u32, u32, &str) {
        match self {
            Self::Skipped(s) => (0, s.function.as_str(), "", 0, 0, s.reason.as_str()),
            Self::ArithOverflow(a) => (
                1,
                a.function.as_str(),
                a.file.as_str(),
                a.offset,
                a.length,
                a.cause,
            ),
        }
    }

    fn to_diagnostic(&self) -> Diagnostic {
        match self {
            Self::Skipped(s) => Diagnostic {
                severity: Severity::Warning,
                code: vow_diag::ErrorCode::VerificationSkipped,
                message: format!("skipped verification of `{}`: {}", s.function, s.reason),
                primary: vow_diag::SourceLocation {
                    file: String::new(),
                    byte_offset: 0,
                    byte_len: 0,
                },
                secondary: vec![],
                blame: vow_diag::Blame::None,
                hints: vec![
                    "the contract is documentary; runtime checks still apply in --mode debug"
                        .to_string(),
                ],
            },
            Self::ArithOverflow(a) => Diagnostic {
                severity: Severity::Warning,
                code: vow_diag::ErrorCode::ArithOverflowReachable,
                message: format!(
                    "checked arithmetic in `{}` can abort: {}",
                    a.function, a.cause
                ),
                primary: vow_diag::SourceLocation {
                    file: a.file.clone(),
                    byte_offset: a.offset,
                    byte_len: a.length,
                },
                secondary: vec![],
                blame: vow_diag::Blame::None,
                hints: vec![
                    "the contract is proved for every execution that returns; this abort is a \
                     separate runtime outcome"
                        .to_string(),
                    "constrain the operands in `requires` to rule the abort out, or use the \
                     wrapping operator if wrapping is intended"
                        .to_string(),
                ],
            },
        }
    }
}

/// Drop warnings that would render as an identical diagnostic, preserving order.
///
/// One checked-arithmetic site can be reported by more than one verify target:
/// a model co-emits its modelable callees, so a site inside `helper` surfaces
/// both when verifying `helper` and when verifying every contracted caller of
/// it. Once each is attributed to its true owner (see `arith_overflow_warning`)
/// those reports become the same diagnostic, and emitting one per caller would
/// bury the reader in duplicates of a single source location.
///
/// Skips are compared the same way, though in practice a function is only
/// skipped once.
fn dedup_warnings(warnings: &[VerifyWarning]) -> Vec<&VerifyWarning> {
    let mut seen = std::collections::HashSet::new();
    warnings
        .iter()
        .filter(|w| seen.insert(w.dedup_key()))
        .collect()
}

/// Map a counterexample `blame` string to the diagnostic error code.
///
/// Note the fallback asymmetry with [`blame_to_diag_blame`]: an unrecognised
/// blame maps to `VowRequiresViolated` (a *caller* code) here, but to
/// `Blame::None` there. This is preserved behaviour, not a fix.
fn blame_to_error_code(blame: &str) -> vow_diag::ErrorCode {
    match blame {
        "caller" => vow_diag::ErrorCode::VowRequiresViolated,
        "callee" => vow_diag::ErrorCode::VowEnsuresViolated,
        _ => vow_diag::ErrorCode::VowRequiresViolated,
    }
}

fn blame_to_diag_blame(blame: &str) -> vow_diag::Blame {
    match blame {
        "caller" => vow_diag::Blame::Caller,
        "callee" => vow_diag::Blame::Callee,
        _ => vow_diag::Blame::None,
    }
}

/// Translate a [`VerifyOutcome`] into a [`BuildOutput`], appending any
/// verification-failure diagnostics to `diagnostics`.
pub(crate) fn to_output(
    outcome: VerifyOutcome,
    diagnostics: Vec<Diagnostic>,
    executable: Option<PathBuf>,
) -> BuildOutput {
    to_output_with_warnings(outcome, diagnostics, &[], executable)
}

/// As [`to_output`], but also emits a Warning for each non-fatal finding the
/// verifier accumulated. Warnings are appended *before* counterexample errors.
pub(crate) fn to_output_with_warnings(
    outcome: VerifyOutcome,
    mut diagnostics: Vec<Diagnostic>,
    warnings: &[VerifyWarning],
    executable: Option<PathBuf>,
) -> BuildOutput {
    for w in dedup_warnings(warnings) {
        diagnostics.push(w.to_diagnostic());
    }
    let (status, counterexamples, verify_status, verify_message) = match outcome {
        VerifyOutcome::Failed {
            function,
            description,
            ref counterexamples,
        } => {
            for sce in counterexamples {
                let primary = match &sce.source {
                    Some(src) => vow_diag::SourceLocation {
                        file: src.file.clone(),
                        byte_offset: src.offset,
                        byte_len: src.length,
                    },
                    None => vow_diag::SourceLocation {
                        file: String::new(),
                        byte_offset: 0,
                        byte_len: 0,
                    },
                };
                let secondary: Vec<vow_diag::SourceLocation> = sce
                    .call_sites
                    .iter()
                    .map(|cs| vow_diag::SourceLocation {
                        file: cs.file.clone(),
                        byte_offset: cs.offset,
                        byte_len: cs.length,
                    })
                    .collect();
                let mut hints = Vec::new();
                match sce.blame.as_str() {
                    "caller" => {
                        hints.push(format!(
                            "the call site violated function `{}`'s precondition",
                            sce.function
                        ));
                        for va in &sce.violating_args {
                            hints.push(format!(
                                "argument `{}` = {} violates the contract",
                                va.param, va.value
                            ));
                        }
                    }
                    "callee" => {
                        hints.push(format!(
                            "function `{}` failed to establish its postcondition",
                            sce.function
                        ));
                    }
                    _ => {}
                }
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: blame_to_error_code(&sce.blame),
                    message: format!(
                        "contract violation in `{}`: {}",
                        sce.function, sce.violation
                    ),
                    primary,
                    secondary,
                    blame: blame_to_diag_blame(&sce.blame),
                    hints,
                });
            }
            (
                BuildStatus::VerifyFailed {
                    function,
                    description,
                },
                counterexamples.clone(),
                None,
                None,
            )
        }
        VerifyOutcome::Timeout { function } => (
            BuildStatus::VerifyFailed {
                function,
                description: "verification timed out".to_string(),
            },
            vec![],
            Some("timeout".to_string()),
            None,
        ),
        VerifyOutcome::Unknown { function, reason } => (
            BuildStatus::VerifyFailed {
                function,
                description: format!("verification result unknown: {reason}"),
            },
            vec![],
            Some("unknown".to_string()),
            Some(reason),
        ),
        VerifyOutcome::Error { function, message } => (
            BuildStatus::VerifyFailed {
                function,
                description: format!("esbmc error: {message}"),
            },
            vec![],
            Some("error".to_string()),
            Some(message),
        ),
        VerifyOutcome::NotRun => (BuildStatus::Unverified, vec![], None, None),
        VerifyOutcome::SkippedNonModelable => (BuildStatus::Skipped, vec![], None, None),
        VerifyOutcome::Proven => (BuildStatus::Verified, vec![], None, None),
        VerifyOutcome::ToolNotFound => {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: vow_diag::ErrorCode::EsbmcNotFound,
                message: "ESBMC not found; install ESBMC or use --no-verify to skip verification"
                    .to_string(),
                primary: vow_diag::SourceLocation {
                    file: String::new(),
                    byte_offset: 0,
                    byte_len: 0,
                },
                secondary: vec![],
                blame: vow_diag::Blame::None,
                hints: vec![
                    "ESBMC is required for contract verification".to_string(),
                    "use --no-verify to compile without verification".to_string(),
                ],
            });
            (
                BuildStatus::VerifyFailed {
                    function: String::new(),
                    description: "ESBMC not found".to_string(),
                },
                vec![],
                Some("tool_not_found".to_string()),
                Some("ESBMC not found; install ESBMC or use --no-verify".to_string()),
            )
        }
    };

    BuildOutput {
        status,
        executable,
        diagnostics,
        counterexamples,
        verify_status,
        verify_message,
    }
}

/// Fail-closed result for a panicked verifier worker (`join()` → `Err`). A
/// verifier crash leaves verification in an unknown state, so the build reports
/// `VerifyFailed` (exit 1) and withholds the executable — the linked binary is
/// removed so no executable masquerades as built (#413).
pub(crate) fn panicked_output(
    diagnostics: Vec<Diagnostic>,
    executable: Option<PathBuf>,
) -> BuildOutput {
    if let Some(path) = &executable {
        let _ = std::fs::remove_file(path);
    }
    BuildOutput {
        status: BuildStatus::VerifyFailed {
            function: String::new(),
            description: "verification thread panicked".to_string(),
        },
        executable: None,
        diagnostics,
        counterexamples: vec![],
        verify_status: Some("panicked".to_string()),
        verify_message: Some("verification thread panicked".to_string()),
    }
}

/// Smart constructor for a `CompileFailed` [`BuildOutput`]: no executable, no
/// counterexamples, no verify status. Single source of truth for the shape
/// every compile-error path returns.
pub(crate) fn compile_failed(message: String, diagnostics: Vec<Diagnostic>) -> BuildOutput {
    BuildOutput {
        status: BuildStatus::CompileFailed { message },
        executable: None,
        diagnostics,
        counterexamples: vec![],
        verify_status: None,
        verify_message: None,
    }
}

/// Fail a build for a backend error while preserving a stable, structured
/// diagnostic alongside the compatibility `message` field.
pub(crate) fn codegen_failed(
    error: &CodegenError,
    file: &str,
    mut diagnostics: Vec<Diagnostic>,
) -> BuildOutput {
    diagnostics.push(Diagnostic {
        severity: Severity::Error,
        code: error.error_code(),
        message: error.to_string(),
        primary: SourceLocation {
            file: file.to_string(),
            byte_offset: 0,
            byte_len: 0,
        },
        secondary: vec![],
        blame: Blame::None,
        hints: vec![],
    });
    compile_failed(format!("{error:?}"), diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildStatus, CeCallSite, CeSource, CeViolatingArg, StructuredCounterexample};
    use vow_codegen::CodegenError;
    use vow_diag::{Blame, ErrorCode, Severity, SourceLocation};

    fn ce(function: &str, blame: &str) -> StructuredCounterexample {
        StructuredCounterexample {
            function: function.to_string(),
            values: vec![],
            violation: "boom".to_string(),
            vow_id: 1,
            source: None,
            blame: blame.to_string(),
            call_sites: vec![],
            violating_args: vec![],
            execution_path: vec![],
            branch_decisions: vec![],
            replay: None,
            replay_reason: None,
            replay_raw_values: vec![],
            replay_raw_output: String::new(),
        }
    }

    fn diag(message: &str) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: ErrorCode::VowRequiresViolated,
            message: message.to_string(),
            primary: SourceLocation {
                file: String::new(),
                byte_offset: 0,
                byte_len: 0,
            },
            secondary: vec![],
            blame: Blame::None,
            hints: vec![],
        }
    }

    #[test]
    fn proven_maps_to_verified() {
        let out = to_output(VerifyOutcome::Proven, vec![], None);
        assert!(matches!(out.status, BuildStatus::Verified));
        assert!(out.executable.is_none());
        assert!(out.counterexamples.is_empty());
        assert!(out.diagnostics.is_empty());
        assert!(out.verify_status.is_none());
        assert!(out.verify_message.is_none());
    }

    #[test]
    fn not_run_maps_to_unverified() {
        let out = to_output(VerifyOutcome::NotRun, vec![], None);
        assert!(matches!(out.status, BuildStatus::Unverified));
        assert!(out.verify_status.is_none());
    }

    #[test]
    fn skipped_non_modelable_maps_to_skipped() {
        let out = to_output(VerifyOutcome::SkippedNonModelable, vec![], None);
        assert!(matches!(out.status, BuildStatus::Skipped));
    }

    #[test]
    fn timeout_maps_to_verifyfailed_with_timeout_status() {
        let out = to_output(
            VerifyOutcome::Timeout {
                function: "f".to_string(),
            },
            vec![],
            None,
        );
        match &out.status {
            BuildStatus::VerifyFailed {
                function,
                description,
            } => {
                assert_eq!(function, "f");
                assert_eq!(description, "verification timed out");
            }
            other => panic!("expected VerifyFailed, got {other:?}"),
        }
        assert_eq!(out.verify_status.as_deref(), Some("timeout"));
        assert!(out.verify_message.is_none());
    }

    #[test]
    fn unknown_maps_to_verifyfailed_with_reason() {
        let out = to_output(
            VerifyOutcome::Unknown {
                function: "f".to_string(),
                reason: "why".to_string(),
            },
            vec![],
            None,
        );
        match &out.status {
            BuildStatus::VerifyFailed {
                function,
                description,
            } => {
                assert_eq!(function, "f");
                assert_eq!(description, "verification result unknown: why");
            }
            other => panic!("expected VerifyFailed, got {other:?}"),
        }
        assert_eq!(out.verify_status.as_deref(), Some("unknown"));
        assert_eq!(out.verify_message.as_deref(), Some("why"));
    }

    #[test]
    fn error_maps_to_verifyfailed_with_message() {
        let out = to_output(
            VerifyOutcome::Error {
                function: "f".to_string(),
                message: "boom".to_string(),
            },
            vec![],
            None,
        );
        match &out.status {
            BuildStatus::VerifyFailed {
                function,
                description,
            } => {
                assert_eq!(function, "f");
                assert_eq!(description, "esbmc error: boom");
            }
            other => panic!("expected VerifyFailed, got {other:?}"),
        }
        assert_eq!(out.verify_status.as_deref(), Some("error"));
        assert_eq!(out.verify_message.as_deref(), Some("boom"));
    }

    #[test]
    fn tool_not_found_pushes_diagnostic_and_verifyfailed() {
        let out = to_output(VerifyOutcome::ToolNotFound, vec![], None);
        match &out.status {
            BuildStatus::VerifyFailed {
                function,
                description,
            } => {
                assert_eq!(function, "");
                assert_eq!(description, "ESBMC not found");
            }
            other => panic!("expected VerifyFailed, got {other:?}"),
        }
        assert_eq!(out.verify_status.as_deref(), Some("tool_not_found"));
        assert_eq!(
            out.verify_message.as_deref(),
            Some("ESBMC not found; install ESBMC or use --no-verify")
        );
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].code, ErrorCode::EsbmcNotFound);
        assert_eq!(out.diagnostics[0].severity, Severity::Error);
        assert_eq!(out.diagnostics[0].hints.len(), 2);
    }

    #[test]
    fn failed_pushes_one_diagnostic_per_counterexample() {
        let out = to_output(
            VerifyOutcome::Failed {
                function: "f".to_string(),
                description: "contract".to_string(),
                counterexamples: vec![ce("f", "caller"), ce("f", "callee")],
            },
            vec![],
            None,
        );
        assert!(matches!(out.status, BuildStatus::VerifyFailed { .. }));
        assert_eq!(out.counterexamples.len(), 2);
        assert_eq!(out.diagnostics.len(), 2);
        assert!(out.verify_status.is_none());
    }

    #[test]
    fn failed_caller_blame_emits_hint_per_violating_arg() {
        let mut cex = ce("f", "caller");
        cex.source = Some(CeSource {
            file: "a.vow".to_string(),
            offset: 10,
            length: 5,
        });
        cex.call_sites = vec![CeCallSite {
            caller_function: "main".to_string(),
            file: "a.vow".to_string(),
            offset: 20,
            length: 3,
        }];
        cex.violating_args = vec![
            CeViolatingArg {
                param: "x".to_string(),
                value: "0".to_string(),
                arg_offset: 21,
                arg_length: 1,
            },
            CeViolatingArg {
                param: "y".to_string(),
                value: "-1".to_string(),
                arg_offset: 23,
                arg_length: 2,
            },
        ];
        let out = to_output(
            VerifyOutcome::Failed {
                function: "f".to_string(),
                description: "contract".to_string(),
                counterexamples: vec![cex],
            },
            vec![],
            None,
        );
        assert_eq!(out.diagnostics.len(), 1);
        let d = &out.diagnostics[0];
        assert_eq!(d.code, ErrorCode::VowRequiresViolated);
        assert_eq!(d.blame, Blame::Caller);
        // Primary from the CE source; secondary from the call sites.
        assert_eq!(d.primary.file, "a.vow");
        assert_eq!(d.primary.byte_offset, 10);
        assert_eq!(d.secondary.len(), 1);
        assert_eq!(d.secondary[0].byte_offset, 20);
        // One "call site violated" hint + one per violating arg.
        assert_eq!(d.hints.len(), 3);
        assert!(d.hints[0].contains("precondition"));
        assert!(d.hints[1].contains('x'));
        assert!(d.hints[2].contains('y'));
    }

    #[test]
    fn failed_callee_blame_emits_single_hint() {
        let out = to_output(
            VerifyOutcome::Failed {
                function: "f".to_string(),
                description: "contract".to_string(),
                counterexamples: vec![ce("f", "callee")],
            },
            vec![],
            None,
        );
        let d = &out.diagnostics[0];
        assert_eq!(d.code, ErrorCode::VowEnsuresViolated);
        assert_eq!(d.blame, Blame::Callee);
        assert_eq!(d.hints.len(), 1);
        assert!(d.hints[0].contains("postcondition"));
    }

    #[test]
    fn failed_source_none_yields_empty_primary_location() {
        let out = to_output(
            VerifyOutcome::Failed {
                function: "f".to_string(),
                description: "contract".to_string(),
                counterexamples: vec![ce("f", "caller")],
            },
            vec![],
            None,
        );
        let d = &out.diagnostics[0];
        assert_eq!(d.primary.file, "");
        assert_eq!(d.primary.byte_offset, 0);
        assert_eq!(d.primary.byte_len, 0);
    }

    // #585: the arith warning must keep the build Verified (exit 0). A reachable
    // `+!` abort is the operator's specified behaviour, not an unproved
    // obligation, so unlike a skip it does not fail the run closed.
    #[test]
    fn arith_overflow_warning_keeps_status_verified() {
        let out = to_output_with_warnings(
            VerifyOutcome::Proven,
            vec![],
            &[VerifyWarning::ArithOverflow(ArithOverflowWarning {
                function: "twice".to_string(),
                cause: "addition overflows",
                file: "t.vow".to_string(),
                offset: 92,
                length: 6,
            })],
            None,
        );
        assert!(matches!(out.status, BuildStatus::Verified));
        assert_eq!(out.diagnostics.len(), 1);
        let d = &out.diagnostics[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, ErrorCode::ArithOverflowReachable);
        assert_eq!(
            d.message,
            "checked arithmetic in `twice` can abort: addition overflows"
        );
        assert_eq!(d.blame, Blame::None);
        // The span points at the operator, unlike a skip warning's empty span.
        assert_eq!(d.primary.file, "t.vow");
        assert_eq!(d.primary.byte_offset, 92);
        assert_eq!(d.primary.byte_len, 6);
    }

    fn arith_warn(function: &str, file: &str, offset: u32) -> VerifyWarning {
        VerifyWarning::ArithOverflow(ArithOverflowWarning {
            function: function.to_string(),
            cause: "addition overflows",
            file: file.to_string(),
            offset,
            length: 6,
        })
    }

    // #585: one checked-arithmetic site can be reported by several verify
    // targets, because a model co-emits its modelable callees. Once each is
    // attributed to its true owner they are the same diagnostic, and the reader
    // should see it once.
    #[test]
    fn identical_arith_warnings_are_reported_once() {
        let out = to_output_with_warnings(
            VerifyOutcome::Proven,
            vec![],
            &[
                arith_warn("helper", "helper.vow", 99),
                arith_warn("helper", "helper.vow", 99),
                arith_warn("helper", "helper.vow", 99),
            ],
            None,
        );
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].code, ErrorCode::ArithOverflowReachable);
    }

    // Distinct sites must all survive — dedup keys on the rendered identity, so
    // a different function, file, or offset is a different diagnostic.
    #[test]
    fn distinct_arith_warnings_are_all_reported() {
        let out = to_output_with_warnings(
            VerifyOutcome::Proven,
            vec![],
            &[
                arith_warn("helper", "helper.vow", 99),
                arith_warn("other", "helper.vow", 99),
                arith_warn("helper", "other.vow", 99),
                arith_warn("helper", "helper.vow", 120),
            ],
            None,
        );
        assert_eq!(out.diagnostics.len(), 4);
    }

    // Dedup preserves first-seen order, so reporting stays deterministic.
    #[test]
    fn dedup_preserves_first_seen_order() {
        let out = to_output_with_warnings(
            VerifyOutcome::Proven,
            vec![],
            &[
                arith_warn("b", "b.vow", 2),
                arith_warn("a", "a.vow", 1),
                arith_warn("b", "b.vow", 2),
            ],
            None,
        );
        let names: Vec<&str> = out
            .diagnostics
            .iter()
            .map(|d| d.primary.file.as_str())
            .collect();
        assert_eq!(names, ["b.vow", "a.vow"]);
    }

    #[test]
    fn to_output_with_warnings_prepends_warning_before_ce_errors() {
        let existing = diag("pre-existing");
        let out = to_output_with_warnings(
            VerifyOutcome::Failed {
                function: "f".to_string(),
                description: "contract".to_string(),
                counterexamples: vec![ce("f", "caller")],
            },
            vec![existing],
            &[VerifyWarning::Skipped(SkippedFunction {
                function: "g".to_string(),
                reason: "nonmodelable".to_string(),
            })],
            None,
        );
        assert_eq!(out.diagnostics.len(), 3);
        // Order: input diagnostics, then skipped warnings, then CE errors.
        assert_eq!(out.diagnostics[0].message, "pre-existing");
        assert_eq!(out.diagnostics[1].severity, Severity::Warning);
        assert_eq!(out.diagnostics[1].code, ErrorCode::VerificationSkipped);
        assert_eq!(out.diagnostics[2].severity, Severity::Error);
    }

    #[test]
    fn skipped_warning_has_expected_shape() {
        let out = to_output_with_warnings(
            VerifyOutcome::Proven,
            vec![],
            &[VerifyWarning::Skipped(SkippedFunction {
                function: "h".to_string(),
                reason: "why".to_string(),
            })],
            None,
        );
        assert!(matches!(out.status, BuildStatus::Verified));
        assert_eq!(out.diagnostics.len(), 1);
        let d = &out.diagnostics[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, ErrorCode::VerificationSkipped);
        assert_eq!(d.message, "skipped verification of `h`: why");
        assert_eq!(d.blame, Blame::None);
        assert_eq!(d.hints.len(), 1);
        assert_eq!(d.primary.file, "");
        assert_eq!(d.primary.byte_offset, 0);
        assert_eq!(d.primary.byte_len, 0);
    }

    #[test]
    fn to_output_adds_no_skipped_warnings() {
        let out = to_output(VerifyOutcome::Proven, vec![], None);
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn blame_to_error_code_maps_caller_callee_and_fallback() {
        assert_eq!(
            blame_to_error_code("caller"),
            ErrorCode::VowRequiresViolated
        );
        assert_eq!(blame_to_error_code("callee"), ErrorCode::VowEnsuresViolated);
        assert_eq!(
            blame_to_error_code("nonsense"),
            ErrorCode::VowRequiresViolated
        );
    }

    #[test]
    fn blame_to_diag_blame_maps_caller_callee_and_fallback() {
        assert_eq!(blame_to_diag_blame("caller"), Blame::Caller);
        assert_eq!(blame_to_diag_blame("callee"), Blame::Callee);
        assert_eq!(blame_to_diag_blame("nonsense"), Blame::None);
    }

    #[test]
    fn blame_fallback_is_asymmetric() {
        // Characterises today's behaviour: an unknown blame yields a *caller*
        // error code but a *None* blame. Pinned so a future fix is deliberate.
        assert_eq!(blame_to_error_code("?"), ErrorCode::VowRequiresViolated);
        assert_eq!(blame_to_diag_blame("?"), Blame::None);
    }

    #[test]
    fn panicked_output_removes_executable_and_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("exe");
        std::fs::write(&path, b"binary").expect("write exe");
        assert!(path.exists());

        let out = panicked_output(vec![], Some(path.clone()));

        assert!(!path.exists(), "executable must be removed (fail-closed)");
        assert!(out.executable.is_none());
        match &out.status {
            BuildStatus::VerifyFailed {
                function,
                description,
            } => {
                assert_eq!(function, "");
                assert_eq!(description, "verification thread panicked");
            }
            other => panic!("expected VerifyFailed, got {other:?}"),
        }
        assert_eq!(out.verify_status.as_deref(), Some("panicked"));
        assert_eq!(
            out.verify_message.as_deref(),
            Some("verification thread panicked")
        );
    }

    #[test]
    fn panicked_output_with_no_executable_is_ok() {
        let out = panicked_output(vec![], None);
        assert!(out.executable.is_none());
        assert!(matches!(out.status, BuildStatus::VerifyFailed { .. }));
    }

    #[test]
    fn compile_failed_builds_expected_output() {
        let out = compile_failed("bad".to_string(), vec![diag("parse error")]);
        match &out.status {
            BuildStatus::CompileFailed { message } => assert_eq!(message, "bad"),
            other => panic!("expected CompileFailed, got {other:?}"),
        }
        assert!(out.executable.is_none());
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].message, "parse error");
        assert!(out.counterexamples.is_empty());
        assert!(out.verify_status.is_none());
        assert!(out.verify_message.is_none());
    }

    #[test]
    fn codegen_failed_attaches_one_structured_diagnostic() {
        let error = CodegenError::UnsupportedOpcode("wide aggregate".to_string());
        let out = codegen_failed(&error, "wide.vow", vec![diag("frontend note")]);

        assert!(matches!(out.status, BuildStatus::CompileFailed { .. }));
        assert!(out.executable.is_none());
        assert_eq!(out.diagnostics.len(), 2);
        assert_eq!(out.diagnostics[0].message, "frontend note");
        assert_eq!(out.diagnostics[1].severity, Severity::Error);
        assert_eq!(out.diagnostics[1].code, ErrorCode::CodegenUnsupported);
        assert_eq!(out.diagnostics[1].message, error.to_string());
        assert_eq!(
            out.diagnostics[1].primary,
            SourceLocation {
                file: "wide.vow".to_string(),
                byte_offset: 0,
                byte_len: 0,
            }
        );
        assert!(out.diagnostics[1].secondary.is_empty());
        assert_eq!(out.diagnostics[1].blame, Blame::None);
        assert!(out.counterexamples.is_empty());
    }
}
