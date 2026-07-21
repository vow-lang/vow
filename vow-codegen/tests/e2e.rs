use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::linker::{find_runtime_lib, link};
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_ir::{
    BasicBlock, BlockId, FuncId, Function, Inst, InstData, InstId, IntegerType, Module, Opcode,
    RegionId, RegionSummary, Ty, VowEntry, VowId,
};
use vow_syntax::span::Span;

fn sp() -> Span {
    Span::new(0, 0)
}

fn inst(id: u32, op: Opcode, ty: Ty, args: Vec<u32>, data: InstData) -> Inst {
    let data = integer_test_data(op, ty, data);
    Inst {
        id: InstId(id),
        opcode: op,
        ty,
        args: args.into_iter().map(InstId).collect(),
        data,
        origin: sp(),
        region: RegionId::Root,
    }
}

fn integer_test_data(op: Opcode, ty: Ty, data: InstData) -> InstData {
    if data != InstData::None
        || !matches!(
            op,
            Opcode::WrappingAdd
                | Opcode::WrappingSub
                | Opcode::WrappingMul
                | Opcode::WrappingDiv
                | Opcode::WrappingRem
                | Opcode::CheckedAdd
                | Opcode::CheckedSub
                | Opcode::CheckedMul
                | Opcode::CheckedDiv
                | Opcode::CheckedRem
                | Opcode::Eq
                | Opcode::Ne
                | Opcode::Lt
                | Opcode::Le
                | Opcode::Gt
                | Opcode::Ge
                | Opcode::BitAnd
                | Opcode::BitOr
                | Opcode::BitXor
                | Opcode::Shl
                | Opcode::Shr
        )
    {
        return data;
    }
    InstData::Integer(match ty {
        Ty::I32 => IntegerType::I32,
        Ty::U64 => IntegerType::U64,
        Ty::U8 => IntegerType::U8,
        _ => IntegerType::I64,
    })
}

/// Build, link, and return the path to a runnable executable in `dir`.
fn compile_and_link(module: &Module, mode: BuildMode, dir: &TempDir) -> Option<PathBuf> {
    let runtime = find_runtime_lib()?;

    let backend = CraneliftBackend::new();
    let obj = backend
        .compile_module(module, mode, TraceMode::Off)
        .expect("codegen failed");

    let obj_path = dir.path().join("out.o");
    obj.write_to_file(&obj_path).expect("write obj failed");

    let exe_path = dir.path().join("out");
    link(&[&obj_path], &runtime, None, &exe_path).expect("link failed");

    Some(exe_path)
}

fn run_exe(exe: &PathBuf) -> std::process::Output {
    Command::new(exe)
        .output()
        .expect("failed to run executable")
}

/// fn main() -> i32 { 42 }
fn make_main_returns_42() -> Module {
    Module {
        name: "exit42".to_string(),
        strings: vec![],
        struct_layouts: vec![],
        enum_layouts: vec![],
        warnings: vec![],
        functions: vec![Function {
            id: FuncId(0),
            name: "main".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    inst(0, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(42)),
                    inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        }],
    }
}

#[test]
fn executable_exits_with_correct_code() {
    let dir = TempDir::new().unwrap();
    let module = make_main_returns_42();
    let Some(exe) = compile_and_link(&module, BuildMode::Release, &dir) else {
        eprintln!("SKIP: vow-runtime not found");
        return;
    };
    let out = run_exe(&exe);
    assert_eq!(
        out.status.code(),
        Some(42),
        "expected exit code 42, got {:?}",
        out.status.code()
    );
}

#[test]
fn debug_executable_exits_with_correct_code() {
    let dir = TempDir::new().unwrap();
    let module = make_main_returns_42();
    let Some(exe) = compile_and_link(&module, BuildMode::Debug, &dir) else {
        eprintln!("SKIP: vow-runtime not found");
        return;
    };
    let out = run_exe(&exe);
    assert_eq!(out.status.code(), Some(42));
}

