//! JSON emission for `vow complexity`. Hand-built (never serde_json) so the
//! Rust and self-hosted compilers stay byte-identical; see the module header in
//! `complexity.rs`. This module owns all output shaping — the shared string
//! `escape`r and the per-function record writer — so the command shell keeps
//! only orchestration and aggregation.

use super::{CxEmit, Totals, cx_popcount_bits};
use std::path::Path;

// Render the top-level `complexity_report` envelope: the file block (its
// aggregates and every function record), the module-coupling block, and the
// summary. Hand-built and byte-identical with the self-hosted emitter, so the
// field order, spacing, and fixed-point formatting here are load-bearing (see
// the module header). All aggregation and the exit-gating decision are computed
// upstream in `Totals`; this seam only shapes them into JSON.
pub(super) fn render_report(
    recs: &[CxEmit],
    totals: &Totals,
    file_nloc: i64,
    source: &Path,
) -> String {
    let mut out = String::from(
        "{\"schema_version\":\"1\",\"kind\":\"complexity_report\",\"tool\":\"vow\",\"files\":[{\"file\":\"",
    );
    out.push_str(&escape(&source.to_string_lossy()));
    out.push_str("\",\"complexity_score\":");
    out.push_str(&totals.file_max.to_string());
    out.push_str(",\"functions_over_threshold\":");
    out.push_str(&totals.over_count.to_string());
    out.push_str(",\"nloc\":");
    out.push_str(&file_nloc.to_string());
    out.push_str(",\"functions\":[");
    for (k, r) in recs.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        out.push_str(&render_fn(r, totals.thr));
    }
    out.push_str("],\"module\":{\"tier\":\"experimental\",\"functions\":");
    out.push_str(&(recs.len() as i64).to_string());
    out.push_str(",\"fan_in_max\":");
    out.push_str(&totals.fan_in_max.to_string());
    out.push_str(",\"fan_out_max\":");
    out.push_str(&totals.fan_out_max.to_string());
    out.push_str(",\"henry_kafura_max\":");
    out.push_str(&totals.hk_max.to_string());
    out.push_str("}}],\"summary\":{\"functions\":");
    out.push_str(&(recs.len() as i64).to_string());
    out.push_str(",\"nloc_total\":");
    out.push_str(&file_nloc.to_string());
    out.push_str(",\"threshold\":");
    out.push_str(&totals.thr.to_string());
    out.push_str(",\"functions_over_threshold\":");
    out.push_str(&totals.over_count.to_string());
    out.push_str(",\"thresholds_exceeded\":[");
    let mut emitted = 0;
    for r in recs {
        if r.score > totals.thr {
            if emitted > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(&escape(&r.name));
            out.push('"');
            emitted += 1;
        }
    }
    out.push_str("]}}");
    out
}

