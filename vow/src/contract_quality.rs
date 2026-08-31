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
    // "References no program value" is tested by the absence of an identifier
    // character, so cast syntax has to come out first: `0 as i64 >= 0` is as
    // constant as `0 >= 0`, but its `as i64` would otherwise read as a value
    // reference and hide the tautology.
    if predicate.is_empty()
        || predicate == "true"
        || !without_casts(predicate)
            .chars()
            .any(|c| c.is_ascii_alphabetic())
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
    // Casts come out before any operator scanning, not after the split. The
    // self-hosted printer renders a cast as ` as <type>` (`compiler/lower.vow`),
    // and those angle brackets would otherwise be picked up as the `<` / `>`
    // comparison operator — splitting `result > 0 as <type>` at the wrong place
    // and classifying it `substantive` where the Rust printer's ` as i64` reads
    // `weak`. Stripping first makes the two compilers agree by construction.
    let pred = &without_casts(pred);
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

/// Both sides arrive already cast-stripped by `is_weak_result_bound`, so
/// `result >= 0 as i64` reads exactly as `result >= 0` does here.
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

/// The text with every ` as <type>` cast removed, trimmed.
///
/// A cast is transparent to shape classification: `0 as u64` bounds a value
/// exactly as `0` does, and `result as i64` names the same value as `result`.
/// Only a primitive numeric type name is stripped, so `x as Foo` is left alone.
/// The self-hosted printer emits the placeholder `as <type>` rather than the
/// type name (`compiler/lower.vow`), so that form is accepted too and both
/// compilers classify the same clause identically.
fn without_casts(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(" as ") {
        let after = &rest[pos + 4..];
        let end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '<' && c != '>')
            .unwrap_or(after.len());
        if is_cast_target(&after[..end]) {
            out.push_str(&rest[..pos]);
            rest = &after[end..];
        } else {
            out.push_str(&rest[..pos + 4]);
            rest = after;
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

fn is_cast_target(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "f32"
            | "f64"
            | "<type>"
    )
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
    fn casts_are_transparent_to_the_weak_shape() {
        // A cast does not make a constant bound substantive. #1104's cast bridge
        // (`v.len() as u64` at every consuming site) produces exactly these, so
        // without the fold the `WEAK_MAX = 0` ratchet would stop meaning
        // anything the day the migration starts.
        assert_eq!(
            quality_of("ensures result >= 0 as i64"),
            ContractQuality::Weak
        );
        assert_eq!(
            quality_of("ensures result as i64 >= 0"),
            ContractQuality::Weak
        );
        assert_eq!(
            quality_of("ensures result <= 100 as i32"),
            ContractQuality::Weak
        );
        // Chained casts fold too.
        assert_eq!(
            quality_of("ensures result >= 0 as i32 as i64"),
            ContractQuality::Weak
        );
        // The self-hosted printer emits a placeholder instead of the type name,
        // so both compilers must reach the same verdict on the same clause.
        assert_eq!(
            quality_of("ensures result >= 0 as <type>"),
            ContractQuality::Weak
        );
        // The strict operators are the regression case: the `<` and `>` inside
        // the `<type>` placeholder must not be read as the comparison operator.
        // Splitting before the cast came out classified these `substantive`
        // self-hosted while the Rust printer's ` as i64` read `weak`.
        assert_eq!(
            quality_of("ensures result > 0 as i64"),
            ContractQuality::Weak
        );
        assert_eq!(
            quality_of("ensures result > 0 as <type>"),
            ContractQuality::Weak
        );
        assert_eq!(
            quality_of("ensures result < 100 as <type>"),
            ContractQuality::Weak
        );
        assert_eq!(
            quality_of("ensures result as <type> > 0"),
            ContractQuality::Weak
        );
    }

    #[test]
    fn a_cast_does_not_turn_a_value_into_a_literal() {
        // Only a genuine literal is a weak bound; `x as i64` is still a program
        // value, so the comparison stays substantive.
        assert_eq!(
            quality_of("ensures result >= x as i64"),
            ContractQuality::Substantive
        );
        // A non-primitive cast target is not cast syntax we recognise.
        assert_eq!(
            quality_of("ensures result >= n as Foo"),
            ContractQuality::Substantive
        );
    }

    #[test]
    fn flags_tautologies() {
        assert_eq!(quality_of("ensures true"), ContractQuality::Tautological);
        // No reference to any program value — constant comparison.
        assert_eq!(quality_of("ensures 0 >= 0"), ContractQuality::Tautological);
        // Cast syntax must not read as a value reference and hide the tautology.
        assert_eq!(
            quality_of("ensures 0 as i64 >= 0"),
            ContractQuality::Tautological
        );
        assert_eq!(
            quality_of("ensures 0 as <type> >= 0 as <type>"),
            ContractQuality::Tautological
        );
    }

    #[test]
    fn analyze_pairs_kind_and_quality() {
        let a = analyze("ensures result >= 0");
        assert_eq!(a.kind, ClauseKind::Ensures);
        assert_eq!(a.quality, ContractQuality::Weak);
    }
}
