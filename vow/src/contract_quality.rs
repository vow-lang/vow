//! Static, no-ESBMC classification of contract clauses by shape.
//!
//! This is the "is this contract actually meaningful?" methodology from
//! `docs/spec/contracts-methodology.md` (issue #81), lifted out of the CLI
//! driver into a deep module. The two-enum interface — [`VowKind`] and
//! [`ContractQuality`] — hides the predicate-shape analysis (the four private
//! helpers below); the module boundary is the test surface. Kinds and qualities
//! are typed here and only lowered to their JSON strings via `as_str` at the
//! `vow contracts` output boundary, so a mistyped verdict can no longer slip
//! through as a bare `&str`.

/// The clause keyword a contract description begins with. `clause_description`
/// formats every vow as `"{kind} {printed_expr}"`, so the keyword is the prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VowKind {
    Requires,
    Ensures,
    Invariant,
    Unknown,
}

impl VowKind {
    /// Classify a clause by its leading keyword.
    pub(crate) fn from_description(desc: &str) -> VowKind {
        if desc.starts_with("requires") {
            VowKind::Requires
        } else if desc.starts_with("ensures") {
            VowKind::Ensures
        } else if desc.starts_with("invariant") {
            VowKind::Invariant
        } else {
            VowKind::Unknown
        }
    }

    /// The exact lowercase string the `vow contracts` JSON exposes.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            VowKind::Requires => "requires",
            VowKind::Ensures => "ensures",
            VowKind::Invariant => "invariant",
            VowKind::Unknown => "unknown",
        }
    }
}

/// Static quality verdict for a contract clause.
///
/// - `Tautological`: the predicate is the constant `true` or references no
///   program value at all (e.g. `0 >= 0`) — it constrains nothing. A `false`
///   predicate is a contradiction, not a tautology, so it stays `Substantive`
///   here; flagging it as vacuous is the deferred `false` re-check.
/// - `Weak`: an `ensures` that only bounds `result` by an integer literal on one
///   side (e.g. `result >= 0`, `result <= 3`). Satisfiable by almost any
///   implementation — the 354-contract trap #81 was filed over.
/// - `Substantive`: everything else (equality, relational, inverse, calls).
///
/// The classifier is deliberately conservative: anything it cannot prove weak is
/// reported `Substantive`, so it never over-flags a meaningful contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContractQuality {
    Tautological,
    Weak,
    Substantive,
}

impl ContractQuality {
    /// Classify a clause of the given `kind` by the shape of its predicate.
    pub(crate) fn classify(kind: VowKind, description: &str) -> ContractQuality {
        let p = predicate_text(description);
        if p.is_empty() || p == "true" || !p.chars().any(|c| c.is_ascii_alphabetic()) {
            return ContractQuality::Tautological;
        }
        if kind == VowKind::Ensures && is_weak_result_bound(p) {
            return ContractQuality::Weak;
        }
        ContractQuality::Substantive
    }

    /// The exact lowercase string the `vow contracts` JSON exposes.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ContractQuality::Tautological => "tautological",
            ContractQuality::Weak => "weak",
            ContractQuality::Substantive => "substantive",
        }
    }
}

/// Strip the leading `requires`/`ensures`/`invariant` keyword from a clause
/// description, leaving the predicate text (everything after the first space,
/// trimmed). Returns `""` when there is no predicate.
pub(crate) fn predicate_text(description: &str) -> &str {
    match description.split_once(' ') {
        Some((_, rest)) => rest.trim(),
        None => "",
    }
}

/// True when `pred` is a single ordering comparison between `result` and an
/// integer literal — the weak postcondition shape. Compound predicates,
/// equalities, and calls are excluded (they are potentially substantive).
fn is_weak_result_bound(pred: &str) -> bool {
    if pred.contains("&&")
        || pred.contains("||")
        || pred.contains("==")
        || pred.contains("!=")
        || pred.contains('(')
    {
        return false;
    }
    for op in ["<=", ">="] {
        if let Some((lhs, rhs)) = pred.split_once(op) {
            return is_weak_result_comparison(lhs, rhs);
        }
    }
    for op in ['<', '>'] {
        if let Some((lhs, rhs)) = pred.split_once(op) {
            return is_weak_result_comparison(lhs, rhs);
        }
    }
    false
}

fn is_weak_result_comparison(lhs: &str, rhs: &str) -> bool {
    let lhs = lhs.trim();
    let rhs = rhs.trim();
    // Reject anything with a second comparison operator on either side.
    if has_ordering_op(lhs) || has_ordering_op(rhs) {
        return false;
    }
    (lhs == "result" && is_int_literal(rhs)) || (rhs == "result" && is_int_literal(lhs))
}

fn has_ordering_op(s: &str) -> bool {
    s.contains('<') || s.contains('>')
}

