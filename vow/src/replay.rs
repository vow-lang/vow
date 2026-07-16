// Counterexample replay (--replay-cex, issue #335).
//
// Differential test of the verifier's IR-to-C model against vow-codegen's
// debug-mode runtime checks. After ESBMC returns a counterexample, we map the
// symbolic assignment to concrete Vow inputs, synthesize a `--mode debug`
// harness that calls the failing function with them, run it, and check the
// runtime VowViolation agrees (same vow_id + blame). NOT part of the proof:
// the static verdict and exit code are unchanged.
//
// The whole pipeline sits behind one entry point, `run_replay_cex`; every type
// and helper below is private to this module. The pure mapping steps
// (`plan_replay_args`, `classify_replay_param_type`, `normalize_replay_scalar`,
// `parse_esbmc_vec_struct`, `expand_aggregate_values`, `generate_replay_harness`,
// `parse_vow_violation_line`, `classify_replay_run`) are unit-tested directly.

use std::path::Path;

use crate::frontend::{FrontendGoal, prepare_frontend};
use crate::{BuildOutput, BuildStatus, StructuredCounterexample, run_pipeline};
use vow_codegen::{BuildMode, TraceMode};
use vow_verify::{CALLER_PRECONDITION_VOW_ID, UNSUPPORTED_OP_VOW_ID};

/// One reconstructed argument for the replay call.
#[derive(Debug, Clone, PartialEq)]
enum ReplayArg {
    /// Scalar literal: `ty` is the surface type text, `value` the literal.
    Scalar { ty: String, value: String },
    /// Bounded vec of scalars: `ty` is e.g. `Vec<i64>`, `elems` the elements.
    VecScalar { ty: String, elems: Vec<String> },
}

/// Per-parameter info the mapper needs, decoupled from AST/IR so the mapping
/// logic ([`plan_replay_args`]) is pure and unit-testable.
#[derive(Debug, Clone, PartialEq)]
struct ReplayParam {
    /// Position among non-Unit params (ESBMC scalar name `p{cl_index}`).
    cl_index: u32,
    /// GetArg instruction id (ESBMC vec names `v{getarg_id}.len` / `.data[k]`;
    /// scalar fallback name `v{getarg_id}`). `None` if no GetArg was found.
    getarg_id: Option<u32>,
    kind: ReplayParamKind,
}