#[test]
fn vow_violation_exits_with_reserved_abort_code_and_blames_caller() {
    // fn divide(x: i64, y: i64) -> i64
    //   vow requires: y != 0  (blame = Caller)
    //   ...
    // fn main() -> i32 { divide(10, 0); 0 }
    //
    // Build the divide function IR manually:
    //   block0:
    //     v0 = get_arg(0)  [x: i64]
    //     v1 = get_arg(1)  [y: i64]
    //     v2 = const_i64(0)
    //     v3 = ne_i64(v1, v2)   [y != 0]
    //     v4 = vow_requires(v3) [vow_id=0, blame=Caller]
    //     v5 = wrapping_div_i64(v0, v1)
    //     return v5
    use vow_diag::Blame;

    let divide = Function {
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
            bindings: vec![("y".to_string(), InstId(1))],
            file: String::new(),
            offset: 0,
        }],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            insts: vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                inst(3, Opcode::Ne, Ty::Bool, vec![1, 2], InstData::None),
                Inst {
                    id: InstId(4),
                    opcode: Opcode::VowRequires,
                    ty: Ty::Unit,
                    args: vec![InstId(3)],
                    data: InstData::VowId(vow_ir::VowId(0)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(5, Opcode::WrappingDiv, Ty::I64, vec![0, 1], InstData::None),
                inst(6, Opcode::Return, Ty::Unit, vec![5], InstData::None),
            ],
        }],
        local_names: std::collections::HashMap::new(),
        summary: RegionSummary::default(),
        source_file: String::new(),
    };

    // main: call divide(10, 0), return 0
    let main_fn = Function {
        id: FuncId(1),
        name: "main".to_string(),
        params: vec![],
        param_names: vec![],
        return_ty: Ty::I32,
        effects: vec![],
        vows: vec![],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            insts: vec![
                inst(
                    10,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(10),
                ),
                inst(11, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                Inst {
                    id: InstId(12),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(10), InstId(11)],
                    data: InstData::CallTarget(FuncId(0)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(13, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(0)),
                inst(14, Opcode::Return, Ty::Unit, vec![13], InstData::None),
            ],
        }],
        local_names: std::collections::HashMap::new(),
        summary: RegionSummary::default(),
        source_file: String::new(),
    };

    let module = Module {
        name: "divide_test".to_string(),
        strings: vec![],
        struct_layouts: vec![],
        enum_layouts: vec![],
        warnings: vec![],
        functions: vec![divide, main_fn],
    };

    let dir = TempDir::new().unwrap();
    let Some(exe) = compile_and_link(&module, BuildMode::Debug, &dir) else {
        eprintln!("SKIP: vow-runtime not found");
        return;
    };
    let out = run_exe(&exe);
    assert_eq!(
        out.status.code(),
        Some(134),
        "expected reserved runtime-abort exit code 134 (vow violation, #877), got {:?}",
        out.status.code()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Caller"),
        "expected blame=Caller in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains(r#""y":0"#),
        r#"expected "y":0 in stderr, got: {stderr}"#
    );
}

