//! Issue #1933: a value stored into a parameter container through an
//! alias-returning call must be allocated in the caller's region, not in a
//! block-local region that is freed while the container still points at it.

use vow_ir::lower::{
    PatternAggregateMap, PayloadScalarMap, StringExprSet, lower_module_with_pattern_aggregates,
};
use vow_ir::region::infer_regions;
use vow_ir::types::{HiddenRegionIdx, Opcode, RegionId};

fn allocation_regions(source: &str, func: &str) -> Vec<RegionId> {
    let (ast, diagnostics) = vow_syntax::parser::parse_module(source, "t.vow");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let item_files = vec!["t.vow".to_string(); ast.items.len()];
    let mut module = lower_module_with_pattern_aggregates(
        &ast,
        &item_files,
        &StringExprSet::new(),
        PatternAggregateMap::new(),
        PayloadScalarMap::new(),
    );
    infer_regions(&mut module);
    let f = module.functions.iter().find(|f| f.name == func).unwrap();
    f.blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| matches!(i.opcode, Opcode::Call | Opcode::RegionAlloc))
        .map(|i| i.region)
        .collect()
}

fn assert_fill_allocates_in_caller_region(source: &str) {
    let regions = allocation_regions(source, "fill");
    assert!(
        regions.iter().all(|r| !matches!(r, RegionId::Block(_))),
        "fill allocates in a block-local region: {regions:?}"
    );
    assert!(regions.contains(&RegionId::Caller(HiddenRegionIdx(0))));
}

#[test]
fn alias_of_call_result_stored_into_param_uses_caller_region() {
    assert_fill_allocates_in_caller_region(include_str!(
        "../../tests/run/region_pass_through_store_into_param.vow"
    ));
}

#[test]
fn alias_of_any_call_result_stored_into_param_uses_caller_region() {
    assert_fill_allocates_in_caller_region(include_str!(
        "../../tests/run/region_pass_through_store_alias_of_any.vow"
    ));
}

#[test]
fn chained_alias_calls_stored_into_param_use_caller_region() {
    assert_fill_allocates_in_caller_region(include_str!(
        "../../tests/run/region_pass_through_store_chained.vow"
    ));
}

#[test]
fn fresh_store_through_forwarding_call_stays_in_caller_region() {
    let source = "\
module FreshForward

struct Ce {
    names: Vec<String>,
}

fn inner(ces: Vec<Ce>) {
    ces.push(Ce { names: Vec::new() });
}

fn outer(ces: Vec<Ce>) {
    inner(ces);
}
";
    let inner = allocation_regions(source, "inner");
    assert!(inner.contains(&RegionId::Caller(HiddenRegionIdx(0))));
    assert!(inner.iter().all(|r| !matches!(r, RegionId::Block(_))));
    let outer = allocation_regions(source, "outer");
    assert!(outer.iter().all(|r| !matches!(r, RegionId::Block(_))));
}