#[derive(Debug, Clone, PartialEq)]
enum ReplayParamKind {
    Scalar { ty: String },
    VecScalar { ty: String },
    Unsupported { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
enum ReplayPlan {
    Ready(Vec<ReplayArg>),
    Skipped(String),
}

/// Extract the element type text from a `Vec<T>` surface type string.
fn replay_vec_elem_ty(ty: &str) -> Option<&str> {
    Some(ty.trim().strip_prefix("Vec<")?.strip_suffix('>')?.trim())
}

/// Parse an ESBMC composite vec value `{ .len=N, .data={ e0, e1, ... } }` into
/// `(len, data_elements)`. ESBMC renders aggregate counterexample values this
/// way (and even shows the whole struct for a `.len` member assignment).
fn parse_esbmc_vec_struct(s: &str) -> Option<(i64, Vec<String>)> {
    let s = s.trim();
    let len_pos = s.find(".len=")? + ".len=".len();
    let len_str: String = s[len_pos..]
        .chars()
        .take_while(|c| *c == '-' || c.is_ascii_digit())
        .collect();
    let len: i64 = len_str.trim().parse().ok()?;
    let data_pos = s.find(".data=")? + ".data=".len();
    let after = s[data_pos..].trim_start();
    let inner = after.strip_prefix('{')?;
    let close = inner.find('}')?;
    let elems = inner[..close]
        .split(',')
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    Some((len, elems))
}

/// Flatten ESBMC composite vec assignments from the raw counterexample text into
/// `{base}.len` / `{base}.data[k]` entries that [`plan_replay_args`] consumes.
/// Later states overwrite earlier ones (the final assigned value wins), so the
/// flattened entries are appended after `raw_values`.
fn expand_aggregate_values(
    raw_values: &[(String, String)],
    raw_output: &str,
) -> Vec<(String, String)> {
    let mut out = raw_values.to_vec();
    for line in raw_output.lines() {
        let line = line.trim();
        let Some(eq) = line.find('=') else { continue };
        let lvalue = line[..eq].trim();
        if lvalue.is_empty() || !lvalue.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        {
            continue;
        }
        let value = line[eq + 1..].trim();
        if !value.starts_with("{ .len=") && !value.starts_with("{.len=") {
            continue;
        }
        // The struct may belong to `v0` or be shown for a member like `v0.len`;
        // the base variable is the part before the first `.`.
        let base = lvalue.split('.').next().unwrap_or(lvalue);
        if let Some((len, elems)) = parse_esbmc_vec_struct(value) {
            out.push((format!("{base}.len"), len.to_string()));
            for (k, e) in elems.iter().enumerate() {
                out.push((format!("{base}.data[{k}]"), e.clone()));
            }
        }
    }
    out
}

/// Normalize an ESBMC scalar value to a Vow literal for the given surface type.
fn normalize_replay_scalar(ty: &str, raw: &str) -> Option<String> {
    let v = raw.trim();
    if ty == "bool" {
        let truthy = matches!(v, "true" | "TRUE" | "True")
            || v.parse::<i64>().map(|n| n != 0).unwrap_or(false);
        return Some(if truthy { "true" } else { "false" }.to_string());
    }
    // i64 / u64: accept a decimal integer (ESBMC prints these); reject anything
    // we cannot render as a Vow integer literal.
    if v.parse::<i64>().is_ok() || v.parse::<u64>().is_ok() {
        return Some(v.to_string());
    }
    None
}

/// Pure mapping: counterexample assignments + parameter classification →
/// concrete replay args, or a skip reason. Unit-tested.
fn plan_replay_args(params: &[ReplayParam], raw_values: &[(String, String)]) -> ReplayPlan {
    use std::collections::HashMap;
    let map: HashMap<&str, &str> = raw_values
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut args = Vec::with_capacity(params.len());
    for (i, p) in params.iter().enumerate() {
        match &p.kind {
            ReplayParamKind::Unsupported { reason } => {
                return ReplayPlan::Skipped(format!("parameter {i}: {reason}"));
            }
            ReplayParamKind::Scalar { ty } => {
                let raw = map
                    .get(format!("p{}", p.cl_index).as_str())
                    .copied()
                    .or_else(|| {
                        p.getarg_id
                            .and_then(|id| map.get(format!("v{id}").as_str()).copied())
                    });
                let Some(raw) = raw else {
                    return ReplayPlan::Skipped(format!(
                        "parameter {i}: no counterexample value for scalar"
                    ));
                };
                match normalize_replay_scalar(ty, raw) {
                    Some(value) => args.push(ReplayArg::Scalar {
                        ty: ty.clone(),
                        value,
                    }),
                    None => {
                        return ReplayPlan::Skipped(format!(
                            "parameter {i}: cannot render value {raw:?} as {ty}"
                        ));
                    }
                }
            }
            ReplayParamKind::VecScalar { ty } => {
                let Some(gid) = p.getarg_id else {
                    return ReplayPlan::Skipped(format!(
                        "parameter {i}: no GetArg for vec parameter"
                    ));
                };
                let Some(len_raw) = map.get(format!("v{gid}.len").as_str()).copied() else {
                    return ReplayPlan::Skipped(format!(
                        "parameter {i}: no counterexample length for vec"
                    ));
                };
                let len: usize = match len_raw.trim().parse::<i64>() {
                    Ok(n) if n >= 0 => n as usize,
                    _ => {
                        return ReplayPlan::Skipped(format!(
                            "parameter {i}: bad vec length {len_raw:?}"
                        ));
                    }
                };
                let elem_ty = replay_vec_elem_ty(ty).unwrap_or("i64");
                let mut elems = Vec::with_capacity(len);
                for k in 0..len {
                    // Unconstrained elements may be absent from the CEX; any
                    // value reaches the violation, so default to 0.
                    let raw = map
                        .get(format!("v{gid}.data[{k}]").as_str())
                        .copied()
                        .unwrap_or("0");
                    match normalize_replay_scalar(elem_ty, raw) {
                        Some(val) => elems.push(val),
                        None => {
                            return ReplayPlan::Skipped(format!(
                                "parameter {i}: cannot render vec element {raw:?} as {elem_ty}"
                            ));
                        }
                    }
                }
                args.push(ReplayArg::VecScalar {
                    ty: ty.clone(),
                    elems,
                });
            }
        }
    }
    ReplayPlan::Ready(args)
}

/// Classify a parameter's surface type into a replay kind.
fn classify_replay_param_type(ty: &vow_syntax::ast::Type) -> ReplayParamKind {
    use vow_syntax::ast::Type;
    match ty {
        Type::Named { name, .. } if matches!(name.as_str(), "i64" | "u64" | "bool") => {
            ReplayParamKind::Scalar { ty: name.clone() }
        }
        Type::Generic { name, args, .. } if name == "Vec" && args.len() == 1 => {
            if let Type::Named { name: e, .. } = &args[0]
                && matches!(e.as_str(), "i64" | "u64" | "bool")
            {
                return ReplayParamKind::VecScalar {
                    ty: vow_syntax::printer::print_type(ty),
                };
            }
            ReplayParamKind::Unsupported {
                reason: format!(
                    "unsupported Vec element type in {}",
                    vow_syntax::printer::print_type(ty)
                ),
            }
        }
        other => ReplayParamKind::Unsupported {
            reason: format!(
                "unsupported parameter type {}",
                vow_syntax::printer::print_type(other)
            ),
        },
    }
}

/// Classify a function's parameters using its AST signature (surface types) and
/// IR (Unit detection, GetArg ids). `Err` is a whole-function skip reason.
fn classify_replay_params(
    ast_fn: &vow_syntax::ast::FnDef,
    ir_fn: &vow_ir::Function,
) -> Result<Vec<ReplayParam>, String> {
    use vow_ir::{InstData, Opcode, Ty};
    if ast_fn.params.len() != ir_fn.params.len() {
        return Err("AST/IR parameter count mismatch".to_string());
    }
    let mut getarg: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for block in &ir_fn.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::GetArg
                && let InstData::ArgIndex(idx) = inst.data
            {
                getarg.entry(idx).or_insert(inst.id.0);
            }
        }
    }
    let mut out = Vec::with_capacity(ast_fn.params.len());
    for (i, param) in ast_fn.params.iter().enumerate() {
        if ir_fn.params[i] == Ty::Unit {
            // Unit params would shift the `p{cl_index}` numbering; rather than
            // model that, skip the whole function. (cl_index == i here.)
            return Err("function has a Unit parameter".to_string());
        }
        out.push(ReplayParam {
            cl_index: i as u32,
            getarg_id: getarg.get(&(i as u32)).copied(),
            kind: classify_replay_param_type(&param.ty),
        });
    }
    Ok(out)
}