fn is_int_literal(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- VowKind::from_description ---------------------------------------

    #[test]
    fn kind_is_read_from_the_leading_keyword() {
        assert_eq!(
            VowKind::from_description("requires x >= 0"),
            VowKind::Requires
        );
        assert_eq!(
            VowKind::from_description("ensures result == x"),
            VowKind::Ensures
        );
        assert_eq!(
            VowKind::from_description("invariant lo <= hi"),
            VowKind::Invariant
        );
        // A description that does not begin with a known keyword is Unknown.
        assert_eq!(VowKind::from_description("result >= 0"), VowKind::Unknown);
        assert_eq!(VowKind::from_description(""), VowKind::Unknown);
    }

    #[test]
    fn kind_as_str_matches_the_json_contract() {
        assert_eq!(VowKind::Requires.as_str(), "requires");
        assert_eq!(VowKind::Ensures.as_str(), "ensures");
        assert_eq!(VowKind::Invariant.as_str(), "invariant");
        assert_eq!(VowKind::Unknown.as_str(), "unknown");
    }

    // --- predicate_text --------------------------------------------------

    #[test]
    fn predicate_text_drops_the_keyword() {
        assert_eq!(predicate_text("ensures result >= 0"), "result >= 0");
        assert_eq!(predicate_text("requires a + b > 0"), "a + b > 0");
        // No space => no predicate.
        assert_eq!(predicate_text("ensures"), "");
        assert_eq!(predicate_text(""), "");
    }

    // --- ContractQuality::as_str ----------------------------------------

    #[test]
    fn quality_as_str_matches_the_json_contract() {
        assert_eq!(ContractQuality::Tautological.as_str(), "tautological");
        assert_eq!(ContractQuality::Weak.as_str(), "weak");
        assert_eq!(ContractQuality::Substantive.as_str(), "substantive");
    }

    // --- ContractQuality::classify: weak (issue #81) --------------------

    #[test]
    fn classify_flags_weak_result_bounds() {
        // The 354-contract trap: an ensures that only bounds result by a constant.
        let weak = |d: &str| ContractQuality::classify(VowKind::Ensures, d);
        assert_eq!(weak("ensures result >= 0"), ContractQuality::Weak);
        assert_eq!(weak("ensures result > 0"), ContractQuality::Weak);
        assert_eq!(weak("ensures result <= 3"), ContractQuality::Weak);
        // result vs negative literal is still a constant bound.
        assert_eq!(weak("ensures result >= -1"), ContractQuality::Weak);
        // Strict single-char operator path (`<`, not `<=`).
        assert_eq!(weak("ensures result < 3"), ContractQuality::Weak);
        // The literal may sit on either side of the operator.
        assert_eq!(weak("ensures 0 <= result"), ContractQuality::Weak);
    }

    // --- ContractQuality::classify: substantive -------------------------

    #[test]
    fn classify_keeps_substantive_clauses() {
        // Equality, relational, inverse, totality, and call shapes are not weak.
        let ens = |d: &str| ContractQuality::classify(VowKind::Ensures, d);
        assert_eq!(
            ens("ensures result == val * 4 + kind"),
            ContractQuality::Substantive
        );
        assert_eq!(ens("ensures result != -1"), ContractQuality::Substantive);
        // Bounded by a program variable, not a literal.
        assert_eq!(ens("ensures result >= a"), ContractQuality::Substantive);
        assert_eq!(
            ens("ensures item_kind(result) == kind"),
            ContractQuality::Substantive
        );
        // A one-sided bound is a legitimate precondition, not a weak postcondition.
        assert_eq!(
            ContractQuality::classify(VowKind::Requires, "requires v <= 255"),
            ContractQuality::Substantive
        );
        // A `false` predicate is a contradiction, not a tautology; the static
        // classifier leaves it substantive (vacuity detection is a follow-up).
        assert_eq!(ens("ensures false"), ContractQuality::Substantive);
    }

    #[test]
    fn classify_rejects_compound_and_double_ordering_predicates() {
        let ens = |d: &str| ContractQuality::classify(VowKind::Ensures, d);
        // Conjunctions/disjunctions of bounds are not the weak single-bound shape.
        assert_eq!(
            ens("ensures result >= 0 && result <= 10"),
            ContractQuality::Substantive
        );
        assert_eq!(
            ens("ensures result < 0 || result > 10"),
            ContractQuality::Substantive
        );
        // A range written as one chained comparison is not a single literal bound.
        assert_eq!(
            ens("ensures 0 <= result < 10"),
            ContractQuality::Substantive
        );
    }

    // --- ContractQuality::classify: tautological ------------------------

    #[test]
    fn classify_flags_tautologies() {
        let ens = |d: &str| ContractQuality::classify(VowKind::Ensures, d);
        assert_eq!(ens("ensures true"), ContractQuality::Tautological);
        // No reference to any program value — constant comparison.
        assert_eq!(ens("ensures 0 >= 0"), ContractQuality::Tautological);
    }
}