#[test]
fn vow_violation_reports_variable_values() {
    // fn nonneg(x: i64) -> i64
    //   vow ensures result > 0
    // fn main() -> i32 { nonneg(-1); 0 }
    //
    // IR:
    //   nonneg block0:
    //     v0 = get_arg(0)  [x: i64]
    //     v1 = const_i64(0)
    //     v2 = gt_i64(v0, v1)      [result > 0, but result IS v0 here]
    //     v3 = vow_ensures(v2)     [vow_id=0, blame=Callee, bindings=[("result", InstId(0))]]
    //     return v0
    use vow_diag::Blame;

    let nonneg = Function {
        id: FuncId(0),
        name: "nonneg".to_string(),
        params: vec![Ty::I64],
        param_names: vec![],
        return_ty: Ty::I64,
        effects: vec![],
        vows: vec![VowEntry {
            id: VowId(0),
            description: "ensures result > 0".to_string(),
            blame: Blame::Callee,
            bindings: vec![("result".to_string(), InstId(0))],
            file: String::new(),
            offset: 0,
        }],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            insts: vec![
                inst(0, Opcode::GetArg, Ty::I64, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
                inst(2, Opcode::Gt, Ty::Bool, vec![0, 1], InstData::None),
                Inst {
                    id: InstId(3),
                    opcode: Opcode::VowEnsures,
                    ty: Ty::Unit,
                    args: vec![InstId(2)],
                    data: InstData::VowId(VowId(0)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        }],
        local_names: std::collections::HashMap::new(),
        summary: RegionSummary::default(),
        source_file: String::new(),
    };

    let main_fn = Function {
        id: FuncId(1),
        name: "main".to_string(),
        params: vec![],
        param_names: vec![],
        return_ty: Ty::I32,
        effects: vec![],
        vows: vec![],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            insts: vec![
                inst(
                    10,
                    Opcode::ConstI64,
                    Ty::I64,
                    vec![],
                    InstData::ConstI64(-1),
                ),
                Inst {
                    id: InstId(11),
                    opcode: Opcode::Call,
                    ty: Ty::I64,
                    args: vec![InstId(10)],
                    data: InstData::CallTarget(FuncId(0)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(12, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(0)),
                inst(13, Opcode::Return, Ty::Unit, vec![12], InstData::None),
            ],
        }],
        local_names: std::collections::HashMap::new(),
        summary: RegionSummary::default(),
        source_file: String::new(),
    };

    let module = Module {
        name: "nonneg_test".to_string(),
        strings: vec![],
        struct_layouts: vec![],
        enum_layouts: vec![],
        warnings: vec![],
        functions: vec![nonneg, main_fn],
    };

    let dir = TempDir::new().unwrap();
    let Some(exe) = compile_and_link(&module, BuildMode::Debug, &dir) else {
        eprintln!("SKIP: vow-runtime not found");
        return;
    };
    let out = run_exe(&exe);
    assert_eq!(
        out.status.code(),
        Some(134),
        "expected reserved runtime-abort exit code 134 (vow violation, #877), got {:?}",
        out.status.code()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Callee"),
        "expected blame=Callee in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains(r#""result":-1"#),
        r#"expected "result":-1 in stderr, got: {stderr}"#
    );
}

#[test]
fn vow_violation_reports_u8_variable_value() {
    // fn takes_byte(x: u8) -> u8
    //   vow requires x > 200
    // fn main() -> i32 { takes_byte(5); 0 }
    //
    // Regression test for a bug where a captured u8 free variable's payload
    // was always zero-filled in the VowViolation binding (the tag correctly
    // said "u8", but the payload-construction match had no u8 arm).
    //
    // IR:
    //   takes_byte block0:
    //     v0 = get_arg(0)         [x: u8]
    //     v1 = const_u8(200)
    //     v2 = gt_u8(v0, v1)      [x > 200]
    //     v3 = vow_requires(v2)   [vow_id=0, blame=Caller, bindings=[("x", InstId(0))]]
    //     return v0
    use vow_diag::Blame;

    let takes_byte = Function {
        id: FuncId(0),
        name: "takes_byte".to_string(),
        params: vec![Ty::U8],
        param_names: vec![],
        return_ty: Ty::U8,
        effects: vec![],
        vows: vec![VowEntry {
            id: VowId(0),
            description: "requires x > 200".to_string(),
            blame: Blame::Caller,
            bindings: vec![("x".to_string(), InstId(0))],
            file: String::new(),
            offset: 0,
        }],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            insts: vec![
                inst(0, Opcode::GetArg, Ty::U8, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::ConstU8, Ty::U8, vec![], InstData::ConstU8(200)),
                Inst {
                    id: InstId(2),
                    opcode: Opcode::Gt,
                    ty: Ty::Bool,
                    args: vec![InstId(0), InstId(1)],
                    data: InstData::Integer(IntegerType::U8),
                    origin: sp(),
                    region: RegionId::Root,
                },
                Inst {
                    id: InstId(3),
                    opcode: Opcode::VowRequires,
                    ty: Ty::Unit,
                    args: vec![InstId(2)],
                    data: InstData::VowId(VowId(0)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(4, Opcode::Return, Ty::Unit, vec![0], InstData::None),
            ],
        }],
        local_names: std::collections::HashMap::new(),
        summary: RegionSummary::default(),
        source_file: String::new(),
    };

    let main_fn = Function {
        id: FuncId(1),
        name: "main".to_string(),
        params: vec![],
        param_names: vec![],
        return_ty: Ty::I32,
        effects: vec![],
        vows: vec![],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            insts: vec![
                inst(10, Opcode::ConstU8, Ty::U8, vec![], InstData::ConstU8(5)),
                Inst {
                    id: InstId(11),
                    opcode: Opcode::Call,
                    ty: Ty::U8,
                    args: vec![InstId(10)],
                    data: InstData::CallTarget(FuncId(0)),
                    origin: sp(),
                    region: RegionId::Root,
                },
                inst(12, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(0)),
                inst(13, Opcode::Return, Ty::Unit, vec![12], InstData::None),
            ],
        }],
        local_names: std::collections::HashMap::new(),
        summary: RegionSummary::default(),
        source_file: String::new(),
    };

    let module = Module {
        name: "u8_vow_test".to_string(),
        strings: vec![],
        struct_layouts: vec![],
        enum_layouts: vec![],
        warnings: vec![],
        functions: vec![takes_byte, main_fn],
    };

    let dir = TempDir::new().unwrap();
    let Some(exe) = compile_and_link(&module, BuildMode::Debug, &dir) else {
        eprintln!("SKIP: vow-runtime not found");
        return;
    };
    let out = run_exe(&exe);
    assert_eq!(
        out.status.code(),
        Some(134),
        "expected reserved runtime-abort exit code 134 (vow violation), got {:?}",
        out.status.code()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Caller"),
        "expected blame=Caller in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains(r#""x":5"#),
        r#"expected "x":5 (real u8 value, not 0) in stderr, got: {stderr}"#
    );
}