/// Render the effect annotation suffix, e.g. ` [io, read]`, empty if pure.
fn render_replay_effects(effects: &[vow_syntax::ast::Effect]) -> String {
    use vow_syntax::ast::Effect;
    if effects.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = effects
        .iter()
        .map(|e| match e {
            Effect::IO => "io",
            Effect::Read => "read",
            Effect::Write => "write",
            Effect::Panic => "panic",
            Effect::Unsafe => "unsafe",
        })
        .collect();
    format!(" [{}]", names.join(", "))
}

/// Build replay harness source: the original source with any top-level `fn main`
/// removed, plus a synthesized `main` that calls `target` with concrete args.
/// Removing `main` is safe — vow ids are per-function-local, so the target's
/// vow numbering is unchanged.
fn generate_replay_harness(
    original_source: &str,
    ast_module: &vow_syntax::ast::Module,
    target: &vow_syntax::ast::FnDef,
    args: &[ReplayArg],
) -> String {
    use vow_syntax::ast::{Item, Type};
    let mut src = original_source.to_string();
    if let Some(main_span) = ast_module.items.iter().find_map(|it| match it {
        Item::Fn(f) if f.name == "main" => Some(f.span),
        _ => None,
    }) {
        let start = main_span.start as usize;
        let end = start + main_span.len as usize;
        if end <= src.len() {
            src.replace_range(start..end, "");
        }
    }

    let mut body = String::new();
    let mut call_args = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        let name = format!("__replay_a{i}");
        match arg {
            ReplayArg::Scalar { ty, value } => {
                body.push_str(&format!("    let {name}: {ty} = {value};\n"));
            }
            ReplayArg::VecScalar { ty, elems } => {
                body.push_str(&format!("    let {name}: {ty} = Vec::new();\n"));
                for e in elems {
                    body.push_str(&format!("    {name}.push({e});\n"));
                }
            }
        }
        call_args.push(name);
    }
    let call = call_args.join(", ");
    let call_line = match &target.return_ty {
        Type::Unit { .. } => format!("    {}({});\n", target.name, call),
        ret => format!(
            "    let __replay_ret: {} = {}({});\n",
            vow_syntax::printer::print_type(ret),
            target.name,
            call
        ),
    };
    body.push_str(&call_line);

    format!(
        "{src}\n\nfn main(){effects} {{\n{body}}}\n",
        effects = render_replay_effects(&target.effects),
    )
}