// Render one function's complexity record as a JSON object. Returns the text
// rather than appending to a caller-owned buffer, so the whole field ordering,
// fixed-point formatting, and string escaping live behind this one call.
pub(super) fn render_fn(r: &CxEmit, thr: i64) -> String {
    let mut out = String::new();
    out.push_str("{\"name\":\"");
    out.push_str(&escape(&r.name));
    out.push_str("\",\"line\":");
    out.push_str(&r.line.to_string());
    out.push_str(",\"complexity_score\":");
    out.push_str(&r.score.to_string());
    out.push_str(",\"score_factors\":{\"cognitive_sub\":");
    out.push_str(&fmt_fixed(r.cog_sub_s));
    out.push_str(",\"size_sub\":");
    out.push_str(&fmt_fixed(r.size_sub_s));
    out.push_str(",\"vow_bump\":");
    out.push_str(&fmt_fixed(r.vow_bump_s));
    out.push_str(",\"base\":");
    out.push_str(&fmt_fixed(r.base_s));
    out.push_str(",\"over_threshold\":");
    out.push_str(if r.score > thr { "true" } else { "false" });
    out.push_str("},\"size\":{\"nloc\":");
    out.push_str(&r.nloc.to_string());
    out.push_str(",\"tokens\":");
    out.push_str(&r.tokens.to_string());
    out.push_str(",\"stmts\":");
    out.push_str(&r.stmts.to_string());
    out.push_str(",\"params\":");
    out.push_str(&r.params.to_string());
    out.push_str("},\"structural\":{\"cyclomatic\":");
    out.push_str(&r.cyclomatic.to_string());
    out.push_str(",\"cyclomatic_ir\":");
    out.push_str(&r.cyclomatic_ir.to_string());
    out.push_str(",\"cognitive\":");
    out.push_str(&r.cognitive.to_string());
    out.push_str(",\"max_nesting\":");
    out.push_str(&r.max_nesting.to_string());
    out.push_str(",\"halstead\":{\"n1\":");
    out.push_str(&r.h_n1.to_string());
    out.push_str(",\"n2\":");
    out.push_str(&r.h_n2.to_string());
    out.push_str(",\"N1\":");
    out.push_str(&r.h_bign1.to_string());
    out.push_str(",\"N2\":");
    out.push_str(&r.h_bign2.to_string());
    out.push_str(",\"vocabulary\":");
    out.push_str(&r.h_vocab.to_string());
    out.push_str(",\"length\":");
    out.push_str(&r.h_length.to_string());
    out.push_str(",\"volume\":");
    out.push_str(&fmt_fixed(r.h_volume_s));
    out.push_str(",\"difficulty\":");
    out.push_str(&fmt_fixed(r.h_difficulty_s));
    out.push_str(",\"effort\":");
    out.push_str(&fmt_fixed(r.h_effort_s));
    out.push_str("}},\"vow\":{\"tier\":\"experimental\",\"effects\":");
    emit_effects(&mut out, r.eff);
    out.push_str(",\"effect_breadth\":");
    out.push_str(&cx_popcount_bits(r.eff).to_string());
    out.push_str(",\"effect_fanout\":");
    out.push_str(&r.effect_fanout.to_string());
    out.push_str(",\"linear_values\":");
    out.push_str(&r.linear_values.to_string());
    out.push_str(",\"linear_consumes\":");
    out.push_str(&r.linear_consumes.to_string());
    out.push_str(",\"linear_borrows\":");
    out.push_str(&r.linear_borrows.to_string());
    out.push_str(",\"contract\":{\"requires\":");
    out.push_str(&r.contract.requires.to_string());
    out.push_str(",\"ensures\":");
    out.push_str(&r.contract.ensures.to_string());
    out.push_str(",\"invariants\":");
    out.push_str(&r.contract.invariants.to_string());
    out.push_str(",\"predicate_nodes\":");
    out.push_str(&r.contract.predicate_nodes.to_string());
    out.push_str(",\"predicate_depth\":");
    out.push_str(&r.contract.predicate_depth.to_string());
    out.push_str(",\"free_vars\":");
    out.push_str(&r.contract.free_vars.to_string());
    out.push_str(",\"has_vec_quantification\":");
    out.push_str(if r.contract.has_vec_quant {
        "true"
    } else {
        "false"
    });
    out.push_str("}},\"verification\":{\"tier\":\"experimental\",\"loops_total\":");
    out.push_str(&r.verif.loops_total.to_string());
    out.push_str(",\"loops_without_invariant\":");
    out.push_str(&r.verif.loops_without_invariant.to_string());
    out.push_str(",\"max_loop_nesting\":");
    out.push_str(&r.verif.max_loop_nesting.to_string());
    out.push_str(",\"contract_predicate_cost\":");
    out.push_str(&r.verif.contract_predicate_cost.to_string());
    out.push_str("}}");
    out
}

fn emit_effects(out: &mut String, eff: i64) {
    out.push('[');
    let mut ne = 0;
    for (bit, name) in [
        (1, "io"),
        (2, "panic"),
        (4, "read"),
        (8, "unsafe"),
        (16, "write"),
    ] {
        if (eff / bit) % 2 != 0 {
            if ne > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(name);
            out.push('"');
            ne += 1;
        }
    }
    out.push(']');
}

// Render a scale-1000 fixed-point integer as a fixed 3-decimal JSON number
// (e.g. 4743 -> "4.743"). Integer-only so both compilers stay byte-identical.
fn fmt_fixed(scaled: i64) -> String {
    let mut s = String::new();
    let mut x = scaled;
    if x < 0 {
        s.push('-');
        x = -x;
    }
    let whole = x / 1000;
    let frac = x - whole * 1000;
    s.push_str(&whole.to_string());
    s.push('.');
    if frac < 100 {
        s.push('0');
    }
    if frac < 10 {
        s.push('0');
    }
    s.push_str(&frac.to_string());
    s
}

