//! Static, no-ESBMC quality classification of contract clauses by shape.
//!
//! A contract clause is described by a `clause_description`-formatted string of
//! the form `"{kind} {printed_predicate}"` (e.g. `"ensures result >= 0"`). This
//! module owns the two purely-syntactic verdicts that `vow contracts` derives
//! from that string without ever invoking ESBMC:
//!
//! - the [`ClauseKind`] the clause leads with, and
//! - the [`ContractQuality`] of its predicate's *shape*.
//!
//! [`analyze`] computes both in one pass. The verdicts are independent of a
//! clause's proof `status`; see `docs/spec/contracts-methodology.md` for the
//! taxonomy this implements. The classifier is deliberately conservative:
//! anything it cannot prove weak or tautological is reported `Substantive`, so
//! it never over-flags a meaningful contract.

/// The clause keyword a contract description leads with.
///
/// `Unknown` covers descriptions that do not begin with one of the three
/// contract keywords — it is a parse fallback, never a valid clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseKind {
    Requires,
    Ensures,
    Invariant,
    Unknown,
}

impl ClauseKind {
    /// Canonical wire string, matching the `kind` field of the
    /// `contracts-result` JSON schema. Must stay byte-stable.
    pub fn as_str(self) -> &'static str {
        match self {
            ClauseKind::Requires => "requires",
            ClauseKind::Ensures => "ensures",
            ClauseKind::Invariant => "invariant",
            ClauseKind::Unknown => "unknown",
        }
    }
}

/// Static shape classification of a clause's predicate, independent of whether
/// the clause is proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractQuality {
    /// The predicate is the constant `true` or references no program value at
    /// all (e.g. `0 >= 0`) — it constrains nothing. A `false` predicate is a
    /// contradiction, not a tautology, so it is left [`Substantive`] here;
    /// flagging it as vacuous is the deferred `false` re-check.
    ///
    /// [`Substantive`]: ContractQuality::Substantive
    Tautological,
    /// An `ensures` that only bounds `result` by an integer literal on one side
    /// (e.g. `result >= 0`, `result > 0`, `result <= 3`). Satisfiable by almost
    /// any implementation — the 354-contract trap #81 was filed over.
    Weak,
    /// Everything else (equality, relational, inverse, calls).
    Substantive,
}

impl ContractQuality {
    /// Canonical wire string, matching the `quality` field of the
    /// `contracts-result` JSON schema. Must stay byte-stable.
    pub fn as_str(self) -> &'static str {
        match self {
            ContractQuality::Tautological => "tautological",
            ContractQuality::Weak => "weak",
            ContractQuality::Substantive => "substantive",
        }
    }
}

/// The kind and static shape of one contract clause, both derived from its
/// description in a single pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClauseAnalysis {
    pub kind: ClauseKind,
    pub quality: ContractQuality,
}

/// Classify a `clause_description`-formatted string by shape, returning both its
/// [`ClauseKind`] and [`ContractQuality`].
pub fn analyze(description: &str) -> ClauseAnalysis {
    let kind = kind_from_description(description);
    let quality = classify(kind, predicate_text(description));
    ClauseAnalysis { kind, quality }
}

/// Parse the leading `requires`/`ensures`/`invariant` keyword.
fn kind_from_description(description: &str) -> ClauseKind {
    if description.starts_with("requires") {
        ClauseKind::Requires
    } else if description.starts_with("ensures") {
        ClauseKind::Ensures
    } else if description.starts_with("invariant") {
        ClauseKind::Invariant
    } else {
        ClauseKind::Unknown
    }
}

/// Strip the leading keyword from a clause description, leaving the predicate
/// text. `clause_description` formats every vow as `"{kind} {printed_expr}"`, so
/// the predicate is everything after the first space.
fn predicate_text(description: &str) -> &str {
    match description.split_once(' ') {
        Some((_, rest)) => rest.trim(),
        None => "",
    }
}

