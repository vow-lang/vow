use vow_codegen::cranelift_backend::CraneliftBackend;
use vow_codegen::{Backend, BuildMode, TraceMode};
use vow_diag::{Diagnostic, DiagnosticEmitter};
use vow_syntax::parser::parse_module;
use vow_types::check::Checker;

const SRC: &str = include_str!("../../benchmarks/hard/H01_stack_push_pop/reference.vow");
const FILE: &str = "<baseline_isa>";

struct Collect(Vec<Diagnostic>);

impl DiagnosticEmitter for Collect {
    fn try_emit(&mut self, diag: &Diagnostic) -> std::io::Result<()> {
        self.0.push(diag.clone());
        Ok(())
    }

    fn try_finish(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn lower() -> vow_ir::Module {
    let (module, diags) = parse_module(SRC, FILE);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let item_files = vec![FILE.to_string(); module.items.len()];
    let mut emitter = Collect(Vec::new());
    let (string_exprs, pattern_aggregates, payload_scalars) = {
        let mut checker = Checker::new(FILE, &mut emitter);
        checker.check_module(&module, &item_files);
        assert!(!checker.has_errors(), "type errors");
        checker.into_lowering_metadata()
    };
    vow_ir::lower::lower_module_with_pattern_aggregates(
        &module,
        &item_files,
        &string_exprs,
        pattern_aggregates,
        payload_scalars,
    )
}

#[test]
fn baseline_isa_backend_emits_an_object_for_every_mode() {
    let ir = lower();
    for mode in [BuildMode::Debug, BuildMode::Release] {
        let object = CraneliftBackend::with_baseline_isa()
            .compile_module(&ir, mode, TraceMode::Off)
            .expect("baseline backend compiles");
        assert!(!object.bytes.is_empty());
    }
}
