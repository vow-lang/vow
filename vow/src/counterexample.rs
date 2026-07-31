// Structured counterexample construction and the call-site index.
//
// When ESBMC reports a contract violation, this module turns the raw
// counterexample (variable assignments, visited blocks, the tripped vow id)
// into a `StructuredCounterexample`: source spans, caller/callee blame, the
// offending call sites, violating arguments, execution path, and branch
// decisions. `build_call_site_index` precomputes, per callee, every call site
// in the module together with the argument spans and any constant-folded
// argument values used to attribute caller blame.
//
// The partial-evaluation of call arguments lives in the sibling `cex_eval`
// module; this module owns the shared `CallSiteInfo` value it indexes over.

use crate::{
    CeBranchDecision, CeCallSite, CePathStep, CeSource, CeViolatingArg, StructuredCounterexample,
};
use vow_verify::{CALLER_PRECONDITION_VOW_ID, Counterexample, UNSUPPORTED_OP_VOW_ID};

fn build_c_to_source_name_map(
    func: &vow_ir::Function,
) -> std::collections::HashMap<String, String> {
    use vow_ir::{InstData, Opcode, Ty};
    let mut map = std::collections::HashMap::new();

    // Map p{cl_idx} → source name (skipping Unit params, matching C emitter logic)
    let mut cl_idx = 0u32;
    for (ir_idx, &ty) in func.params.iter().enumerate() {
        if ty != Ty::Unit {
            if let Some(name) = func.param_names.get(ir_idx) {
                map.insert(format!("p{cl_idx}"), name.clone());
            }
            cl_idx += 1;
        }
    }

    // Map v{inst_id} → source name for GetArg instructions
    let mut arg_var_map: Vec<(u32, u32)> = Vec::new(); // (ir_idx, cl_idx)
    let mut ci = 0u32;
    for (ir_idx, &ty) in func.params.iter().enumerate() {
        if ty != Ty::Unit {
            arg_var_map.push((ir_idx as u32, ci));
            ci += 1;
        }
    }

    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::GetArg
                && let InstData::ArgIndex(idx) = inst.data
                && let Some(name) = func.param_names.get(idx as usize)
            {
                map.insert(format!("v{}", inst.id.0), name.clone());
            }
        }
    }

    for (&inst_id, name) in &func.local_names {
        map.entry(format!("v{inst_id}"))
            .or_insert_with(|| name.clone());
    }

    map
}

fn map_counterexample_values(
    values: &[(String, String)],
    name_map: &std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
    values
        .iter()
        .map(|(c_name, value)| {
            let source_name = name_map
                .get(c_name)
                .cloned()
                .unwrap_or_else(|| format!("_esbmc_{c_name}"));
            (source_name, value.clone())
        })
        .collect()
}

fn call_site_args_match_counterexample(
    callee: &vow_ir::Function,
    entry: &vow_ir::VowEntry,
    mapped_values: &[(String, String)],
    call_site: &CallSiteInfo,
) -> bool {
    let mut matched_binding = false;
    for (binding_name, _) in &entry.bindings {
        let Some(param_idx) = callee.param_names.iter().position(|n| n == binding_name) else {
            return false;
        };
        let Some((_, expected_value)) = mapped_values
            .iter()
            .find(|(name, value)| name == binding_name && !value.is_empty())
        else {
            return false;
        };
        let Some(Some(actual_value)) = call_site.arg_values.get(param_idx) else {
            return false;
        };
        if actual_value != expected_value {
            return false;
        }
        matched_binding = true;
    }
    matched_binding
}

fn callee_precondition_predicate_inst_id(
    callee: &vow_ir::Function,
    entry: &vow_ir::VowEntry,
) -> Option<u32> {
    use vow_ir::{InstData, Opcode};
    for block in &callee.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::VowRequires
                && let InstData::VowId(id) = inst.data
                && id == entry.id
            {
                return inst.args.first().map(|arg| arg.0);
            }
        }
    }
    None
}

fn filter_callee_precondition_call_sites(
    candidates: Vec<CallSiteInfo>,
    callee: &vow_ir::Function,
    entry: &vow_ir::VowEntry,
    mapped_values: &[(String, String)],
) -> Vec<CallSiteInfo> {
    let inst_by_id: std::collections::HashMap<u32, &vow_ir::Inst> = callee
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .map(|inst| (inst.id.0, inst))
        .collect();
    let predicate_inst_id = callee_precondition_predicate_inst_id(callee, entry);
    let exact_matches: Vec<CallSiteInfo> = candidates
        .iter()
        .filter(|cs| {
            call_site_args_match_counterexample(callee, entry, mapped_values, cs)
                || predicate_inst_id.and_then(|id| {
                    crate::cex_eval::eval_callee_bool_for_call_site(id, &inst_by_id, cs)
                }) == Some(false)
        })
        .cloned()
        .collect();
    if exact_matches.is_empty() {
        candidates
    } else {
        exact_matches
    }
}

