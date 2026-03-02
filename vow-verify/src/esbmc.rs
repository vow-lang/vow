use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use vow_ir::{Function, Ty};

use crate::c_emitter::emit_c_module;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Counterexample {
    pub description: String,
    pub vow_id: Option<u32>,
    pub inputs: Vec<(String, String)>,
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
    let inputs = extract_variable_assignments(output);
    let description = output
        .lines()
        .find(|l| l.contains("Counterexample") || l.contains("violation") || l.contains("FAILED"))
        .unwrap_or("unknown counterexample")
        .to_string();

    Counterexample {
        description,
        vow_id,
        inputs,
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
    let esbmc = match find_esbmc() {
        Some(p) => p,
        None => return VerificationResult::ToolNotFound,
    };

    let mut c_src = emit_c_module(&[func]);
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
        assert_eq!(ce.inputs.len(), 2);
        assert_eq!(ce.inputs[0], ("v1".to_string(), "0".to_string()));
        assert_eq!(ce.inputs[1], ("v3".to_string(), "0".to_string()));
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
        assert_eq!(ce.inputs.len(), 1);
        assert_eq!(ce.inputs[0], ("v0".to_string(), "42".to_string()));
    }

    #[test]
    fn parse_esbmc_no_counterexample_section() {
        let output = "VERIFICATION FAILED\nsome other error";
        let ce = parse_esbmc_output(output);
        assert_eq!(ce.vow_id, None);
        assert!(ce.inputs.is_empty());
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
}
