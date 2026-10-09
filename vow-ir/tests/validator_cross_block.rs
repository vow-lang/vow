use vow_ir::lower::lower_module_with_pattern_aggregates;
use vow_ir::lower::{PatternAggregateMap, PayloadScalarMap, StringExprSet};
use vow_ir::validator::{ValidationError, validate_function};

const SOURCE: &str = "\
module LoopValidate

fn sum_to(n: i64) -> i64 {
    let mut i: i64 = 0;
    let mut acc: i64 = 0;
    while i < n {
        if i > 5 {
            acc = acc + i;
        } else {
            acc = acc + 1;
        }
        i = i + 1;
    }
    return acc;
}
";

#[test]
fn lowered_loop_has_no_undefined_inst_refs() {
    let (ast, diagnostics) = vow_syntax::parser::parse_module(SOURCE, "loop.vow");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let item_files = vec!["loop.vow".to_string(); ast.items.len()];
    let module = lower_module_with_pattern_aggregates(
        &ast,
        &item_files,
        &StringExprSet::new(),
        PatternAggregateMap::new(),
        PayloadScalarMap::new(),
    );
    assert!(module.functions.iter().any(|f| f.blocks.len() > 1));
    for func in &module.functions {
        let undefined: Vec<_> = validate_function(func)
            .errors
            .into_iter()
            .filter(|e| matches!(e, ValidationError::UndefinedInstRef { .. }))
            .collect();
        assert!(undefined.is_empty(), "{}: {undefined:?}", func.name);
    }
}
