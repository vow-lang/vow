use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use tempfile::TempDir;
use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::linker::link;
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_ir::{
    BasicBlock, BlockId, FuncId, Function, InstData, InstId, Module, Opcode, RegionSummary, Ty,
};
use vow_perf::{ComplexityClass, Sample, Verdict, analyze, instrument_module};

mod common;
use common::instruction;

const SIZES: [u64; 6] = [16, 32, 64, 128, 256, 512];

fn runtime_archive() -> PathBuf {
    // The dev dependency builds a hashed static archive in this test binary's
    // deps directory, even when no top-level libvow_runtime.a was built.
    let test_binary = std::env::current_exe().expect("test binary path");
    let deps = test_binary.parent().expect("Cargo deps directory");
    let mut archives: Vec<_> = std::fs::read_dir(deps)
        .expect("read Cargo deps directory")
        .map(|entry| entry.expect("Cargo deps entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("libvow_runtime-") && name.ends_with(".a"))
        })
        .collect();
    archives.sort_by_key(|path| {
        path.metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    archives.pop().expect("Cargo built vow-runtime staticlib")
}

// Build a complete program so the production runtime constructs real maps.
// Resetting the counter immediately before each lookup excludes setup work.
fn lookup_program() -> Module {
    let mut insts = Vec::new();
    let mut emit = |opcode, ty, args, data| {
        let id = InstId(insts.len() as u32);
        insts.push(instruction(id.0, opcode, ty, args, data));
        id
    };

    let map = emit(
        Opcode::Call,
        Ty::Ptr,
        vec![],
        InstData::CallExtern("__vow_map_new".into()),
    );
    for index in 0..SIZES[SIZES.len() - 1] {
        let key = emit(
            Opcode::ConstI64,
            Ty::I64,
            vec![],
            InstData::ConstI64(index as i64),
        );
        emit(
            Opcode::Call,
            Ty::Unit,
            vec![map, key, key],
            InstData::CallExtern("__vow_map_insert".into()),
        );
        if SIZES.contains(&(index + 1)) {
            let missing = emit(Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(-1));
            emit(
                Opcode::Call,
                Ty::Unit,
                vec![],
                InstData::CallExtern("__vow_perf_counter_reset".into()),
            );
            emit(
                Opcode::Call,
                Ty::Bool,
                vec![map, missing],
                InstData::CallExtern("__vow_map_contains".into()),
            );
            let count = emit(
                Opcode::Call,
                Ty::U64,
                vec![],
                InstData::CallExtern("__vow_perf_counter_read".into()),
            );
            emit(
                Opcode::Call,
                Ty::Unit,
                vec![count],
                InstData::CallExtern("__vow_print_u64".into()),
            );
            let separator = emit(Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(-1));
            emit(
                Opcode::Call,
                Ty::Unit,
                vec![separator],
                InstData::CallExtern("__vow_print_i64".into()),
            );
        }
    }
    let zero = emit(Opcode::ConstI32, Ty::I32, vec![], InstData::ConstI32(0));
    emit(Opcode::Return, Ty::Unit, vec![zero], InstData::None);

    Module {
        name: "map_lookup_cost".into(),
        functions: vec![Function {
            id: FuncId(0),
            name: "main".into(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I32,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts,
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
fn map_lookup_hidden_scan_rejects_constant_and_accepts_linear() {
    let runtime = runtime_archive();
    let dir = TempDir::new().expect("temporary executable directory");
    let instrumented = instrument_module(&lookup_program()).expect("instrument lookup program");
    let object = CraneliftBackend::new()
        .compile_module(instrumented.as_module(), BuildMode::Release, TraceMode::Off)
        .expect("compile instrumented lookup program");
    let object_path = dir.path().join("lookup.o");
    object.write_to_file(&object_path).expect("write object");
    let executable = dir.path().join("lookup");
    link(&[&object_path], &runtime, None, &executable).expect("link with Vow runtime");
    let output = Command::new(executable)
        .output()
        .expect("run lookup program");
    assert!(output.status.success(), "lookup failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("numeric counts");
    let counts: Vec<u64> = stdout
        .split("-1")
        .filter(|text| !text.is_empty())
        .map(|text| text.parse().expect("operation count"))
        .collect();
    assert_eq!(
        counts.len(),
        SIZES.len(),
        "one count per map size: {stdout}"
    );
    let samples: Vec<_> = SIZES
        .iter()
        .zip(counts)
        .map(|(&size, operations)| Sample::new(size, operations))
        .collect();
    assert_eq!(
        analyze(ComplexityClass::Linear, &samples).unwrap().verdict,
        Verdict::Pass,
        "linear lookup must pass: {samples:?}"
    );
    assert_eq!(
        analyze(ComplexityClass::Constant, &samples)
            .unwrap()
            .verdict,
        Verdict::Fail,
        "constant lookup must fail: {samples:?}"
    );
}