fn classify(kind: ClauseKind, predicate: &str) -> ContractQuality {
    if predicate.is_empty()
        || predicate == "true"
        || !predicate.chars().any(|c| c.is_ascii_alphabetic())
    {
        return ContractQuality::Tautological;
    }
    if kind == ClauseKind::Ensures && is_weak_result_bound(predicate) {
        return ContractQuality::Weak;
    }
    ContractQuality::Substantive
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

    fn quality_of(description: &str) -> ContractQuality {
        analyze(description).quality
    }

    #[test]
    fn analyze_reads_the_leading_keyword_as_the_clause_kind() {
        assert_eq!(analyze("requires v <= 255").kind, ClauseKind::Requires);
        assert_eq!(analyze("ensures result >= 0").kind, ClauseKind::Ensures);
        assert_eq!(analyze("invariant lo <= hi").kind, ClauseKind::Invariant);
        // A description that does not lead with a contract keyword.
        assert_eq!(analyze("nonsense clause").kind, ClauseKind::Unknown);
    }

    #[test]
    fn clause_kind_wire_strings_are_stable() {
        assert_eq!(ClauseKind::Requires.as_str(), "requires");
        assert_eq!(ClauseKind::Ensures.as_str(), "ensures");
        assert_eq!(ClauseKind::Invariant.as_str(), "invariant");
        assert_eq!(ClauseKind::Unknown.as_str(), "unknown");
    }

    #[test]
    fn contract_quality_wire_strings_are_stable() {
        assert_eq!(ContractQuality::Weak.as_str(), "weak");
        assert_eq!(ContractQuality::Tautological.as_str(), "tautological");
        assert_eq!(ContractQuality::Substantive.as_str(), "substantive");
    }

    #[test]
    fn flags_weak_result_bounds() {
        // The 354-contract trap: an ensures that only bounds result by a constant.
        assert_eq!(quality_of("ensures result >= 0"), ContractQuality::Weak);
        assert_eq!(quality_of("ensures result > 0"), ContractQuality::Weak);
        assert_eq!(quality_of("ensures result <= 3"), ContractQuality::Weak);
        // result vs negative literal is still a constant bound.
        assert_eq!(quality_of("ensures result >= -1"), ContractQuality::Weak);
        // Strict single-char operator path (`<`, not `<=`).
        assert_eq!(quality_of("ensures result < 3"), ContractQuality::Weak);
    }

    #[test]
    fn keeps_substantive_clauses() {
        // Equality, relational, inverse, totality, and call shapes are not weak.
        assert_eq!(
            quality_of("ensures result == val * 4 + kind"),
            ContractQuality::Substantive
        );
        assert_eq!(
            quality_of("ensures result != -1"),
            ContractQuality::Substantive
        );
        assert_eq!(
            quality_of("ensures result >= a"),
            ContractQuality::Substantive
        );
        assert_eq!(
            quality_of("ensures item_kind(result) == kind"),
            ContractQuality::Substantive
        );
        // A one-sided bound is a legitimate precondition, not a weak postcondition.
        assert_eq!(
            quality_of("requires v <= 255"),
            ContractQuality::Substantive
        );
        // A `false` predicate is a contradiction, not a tautology; the static
        // classifier leaves it substantive (vacuity detection is a follow-up).
        assert_eq!(quality_of("ensures false"), ContractQuality::Substantive);
    }

    #[test]
    fn flags_tautologies() {
        assert_eq!(quality_of("ensures true"), ContractQuality::Tautological);
        // No reference to any program value — constant comparison.
        assert_eq!(quality_of("ensures 0 >= 0"), ContractQuality::Tautological);
    }

    #[test]
    fn analyze_pairs_kind_and_quality() {
        let a = analyze("ensures result >= 0");
        assert_eq!(a.kind, ClauseKind::Ensures);
        assert_eq!(a.quality, ContractQuality::Weak);
    }
}
