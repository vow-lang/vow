use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use std::collections::HashMap;

use vow_ir::{FuncId, Function, Module, Ty};

use crate::c_emitter::{ConstantValue, detect_constant_functions, emit_c_module};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Counterexample {
    pub description: String,
    pub vow_id: Option<u32>,
    pub values: Vec<(String, String)>,
    pub block_visits: Vec<u32>,
    pub raw_output: String,
}

#[derive(Debug)]
pub enum VerificationResult {
    Proven,
    Failed(Counterexample),
    Timeout,
    ToolNotFound,
    ToolError(String),
}

// ---------------------------------------------------------------------------
// Locating ESBMC
// ---------------------------------------------------------------------------

pub fn find_esbmc() -> Option<PathBuf> {
    let known = PathBuf::from("/home/pmatos/installs/esbmc-20260226/bin/esbmc");
    if known.exists() {
        return Some(known);
    }
    which("esbmc")
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full = dir.join(name);
            if full.is_file() { Some(full) } else { None }
        })
    })
}

// ---------------------------------------------------------------------------
// Harness generation
// ---------------------------------------------------------------------------

fn esbmc_nondet_call(ty: Ty) -> &'static str {
    match ty {
        Ty::I32 => "__VERIFIER_nondet_int()",
        Ty::I64 => "__VERIFIER_nondet_long()",
        Ty::F32 => "__VERIFIER_nondet_float()",
        Ty::F64 => "__VERIFIER_nondet_double()",
        Ty::Bool => "__VERIFIER_nondet_bool()",
        _ => "0",
    }
}

fn emit_harness(func: &Function) -> String {
    let args: Vec<&str> = func
        .params
        .iter()
        .filter(|&&ty| ty != Ty::Unit)
        .map(|&ty| esbmc_nondet_call(ty))
        .collect();
    format!(
        "int main(void) {{ {}({}); return 0; }}\n",
        func.name,
        args.join(", ")
    )
}

// ---------------------------------------------------------------------------
// ESBMC output parsing
// ---------------------------------------------------------------------------

pub fn parse_esbmc_output(output: &str) -> Counterexample {
    let vow_id = extract_vow_id(output);
    let all_assignments = extract_variable_assignments(output);

    let mut values = Vec::new();
    let mut block_visits = Vec::new();

    for (name, value) in all_assignments {
        if let Some(rest) = name.strip_prefix("__blk_") {
            if let Ok(blk_id) = rest.parse::<u32>()
                && value.trim() == "1"
            {
                block_visits.push(blk_id);
            }
        } else {
            values.push((name, value));
        }
    }

    block_visits.sort();

    let description = output
        .lines()
        .find(|l| l.contains("Counterexample") || l.contains("violation") || l.contains("FAILED"))
        .unwrap_or("unknown counterexample")
        .to_string();

    Counterexample {
        description,
        vow_id,
        values,
        block_visits,
        raw_output: output.to_string(),
    }
}

fn extract_vow_id(output: &str) -> Option<u32> {
    let mut in_violated = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed == "Violated property:" {
            in_violated = true;
            continue;
        }
        if in_violated
            && let Some(rest) = trimmed.strip_prefix("vow:")
            && let Ok(id) = rest.parse::<u32>()
        {
            return Some(id);
        }
    }
    None
}

fn extract_variable_assignments(output: &str) -> Vec<(String, String)> {
    let mut assignments = Vec::new();
    let mut in_counterexample = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed == "[Counterexample]" {
            in_counterexample = true;
            continue;
        }
        if trimmed == "Violated property:" {
            break;
        }
        if !in_counterexample {
            continue;
        }

        if let Some((name, value)) = parse_assignment_line(trimmed) {
            assignments.push((name, value));
        }
    }
    assignments
}