/// Verdict of replaying one counterexample.
struct ReplayOutcome {
    status: &'static str, // "confirmed" | "diverged" | "skipped"
    reason: Option<String>,
}

fn replay_skip(reason: String) -> ReplayOutcome {
    ReplayOutcome {
        status: "skipped",
        reason: Some(reason),
    }
}

fn replay_diverge(reason: String) -> ReplayOutcome {
    ReplayOutcome {
        status: "diverged",
        reason: Some(reason),
    }
}

/// Parse a `{"error":"VowViolation","vow_id":N,"blame":"Caller",...}` stderr
/// line into `(vow_id, blame)`.
fn parse_vow_violation_line(line: &str) -> Option<(u32, String)> {
    let l = line.trim();
    if !l.starts_with('{') || !l.contains("\"error\":\"VowViolation\"") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(l).ok()?;
    let vid = v.get("vow_id")?.as_u64()? as u32;
    let blame = v.get("blame")?.as_str()?.to_string();
    Some((vid, blame))
}

/// Classify a finished harness run against the counterexample's prediction.
fn classify_replay_run(
    success: bool,
    code: Option<i32>,
    stderr: &str,
    ce: &StructuredCounterexample,
) -> ReplayOutcome {
    match stderr.lines().find_map(parse_vow_violation_line) {
        Some((vid, blame)) => {
            if vid == ce.vow_id && blame.eq_ignore_ascii_case(&ce.blame) {
                ReplayOutcome {
                    status: "confirmed",
                    reason: None,
                }
            } else {
                replay_diverge(format!(
                    "runtime VowViolation vow_id={vid} blame={blame}, but counterexample predicted vow_id={} blame={}",
                    ce.vow_id, ce.blame
                ))
            }
        }
        None if success => {
            replay_diverge("harness exited cleanly; no VowViolation fired at runtime".to_string())
        }
        None => replay_diverge(format!(
            "harness exited with status {code:?} but emitted no VowViolation"
        )),
    }
}