#[cfg(test)]
fn build_structured_counterexample(
    func: &vow_ir::Function,
    ce: &Counterexample,
    file: &str,
    call_site_index: &std::collections::HashMap<String, Vec<CallSiteInfo>>,
) -> StructuredCounterexample {
    build_structured_counterexample_with_module(func, None, ce, file, call_site_index)
}

pub(crate) fn build_structured_counterexample_with_module(
    func: &vow_ir::Function,
    module: Option<&vow_ir::Module>,
    ce: &Counterexample,
    file: &str,
    call_site_index: &std::collections::HashMap<String, Vec<CallSiteInfo>>,
) -> StructuredCounterexample {
    use vow_ir::InstData;
    let vid = ce.vow_id.unwrap_or(0);
    let resolved_callee_precondition = ce.callee_precondition.and_then(|pre| {
        let module = module?;
        let callee = module.functions.iter().find(|f| f.id.0 == pre.func_id)?;
        let entry = callee.vows.iter().find(|v| v.id.0 == pre.vow_id)?;
        Some((callee, entry))
    });

    // ESBMC tripped a fail-closed assertion that vow-verify's c_emitter inserts
    // for opcodes the verifier model does not handle. The sentinel id is
    // reserved and never matches a user-authored vow, so synthesize a
    // diagnostic that an agent can act on instead of letting the code below
    // fall through to the generic "unmatched id" path.
    let unsupported_op = ce.vow_id == Some(UNSUPPORTED_OP_VOW_ID);
    // A co-emitted callee's `requires` was violated by this caller. New C labels
    // carry the callee function id and callee-local vow id; keep the old
    // sentinel branch for stale cache entries or older verifier output.
    let caller_precondition =
        ce.callee_precondition.is_some() || ce.vow_id == Some(CALLER_PRECONDITION_VOW_ID);
    let vow_func = resolved_callee_precondition
        .map(|(callee, _)| callee)
        .unwrap_or(func);
    let vow_entry = resolved_callee_precondition
        .map(|(_, entry)| entry)
        .or_else(|| {
            ce.vow_id
                .and_then(|id| func.vows.iter().find(|v| v.id.0 == id))
        });
    let violation = if unsupported_op {
        "function uses side-effecting operations not supported for verification".to_string()
    } else if let Some(entry) = vow_entry {
        entry.description.clone()
    } else if caller_precondition {
        "callee precondition violated by the caller".to_string()
    } else {
        ce.description.clone()
    };
    let blame = if caller_precondition {
        "caller".to_string()
    } else {
        vow_entry
            .map(|v| match v.blame {
                vow_diag::Blame::Caller => "caller",
                vow_diag::Blame::Callee => "callee",
                vow_diag::Blame::None => "none",
            })
            .unwrap_or("none")
            .to_string()
    };
    let source = if unsupported_op {
        None
    } else {
        vow_entry.and_then(|entry| {
            find_vow_span(vow_func, entry.id.0).map(|span| {
                let source_file = if !entry.file.is_empty() {
                    entry.file.clone()
                } else if !vow_func.source_file.is_empty() {
                    vow_func.source_file.clone()
                } else {
                    file.to_string()
                };
                CeSource {
                    file: source_file,
                    offset: span.start,
                    length: span.len,
                }
            })
        })
    };
    let value_func = vow_func;
    let name_map = build_c_to_source_name_map(value_func);
    let mapped_values = map_counterexample_values(&ce.values, &name_map);
    let sites_raw: Vec<CallSiteInfo> = if blame == "caller" {
        if let Some((callee, entry)) = resolved_callee_precondition {
            let candidates = call_site_index
                .get(&callee.name)
                .map(|sites| {
                    sites
                        .iter()
                        .filter(|cs| cs.caller_function == func.name)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            filter_callee_precondition_call_sites(candidates, callee, entry, &mapped_values)
        } else {
            call_site_index.get(&func.name).cloned().unwrap_or_default()
        }
    } else {
        vec![]
    };
    let call_sites: Vec<CeCallSite> = sites_raw
        .iter()
        .map(|cs| CeCallSite {
            caller_function: cs.caller_function.clone(),
            file: cs.file.clone(),
            offset: cs.offset,
            length: cs.length,
        })
        .collect();

    // Violating args: for caller-blame, map bindings to param indices and arg spans
    let violating_args = if blame == "caller" {
        if let Some(entry) = vow_entry {
            let mut args = Vec::new();
            for (binding_name, _inst_id) in &entry.bindings {
                if let Some(param_idx) = value_func
                    .param_names
                    .iter()
                    .position(|n| n == binding_name)
                {
                    let mapped_value = mapped_values
                        .iter()
                        .find(|(n, _)| n == binding_name)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    for cs in &sites_raw {
                        if let Some(&(off, len)) = cs.arg_spans.get(param_idx) {
                            let value = if mapped_value.is_empty() {
                                cs.arg_values
                                    .get(param_idx)
                                    .and_then(|v| v.clone())
                                    .unwrap_or_default()
                            } else {
                                mapped_value.clone()
                            };
                            args.push(CeViolatingArg {
                                param: binding_name.clone(),
                                value,
                                arg_offset: off,
                                arg_length: len,
                            });
                        }
                    }
                }
            }
            args
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    // Execution path from block visits
    let visited: std::collections::HashSet<u32> = ce.block_visits.iter().copied().collect();
    let mut execution_path: Vec<CePathStep> = Vec::new();
    for block in &func.blocks {
        if visited.contains(&block.id.0) {
            let span = block
                .insts
                .iter()
                .find(|i| i.origin.start != 0 || i.origin.len != 0)
                .map(|i| i.origin);
            if let Some(s) = span {
                execution_path.push(CePathStep {
                    block_id: block.id.0,
                    offset: s.start,
                    length: s.len,
                });
            } else {
                execution_path.push(CePathStep {
                    block_id: block.id.0,
                    offset: 0,
                    length: 0,
                });
            }
        }
    }

    // Branch decisions
    let mut branch_decisions: Vec<CeBranchDecision> = Vec::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == vow_ir::Opcode::Branch
                && let InstData::BranchTargets {
                    then_block,
                    else_block,
                } = &inst.data
            {
                let then_visited = visited.contains(&then_block.0);
                let else_visited = visited.contains(&else_block.0);
                let taken = match (then_visited, else_visited) {
                    (true, false) => "then",
                    (false, true) => "else",
                    _ => continue,
                };
                branch_decisions.push(CeBranchDecision {
                    condition_offset: inst.origin.start,
                    condition_length: inst.origin.len,
                    taken: taken.to_string(),
                });
            }
        }
    }

    StructuredCounterexample {
        function: func.name.clone(),
        values: mapped_values,
        violation,
        vow_id: vid,
        source,
        blame,
        call_sites,
        violating_args,
        execution_path,
        branch_decisions,
        replay: None,
        replay_reason: None,
        replay_raw_values: ce.values.clone(),
        replay_raw_output: ce.raw_output.clone(),
    }
}

fn find_vow_span(func: &vow_ir::Function, vow_id: u32) -> Option<vow_syntax::span::Span> {
    use vow_ir::{InstData, Opcode};
    for block in &func.blocks {
        for inst in &block.insts {
            if matches!(
                inst.opcode,
                Opcode::VowRequires | Opcode::VowEnsures | Opcode::VowInvariant
            ) && let InstData::VowId(vid) = inst.data
                && vid.0 == vow_id
            {
                return Some(inst.origin);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Call-site index
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct CallSiteInfo {
    pub(crate) caller_function: String,
    pub(crate) file: String,
    pub(crate) offset: u32,
    pub(crate) length: u32,
    pub(crate) arg_spans: Vec<(u32, u32)>,
    pub(crate) arg_values: Vec<Option<String>>,
}

pub(crate) fn build_call_site_index(
    module: &vow_ir::Module,
    file: &str,
) -> std::collections::HashMap<String, Vec<CallSiteInfo>> {
    use vow_ir::{InstData, Opcode};
    let mut index: std::collections::HashMap<String, Vec<CallSiteInfo>> =
        std::collections::HashMap::new();

    let func_by_id: std::collections::HashMap<u32, &str> = module
        .functions
        .iter()
        .map(|f| (f.id.0, f.name.as_str()))
        .collect();

    for func in &module.functions {
        let inst_span: std::collections::HashMap<u32, vow_syntax::span::Span> = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .map(|i| (i.id.0, i.origin))
            .collect();
        let inst_by_id: std::collections::HashMap<u32, &vow_ir::Inst> = func
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .map(|i| (i.id.0, i))
            .collect();

        for block in &func.blocks {
            for inst in &block.insts {
                if inst.opcode == Opcode::Call
                    && let InstData::CallTarget(fid) = &inst.data
                    && let Some(&callee_name) = func_by_id.get(&fid.0)
                {
                    let arg_spans: Vec<(u32, u32)> = inst
                        .args
                        .iter()
                        .map(|a| {
                            inst_span
                                .get(&a.0)
                                .map(|s| (s.start, s.len))
                                .unwrap_or((0, 0))
                        })
                        .collect();
                    let arg_values: Vec<Option<String>> = inst
                        .args
                        .iter()
                        .map(|a| {
                            crate::cex_eval::eval_const_i64_for_counterexample(a.0, &inst_by_id)
                                .map(|v| v.to_string())
                                .or_else(|| {
                                    inst_by_id.get(&a.0).and_then(|inst| {
                                        crate::cex_eval::inst_constant_value_for_counterexample(
                                            inst,
                                        )
                                    })
                                })
                        })
                        .collect();
                    index
                        .entry(callee_name.to_string())
                        .or_default()
                        .push(CallSiteInfo {
                            caller_function: func.name.clone(),
                            file: file.to_string(),
                            offset: inst.origin.start,
                            length: inst.origin.len,
                            arg_spans,
                            arg_values,
                        });
                }
            }
        }
    }

    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_c_to_source_name_map_basic() {
        use vow_ir::{
            BasicBlock, BlockId, FuncId, Inst, InstData, InstId, Opcode, RegionId, RegionSummary,
            Ty,
        };
        use vow_syntax::span::Span;
        let func = vow_ir::Function {
            id: FuncId(0),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["x".to_string(), "y".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(0),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let map = build_c_to_source_name_map(&func);
        assert_eq!(map.get("p0"), Some(&"x".to_string()));
        assert_eq!(map.get("p1"), Some(&"y".to_string()));
        assert_eq!(map.get("v0"), Some(&"x".to_string()));
        assert_eq!(map.get("v1"), Some(&"y".to_string()));
    }

    #[test]
    fn build_c_to_source_name_map_skips_unit_params() {
        use vow_ir::{
            BasicBlock, BlockId, FuncId, Inst, InstData, InstId, Opcode, RegionId, RegionSummary,
            Ty,
        };
        use vow_syntax::span::Span;
        let func = vow_ir::Function {
            id: FuncId(0),
            name: "f".to_string(),
            params: vec![Ty::Unit, Ty::I64, Ty::I64],
            param_names: vec!["_u".to_string(), "a".to_string(), "b".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(2),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let map = build_c_to_source_name_map(&func);
        // p0 maps to "a" (first non-Unit), p1 maps to "b"
        assert_eq!(map.get("p0"), Some(&"a".to_string()));
        assert_eq!(map.get("p1"), Some(&"b".to_string()));
        // v0 → GetArg(1) → "a", v1 → GetArg(2) → "b"
        assert_eq!(map.get("v0"), Some(&"a".to_string()));
        assert_eq!(map.get("v1"), Some(&"b".to_string()));
    }

    #[test]
    fn map_counterexample_values_applies_mapping() {
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("p0".to_string(), "x".to_string());
        name_map.insert("p1".to_string(), "y".to_string());
        name_map.insert("v0".to_string(), "x".to_string());
        name_map.insert("v1".to_string(), "y".to_string());

        let values = vec![
            ("v1".to_string(), "0".to_string()),
            ("v3".to_string(), "0".to_string()),
        ];
        let mapped = map_counterexample_values(&values, &name_map);
        assert_eq!(mapped[0], ("y".to_string(), "0".to_string()));
        assert_eq!(mapped[1], ("_esbmc_v3".to_string(), "0".to_string()));
    }

    #[test]
    fn build_c_to_source_name_map_empty_param_names() {
        use vow_ir::{BasicBlock, BlockId, FuncId, RegionSummary, Ty};
        let func = vow_ir::Function {
            id: FuncId(0),
            name: "f".to_string(),
            params: vec![Ty::I64],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let map = build_c_to_source_name_map(&func);
        assert!(map.is_empty());
    }

    #[test]
    fn build_call_site_index_finds_internal_calls() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let module = Module {
            name: "test".to_string(),
            functions: vec![
                Function {
                    id: FuncId(0),
                    name: "callee".to_string(),
                    params: vec![Ty::I64],
                    param_names: vec!["x".to_string()],
                    return_ty: Ty::I64,
                    effects: vec![],
                    vows: vec![],
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        insts: vec![
                            Inst {
                                id: InstId(0),
                                opcode: Opcode::GetArg,
                                ty: Ty::I64,
                                args: vec![],
                                data: InstData::ArgIndex(0),
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(1),
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(0)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                    summary: RegionSummary::default(),
                    source_file: String::new(),
                },
                Function {
                    id: FuncId(1),
                    name: "caller_a".to_string(),
                    params: vec![],
                    param_names: vec![],
                    return_ty: Ty::I64,
                    effects: vec![],
                    vows: vec![],
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        insts: vec![
                            Inst {
                                id: InstId(0),
                                opcode: Opcode::ConstI64,
                                ty: Ty::I64,
                                args: vec![],
                                data: InstData::ConstI64(5),
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(1),
                                opcode: Opcode::Call,
                                ty: Ty::I64,
                                args: vec![InstId(0)],
                                data: InstData::CallTarget(FuncId(0)),
                                origin: Span::new(100, 10),
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(2),
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(1)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                    summary: RegionSummary::default(),
                    source_file: String::new(),
                },
                Function {
                    id: FuncId(2),
                    name: "caller_b".to_string(),
                    params: vec![],
                    param_names: vec![],
                    return_ty: Ty::I64,
                    effects: vec![],
                    vows: vec![],
                    blocks: vec![BasicBlock {
                        id: BlockId(0),
                        insts: vec![
                            Inst {
                                id: InstId(0),
                                opcode: Opcode::ConstI64,
                                ty: Ty::I64,
                                args: vec![],
                                data: InstData::ConstI64(10),
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(1),
                                opcode: Opcode::Call,
                                ty: Ty::I64,
                                args: vec![InstId(0)],
                                data: InstData::CallTarget(FuncId(0)),
                                origin: Span::new(200, 15),
                                region: RegionId::Root,
                            },
                            Inst {
                                id: InstId(2),
                                opcode: Opcode::Return,
                                ty: Ty::Unit,
                                args: vec![InstId(1)],
                                data: InstData::None,
                                origin: Span::new(0, 0),
                                region: RegionId::Root,
                            },
                        ],
                    }],
                    local_names: std::collections::HashMap::new(),
                    summary: RegionSummary::default(),
                    source_file: String::new(),
                },
            ],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };

        let index = build_call_site_index(&module, "test.vow");
        let callee_sites = index.get("callee").expect("callee should have call sites");
        assert_eq!(callee_sites.len(), 2);
        assert_eq!(callee_sites[0].caller_function, "caller_a");
        assert_eq!(callee_sites[0].offset, 100);
        assert_eq!(callee_sites[0].length, 10);
        assert_eq!(callee_sites[1].caller_function, "caller_b");
        assert_eq!(callee_sites[1].offset, 200);
        assert_eq!(callee_sites[1].length, 15);
        assert!(!index.contains_key("caller_a"));
    }

    #[test]
    fn structured_counterexample_includes_blame_caller() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let func = Function {
            id: FuncId(0),
            name: "safe_div".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["x".to_string(), "y".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "y != 0".to_string(),
                blame: vow_diag::Blame::Caller,
                bindings: vec![],
                file: "test.vow".to_string(),
                offset: 42,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(0),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(0, 0),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(1)],
                        data: InstData::VowId(VowId(0)),
                        origin: Span::new(42, 6),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };

        let ce = vow_verify::Counterexample {
            description: "y != 0".to_string(),
            vow_id: Some(0),
            callee_precondition: None,
            values: vec![
                ("p0".to_string(), "10".to_string()),
                ("p1".to_string(), "0".to_string()),
            ],
            block_visits: vec![0],
            raw_output: String::new(),
        };

        let mut call_sites = std::collections::HashMap::new();
        call_sites.insert(
            "safe_div".to_string(),
            vec![CallSiteInfo {
                caller_function: "main".to_string(),
                file: "test.vow".to_string(),
                offset: 120,
                length: 18,
                arg_spans: vec![],
                arg_values: vec![],
            }],
        );

        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_sites);
        assert_eq!(sce.blame, "caller");
        assert_eq!(sce.call_sites.len(), 1);
        assert_eq!(sce.call_sites[0].caller_function, "main");
        assert_eq!(sce.call_sites[0].offset, 120);
    }

    #[test]
    fn structured_counterexample_unsupported_op_sentinel() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let func = Function {
            id: FuncId(0),
            name: "uses_unsupported".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "some real vow".to_string(),
                blame: vow_diag::Blame::Callee,
                bindings: vec![],
                file: "test.vow".to_string(),
                offset: 0,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![Inst {
                    id: InstId(0),
                    opcode: Opcode::Return,
                    ty: Ty::Unit,
                    args: vec![],
                    data: InstData::None,
                    origin: Span::new(0, 0),
                    region: RegionId::Root,
                }],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };

        let ce = vow_verify::Counterexample {
            description: "[Counterexample]".to_string(),
            vow_id: Some(UNSUPPORTED_OP_VOW_ID),
            callee_precondition: None,
            values: vec![],
            block_visits: vec![],
            raw_output: String::new(),
        };

        let call_sites = std::collections::HashMap::new();
        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_sites);

        assert_eq!(sce.vow_id, UNSUPPORTED_OP_VOW_ID);
        assert!(
            sce.violation.contains("not supported for verification"),
            "expected unsupported-op message, got {:?}",
            sce.violation
        );
        assert_ne!(
            sce.violation, "[Counterexample]",
            "must not fall through to raw ESBMC line"
        );
        assert_eq!(sce.blame, "none");
        assert!(sce.source.is_none());
    }

    #[test]
    fn structured_counterexample_callee_blame_no_call_sites() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let func = Function {
            id: FuncId(0),
            name: "buggy".to_string(),
            params: vec![Ty::I64],
            param_names: vec!["x".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "result == x + x".to_string(),
                blame: vow_diag::Blame::Callee,
                bindings: vec![],
                file: "test.vow".to_string(),
                offset: 30,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![Inst {
                    id: InstId(0),
                    opcode: Opcode::VowEnsures,
                    ty: Ty::Unit,
                    args: vec![],
                    data: InstData::VowId(VowId(0)),
                    origin: Span::new(30, 20),
                    region: RegionId::Root,
                }],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };

        let ce = vow_verify::Counterexample {
            description: "result == x + x".to_string(),
            vow_id: Some(0),
            callee_precondition: None,
            values: vec![("p0".to_string(), "5".to_string())],
            block_visits: vec![0],
            raw_output: String::new(),
        };

        let mut call_sites = std::collections::HashMap::new();
        call_sites.insert(
            "buggy".to_string(),
            vec![CallSiteInfo {
                caller_function: "main".to_string(),
                file: "test.vow".to_string(),
                offset: 100,
                length: 10,
                arg_spans: vec![],
                arg_values: vec![],
            }],
        );

        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_sites);
        assert_eq!(sce.blame, "callee");
        assert!(
            sce.call_sites.is_empty(),
            "callee blame should have no call_sites"
        );
    }

    #[test]
    fn call_site_index_captures_arg_spans() {
        use vow_ir::*;
        use vow_syntax::span::Span;
        let callee = Function {
            id: FuncId(0),
            name: "callee".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["a".to_string(), "b".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(0),
                        origin: Span::new(10, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::None,
                        origin: Span::new(12, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let caller = Function {
            id: FuncId(1),
            name: "caller".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(10),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(5),
                        origin: Span::new(100, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(11),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(0),
                        origin: Span::new(103, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(12),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(10), InstId(11)],
                        data: InstData::CallTarget(FuncId(0)),
                        origin: Span::new(95, 12),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(13),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(12)],
                        data: InstData::None,
                        origin: Span::new(110, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let module = Module {
            name: "test".to_string(),
            functions: vec![callee, caller],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let index = build_call_site_index(&module, "test.vow");
        let sites = index.get("callee").expect("callee should have call sites");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].arg_spans.len(), 2);
        assert_eq!(sites[0].arg_spans[0], (100, 1));
        assert_eq!(sites[0].arg_spans[1], (103, 1));
    }

    #[test]
    fn violating_args_populated_for_caller_blame() {
        use vow_ir::*;
        use vow_syntax::span::Span;
        let func = Function {
            id: FuncId(0),
            name: "divide".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["x".to_string(), "y".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "y != 0".to_string(),
                blame: vow_diag::Blame::Caller,
                bindings: vec![("y".to_string(), InstId(1))],
                file: "test.vow".to_string(),
                offset: 20,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(0),
                        origin: Span::new(10, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(15, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::None,
                        origin: Span::new(20, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let ce = vow_verify::Counterexample {
            description: "test".to_string(),
            vow_id: Some(0),
            callee_precondition: None,
            values: vec![
                ("p0".to_string(), "10".to_string()),
                ("p1".to_string(), "0".to_string()),
            ],
            block_visits: vec![0],
            raw_output: String::new(),
        };
        let mut call_site_index = std::collections::HashMap::new();
        call_site_index.insert(
            "divide".to_string(),
            vec![CallSiteInfo {
                caller_function: "main".to_string(),
                file: "test.vow".to_string(),
                offset: 50,
                length: 15,
                arg_spans: vec![(55, 2), (59, 1)],
                arg_values: vec![Some("10".to_string()), Some("0".to_string())],
            }],
        );
        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_site_index);
        assert_eq!(sce.blame, "caller");
        assert_eq!(sce.violating_args.len(), 1);
        assert_eq!(sce.violating_args[0].param, "y");
        assert_eq!(sce.violating_args[0].value, "0");
        assert_eq!(sce.violating_args[0].arg_offset, 59);
        assert_eq!(sce.violating_args[0].arg_length, 1);
    }

    #[test]
    fn callee_precondition_counterexample_resolves_callee_contract_and_current_call_site() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let callee = Function {
            id: FuncId(7),
            name: "g".to_string(),
            params: vec![Ty::I64, Ty::I64],
            param_names: vec!["x".to_string(), "y".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![
                VowEntry {
                    id: VowId(0),
                    description: "x > 0".to_string(),
                    blame: vow_diag::Blame::Caller,
                    bindings: vec![("x".to_string(), InstId(0))],
                    file: "test.vow".to_string(),
                    offset: 20,
                },
                VowEntry {
                    id: VowId(1),
                    description: "requires y != 0".to_string(),
                    blame: vow_diag::Blame::Caller,
                    bindings: vec![("y".to_string(), InstId(1))],
                    file: "test.vow".to_string(),
                    offset: 30,
                },
            ],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(0),
                        origin: Span::new(10, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(1),
                        origin: Span::new(12, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(1)],
                        data: InstData::VowId(VowId(1)),
                        origin: Span::new(30, 15),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::None,
                        origin: Span::new(50, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: "test.vow".to_string(),
        };

        let target = Function {
            id: FuncId(1),
            name: "f".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(10),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(1),
                        origin: Span::new(100, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(11),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(0),
                        origin: Span::new(103, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(12),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(10), InstId(11)],
                        data: InstData::CallTarget(FuncId(7)),
                        origin: Span::new(95, 12),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(13),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(12)],
                        data: InstData::None,
                        origin: Span::new(110, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: "test.vow".to_string(),
        };

        let other_caller = Function {
            id: FuncId(2),
            name: "h".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(20),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(1),
                        origin: Span::new(200, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(21),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(0),
                        origin: Span::new(203, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(22),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(20), InstId(21)],
                        data: InstData::CallTarget(FuncId(7)),
                        origin: Span::new(195, 12),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(23),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(22)],
                        data: InstData::None,
                        origin: Span::new(210, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: "test.vow".to_string(),
        };

        let module = Module {
            name: "test".to_string(),
            functions: vec![callee, target.clone(), other_caller],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let call_site_index = build_call_site_index(&module, "test.vow");
        let ce = vow_verify::Counterexample {
            description: "raw".to_string(),
            vow_id: Some(1),
            callee_precondition: Some(vow_verify::CalleePrecondition {
                func_id: 7,
                vow_id: 1,
            }),
            values: vec![
                ("p0".to_string(), "1".to_string()),
                ("p1".to_string(), "0".to_string()),
            ],
            block_visits: vec![0],
            raw_output: String::new(),
        };

        let sce = build_structured_counterexample_with_module(
            &target,
            Some(&module),
            &ce,
            "test.vow",
            &call_site_index,
        );

        assert_eq!(sce.function, "f");
        assert_eq!(sce.blame, "caller");
        assert_eq!(sce.vow_id, 1);
        assert_eq!(sce.violation, "requires y != 0");
        assert_eq!(sce.source.as_ref().map(|s| s.offset), Some(30));
        assert_eq!(sce.source.as_ref().map(|s| s.length), Some(15));
        assert_eq!(sce.call_sites.len(), 1);
        assert_eq!(sce.call_sites[0].caller_function, "f");
        assert_eq!(sce.call_sites[0].offset, 95);
        assert_eq!(sce.violating_args.len(), 1);
        assert_eq!(sce.violating_args[0].param, "y");
        assert_eq!(sce.violating_args[0].value, "0");
        assert_eq!(sce.violating_args[0].arg_offset, 103);
        assert_eq!(sce.violating_args[0].arg_length, 1);
    }

    #[test]
    fn callee_precondition_counterexample_filters_same_caller_calls_by_arg_value() {
        use vow_ir::*;
        use vow_syntax::span::Span;

        let callee = Function {
            id: FuncId(7),
            name: "g".to_string(),
            params: vec![Ty::I64],
            param_names: vec!["x".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "requires x > 0".to_string(),
                blame: vow_diag::Blame::Caller,
                bindings: vec![("x".to_string(), InstId(0))],
                file: "test.vow".to_string(),
                offset: 20,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(0),
                        opcode: Opcode::GetArg,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ArgIndex(0),
                        origin: Span::new(10, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(1),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(0),
                        origin: Span::new(14, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(2),
                        opcode: Opcode::Gt,
                        ty: Ty::Bool,
                        args: vec![InstId(0), InstId(1)],
                        data: InstData::None,
                        origin: Span::new(10, 5),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(3),
                        opcode: Opcode::VowRequires,
                        ty: Ty::Unit,
                        args: vec![InstId(2)],
                        data: InstData::VowId(VowId(0)),
                        origin: Span::new(20, 15),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(4),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(0)],
                        data: InstData::None,
                        origin: Span::new(40, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: "test.vow".to_string(),
        };

        let target = Function {
            id: FuncId(1),
            name: "f".to_string(),
            params: vec![],
            param_names: vec![],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                insts: vec![
                    Inst {
                        id: InstId(10),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(1),
                        origin: Span::new(95, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(11),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(10)],
                        data: InstData::CallTarget(FuncId(7)),
                        origin: Span::new(90, 4),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(12),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(0),
                        origin: Span::new(125, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(13),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(5),
                        origin: Span::new(129, 1),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(14),
                        opcode: Opcode::WrappingSub,
                        ty: Ty::I64,
                        args: vec![InstId(12), InstId(13)],
                        data: InstData::None,
                        origin: Span::new(125, 5),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(15),
                        opcode: Opcode::Call,
                        ty: Ty::I64,
                        args: vec![InstId(14)],
                        data: InstData::CallTarget(FuncId(7)),
                        origin: Span::new(120, 10),
                        region: RegionId::Root,
                    },
                    Inst {
                        id: InstId(16),
                        opcode: Opcode::Return,
                        ty: Ty::Unit,
                        args: vec![InstId(15)],
                        data: InstData::None,
                        origin: Span::new(140, 1),
                        region: RegionId::Root,
                    },
                ],
            }],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: "test.vow".to_string(),
        };

        let module = Module {
            name: "test".to_string(),
            functions: vec![callee, target.clone()],
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        };
        let call_site_index = build_call_site_index(&module, "test.vow");
        let ce = vow_verify::Counterexample {
            description: "raw".to_string(),
            vow_id: Some(0),
            callee_precondition: Some(vow_verify::CalleePrecondition {
                func_id: 7,
                vow_id: 0,
            }),
            values: vec![],
            block_visits: vec![0],
            raw_output: String::new(),
        };

        let sce = build_structured_counterexample_with_module(
            &target,
            Some(&module),
            &ce,
            "test.vow",
            &call_site_index,
        );

        assert_eq!(sce.call_sites.len(), 1);
        assert_eq!(sce.call_sites[0].caller_function, "f");
        assert_eq!(sce.call_sites[0].offset, 120);
        assert_eq!(sce.violating_args.len(), 1);
        assert_eq!(sce.violating_args[0].param, "x");
        assert_eq!(sce.violating_args[0].value, "-5");
        assert_eq!(sce.violating_args[0].arg_offset, 125);
        assert_eq!(sce.violating_args[0].arg_length, 5);
    }

    #[test]
    fn execution_path_and_branch_decisions_from_block_visits() {
        use vow_ir::*;
        use vow_syntax::span::Span;
        let func = Function {
            id: FuncId(0),
            name: "branchy".to_string(),
            params: vec![Ty::Bool],
            param_names: vec!["cond".to_string()],
            return_ty: Ty::I64,
            effects: vec![],
            vows: vec![VowEntry {
                id: VowId(0),
                description: "result >= 0".to_string(),
                blame: vow_diag::Blame::Callee,
                bindings: vec![],
                file: "test.vow".to_string(),
                offset: 0,
            }],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    insts: vec![
                        Inst {
                            id: InstId(0),
                            opcode: Opcode::GetArg,
                            ty: Ty::Bool,
                            args: vec![],
                            data: InstData::ArgIndex(0),
                            origin: Span::new(10, 4),
                            region: RegionId::Root,
                        },
                        Inst {
                            id: InstId(1),
                            opcode: Opcode::Branch,
                            ty: Ty::Unit,
                            args: vec![InstId(0)],
                            data: InstData::BranchTargets {
                                then_block: BlockId(1),
                                else_block: BlockId(2),
                            },
                            origin: Span::new(20, 8),
                            region: RegionId::Root,
                        },
                    ],
                },
                BasicBlock {
                    id: BlockId(1),
                    insts: vec![Inst {
                        id: InstId(2),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(1),
                        origin: Span::new(30, 1),
                        region: RegionId::Root,
                    }],
                },
                BasicBlock {
                    id: BlockId(2),
                    insts: vec![Inst {
                        id: InstId(3),
                        opcode: Opcode::ConstI64,
                        ty: Ty::I64,
                        args: vec![],
                        data: InstData::ConstI64(-1),
                        origin: Span::new(40, 2),
                        region: RegionId::Root,
                    }],
                },
            ],
            local_names: std::collections::HashMap::new(),
            summary: RegionSummary::default(),
            source_file: String::new(),
        };
        let ce = vow_verify::Counterexample {
            description: "test".to_string(),
            vow_id: Some(0),
            callee_precondition: None,
            values: vec![("p0".to_string(), "0".to_string())],
            block_visits: vec![0, 2],
            raw_output: String::new(),
        };
        let call_site_index = std::collections::HashMap::new();
        let sce = build_structured_counterexample(&func, &ce, "test.vow", &call_site_index);

        assert_eq!(sce.execution_path.len(), 2);
        assert_eq!(sce.execution_path[0].block_id, 0);
        assert_eq!(sce.execution_path[0].offset, 10);
        assert_eq!(sce.execution_path[1].block_id, 2);
        assert_eq!(sce.execution_path[1].offset, 40);

        assert_eq!(sce.branch_decisions.len(), 1);
        assert_eq!(sce.branch_decisions[0].taken, "else");
        assert_eq!(sce.branch_decisions[0].condition_offset, 20);
        assert_eq!(sce.branch_decisions[0].condition_length, 8);
    }
}
