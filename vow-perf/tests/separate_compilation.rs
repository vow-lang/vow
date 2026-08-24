use std::collections::{HashMap, HashSet};
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_diag::Blame;
use vow_ir::{
    BasicBlock, BlockId, FuncId, Function, Inst, InstData, InstId, Module, Opcode, RegionId,
    RegionSummary, Ty, VowEntry, VowId, decode_module, encode_module, validate,
};
use vow_perf::{InstrumentationError, instrument_module};
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

fn assert_vmod_round_trip(module: &Module) {
    let encoded = encode_module(module);
    let decoded = decode_module(&encoded).expect("decode instrumented IR");
    assert_eq!(
        decoded, *module,
        "instrumented IR changed across the .vmod round trip"
    );
    assert_eq!(
        encode_module(&decoded),
        encoded,
        "instrumented IR encoding is not canonical"
    );
}

fn vec_sort_module() -> Module {
    let mut module = production_module();
    module.name = "vec_sort_cost".to_string();

    let function = &mut module.functions[0];
    function.name = "sort_values".to_string();
    function.params = vec![Ty::Ptr];
    function.param_names = vec!["values".to_string()];
    function.return_ty = Ty::Ptr;
    function.blocks[0].insts = vec![
        instruction(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
        instruction(
            1,
            Opcode::Call,
            Ty::Ptr,
            vec![InstId(0)],
            InstData::CallExtern("__vow_vec_sort".to_string()),
        ),
        instruction(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(7)),
        // Uncatalogued size-dependent helper: it must keep the plain counter.
        instruction(
            3,
            Opcode::Call,
            Ty::Bool,
            vec![InstId(1), InstId(2)],
            InstData::CallExtern("__vow_map_contains".to_string()),
        ),
        instruction(4, Opcode::Return, Ty::Unit, vec![InstId(1)], InstData::None),
    ];

    module
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

#[test]
fn vec_sort_calls_receive_size_dependent_cost_adapter() {
    let source = vec_sort_module();

    let instrumented = instrument_module(&source).expect("instrument Vec::sort wrapper");
    let instructions = &instrumented.as_module().functions[0].blocks[0].insts;
    let extern_calls: Vec<(&str, &[InstId])> = instructions
        .iter()
        .filter_map(|inst| match &inst.data {
            InstData::CallExtern(symbol) => Some((symbol.as_str(), inst.args.as_slice())),
            _ => None,
        })
        .collect();

    assert_eq!(
        extern_calls,
        vec![
            ("__vow_perf_count", &[] as &[InstId]),
            ("__vow_perf_count_vec_sort", &[InstId(0)]),
            ("__vow_vec_sort", &[InstId(0)]),
            ("__vow_perf_count", &[]),
            ("__vow_perf_count", &[]),
            ("__vow_map_contains", &[InstId(1), InstId(2)]),
            ("__vow_perf_count", &[]),
        ],
        "Vec::sort must count its hidden size-dependent work without changing its operand, \
         and an uncatalogued helper must keep the plain operand-free counter"
    );
    assert!(
        validate(instrumented.as_module()).is_ok(),
        "instrumented Vec::sort IR must remain valid"
    );

    assert_vmod_round_trip(instrumented.as_module());

    CraneliftBackend::new()
        .compile_module(instrumented.as_module(), BuildMode::Release, TraceMode::Off)
        .expect("compile instrumented Vec::sort wrapper");
}

#[test]
fn vec_sort_call_with_unexpected_arity_is_rejected() {
    let mut source = vec_sort_module();
    source.functions[0].blocks[0].insts[1].args.push(InstId(0));

    // Falling back to the plain counter here would charge one operation for a
    // sort and let a Linear verdict pass on n log n work, so a catalogued
    // helper whose operands no longer match its adapter must fail closed.
    let error = instrument_module(&source).expect_err("off-ABI Vec::sort call must fail closed");

    assert_eq!(
        error,
        InstrumentationError::CatalogedHelperArity {
            function: "sort_values".to_string(),
            symbol: "__vow_vec_sort".to_string(),
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        error.to_string(),
        "operation-count instrumentation found `__vow_vec_sort` with 2 operands in \
         function `sort_values`, but its cost adapter takes exactly 1"
    );
}

#[test]
fn instrumented_multiblock_ir_preserves_canonical_ids_and_round_trips() {
    let mut source = production_module();
    let function = &mut source.functions[0];
    function.vows.push(VowEntry {
        id: VowId(0),
        description: "then result".to_string(),
        blame: Blame::Callee,
        bindings: vec![("result".to_string(), InstId(2))],
        file: "test.vow".to_string(),
        offset: 0,
    });
    function.local_names.insert(2, "then_result".to_string());
    function.blocks = vec![
        BasicBlock {
            id: BlockId(0),
            insts: vec![
                instruction(
                    0,
                    Opcode::ConstBool,
                    Ty::Bool,
                    vec![],
                    InstData::ConstBool(true),
                ),
                instruction(
                    1,
                    Opcode::Branch,
                    Ty::Unit,
                    vec![InstId(0)],
                    InstData::BranchTargets {
                        then_block: BlockId(1),
                        else_block: BlockId(2),
                    },
                ),
            ],
        },
        BasicBlock {
            id: BlockId(1),
            insts: vec![
                instruction(2, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(7)),
                instruction(3, Opcode::Return, Ty::Unit, vec![InstId(2)], InstData::None),
            ],
        },
        BasicBlock {
            id: BlockId(2),
            insts: vec![
                instruction(4, Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(9)),
                instruction(5, Opcode::Return, Ty::Unit, vec![InstId(4)], InstData::None),
            ],
        },
    ];

    let instrumented = instrument_module(&source).expect("instrument multi-block IR");
    let module = instrumented.as_module();
    let function = &module.functions[0];
    let ids: Vec<InstId> = function
        .blocks
        .iter()
        .flat_map(|block| block.insts.iter().map(|inst| inst.id))
        .collect();
    let unique_ids: HashSet<InstId> = ids.iter().copied().collect();

    assert_eq!(
        ids.len(),
        unique_ids.len(),
        "instruction IDs must be unique"
    );
    assert!(
        validate(module).is_ok(),
        "instrumented IR must remain valid"
    );
    assert_eq!(function.vows[0].bindings[0].1, InstId(2));
    assert_eq!(
        function.local_names.get(&2).map(String::as_str),
        Some("then_result")
    );
    assert_eq!(
        function
            .blocks
            .iter()
            .flat_map(|block| &block.insts)
            .filter(|inst| inst.id == InstId(2) && inst.opcode == Opcode::ConstI32)
            .count(),
        1,
        "existing ID references must keep naming the original instruction"
    );

    assert_vmod_round_trip(module);
}