/// Compile the harness in debug mode (no verify), run it, and classify.
fn compile_and_run_replay(
    source: &Path,
    harness: &str,
    ce: &StructuredCounterexample,
) -> ReplayOutcome {
    let dir = source.parent().unwrap_or_else(|| Path::new("."));
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "src".to_string());
    let tag = format!("__vow_replay_{stem}_{}_{}", ce.vow_id, std::process::id());
    let harness_path = dir.join(format!("{tag}.vow"));
    let bin_path = dir.join(&tag);

    let cleanup = || {
        let _ = std::fs::remove_file(&harness_path);
        let _ = std::fs::remove_file(&bin_path);
        let _ = std::fs::remove_file(bin_path.with_extension("o"));
    };

    if let Err(e) = std::fs::write(&harness_path, harness) {
        return replay_skip(format!("replay: could not write harness: {e}"));
    }

    let build = run_pipeline(
        &harness_path,
        Some(&bin_path),
        BuildMode::Debug,
        true, // no_verify: harness is for execution, not re-proof
        false,
        TraceMode::Off,
    );
    let built_ok = matches!(
        build.status,
        BuildStatus::Unverified | BuildStatus::Verified
    ) && build.executable.is_some();
    if !built_ok {
        let msg = match &build.status {
            BuildStatus::CompileFailed { message } => message.clone(),
            other => format!("{other:?}"),
        };
        cleanup();
        return replay_skip(format!("replay: harness did not compile ({msg})"));
    }

    // Run the harness with a bounded timeout: a replayed function could loop
    // forever (the very drift being tested), and an unbounded wait would hang
    // `verify --replay-cex` instead of returning the original VerifyFailed JSON.
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut child = match std::process::Command::new(&bin_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            cleanup();
            return replay_skip(format!("replay: harness failed to execute: {e}"));
        }
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                cleanup();
                return replay_skip(format!("replay: error waiting on harness: {e}"));
            }
        }
    };
    let mut stderr = String::new();
    if let Some(mut es) = child.stderr.take() {
        let _ = es.read_to_string(&mut stderr);
    }
    cleanup();
    match status {
        None => replay_skip("replay: harness timed out".to_string()),
        Some(s) => classify_replay_run(s.success(), s.code(), &stderr, ce),
    }
}

/// Replay a single counterexample end to end.
fn replay_one(
    source: &Path,
    original_source: &str,
    ast_module: &vow_syntax::ast::Module,
    ir_module: &vow_ir::Module,
    ce: &StructuredCounterexample,
) -> ReplayOutcome {
    use vow_syntax::ast::Item;
    if ce.function == "main" {
        return replay_skip("cannot replay the entry function `main`".to_string());
    }
    // Synthetic verifier-only ids (caller-precondition / unsupported-op
    // sentinels) never match a runtime-emitted vow_id, so an exact id comparison
    // would always diverge — skip them.
    if ce.vow_id == CALLER_PRECONDITION_VOW_ID || ce.vow_id == UNSUPPORTED_OP_VOW_ID {
        return replay_skip(
            "replay: synthetic counterexample id (caller-precondition / unsupported-op sentinel)"
                .to_string(),
        );
    }
    let Some(ast_fn) = ast_module.items.iter().find_map(|it| match it {
        Item::Fn(f) if f.name == ce.function => Some(f),
        _ => None,
    }) else {
        return replay_skip(format!(
            "function `{}` is not defined in the entry file",
            ce.function
        ));
    };
    let Some(ir_fn) = ir_module.functions.iter().find(|f| f.name == ce.function) else {
        return replay_skip(format!("no IR for function `{}`", ce.function));
    };
    let params = match classify_replay_params(ast_fn, ir_fn) {
        Ok(p) => p,
        Err(e) => return replay_skip(e),
    };
    let values = expand_aggregate_values(&ce.replay_raw_values, &ce.replay_raw_output);
    let args = match plan_replay_args(&params, &values) {
        ReplayPlan::Ready(a) => a,
        ReplayPlan::Skipped(r) => return replay_skip(r),
    };
    let harness = generate_replay_harness(original_source, ast_module, ast_fn, &args);
    compile_and_run_replay(source, &harness, ce)
}

