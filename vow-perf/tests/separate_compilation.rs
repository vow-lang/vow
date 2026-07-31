use std::collections::HashMap;
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_ir::{
    BasicBlock, BlockId, FuncId, Function, Inst, InstData, InstId, Module, Opcode, RegionId,
    RegionSummary, Ty,
};
use vow_perf::instrument_module;
use vow_syntax::span::Span;

fn instruction(id: u32, opcode: Opcode, ty: Ty, args: Vec<InstId>, data: InstData) -> Inst {
    Inst {
        id: InstId(id),
        opcode,
        ty,
        args,
        data,
        origin: Span::new(0, 0),
        region: RegionId::Root,
    }
}

fn production_module() -> Module {
    Module {
        name: "separate_compilation".to_string(),
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
                    instruction(0, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(42)),
                    instruction(
                        1,
                        Opcode::ComplexityDescriptor,
                        Ty::Unit,
                        vec![],
                        InstData::None,
                    ),
                    instruction(2, Opcode::Return, Ty::Unit, vec![InstId(0)], InstData::None),
                ],
            }],
            local_names: HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        }],
        strings: vec![],
        struct_layouts: vec![],
        enum_layouts: vec![],
        warnings: vec![],
    }
}

#[test]
fn instrumentation_is_a_separate_compilation_artifact() {
    let source = production_module();
    let source_snapshot = source.clone();
    let backend = CraneliftBackend::new();
    let production_before = backend
        .compile_module(&source, BuildMode::Release, TraceMode::Off)
        .expect("production codegen before instrumentation");

    let instrumented = instrument_module(&source).expect("instrument source clone");

    let production_after = backend
        .compile_module(&source, BuildMode::Release, TraceMode::Off)
        .expect("production codegen after instrumentation");
    let instrumented_object = backend
        .compile_module(instrumented.as_module(), BuildMode::Release, TraceMode::Off)
        .expect("instrumented codegen");

    assert_eq!(source, source_snapshot, "instrumentation mutated source IR");
    assert_eq!(
        production_before.bytes, production_after.bytes,
        "requesting instrumentation changed production object bytes"
    );
    assert_ne!(
        production_before.bytes, instrumented_object.bytes,
        "instrumented object must be distinct from the production object"
    );
}
