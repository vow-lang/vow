//! Pure rendering of the `--mode profile` call-count report.
//!
//! The `report()` closure registered with `atexit` in `lib.rs`
//! (`__vow_profile_init`) is the side-effecting adapter: it locks the global
//! `PROFILE_COUNTERS` map, collects its entries, calls into this module to build
//! the report text, and writes it to stderr. All the reporting logic — the
//! descending call-count sort, the total, the per-function percentages, the
//! top-20 truncation, and the column layout documented in `docs/spec/cli.md`
//! (§"Profile Output") — lives here, free of `unsafe`, global state, and process
//! exit, so it can be unit-tested directly.

use std::fmt::Write;

/// The report lists at most this many of the most-called functions.
/// Documented in `docs/spec/cli.md` (§"Profile Output").
const PROFILE_REPORT_TOP_N: usize = 20;

/// Render the `--mode profile` call-count report from a set of
/// `(function name, call count)` entries.
///
/// Entries need not be sorted: they are ordered by descending call count (ties
/// keep their input order via a stable sort). Returns `None` when there is
/// nothing to report, so the caller emits no output at all; otherwise the
/// returned string is the complete report, newline-terminated and ready to write
/// verbatim to stderr.
pub(crate) fn render_profile_report(entries: &[(&str, u64)]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut sorted: Vec<(&str, u64)> = entries.to_vec();
    sorted.sort_by_key(|item| std::cmp::Reverse(item.1));
    let total: u64 = sorted.iter().map(|item| item.1).sum();
    let unique = sorted.len();
    let limit = unique.min(PROFILE_REPORT_TOP_N);

    let mut out = String::new();
    let _ = writeln!(out, "\n--- vow profile report ---");
    let _ = writeln!(out, "{:<40} {:>12} {:>7}", "function", "calls", "%");
    let _ = writeln!(out, "{}", "-".repeat(61));
    for (name, count) in &sorted[..limit] {
        let pct = (*count as f64 / total as f64) * 100.0;
        let _ = writeln!(out, "{:<40} {:>12} {:>6.1}%", name, count, pct);
    }
    if unique > limit {
        let _ = writeln!(out, "  ... and {} more functions", unique - limit);
    }
    let _ = writeln!(out, "{}", "-".repeat(61));
    let _ = writeln!(out, "total calls: {total}, unique functions: {unique}");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Function names of the report's data rows, in listed order. A data row's
    // first whitespace-delimited token is the function name; the header ("function"),
    // rules, overflow ("..."), and footer ("total") tokens are never in `known`.
    fn data_row_names(out: &str, known: &[&str]) -> Vec<String> {
        out.lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|tok| known.contains(tok))
            .map(str::to_string)
            .collect()
    }

    // Independent oracle: nothing to report -> no output.
    #[test]
    fn returns_none_for_empty() {
        assert_eq!(render_profile_report(&[]), None);
    }

    // Independent oracle: bytes captured from the pre-refactor report() body
    // (vow-runtime/src/lib.rs) run against this exact (unsorted) fixture.
    // Anchors the column layout in docs/spec/cli.md §"Profile Output".
    #[test]
    fn matches_pre_refactor_golden_small() {
        let out = render_profile_report(&[("main", 40), ("infer", 60)]).unwrap();
        assert_eq!(
            out,
            "\n--- vow profile report ---\nfunction                                        calls       %\n-------------------------------------------------------------\ninfer                                              60   60.0%\nmain                                               40   40.0%\n-------------------------------------------------------------\ntotal calls: 100, unique functions: 2\n"
        );
    }

    // Independent oracle: rows must appear in descending call-count order
    // regardless of input order.
    #[test]
    fn sorts_descending_by_count() {
        let out = render_profile_report(&[("a", 1), ("b", 3), ("c", 2)]).unwrap();
        assert_eq!(data_row_names(&out, &["a", "b", "c"]), ["b", "c", "a"]);
    }

    // Independent oracle: percentages are call-count / total. 3/4 = 75.0, 1/4 = 25.0.
    #[test]
    fn computes_percentages_from_total() {
        let out = render_profile_report(&[("x", 3), ("y", 1)]).unwrap();
        assert!(out.contains("75.0%"), "expected 75.0% in:\n{out}");
        assert!(out.contains("25.0%"), "expected 25.0% in:\n{out}");
    }

    // Independent oracle: percentages round to one decimal. 2/3 = 66.66.. -> 66.7,
    // 1/3 = 33.33.. -> 33.3.
    #[test]
    fn rounds_percentage_to_one_decimal() {
        let out = render_profile_report(&[("a", 2), ("b", 1)]).unwrap();
        assert!(out.contains("66.7%"), "expected 66.7% in:\n{out}");
        assert!(out.contains("33.3%"), "expected 33.3% in:\n{out}");
    }

    // Independent oracle: with 23 functions only the top 20 are listed, followed
    // by the "... and N more functions" overflow line; the footer still counts
    // all 23. N = 23 - 20 = 3.
    #[test]
    fn truncates_to_top_20_with_overflow_line() {
        let names = [
            "f00", "f01", "f02", "f03", "f04", "f05", "f06", "f07", "f08", "f09", "f10", "f11",
            "f12", "f13", "f14", "f15", "f16", "f17", "f18", "f19", "f20", "f21", "f22",
        ];
        let entries: Vec<(&str, u64)> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (*name, (100 - i as u64) * 10))
            .collect();
        let out = render_profile_report(&entries).unwrap();

        let shown = data_row_names(&out, &names);
        assert_eq!(shown.len(), 20, "expected 20 data rows in:\n{out}");
        assert_eq!(shown[0], "f00", "highest count listed first");
        assert!(
            out.contains("  ... and 3 more functions"),
            "expected overflow line in:\n{out}"
        );
        assert!(
            out.contains("total calls: 20470, unique functions: 23"),
            "expected footer counting all 23 in:\n{out}"
        );
    }
}