pub(super) fn escape(s: &str) -> String {
    // Iterate chars, not bytes: `byte as char` would map a UTF-8 lead byte like
    // 0xC3 to U+00C3 and re-encode it as two bytes (mojibake), diverging from the
    // self-hosted escaper, which appends the original UTF-8 bytes. Pushing the
    // char preserves those bytes and stays byte-identical for non-ASCII paths.
    let mut r = String::new();
    for c in s.chars() {
        match c {
            '"' => r.push_str("\\\""),
            '\\' => r.push_str("\\\\"),
            '\n' => r.push_str("\\n"),
            '\r' => r.push_str("\\r"),
            '\t' => r.push_str("\\t"),
            '\x08' => r.push_str("\\b"),
            '\x0c' => r.push_str("\\f"),
            c if (c as u32) < 32 => r.push('?'),
            c => r.push(c),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::super::{Contract, CxEmit, Totals, Verif};
    use super::*;
    use std::path::Path;

    // ---- render_fn fixtures ----

    // A fully-populated record with recognizable field values. Fixed-point
    // fields carry `scale=1000` integers whose 3-decimal rendering is easy to
    // predict independently (e.g. 4743 -> "4.743"). Values are chosen for
    // legible rendering, not to be a self-consistent run of the score pipeline
    // (the writer emits the struct verbatim; scoring is tested elsewhere).
    fn sample_emit() -> CxEmit {
        CxEmit {
            name: "sample".to_string(),
            line: 12,
            nloc: 5,
            tokens: 7,
            stmts: 3,
            params: 2,
            cyclomatic: 4,
            cyclomatic_ir: 4,
            cognitive: 6,
            max_nesting: 2,
            h_n1: 3,
            h_n2: 4,
            h_bign1: 8,
            h_bign2: 9,
            h_vocab: 7,
            h_length: 17,
            h_volume_s: 4743,
            h_difficulty_s: 500,
            h_effort_s: 2371,
            eff: 5, // io (1) | read (4)
            effect_fanout: 1,
            linear_values: 0,
            linear_consumes: 0,
            linear_borrows: 0,
            score: 42,
            cog_sub_s: 123,
            size_sub_s: 66,
            base_s: 189,
            vow_bump_s: 0,
            contract: Contract {
                requires: 1,
                ensures: 2,
                invariants: 0,
                predicate_nodes: 3,
                predicate_depth: 2,
                free_vars: 1,
                has_vec_quant: false,
            },
            verif: Verif {
                loops_total: 0,
                loops_without_invariant: 0,
                max_loop_nesting: 0,
                contract_predicate_cost: 4,
            },
        }
    }

    fn render_value(emit: &CxEmit, thr: i64) -> serde_json::Value {
        let raw = render_fn(emit, thr);
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("render_fn produced invalid JSON: {e}\n{raw}"))
    }

    // ---- render_fn ----

    #[test]
    fn render_fn_emits_the_documented_top_level_shape() {
        let v = render_value(&sample_emit(), 80);
        let obj = v.as_object().expect("record must be a JSON object");
        for key in [
            "name",
            "line",
            "complexity_score",
            "score_factors",
            "size",
            "structural",
            "vow",
            "verification",
        ] {
            assert!(obj.contains_key(key), "missing top-level key {key:?}");
        }
    }

    #[test]
    fn render_fn_reports_the_score_as_an_integer_in_range() {
        let v = render_value(&sample_emit(), 80);
        let score = v["complexity_score"]
            .as_i64()
            .expect("complexity_score must be a JSON integer");
        assert_eq!(score, 42);
        assert!((0..=100).contains(&score));
    }

    #[test]
    fn render_fn_marks_over_threshold_by_comparing_score_to_threshold() {
        // score is 42: below 80, above 10.
        assert_eq!(
            render_value(&sample_emit(), 80)["score_factors"]["over_threshold"],
            false
        );
        assert_eq!(
            render_value(&sample_emit(), 10)["score_factors"]["over_threshold"],
            true
        );
    }

    #[test]
    fn render_fn_formats_fixed_point_fields_with_three_decimals() {
        let emit = sample_emit();
        let raw = render_fn(&emit, 80);
        // scale=1000: 4743 -> "4.743", 189 -> "0.189", 66 -> "0.066" (padded).
        assert!(
            raw.contains("\"volume\":4.743"),
            "volume not 3-decimal: {raw}"
        );
        assert!(raw.contains("\"base\":0.189"), "base not 3-decimal: {raw}");
        assert!(
            raw.contains("\"size_sub\":0.066"),
            "size_sub not zero-padded: {raw}"
        );
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let vol = v["structural"]["halstead"]["volume"].as_f64().unwrap();
        assert!((vol - 4.743).abs() < 1e-9);
    }

    #[test]
    fn render_fn_lists_effects_and_their_breadth() {
        let v = render_value(&sample_emit(), 80);
        assert_eq!(v["vow"]["effects"], serde_json::json!(["io", "read"]));
        assert_eq!(v["vow"]["effect_breadth"].as_i64(), Some(2));
    }

    #[test]
    fn render_fn_escapes_special_characters_in_the_function_name() {
        let mut emit = sample_emit();
        emit.name = "weird\"na\\me\nend".to_string();
        let v = render_value(&emit, 80);
        assert_eq!(v["name"], "weird\"na\\me\nend");
    }

    // ---- escape ----

    #[test]
    fn escape_passes_plain_ascii_through_unchanged() {
        assert_eq!(escape("hello_world"), "hello_world");
    }

    #[test]
    fn escape_quotes_and_backslashes_per_json_spec() {
        assert_eq!(escape("\""), "\\\"");
        assert_eq!(escape("\\"), "\\\\");
        assert_eq!(escape("a\"b\\c"), "a\\\"b\\\\c");
    }

    #[test]
    fn escape_named_control_characters() {
        assert_eq!(escape("\n"), "\\n");
        assert_eq!(escape("\r"), "\\r");
        assert_eq!(escape("\t"), "\\t");
        assert_eq!(escape("\u{8}"), "\\b");
        assert_eq!(escape("\u{c}"), "\\f");
    }

    #[test]
    fn escape_replaces_unnamed_control_characters_with_question_mark() {
        // Control bytes below 0x20 that lack a JSON short escape are dropped to
        // '?' rather than emitted as  — a lossy but valid-JSON choice the
        // self-hosted escaper shares.
        assert_eq!(escape("\u{1}"), "?");
        assert_eq!(escape("\u{1f}"), "?");
        assert_eq!(escape("x\u{0}y"), "x?y");
    }

    #[test]
    fn escape_preserves_non_ascii_bytes() {
        assert_eq!(escape("café"), "café");
        assert_eq!(escape("λx"), "λx");
    }

    #[test]
    fn escape_output_round_trips_through_a_real_json_parser() {
        for input in ["a\"b\\c", "line\nbreak\ttab", "quote\"only", "café/λ"] {
            let json = format!("\"{}\"", escape(input));
            let parsed: String = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("escape({input:?}) produced invalid JSON string: {e}"));
            assert_eq!(&parsed, input);
        }
    }

    // ---- render_report (the top-level complexity_report envelope) ----

    fn totals(thr: i64, file_max: i64, over_count: i64) -> Totals {
        Totals {
            thr,
            file_max,
            over_count,
            exit_code: 0,
            hk_max: 7,
            fan_in_max: 3,
            fan_out_max: 5,
        }
    }

    fn named(name: &str, score: i64) -> CxEmit {
        let mut e = sample_emit();
        e.name = name.to_string();
        e.score = score;
        e
    }

    fn report_value(
        recs: &[CxEmit],
        t: &Totals,
        file_nloc: i64,
        source: &str,
    ) -> serde_json::Value {
        let raw = render_report(recs, t, file_nloc, Path::new(source));
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("render_report produced invalid JSON: {e}\n{raw}"))
    }

    #[test]
    fn render_report_emits_the_documented_envelope_shape() {
        let recs = [named("f", 42)];
        let v = report_value(&recs, &totals(80, 42, 0), 14, "src/x.vow");
        assert_eq!(v["schema_version"], "1");
        assert_eq!(v["kind"], "complexity_report");
        assert_eq!(v["tool"], "vow");
        let file = &v["files"][0];
        assert_eq!(file["file"], "src/x.vow");
        assert_eq!(file["complexity_score"], 42);
        assert_eq!(file["functions_over_threshold"], 0);
        assert_eq!(file["nloc"], 14);
        assert_eq!(file["functions"].as_array().unwrap().len(), 1);
        let module = &file["module"];
        assert_eq!(module["functions"], 1);
        assert_eq!(module["fan_in_max"], 3);
        assert_eq!(module["fan_out_max"], 5);
        assert_eq!(module["henry_kafura_max"], 7);
        let summary = &v["summary"];
        assert_eq!(summary["functions"], 1);
        assert_eq!(summary["nloc_total"], 14);
        assert_eq!(summary["threshold"], 80);
        assert_eq!(summary["functions_over_threshold"], 0);
    }

    #[test]
    fn render_report_lists_only_over_threshold_functions_in_summary() {
        // With thr=40: "hi" (90) is over, "lo" (10) is not.
        let recs = [named("lo", 10), named("hi", 90)];
        let v = report_value(&recs, &totals(40, 90, 1), 20, "m.vow");
        assert_eq!(
            v["summary"]["thresholds_exceeded"],
            serde_json::json!(["hi"])
        );
        // Each function record carries its own over_threshold flag against thr.
        let fns = v["files"][0]["functions"].as_array().unwrap();
        assert_eq!(fns[0]["score_factors"]["over_threshold"], false);
        assert_eq!(fns[1]["score_factors"]["over_threshold"], true);
    }

    #[test]
    fn render_report_escapes_the_source_path() {
        let recs = [named("f", 1)];
        let v = report_value(&recs, &totals(80, 1, 0), 1, "we\"ird.vow");
        assert_eq!(v["files"][0]["file"], "we\"ird.vow");
    }

    #[test]
    fn render_report_handles_zero_functions() {
        let v = report_value(&[], &totals(80, 0, 0), 0, "empty.vow");
        assert_eq!(v["files"][0]["functions"].as_array().unwrap().len(), 0);
        assert_eq!(
            v["summary"]["thresholds_exceeded"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