fn parse_assignment_line(line: &str) -> Option<(String, String)> {
    let eq_pos = line.find('=')?;
    let name = line[..eq_pos].trim().to_string();
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let value_part = line[eq_pos + 1..].trim();
    let value = if let Some(paren_pos) = value_part.find(" (") {
        value_part[..paren_pos].trim().to_string()
    } else {
        value_part.to_string()
    };
    Some((name, value))
}

// ---------------------------------------------------------------------------
// Verification entry point
// ---------------------------------------------------------------------------

pub fn verify_function(func: &Function) -> VerificationResult {
    let empty: HashMap<FuncId, ConstantValue> = HashMap::new();
    verify_function_inner(func, &empty)
}

pub fn verify_function_with_module(func: &Function, module: &Module) -> VerificationResult {
    let const_fns = detect_constant_functions(module);
    verify_function_inner(func, &const_fns)
}

fn verify_function_inner(
    func: &Function,
    const_fns: &HashMap<FuncId, ConstantValue>,
) -> VerificationResult {
    let esbmc = match find_esbmc() {
        Some(p) => p,
        None => return VerificationResult::ToolNotFound,
    };

    let mut c_src = emit_c_module(&[func], const_fns);
    c_src.push_str(&emit_harness(func));

    let mut tmp = match tempfile::Builder::new().suffix(".c").tempfile() {
        Ok(f) => f,
        Err(e) => return VerificationResult::ToolError(e.to_string()),
    };
    if let Err(e) = tmp.write_all(c_src.as_bytes()) {
        return VerificationResult::ToolError(e.to_string());
    }
    if let Err(e) = tmp.flush() {
        return VerificationResult::ToolError(e.to_string());
    }

    let output = match Command::new(&esbmc)
        .arg(tmp.path())
        .arg("--no-bounds-check")
        .arg("--no-pointer-check")
        .arg("--unwind")
        .arg("10")
        .arg("--64")
        .output()
    {
        Ok(o) => o,
        Err(e) => return VerificationResult::ToolError(e.to_string()),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    if combined.contains("VERIFICATION SUCCESSFUL") {
        VerificationResult::Proven
    } else if combined.contains("VERIFICATION FAILED") {
        VerificationResult::Failed(parse_esbmc_output(&combined))
    } else if combined.to_lowercase().contains("timeout") {
        VerificationResult::Timeout
    } else {
        VerificationResult::ToolError(format!("unexpected esbmc output:\n{combined}"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vow_diag::Blame;
    use vow_ir::{BasicBlock, BlockId, FuncId, Inst, InstData, InstId, Opcode, VowEntry, VowId};
    use vow_syntax::span::Span;

    fn sp() -> Span {
        Span::new(0, 0)
    }

    fn inst(id: u32, op: Opcode, ty: Ty, args: Vec<u32>, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode: op,
            ty,
            args: args.into_iter().map(InstId).collect(),
            data,
            origin: sp(),
        }
    }

    fn trivially_true_func() -> Function {
        // fn always_ok() -> i32 ensures true { 42 }
        Function {
            id: FuncId(0),
            name: "always_ok".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "true".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(
                        0,
                        Opcode::ConstBool,
                        Ty::Bool,
                        vec![],
                        InstData::ConstBool(true),
                    ),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(2, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(42)),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    fn trivially_false_func() -> Function {
        // fn always_bad() -> i32 ensures false { 42 }
        Function {
            id: FuncId(0),
            name: "always_bad".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "false".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(
                        0,
                        Opcode::ConstBool,
                        Ty::Bool,
                        vec![],
                        InstData::ConstBool(false),
                    ),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(2, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(42)),
                    inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn verify_trivially_true_ensures() {
        let func = trivially_true_func();
        match verify_function(&func) {
            VerificationResult::Proven => {}
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Proven or ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn verify_trivially_false_ensures() {
        let func = trivially_false_func();
        match verify_function(&func) {
            VerificationResult::Failed(_) => {}
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Failed or ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn verify_divide_with_requires() {
        let func = Function {
            id: FuncId(0),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "y != 0".to_string(),
                blame: Blame::Caller,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                    inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(3, Opcode::NeI64, Ty::Bool, vec![1, 2], InstData::None),
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(3)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(
                        5,
                        Opcode::WrappingDivI64,
                        Ty::I64,
                        vec![0, 1],
                        InstData::None,
                    ),
                    inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        };
        match verify_function(&func) {
            VerificationResult::Proven | VerificationResult::ToolNotFound => {}
            other => panic!("expected Proven or ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn verify_trivially_false_has_structured_counterexample() {
        let func = trivially_false_func();
        match verify_function(&func) {
            VerificationResult::Failed(ce) => {
                assert_eq!(ce.vow_id, Some(0), "vow_id should be 0");
                assert!(!ce.raw_output.is_empty(), "raw_output should be non-empty");
            }
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Failed or ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn parse_esbmc_counterexample_output() {
        let output = "\
ESBMC version 8.0.0 64-bit x86_64 linux
Starting Bounded Model Checking

[Counterexample]


State 1 file /tmp/test.c line 9 column 3 function divide thread 0
----------------------------------------------------
  v1 = 0 (00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000)

State 2 file /tmp/test.c line 11 column 3 function divide thread 0
----------------------------------------------------
  v3 = 0

State 3 file /tmp/test.c line 12 column 3 function divide thread 0
----------------------------------------------------
Violated property:
  file /tmp/test.c line 12 column 3 function divide
  vow:0
  v3


VERIFICATION FAILED";

        let ce = parse_esbmc_output(output);
        assert_eq!(ce.vow_id, Some(0));
        assert_eq!(ce.values.len(), 2);
        assert_eq!(ce.values[0], ("v1".to_string(), "0".to_string()));
        assert_eq!(ce.values[1], ("v3".to_string(), "0".to_string()));
        assert!(ce.description.contains("Counterexample"));
    }

    #[test]
    fn parse_esbmc_counterexample_with_vow_id_2() {
        let output = "\
[Counterexample]

State 1 file /tmp/test.c line 5 column 3 function f thread 0
----------------------------------------------------
  v0 = 42

State 2 file /tmp/test.c line 8 column 3 function f thread 0
----------------------------------------------------
Violated property:
  file /tmp/test.c line 8 column 3 function f
  vow:2
  v0


VERIFICATION FAILED";

        let ce = parse_esbmc_output(output);
        assert_eq!(ce.vow_id, Some(2));
        assert_eq!(ce.values.len(), 1);
        assert_eq!(ce.values[0], ("v0".to_string(), "42".to_string()));
    }

    #[test]
    fn parse_esbmc_no_counterexample_section() {
        let output = "VERIFICATION FAILED\nsome other error";
        let ce = parse_esbmc_output(output);
        assert_eq!(ce.vow_id, None);
        assert!(ce.values.is_empty());
    }

    #[test]
    fn parse_assignment_line_basic() {
        assert_eq!(
            parse_assignment_line("  v1 = 0"),
            Some(("v1".to_string(), "0".to_string()))
        );
    }

    #[test]
    fn parse_assignment_line_with_binary() {
        assert_eq!(
            parse_assignment_line("  v1 = 0 (00000000 00000000)"),
            Some(("v1".to_string(), "0".to_string()))
        );
    }

    #[test]
    fn parse_assignment_line_separator() {
        assert_eq!(
            parse_assignment_line("----------------------------------------------------"),
            None
        );
    }

    #[test]
    fn parse_assignment_line_empty() {
        assert_eq!(parse_assignment_line(""), None);
    }

    #[test]
    fn parse_block_visits_from_counterexample() {
        let output = "\
[Counterexample]

State 1 file /tmp/test.c line 5 column 3 function f thread 0
----------------------------------------------------
  __blk_0 = 1

State 2 file /tmp/test.c line 6 column 3 function f thread 0
----------------------------------------------------
  __blk_1 = 1

State 3 file /tmp/test.c line 7 column 3 function f thread 0
----------------------------------------------------
  __blk_2 = 0

State 4 file /tmp/test.c line 8 column 3 function f thread 0
----------------------------------------------------
  v0 = 42

Violated property:
  file /tmp/test.c line 10 column 3 function f
  vow:0
  v0


VERIFICATION FAILED";

        let ce = parse_esbmc_output(output);
        assert_eq!(ce.block_visits, vec![0, 1], "blocks 0 and 1 visited");
        assert_eq!(ce.values.len(), 1, "only v0 in values, __blk_* filtered");
        assert_eq!(ce.values[0], ("v0".to_string(), "42".to_string()));
    }

    #[test]
    fn parse_no_block_visits() {
        let output = "\
[Counterexample]

State 1 file /tmp/test.c line 5 column 3 function f thread 0
----------------------------------------------------
  v0 = 7

Violated property:
  vow:0
  v0

VERIFICATION FAILED";

        let ce = parse_esbmc_output(output);
        assert!(ce.block_visits.is_empty());
        assert_eq!(ce.values.len(), 1);
    }

    // --- Vec verification integration tests ---

    fn vec_push_one_ensures_len_1() -> Function {
        // fn make_one() -> Vec<i64> { ensures: result.len() == 1 }
        // { let v = Vec::new(); v.push(42); v }
        //
        // IR: create vec, push, get len, assert len==1, return
        use vow_ir::InstId;
        Function {
            id: FuncId(0),
            name: "make_one".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "ensures result.len() == 1".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    // v0, v1 = size/align constants for vec_new
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    // v2 = Vec::new()
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(0), InstId(1)],
                        data: InstData::CallExtern("__vow_vec_new".to_string()),
                        origin: sp(),
                    },
                    // v3 = 42
                    inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                    // v.push(42)
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::Call,
                        ty: Ty::Unit,
                        args: vec![InstId(2), InstId(3)],
                        data: InstData::CallExtern("__vow_vec_push_val".to_string()),
                        origin: sp(),
                    },
                    // v5 = v.len()
                    Inst {
                        id: InstId(5),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(2)],
                        data: InstData::CallExtern("__vow_vec_len".to_string()),
                        origin: sp(),
                    },
                    // v6 = 1
                    inst(6, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                    // v7 = (v5 == v6)
                    inst(7, Opcode::EqI64, Ty::Bool, vec![5, 6], InstData::None),
                    // ensures: v7
                    Inst {
                        id: InstId(8),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(7)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(9, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    fn vec_empty_ensures_len_1_violated() -> Function {
        // fn make_empty() -> Vec<i64> { ensures: result.len() == 1 }
        // { let v = Vec::new(); v }  -- VIOLATED: len is 0 not 1
        use vow_ir::InstId;
        Function {
            id: FuncId(0),
            name: "make_empty".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "ensures result.len() == 1".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(0), InstId(1)],
                        data: InstData::CallExtern("__vow_vec_new".to_string()),
                        origin: sp(),
                    },
                    // v3 = v.len()
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(2)],
                        data: InstData::CallExtern("__vow_vec_len".to_string()),
                        origin: sp(),
                    },
                    inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                    inst(5, Opcode::EqI64, Ty::Bool, vec![3, 4], InstData::None),
                    Inst {
                        id: InstId(6),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(5)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(7, Opcode::Return, Ty::Unit, vec![2], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn verify_vec_push_ensures_len() {
        let func = vec_push_one_ensures_len_1();
        match verify_function(&func) {
            VerificationResult::Proven => {}
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Proven or ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn verify_vec_violated_len_contract() {
        let func = vec_empty_ensures_len_1_violated();
        match verify_function(&func) {
            VerificationResult::Failed(ce) => {
                assert_eq!(ce.vow_id, Some(0), "vow_id should be 0");
            }
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Failed or ToolNotFound, got {other:?}"),
        }
    }

    // --- String verification integration tests ---

    fn string_push_byte_ensures_len_gt_0() -> Function {
        // fn make_nonempty() -> String { ensures: result.len() > 0 }
        // { let s = String::from(""); s.push_byte(65); s }
        //
        // from_cstr → nondet len >= 0; push_byte increments len; ensures len > 0
        use vow_ir::InstId;
        Function {
            id: FuncId(0),
            name: "make_nonempty".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "ensures result.len() > 0".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    // v0 = ConstStr (the literal pointer)
                    inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    // v1 = String::from(v0)
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(0)],
                        data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                        origin: sp(),
                    },
                    // v2 = 65 (byte 'A')
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(65)),
                    // s.push_byte(65)
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Call,
                        ty: Ty::Unit,
                        args: vec![InstId(1), InstId(2)],
                        data: InstData::CallExtern("__vow_string_push_byte".to_string()),
                        origin: sp(),
                    },
                    // v4 = s.len()
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(1)],
                        data: InstData::CallExtern("__vow_string_len".to_string()),
                        origin: sp(),
                    },
                    // v5 = 0
                    inst(5, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    // v6 = (v4 > v5)
                    inst(6, Opcode::GtI64, Ty::Bool, vec![4, 5], InstData::None),
                    // ensures: v6
                    Inst {
                        id: InstId(7),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(6)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(8, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    fn string_empty_ensures_len_gt_0_violated() -> Function {
        // fn make_empty() -> String { ensures: result.len() > 0 }
        // { let s = String::from(""); s }  -- VIOLATED: from_cstr gives nondet len >= 0
        use vow_ir::InstId;
        Function {
            id: FuncId(0),
            name: "make_empty".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Ptr,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "ensures result.len() > 0".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![InstId(0)],
                        data: InstData::CallExtern("__vow_string_from_cstr".to_string()),
                        origin: sp(),
                    },
                    // v2 = s.len()
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(1)],
                        data: InstData::CallExtern("__vow_string_len".to_string()),
                        origin: sp(),
                    },
                    inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                    inst(4, Opcode::GtI64, Ty::Bool, vec![2, 3], InstData::None),
                    Inst {
                        id: InstId(5),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(4)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(6, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn verify_string_push_byte_ensures_len() {
        let func = string_push_byte_ensures_len_gt_0();
        match verify_function(&func) {
            VerificationResult::Proven => {}
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Proven or ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn verify_string_violated_len_contract() {
        let func = string_empty_ensures_len_gt_0_violated();
        match verify_function(&func) {
            VerificationResult::Failed(ce) => {
                assert_eq!(ce.vow_id, Some(0), "vow_id should be 0");
            }
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Failed or ToolNotFound, got {other:?}"),
        }
    }

    // --- HashMap verification integration tests ---

    fn hashmap_insert_ensures_contains() -> Function {
        // fn insert_and_check() -> bool { ensures: result == true }
        // { let m = HashMap::new(); m.insert(42, 100); m.contains_key(42) }
        use vow_ir::InstId;
        Function {
            id: FuncId(0),
            name: "insert_and_check".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::Bool,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "ensures result == true".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    // v0 = HashMap::new()
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![],
                        data: InstData::CallExtern("__vow_map_new".to_string()),
                        origin: sp(),
                    },
                    // v1 = 42 (key)
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                    // v2 = 100 (value)
                    inst(
                        2,
                        Opcode::ConstI64,
                        Ty::I64,
                        vec![],
                        InstData::ConstI64(100),
                    ),
                    // m.insert(42, 100)
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Call,
                        ty: Ty::Unit,
                        args: vec![InstId(0), InstId(1), InstId(2)],
                        data: InstData::CallExtern("__vow_map_insert".to_string()),
                        origin: sp(),
                    },
                    // v4 = 42 (key for contains_key)
                    inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(42)),
                    // v5 = m.contains_key(42)
                    Inst {
                        id: InstId(5),
                        opcode: Opcode::Call,
                        ty: Ty::Bool,
                        args: vec![InstId(0), InstId(4)],
                        data: InstData::CallExtern("__vow_map_contains".to_string()),
                        origin: sp(),
                    },
                    // v6 = true
                    inst(
                        6,
                        Opcode::ConstBool,
                        Ty::Bool,
                        vec![],
                        InstData::ConstBool(true),
                    ),
                    // v7 = (v5 == v6)
                    inst(7, Opcode::EqI64, Ty::Bool, vec![5, 6], InstData::None),
                    // ensures: v7
                    Inst {
                        id: InstId(8),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(7)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(9, Opcode::Return, Ty::Unit, vec![5], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    fn hashmap_insert_ensures_len_1() -> Function {
        // fn insert_one() -> i64 { ensures: result == 1 }
        // { let m = HashMap::new(); m.insert(10, 20); m.len() }
        use vow_ir::InstId;
        Function {
            id: FuncId(0),
            name: "insert_one".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "ensures result == 1".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    // v0 = HashMap::new()
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![],
                        data: InstData::CallExtern("__vow_map_new".to_string()),
                        origin: sp(),
                    },
                    // v1 = 10 (key)
                    inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(10)),
                    // v2 = 20 (value)
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(20)),
                    // m.insert(10, 20)
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Call,
                        ty: Ty::Unit,
                        args: vec![InstId(0), InstId(1), InstId(2)],
                        data: InstData::CallExtern("__vow_map_insert".to_string()),
                        origin: sp(),
                    },
                    // v4 = m.len()
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(0)],
                        data: InstData::CallExtern("__vow_map_len".to_string()),
                        origin: sp(),
                    },
                    // v5 = 1
                    inst(5, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                    // v6 = (v4 == v5)
                    inst(6, Opcode::EqI64, Ty::Bool, vec![4, 5], InstData::None),
                    // ensures: v6
                    Inst {
                        id: InstId(7),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(6)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(8, Opcode::Return, Ty::Unit, vec![4], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    fn hashmap_empty_ensures_len_1_violated() -> Function {
        // fn empty_map() -> i64 { ensures: result == 1 }
        // { let m = HashMap::new(); m.len() }  -- VIOLATED: len is 0
        use vow_ir::InstId;
        Function {
            id: FuncId(0),
            name: "empty_map".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "ensures result == 1".to_string(),
                blame: Blame::Callee,
                bindings: vec![],
                file: String::new(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    // v0 = HashMap::new()
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::Call,
                        ty: Ty::Ptr,
                        args: vec![],
                        data: InstData::CallExtern("__vow_map_new".to_string()),
                        origin: sp(),
                    },
                    // v1 = m.len()
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(0)],
                        data: InstData::CallExtern("__vow_map_len".to_string()),
                        origin: sp(),
                    },
                    // v2 = 1
                    inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
                    // v3 = (v1 == v2)
                    inst(3, Opcode::EqI64, Ty::Bool, vec![1, 2], InstData::None),
                    // ensures: v3
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::VowEnsures,
                        ty: Ty::Unit,
                        args: vec![InstId(3)],
                        data: InstData::VowId(VowId(0)),
                        origin: sp(),
                    },
                    inst(5, Opcode::Return, Ty::Unit, vec![1], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn verify_hashmap_insert_ensures_contains() {
        let func = hashmap_insert_ensures_contains();
        match verify_function(&func) {
            VerificationResult::Proven => {}
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Proven or ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn verify_hashmap_insert_ensures_len() {
        let func = hashmap_insert_ensures_len_1();
        match verify_function(&func) {
            VerificationResult::Proven => {}
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Proven or ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn verify_hashmap_violated_len_contract() {
        let func = hashmap_empty_ensures_len_1_violated();
        match verify_function(&func) {
            VerificationResult::Failed(ce) => {
                assert_eq!(ce.vow_id, Some(0), "vow_id should be 0");
            }
            VerificationResult::ToolNotFound => {
                eprintln!("SKIP: esbmc not found");
            }
            other => panic!("expected Failed or ToolNotFound, got {other:?}"),
        }
    }
}