/// Annotate each counterexample in a finished build/verify output with its
/// `--replay-cex` verdict. Best-effort: never changes status or exit code.
pub(crate) fn run_replay_cex(source: &Path, output: &mut BuildOutput) {
    if output.counterexamples.is_empty() {
        return;
    }
    let mark_all = |output: &mut BuildOutput, reason: &str| {
        for ce in &mut output.counterexamples {
            ce.replay = Some("skipped".to_string());
            ce.replay_reason = Some(reason.to_string());
        }
    };
    // Re-run the frontend to recover AST + IR together (failure path only).
    let bundle = match prepare_frontend(source, FrontendGoal::LoweredIr) {
        Ok(b) => b,
        Err(_) => return mark_all(output, "replay: could not re-parse source"),
    };
    if bundle.ir().is_none() {
        return mark_all(output, "replay: no IR available");
    }
    let original_source = match std::fs::read_to_string(source) {
        Ok(s) => s,
        Err(e) => return mark_all(output, &format!("replay: could not read source: {e}")),
    };
    let ast_module = bundle.module();
    let ir_module = bundle.ir().expect("ir presence checked above");
    for ce in &mut output.counterexamples {
        let outcome = replay_one(source, &original_source, ast_module, ir_module, ce);
        ce.replay = Some(outcome.status.to_string());
        ce.replay_reason = if outcome.status == "confirmed" {
            None
        } else {
            outcome.reason
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_source(dir: &TempDir, name: &str, src: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, src).unwrap();
        path
    }

    fn ce(function: &str, vow_id: u32) -> StructuredCounterexample {
        StructuredCounterexample {
            function: function.to_string(),
            values: vec![],
            violation: String::new(),
            vow_id,
            source: None,
            blame: "Callee".to_string(),
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

    fn output_with(ces: Vec<StructuredCounterexample>) -> BuildOutput {
        BuildOutput {
            status: BuildStatus::VerifyFailed {
                function: "f".into(),
                description: "d".into(),
            },
            executable: None,
            diagnostics: vec![],
            counterexamples: ces,
            verify_status: None,
            verify_message: None,
        }
    }

    // ---- Seam tests for the deepened module's single entry point ----

    #[test]
    fn run_replay_cex_no_counterexamples_is_noop() {
        let mut out = output_with(vec![]);
        run_replay_cex(Path::new("/nonexistent/whatever.vow"), &mut out);
        assert!(out.counterexamples.is_empty());
    }

    #[test]
    fn run_replay_cex_marks_all_skipped_when_source_missing() {
        let mut out = output_with(vec![ce("g", 3), ce("h", 4)]);
        run_replay_cex(Path::new("/nonexistent/replay_src_missing.vow"), &mut out);
        for c in &out.counterexamples {
            assert_eq!(c.replay.as_deref(), Some("skipped"));
            assert_eq!(
                c.replay_reason.as_deref(),
                Some("replay: could not re-parse source")
            );
        }
    }

    #[test]
    fn run_replay_cex_skips_main_and_synthetic_ids_without_codegen() {
        // A valid source, but two counterexamples that `replay_one` short-circuits
        // to `skipped` before any harness codegen: the entry function `main`, and a
        // synthetic caller-precondition sentinel id.
        let dir = TempDir::new().unwrap();
        let src = "module M\n\nfn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() [io] {\n    let x: i64 = 0;\n}\n";
        let path = write_source(&dir, "m.vow", src);
        let mut out = output_with(vec![ce("main", 1), ce("add", CALLER_PRECONDITION_VOW_ID)]);
        run_replay_cex(&path, &mut out);
        assert_eq!(out.counterexamples[0].replay.as_deref(), Some("skipped"));
        assert_eq!(out.counterexamples[1].replay.as_deref(), Some("skipped"));
    }

    // ---- Counterexample replay (--replay-cex, issue #335) ----

    #[test]
    fn normalize_replay_scalar_handles_bool_and_ints() {
        assert_eq!(
            normalize_replay_scalar("bool", "1").as_deref(),
            Some("true")
        );
        assert_eq!(
            normalize_replay_scalar("bool", "0").as_deref(),
            Some("false")
        );
        assert_eq!(
            normalize_replay_scalar("bool", "TRUE").as_deref(),
            Some("true")
        );
        assert_eq!(
            normalize_replay_scalar("i64", "-9223372036854775808").as_deref(),
            Some("-9223372036854775808")
        );
        assert_eq!(
            normalize_replay_scalar("u64", "18446744073709551615").as_deref(),
            Some("18446744073709551615")
        );
        assert_eq!(normalize_replay_scalar("i64", "0xdead"), None);
    }

    #[test]
    fn replay_vec_elem_ty_extracts_inner() {
        assert_eq!(replay_vec_elem_ty("Vec<i64>"), Some("i64"));
        assert_eq!(replay_vec_elem_ty("Vec<u64>"), Some("u64"));
        assert_eq!(replay_vec_elem_ty("i64"), None);
    }

    #[test]
    fn plan_replay_args_maps_scalars() {
        let params = vec![
            ReplayParam {
                cl_index: 0,
                getarg_id: Some(10),
                kind: ReplayParamKind::Scalar { ty: "i64".into() },
            },
            ReplayParam {
                cl_index: 1,
                getarg_id: Some(11),
                kind: ReplayParamKind::Scalar { ty: "i64".into() },
            },
        ];
        let vals = vec![
            ("p0".to_string(), "5".to_string()),
            ("p1".to_string(), "0".to_string()),
        ];
        assert_eq!(
            plan_replay_args(&params, &vals),
            ReplayPlan::Ready(vec![
                ReplayArg::Scalar {
                    ty: "i64".into(),
                    value: "5".into()
                },
                ReplayArg::Scalar {
                    ty: "i64".into(),
                    value: "0".into()
                },
            ])
        );
    }

    #[test]
    fn plan_replay_args_scalar_falls_back_to_getarg_name() {
        let params = vec![ReplayParam {
            cl_index: 0,
            getarg_id: Some(7),
            kind: ReplayParamKind::Scalar { ty: "i64".into() },
        }];
        // Only the GetArg-aliased name is present, not p0.
        let vals = vec![("v7".to_string(), "42".to_string())];
        assert_eq!(
            plan_replay_args(&params, &vals),
            ReplayPlan::Ready(vec![ReplayArg::Scalar {
                ty: "i64".into(),
                value: "42".into()
            }])
        );
    }

    #[test]
    fn plan_replay_args_skips_when_value_missing() {
        let params = vec![ReplayParam {
            cl_index: 0,
            getarg_id: None,
            kind: ReplayParamKind::Scalar { ty: "i64".into() },
        }];
        assert!(matches!(
            plan_replay_args(&params, &[]),
            ReplayPlan::Skipped(_)
        ));
    }

    #[test]
    fn plan_replay_args_reconstructs_bounded_vec() {
        let params = vec![ReplayParam {
            cl_index: 0,
            getarg_id: Some(3),
            kind: ReplayParamKind::VecScalar {
                ty: "Vec<i64>".into(),
            },
        }];
        let vals = vec![
            ("v3.len".to_string(), "3".to_string()),
            ("v3.data[0]".to_string(), "10".to_string()),
            ("v3.data[2]".to_string(), "30".to_string()),
            // data[1] absent -> defaults to 0
        ];
        assert_eq!(
            plan_replay_args(&params, &vals),
            ReplayPlan::Ready(vec![ReplayArg::VecScalar {
                ty: "Vec<i64>".into(),
                elems: vec!["10".into(), "0".into(), "30".into()],
            }])
        );
    }

    #[test]
    fn plan_replay_args_skips_unsupported_param() {
        let params = vec![ReplayParam {
            cl_index: 0,
            getarg_id: Some(1),
            kind: ReplayParamKind::Unsupported {
                reason: "String".into(),
            },
        }];
        assert!(matches!(
            plan_replay_args(&params, &[]),
            ReplayPlan::Skipped(_)
        ));
    }

    #[test]
    fn parse_esbmc_vec_struct_extracts_len_and_data() {
        let s = "{ .len=2, .data={ 7, 9, 0, 0 } }";
        assert_eq!(
            parse_esbmc_vec_struct(s),
            Some((2, vec!["7".into(), "9".into(), "0".into(), "0".into()]))
        );
        assert_eq!(parse_esbmc_vec_struct("123"), None);
    }

    #[test]
    fn expand_aggregate_values_flattens_vec_struct() {
        let raw_output = "State 1\n  v0 = { .len=0, .data={ 0, 0 } }\nState 2\n  v0.len = { .len=2, .data={ 7, 9 } }\n";
        let expanded = expand_aggregate_values(&[], raw_output);
        // The final (State 2) values win when collected into a map.
        let plan = plan_replay_args(
            &[ReplayParam {
                cl_index: 0,
                getarg_id: Some(0),
                kind: ReplayParamKind::VecScalar {
                    ty: "Vec<i64>".into(),
                },
            }],
            &expanded,
        );
        assert_eq!(
            plan,
            ReplayPlan::Ready(vec![ReplayArg::VecScalar {
                ty: "Vec<i64>".into(),
                elems: vec!["7".into(), "9".into()],
            }])
        );
    }

    #[test]
    fn parse_vow_violation_line_extracts_id_and_blame() {
        let line = r#"{"error":"VowViolation","vow_id":2,"blame":"Caller","description":"x","file":"a.vow","offset":3}"#;
        assert_eq!(
            parse_vow_violation_line(line),
            Some((2, "Caller".to_string()))
        );
        assert_eq!(parse_vow_violation_line("not json"), None);
        assert_eq!(parse_vow_violation_line(r#"{"error":"Other"}"#), None);
    }

    #[test]
    fn replay_harness_generation_end_to_end() {
        let dir = TempDir::new().unwrap();
        let src = "module M\n\nfn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() [io] {\n    let __orig_marker: i64 = 99;\n}\n";
        let path = write_source(&dir, "m.vow", src);
        let bundle = prepare_frontend(&path, FrontendGoal::LoweredIr).unwrap();
        let ast = bundle.module();
        let ir = bundle.ir().unwrap();
        let ast_fn = ast
            .items
            .iter()
            .find_map(|it| match it {
                vow_syntax::ast::Item::Fn(f) if f.name == "add" => Some(f),
                _ => None,
            })
            .unwrap();
        let ir_fn = ir.functions.iter().find(|f| f.name == "add").unwrap();

        let params = classify_replay_params(ast_fn, ir_fn).unwrap();
        assert_eq!(params.len(), 2);
        assert!(matches!(params[0].kind, ReplayParamKind::Scalar { .. }));

        let args = match plan_replay_args(
            &params,
            &[
                ("p0".to_string(), "5".to_string()),
                ("p1".to_string(), "7".to_string()),
            ],
        ) {
            ReplayPlan::Ready(a) => a,
            ReplayPlan::Skipped(r) => panic!("unexpected skip: {r}"),
        };
        let harness = generate_replay_harness(src, ast, ast_fn, &args);

        // Synthesized main calls the target with the concrete args.
        assert!(harness.contains("let __replay_a0: i64 = 5;"));
        assert!(harness.contains("let __replay_a1: i64 = 7;"));
        assert!(harness.contains("add(__replay_a0, __replay_a1)"));
        // The original `main` (and its body marker) was removed.
        assert!(!harness.contains("__orig_marker"));
        // Exactly one `fn main` remains.
        assert_eq!(harness.matches("fn main(").count(), 1);
    }

    #[test]
    fn classify_replay_params_marks_aggregates_unsupported() {
        let dir = TempDir::new().unwrap();
        let src = "module S\n\nfn takes_str(s: String) -> i64 {\n    0\n}\n";
        let path = write_source(&dir, "s.vow", src);
        let bundle = prepare_frontend(&path, FrontendGoal::LoweredIr).unwrap();
        let ast = bundle.module();
        let ir = bundle.ir().unwrap();
        let ast_fn = ast
            .items
            .iter()
            .find_map(|it| match it {
                vow_syntax::ast::Item::Fn(f) if f.name == "takes_str" => Some(f),
                _ => None,
            })
            .unwrap();
        let ir_fn = ir.functions.iter().find(|f| f.name == "takes_str").unwrap();
        let params = classify_replay_params(ast_fn, ir_fn).unwrap();
        assert!(matches!(
            params[0].kind,
            ReplayParamKind::Unsupported { .. }
        ));
        assert!(matches!(
            plan_replay_args(&params, &[]),
            ReplayPlan::Skipped(_)
        ));
    }
}
