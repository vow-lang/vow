use vow_diag::{Diagnostic, DiagnosticEmitter};
use vow_ir::types::{Function, InstData, Module, Opcode};
use vow_syntax::parser::parse_module;
use vow_types::check::Checker;

struct Collecting(Vec<Diagnostic>);

impl DiagnosticEmitter for Collecting {
    fn try_emit(&mut self, diag: &Diagnostic) -> std::io::Result<()> {
        self.0.push(diag.clone());
        Ok(())
    }

    fn try_finish(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The fixture type-checked and lowered the way the driver does, so the lowerer sees
/// the checker's element types.
fn lowered_fixture() -> Module {
    let source = include_str!("../../tests/fixtures/vec_float_layout.vow");
    let file = "vec_float_layout.vow";
    let (module, parse_diags) = parse_module(source, file);
    assert!(parse_diags.is_empty(), "{parse_diags:?}");
    let mut emitter = Collecting(Vec::new());
    let item_files = vec![file.to_string(); module.items.len()];
    let (string_exprs, pattern_aggregates, payload_scalars) = {
        let mut checker = Checker::new(file, &mut emitter);
        checker.check_module(&module, &item_files);
        assert!(!checker.has_errors(), "{:?}", emitter.0);
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

fn function<'a>(module: &'a Module, name: &str) -> &'a Function {
    module
        .functions
        .iter()
        .find(|func| func.name == name)
        .unwrap_or_else(|| panic!("function `{name}`"))
}

fn count(module: &Module, name: &str, opcode: Opcode) -> usize {
    function(module, name)
        .blocks
        .iter()
        .flat_map(|block| &block.insts)
        .filter(|inst| inst.opcode == opcode)
        .count()
}

/// A `Vec<f32>` or `Vec<f64>` element crosses the runtime's `i64` slot through an
/// explicit bit reinterpretation: `push`, an indexed write, an index read and a `for`
/// binding each emit exactly one, even when the `Vec` is the unannotated result of a
/// call, and every other element type emits none. Twin of
/// `compiler/tests/test_lower_vec_float_slots.vow`.
#[test]
fn vec_float_elements_cross_the_slot_by_bit_reinterpretation() {
    let module = lowered_fixture();
    for (name, opcode) in [
        ("push_f64", Opcode::F64ToBits),
        ("write_f64", Opcode::F64ToBits),
        ("read_f64", Opcode::BitsToF64),
        ("sum_f64", Opcode::BitsToF64),
        ("push_f32", Opcode::F32ToBits),
        ("write_f32", Opcode::F32ToBits),
        ("read_f32", Opcode::BitsToF32),
        ("sum_f32", Opcode::BitsToF32),
        ("make_f64", Opcode::F64ToBits),
        ("read_unannotated", Opcode::BitsToF64),
        ("sum_call", Opcode::BitsToF64),
    ] {
        assert_eq!(count(&module, name, opcode), 1, "{name}");
    }
    for name in ["read_i64", "push_i64"] {
        for opcode in [
            Opcode::F32ToBits,
            Opcode::F64ToBits,
            Opcode::BitsToF32,
            Opcode::BitsToF64,
        ] {
            assert_eq!(count(&module, name, opcode), 0, "{name} {opcode:?}");
        }
    }
}

/// The slot read is typed as the float the program wrote, and its operand is the raw
/// `__vow_vec_get_val` result.
#[test]
fn float_slot_read_takes_the_extern_result() {
    let module = lowered_fixture();
    let insts: Vec<_> = function(&module, "read_f64")
        .blocks
        .iter()
        .flat_map(|block| &block.insts)
        .collect();
    let result = insts
        .iter()
        .find(|inst| inst.opcode == Opcode::BitsToF64)
        .expect("BitsToF64");
    assert_eq!(result.ty, vow_ir::types::Ty::F64);
    let slot = insts
        .iter()
        .find(|inst| inst.id == result.args[0])
        .expect("slot read");
    assert_eq!(
        slot.data,
        InstData::CallExtern("__vow_vec_get_val".to_string())
    );
}
