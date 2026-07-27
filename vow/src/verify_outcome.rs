//! Translation layer from a terminal build/verify outcome into the external
//! [`BuildOutput`] result shape.
//!
//! The verification driver (`run_verification_sync` / `verify_one_function` in
//! `main.rs`) produces a [`VerifyOutcome`] verdict plus a list of
//! [`SkippedFunction`]s. This module owns that vocabulary and is the single
//! place that turns any terminal outcome — a proof, a counterexample, a
//! compile failure, or a panicked verifier — into a [`BuildOutput`] with the
//! matching diagnostics. `BuildOutput`/`BuildStatus`/`StructuredCounterexample`
//! remain the crate's shared output vocabulary and live in `main.rs`.

use std::path::PathBuf;

use vow_diag::Diagnostic;

use crate::{BuildOutput, SkippedFunction, VerifyOutcome};

/// Map a counterexample `blame` string to the diagnostic error code.
///
/// Note the fallback asymmetry with [`blame_to_diag_blame`]: an unrecognised
/// blame maps to `VowRequiresViolated` (a *caller* code) here, but to
/// `Blame::None` there. This is preserved behaviour, not a fix.
fn blame_to_error_code(_blame: &str) -> vow_diag::ErrorCode {
    todo!("moved in GREEN")
}

fn blame_to_diag_blame(_blame: &str) -> vow_diag::Blame {
    todo!("moved in GREEN")
}

/// Translate a [`VerifyOutcome`] into a [`BuildOutput`], appending any
/// verification-failure diagnostics to `diagnostics`.
pub(crate) fn to_output(
    _outcome: VerifyOutcome,
    _diagnostics: Vec<Diagnostic>,
    _executable: Option<PathBuf>,
) -> BuildOutput {
    todo!("moved in GREEN")
}

/// As [`to_output`], but also emits a `VerificationSkipped` warning for each
/// vowed function the verifier skipped. Skipped warnings are appended *before*
/// counterexample errors.
pub(crate) fn to_output_with_skipped(
    _outcome: VerifyOutcome,
    _diagnostics: Vec<Diagnostic>,
    _skipped: &[SkippedFunction],
    _executable: Option<PathBuf>,
) -> BuildOutput {
    todo!("moved in GREEN")
}

/// Fail-closed result for a panicked verifier worker (`join()` → `Err`). A
/// verifier crash leaves verification in an unknown state, so the build reports
/// `VerifyFailed` (exit 1) and withholds the executable — the linked binary is
/// removed so no executable masquerades as built (#413).
pub(crate) fn panicked_output(
    _diagnostics: Vec<Diagnostic>,
    _executable: Option<PathBuf>,
) -> BuildOutput {
    todo!("moved in GREEN")
}

/// Smart constructor for a `CompileFailed` [`BuildOutput`]: no executable, no
/// counterexamples, no verify status. Single source of truth for the shape
/// every compile-error path returns.
pub(crate) fn compile_failed(_message: String, _diagnostics: Vec<Diagnostic>) -> BuildOutput {
    todo!("moved in GREEN")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildStatus, CeCallSite, CeSource, CeViolatingArg, StructuredCounterexample};
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

    #[test]
    fn to_output_with_skipped_prepends_warning_before_ce_errors() {
        let existing = diag("pre-existing");
        let out = to_output_with_skipped(
            VerifyOutcome::Failed {
                function: "f".to_string(),
                description: "contract".to_string(),
                counterexamples: vec![ce("f", "caller")],
            },
            vec![existing],
            &[SkippedFunction {
                function: "g".to_string(),
                reason: "nonmodelable".to_string(),
            }],
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
        let out = to_output_with_skipped(
            VerifyOutcome::Proven,
            vec![],
            &[SkippedFunction {
                function: "h".to_string(),
                reason: "why".to_string(),
            }],
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
}
