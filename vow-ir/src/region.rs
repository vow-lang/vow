//! Region inference pass (arena-per-scope, Phase 3).
//!
//! Implements `docs/design/arena_memory.md` §4. Runs after type/effect/linear
//! checking and before lowering / codegen. Populates `Inst.region` and
//! `Function.summary` (both already plumbed in Phase 2). Lowering still
//! ignores both fields — Phase 4 will switch lowering to consume them.
//!
//! ## Algorithm summary
//!
//! 1. Build the call graph from `Opcode::Call` + `InstData::CallTarget`.
//!    `CallExtern` callees default to `ConstantGlobal` summaries.
//! 2. Compute SCCs (Tarjan) and iterate them in reverse-topo order so callees
//!    are summarised before callers — except inside an SCC, which uses a
//!    monotone fixed-point seeded at the internal `Uninit` ⊥ element
//!    (spec §4.3 step 1).
//! 3. Per function, walk every block / inst, collecting `must_outlive(I)`
//!    sets. Heap-producing instructions (`Opcode::RegionAlloc` plus
//!    recognised fresh runtime allocation extern calls) get their `region`
//!    field set from the LUB.
//! 4. Per function, derive a `RegionSummary` from the `Return` arg's
//!    `must_outlive` set + per-call store-effect contributions.
//! 5. After fixed point, every still-`Uninit` summary is resolved by
//!    inspecting the return-expression structure (spec §4.3 step 5).
//! 6. Canonicalise `AliasOfAny` ascending+dedup; `store_effects` ascending by
//!    target index. Both compilers MUST agree on canonical form.
//!
//! ## Invariants
//!
//! - `Uninit` is internal-only: it MUST NOT be visible in `Function.summary`
//!   after `infer_regions` returns, even on the error path (`RegionConflict`
//!   completes iteration before returning).
//! - The `RegionConstraint` enum (`crate::types`) literally has no `Uninit`
//!   variant, so a compiler-enforced structural barrier holds at the boundary.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use vow_diag::{Blame, Diagnostic, ErrorCode, Severity, SourceLocation};
use vow_syntax::span::Span;

use crate::types::{
    BlockId, Function, HiddenRegionIdx, Inst, InstData, InstId, Module, Opcode, RegionConstraint,
    RegionId, RegionSummary, RegionVar, StoreEffect, Ty,
};

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

/// Run region inference over `module`. Mutates each `Function.summary` and
/// each heap-producing `Inst.region`. Diagnostics are pushed onto
/// `module.warnings` (existing channel — same one `lower_module` uses).
///
/// Each emitted diagnostic is labelled with the source file recorded on
/// the analysing `Function.source_file` (set by `lower_module` from
/// `merge_modules`'s per-item path) — required for correct labels under
/// multi-module compilation (#254).
///
/// On `RegionConflict`, iteration completes anyway so internal `Uninit`
/// state never leaks into `Function.summary` (spec §4.3).
pub fn infer_regions(module: &mut Module) {
    let n_funcs = module.functions.len();
    if n_funcs == 0 {
        return;
    }

    // Build call graph (FuncId.0 → list of FuncId.0 of called functions).
    let call_graph = build_call_graph(module);
    let sccs = tarjan_sccs(&call_graph);

    // Collect emitted diagnostics here; merge into module.warnings at end.
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Per-function inferred state, keyed by FuncId.0.
    let mut summaries: Vec<InternalSummary> = (0..n_funcs)
        .map(|i| InternalSummary::seed(module.functions[i].params.len()))
        .collect();

    // Region maps for the final inst.region populate pass, keyed by FuncId.0
    // → BTreeMap<InstId, RegionId>. BTreeMap is used over HashMap for
    // deterministic iteration order.
    let mut region_maps: Vec<BTreeMap<InstId, RegionId>> = vec![BTreeMap::new(); n_funcs];

    // SCCs come out in reverse topological order from Tarjan; that's the
    // order we want — callees before callers.
    for scc in &sccs {
        // Lattice height per function: |params| + 2 (Uninit → AliasOf →
        // AliasOfAny → FreshInCaller). store_effects bound: |params|. Total
        // bound for the SCC: sum * 2 to give monotone slack.
        let mut bound: usize = 0;
        for &fidx in scc {
            let nparams = module.functions[fidx as usize].params.len();
            bound = bound.saturating_add(nparams.saturating_add(2).saturating_mul(2));
        }
        bound = bound.max(8); // ensure SCCs of size 1 still get a few rounds

        // Diagnostics-per-iteration buffer. The SCC fixed-point loop calls
        // `analyze_function` once per round; `check_store_conflict` pushes
        // a Diagnostic per violating call site each round, so a real
        // conflict would be reported `iters` times if we accumulated
        // straight into `diagnostics`. We keep only the *last* iteration's
        // emissions — at convergence the published summaries are final, so
        // the conflict set is canonical. Intermediate emissions can be
        // stale (a callee summary upgrades from Uninit → AliasOf mid-loop
        // and changes which arg pairs are checked).
        let mut iters = 0;
        loop {
            iters += 1;
            let mut iter_diagnostics: Vec<Diagnostic> = Vec::new();
            let mut changed = false;
            for &fidx in scc {
                let func = &module.functions[fidx as usize];
                let new_summary = analyze_function(
                    func,
                    &summaries,
                    &mut region_maps[fidx as usize],
                    &mut iter_diagnostics,
                );
                if !summaries[fidx as usize].equals(&new_summary) {
                    summaries[fidx as usize] = new_summary;
                    changed = true;
                }
            }
            if !changed {
                // Convergence iteration's diagnostics are canonical.
                diagnostics.extend(iter_diagnostics);
                break;
            }
            if iters > bound {
                // Should never happen — the lattice is finite and joins are
                // monotone. Emit a structured ICE diagnostic; do NOT panic
                // (CLAUDE.md production-quality rule). Preserve the partial
                // last-iteration diagnostics alongside the ICE so the user
                // sees what was emitted before we gave up.
                diagnostics.extend(iter_diagnostics);
                diagnostics.push(internal_compiler_error(
                    "region inference SCC exceeded monotone iteration bound",
                ));
                break;
            }
        }

        // §4.3 step 5: resolve any function in this SCC still at Uninit.
        for &fidx in scc {
            if summaries[fidx as usize].return_region.is_uninit() {
                let resolved = resolve_uninit_return(&module.functions[fidx as usize]);
                summaries[fidx as usize].return_region = InternalReturnRegion::Published(resolved);
            }
        }
    }

    // Commit summaries to Function.summary. Conversion structurally drops
    // any internal Uninit (it's a separate enum), satisfying the no-leak
    // invariant.
    for (fidx, summary) in summaries.iter().enumerate() {
        let canonical = summary.to_published(module.functions[fidx].params.len());
        debug_assert!(
            !matches!(canonical.return_region, RegionConstraint::AliasOfAny(ref v) if v.is_empty()),
            "AliasOfAny must never be empty after canonicalisation"
        );
        module.functions[fidx].summary = canonical;
    }

    // Commit per-inst regions.
    for (fidx, region_map) in region_maps.iter().enumerate() {
        for block in &mut module.functions[fidx].blocks {
            for inst in &mut block.insts {
                if let Some(&rid) = region_map.get(&inst.id) {
                    inst.region = rid;
                }
            }
        }
    }

    check_linear_regions(module, &mut diagnostics);

    module.warnings.extend(diagnostics);
}

fn check_linear_regions(module: &Module, diagnostics: &mut Vec<Diagnostic>) {
    for func in &module.functions {
        check_function_linear_regions(func, diagnostics);
    }
}

fn check_function_linear_regions(func: &Function, diagnostics: &mut Vec<Diagnostic>) {
    let mut inst_lookup: BTreeMap<InstId, (BlockId, &Inst)> = BTreeMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            inst_lookup.insert(inst.id, (block.id, inst));
        }
    }
    let predecessors = predecessor_map(func);

    let mut block_in: BTreeMap<BlockId, BTreeSet<InstId>> = BTreeMap::new();
    let mut block_out: BTreeMap<BlockId, BTreeSet<InstId>> = BTreeMap::new();
    for block in &func.blocks {
        block_in.insert(block.id, BTreeSet::new());
        block_out.insert(block.id, BTreeSet::new());
    }

    let mut changed = true;
    let mut iters = 0usize;
    let bound = func
        .blocks
        .len()
        .saturating_mul(func.blocks.len().max(1) + 1)
        .max(8);
    while changed && iters <= bound {
        iters += 1;
        changed = false;
        for block in &func.blocks {
            let mut incoming = BTreeSet::new();
            if let Some(preds) = predecessors.get(&block.id) {
                for pred in preds {
                    if let Some(out) = block_out.get(pred) {
                        incoming.extend(out.iter().copied());
                    }
                }
            }
            let previous_in = block_in.insert(block.id, incoming.clone());
            if previous_in.as_ref() != Some(&incoming) {
                changed = true;
            }

            let out = transfer_linear_block(&incoming, block, &inst_lookup);
            if block_out.insert(block.id, out.clone()) != Some(out) {
                changed = true;
            }
        }
    }

    let mut emitted: BTreeSet<InstId> = BTreeSet::new();
    for block in &func.blocks {
        let incoming = block_in.get(&block.id).cloned().unwrap_or_default();
        let mut live = incoming;
        for inst in &block.insts {
            match inst.opcode {
                Opcode::Return => {
                    if let Some(&arg) = inst.args.first() {
                        remove_linear_origins(&mut live, arg, &inst_lookup);
                    }
                    emit_live_linear_errors(func, &live, &inst_lookup, &mut emitted, diagnostics);
                    live.clear();
                }
                Opcode::Unreachable => {
                    live.clear();
                }
                _ => apply_linear_transfer(inst, &mut live, &inst_lookup),
            }
        }
    }
}

fn transfer_linear_block(
    incoming: &BTreeSet<InstId>,
    block: &crate::types::BasicBlock,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
) -> BTreeSet<InstId> {
    let mut live = incoming.clone();
    for inst in &block.insts {
        match inst.opcode {
            Opcode::Return => {
                if let Some(&arg) = inst.args.first() {
                    remove_linear_origins(&mut live, arg, inst_lookup);
                }
                live.clear();
            }
            Opcode::Unreachable => live.clear(),
            _ => apply_linear_transfer(inst, &mut live, inst_lookup),
        }
    }
    live
}

fn apply_linear_transfer(
    inst: &Inst,
    live: &mut BTreeSet<InstId>,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
) {
    if inst.ty == Ty::LinearPtr
        && matches!(
            inst.opcode,
            Opcode::GetArg | Opcode::RegionAlloc | Opcode::Call | Opcode::Phi
        )
    {
        // LinearPtr Phi is its own fresh origin (arms are transferred in via Upsilon).
        live.insert(inst.id);
    }
    if inst.opcode == Opcode::Upsilon
        && let Some(&arg) = inst.args.first()
        && inst_lookup
            .get(&arg)
            .is_some_and(|(_, a)| a.ty == Ty::LinearPtr)
    {
        // Path-local transfer of the arm's origin into the target Phi (Upsilon ty is Unit, hence the arg-type check).
        remove_linear_origins(live, arg, inst_lookup);
    }
    if inst.opcode == Opcode::LinearConsume
        && let Some(&arg) = inst.args.first()
    {
        remove_linear_origins(live, arg, inst_lookup);
    }
}

fn remove_linear_origins(
    live: &mut BTreeSet<InstId>,
    id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
) {
    for origin in linear_origins(id, inst_lookup) {
        live.remove(&origin);
    }
}

fn linear_origins(
    id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
) -> BTreeSet<InstId> {
    // LinearPtr Phi is a leaf origin — tracing through arms here would discharge sibling-path origins (path-insensitive double-removal bug).
    let mut out = BTreeSet::new();
    let mut stack = vec![id];
    let mut seen = BTreeSet::new();
    while let Some(cur) = stack.pop() {
        if !seen.insert(cur) {
            continue;
        }
        let Some((_, inst)) = inst_lookup.get(&cur) else {
            continue;
        };
        match inst.opcode {
            Opcode::Upsilon => stack.extend(inst.args.iter().copied()),
            _ if inst.ty == Ty::LinearPtr => {
                out.insert(cur);
            }
            _ => {}
        }
    }
    out
}

fn predecessor_map(func: &Function) -> BTreeMap<BlockId, Vec<BlockId>> {
    let mut preds: BTreeMap<BlockId, Vec<BlockId>> = BTreeMap::new();
    for block in &func.blocks {
        preds.entry(block.id).or_default();
    }
    for block in &func.blocks {
        for succ in block_successors(block) {
            preds.entry(succ).or_default().push(block.id);
        }
    }
    preds
}

fn block_successors(block: &crate::types::BasicBlock) -> Vec<BlockId> {
    let Some(term) = block
        .insts
        .iter()
        .rev()
        .find(|inst| inst.opcode.is_terminal())
    else {
        return vec![];
    };
    match &term.data {
        InstData::BranchTargets {
            then_block,
            else_block,
        } if term.opcode == Opcode::Branch => vec![*then_block, *else_block],
        InstData::JumpTarget(target) if term.opcode == Opcode::Jump => vec![*target],
        _ => vec![],
    }
}

fn detect_loop_back_edges(func: &Function) -> Vec<(BlockId, BlockId)> {
    let blocks: BTreeMap<BlockId, &crate::types::BasicBlock> =
        func.blocks.iter().map(|block| (block.id, block)).collect();
    let mut visited = BTreeSet::new();
    let mut on_stack = BTreeSet::new();
    let mut back_edges = BTreeSet::new();
    let mut starts = Vec::new();
    if let Some(entry) = func.blocks.first() {
        starts.push(entry.id);
    }
    for &block in blocks.keys() {
        if !starts.contains(&block) {
            starts.push(block);
        }
    }

    for start in starts {
        if visited.contains(&start) {
            continue;
        }
        let mut stack = vec![BlockDfsFrame::Enter(start, 0)];
        while let Some(frame) = stack.pop() {
            match frame {
                BlockDfsFrame::Enter(id, _) => {
                    if visited.contains(&id) {
                        continue;
                    }
                    let Some(block) = blocks.get(&id) else {
                        continue;
                    };
                    visited.insert(id);
                    on_stack.insert(id);
                    stack.push(BlockDfsFrame::Exit(id));

                    let mut succs = block_successors(block);
                    succs.sort_unstable();
                    succs.dedup();
                    for succ in succs.into_iter().rev() {
                        if !blocks.contains_key(&succ) {
                            continue;
                        }
                        if on_stack.contains(&succ) {
                            back_edges.insert((id, succ));
                            continue;
                        }
                        if !visited.contains(&succ) {
                            stack.push(BlockDfsFrame::Enter(succ, 0));
                        }
                    }
                }
                BlockDfsFrame::Exit(id) => {
                    on_stack.remove(&id);
                }
            }
        }
    }

    back_edges.into_iter().collect()
}

fn forward_graph_without_back_edges(
    func: &Function,
    back_edges: &[(BlockId, BlockId)],
) -> BTreeMap<BlockId, BTreeSet<BlockId>> {
    let back_edge_set: BTreeSet<(BlockId, BlockId)> = back_edges.iter().copied().collect();
    let mut graph: BTreeMap<BlockId, BTreeSet<BlockId>> = func
        .blocks
        .iter()
        .map(|block| (block.id, BTreeSet::new()))
        .collect();
    let block_ids: BTreeSet<BlockId> = graph.keys().copied().collect();

    for block in &func.blocks {
        let mut succs = block_successors(block);
        succs.sort_unstable();
        succs.dedup();
        for succ in succs {
            if block_ids.contains(&succ) && !back_edge_set.contains(&(block.id, succ)) {
                graph.entry(block.id).or_default().insert(succ);
            }
        }
    }

    graph
}

fn reverse_graph(
    graph: &BTreeMap<BlockId, BTreeSet<BlockId>>,
) -> BTreeMap<BlockId, BTreeSet<BlockId>> {
    let mut reverse: BTreeMap<BlockId, BTreeSet<BlockId>> = graph
        .keys()
        .copied()
        .map(|block| (block, BTreeSet::new()))
        .collect();
    for (&pred, succs) in graph {
        reverse.entry(pred).or_default();
        for &succ in succs {
            reverse.entry(succ).or_default().insert(pred);
        }
    }
    reverse
}

fn reachable_from(
    starts: impl IntoIterator<Item = BlockId>,
    graph: &BTreeMap<BlockId, BTreeSet<BlockId>>,
) -> BTreeSet<BlockId> {
    let mut reachable = BTreeSet::new();
    let mut stack: Vec<BlockId> = starts.into_iter().collect();
    stack.sort_unstable();
    stack.dedup();
    while let Some(block) = stack.pop() {
        if !reachable.insert(block) {
            continue;
        }
        let Some(succs) = graph.get(&block) else {
            continue;
        };
        for &succ in succs.iter().rev() {
            if !reachable.contains(&succ) {
                stack.push(succ);
            }
        }
    }
    reachable
}

fn backedge_refresh_regions_by_edge(
    func: &Function,
    block_regions: &BTreeSet<BlockId>,
) -> BTreeMap<(BlockId, BlockId), Vec<BlockId>> {
    let mut back_edges = detect_loop_back_edges(func);
    back_edges.sort_unstable();
    back_edges.dedup();
    if back_edges.is_empty() {
        return BTreeMap::new();
    }

    let forward = forward_graph_without_back_edges(func, &back_edges);
    let reverse = reverse_graph(&forward);
    let mut by_pred: BTreeMap<(BlockId, BlockId), BTreeSet<BlockId>> = BTreeMap::new();
    for (pred, header) in back_edges {
        let reachable_from_header = reachable_from([header], &forward);
        let reaches_pred = reachable_from([pred], &reverse);
        for &region_block in block_regions {
            if reachable_from_header.contains(&region_block) && reaches_pred.contains(&region_block)
            {
                by_pred
                    .entry((pred, header))
                    .or_default()
                    .insert(region_block);
            }
        }
    }

    by_pred
        .into_iter()
        .map(|(pred, regions)| (pred, regions.into_iter().collect()))
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EdgeRegionMarkers {
    closes: Vec<BlockId>,
    refreshes: Vec<BlockId>,
}

impl EdgeRegionMarkers {
    fn is_empty(&self) -> bool {
        self.closes.is_empty() && self.refreshes.is_empty()
    }
}

#[derive(Debug, Clone)]
struct BlockTree {
    parent: BTreeMap<BlockId, Option<BlockId>>,
    depth: BTreeMap<BlockId, u32>,
}

#[derive(Debug, Clone, Copy)]
enum BlockDfsFrame {
    Enter(BlockId, usize),
    Exit(BlockId),
}

impl BlockTree {
    fn from_function(func: &Function) -> Self {
        let blocks: BTreeMap<BlockId, &crate::types::BasicBlock> =
            func.blocks.iter().map(|block| (block.id, block)).collect();
        let mut forward_successors: BTreeMap<BlockId, BTreeSet<BlockId>> = blocks
            .keys()
            .copied()
            .map(|id| (id, BTreeSet::new()))
            .collect();
        let mut visited = BTreeSet::new();
        let mut on_stack = BTreeSet::new();
        let mut component: BTreeMap<BlockId, usize> = BTreeMap::new();
        let mut roots: Vec<BlockId> = Vec::new();

        if let Some(entry) = func.blocks.first() {
            Self::visit_root(
                entry.id,
                roots.len(),
                &blocks,
                &mut visited,
                &mut on_stack,
                &mut component,
                &mut forward_successors,
            );
            roots.push(entry.id);
        }

        for block in &func.blocks {
            if !visited.contains(&block.id) {
                Self::visit_root(
                    block.id,
                    roots.len(),
                    &blocks,
                    &mut visited,
                    &mut on_stack,
                    &mut component,
                    &mut forward_successors,
                );
                roots.push(block.id);
            }
        }

        let mut tree = Self {
            parent: Self::dominance_parent(&blocks, &forward_successors, &component, &roots),
            depth: BTreeMap::new(),
        };
        for &block in blocks.keys() {
            tree.depth.insert(block, tree.compute_depth(block));
        }
        tree
    }

    fn visit_root(
        root: BlockId,
        comp: usize,
        blocks: &BTreeMap<BlockId, &crate::types::BasicBlock>,
        visited: &mut BTreeSet<BlockId>,
        on_stack: &mut BTreeSet<BlockId>,
        component: &mut BTreeMap<BlockId, usize>,
        forward_successors: &mut BTreeMap<BlockId, BTreeSet<BlockId>>,
    ) {
        let mut stack = vec![BlockDfsFrame::Enter(root, comp)];
        while let Some(frame) = stack.pop() {
            match frame {
                BlockDfsFrame::Enter(id, comp) => {
                    if visited.contains(&id) {
                        continue;
                    }
                    let Some(block) = blocks.get(&id) else {
                        continue;
                    };
                    visited.insert(id);
                    on_stack.insert(id);
                    component.insert(id, comp);
                    stack.push(BlockDfsFrame::Exit(id));

                    let mut succs = block_successors(block);
                    succs.sort_unstable();
                    succs.dedup();
                    for succ in succs.into_iter().rev() {
                        if !blocks.contains_key(&succ) {
                            continue;
                        }
                        if on_stack.contains(&succ) {
                            continue;
                        }
                        if component
                            .get(&succ)
                            .is_some_and(|&succ_comp| succ_comp != comp)
                        {
                            continue;
                        }
                        forward_successors.entry(id).or_default().insert(succ);
                        if !visited.contains(&succ) {
                            stack.push(BlockDfsFrame::Enter(succ, comp));
                        }
                    }
                }
                BlockDfsFrame::Exit(id) => {
                    on_stack.remove(&id);
                }
            }
        }
    }

    fn dominance_parent(
        blocks: &BTreeMap<BlockId, &crate::types::BasicBlock>,
        forward_successors: &BTreeMap<BlockId, BTreeSet<BlockId>>,
        component: &BTreeMap<BlockId, usize>,
        roots: &[BlockId],
    ) -> BTreeMap<BlockId, Option<BlockId>> {
        let mut predecessors: BTreeMap<BlockId, Vec<BlockId>> =
            blocks.keys().copied().map(|id| (id, Vec::new())).collect();
        for (&pred, succs) in forward_successors {
            for &succ in succs {
                predecessors.entry(succ).or_default().push(pred);
            }
        }
        for preds in predecessors.values_mut() {
            preds.sort_unstable();
            preds.dedup();
        }

        let mut by_component: BTreeMap<usize, Vec<BlockId>> = BTreeMap::new();
        for (&block, &comp) in component {
            by_component.entry(comp).or_default().push(block);
        }

        let mut parent: BTreeMap<BlockId, Option<BlockId>> = BTreeMap::new();
        for (comp, mut nodes) in by_component {
            nodes.sort_unstable();
            let Some(&root) = roots.get(comp) else {
                continue;
            };
            let all_nodes: BTreeSet<BlockId> = nodes.iter().copied().collect();
            let mut dom: BTreeMap<BlockId, BTreeSet<BlockId>> = BTreeMap::new();
            for &node in &nodes {
                if node == root {
                    dom.insert(node, BTreeSet::from([node]));
                } else {
                    dom.insert(node, all_nodes.clone());
                }
            }

            let bound = nodes.len().saturating_mul(nodes.len().max(1)).max(1);
            for _ in 0..bound {
                let mut changed = false;
                for &node in &nodes {
                    if node == root {
                        continue;
                    }
                    let preds: Vec<BlockId> = predecessors
                        .get(&node)
                        .into_iter()
                        .flat_map(|ps| ps.iter().copied())
                        .filter(|pred| component.get(pred) == Some(&comp))
                        .collect();
                    let mut pred_iter = preds.into_iter();
                    let mut new_dom = if let Some(first_pred) = pred_iter.next() {
                        dom.get(&first_pred).cloned().unwrap_or_default()
                    } else {
                        BTreeSet::new()
                    };
                    for pred in pred_iter {
                        let pred_dom = dom.get(&pred).cloned().unwrap_or_default();
                        new_dom = new_dom.intersection(&pred_dom).copied().collect();
                    }
                    new_dom.insert(node);
                    if dom.get(&node) != Some(&new_dom) {
                        dom.insert(node, new_dom);
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }

            for &node in &nodes {
                if node == root {
                    parent.insert(node, None);
                    continue;
                }
                let mut strict_doms = dom.get(&node).cloned().unwrap_or_default();
                strict_doms.remove(&node);
                let idom = strict_doms
                    .into_iter()
                    .max_by_key(|candidate| dom.get(candidate).map(|s| s.len()).unwrap_or(0));
                parent.insert(node, idom);
            }
        }

        parent
    }

    fn compute_depth(&self, block: BlockId) -> u32 {
        let mut depth = 0u32;
        let mut seen = BTreeSet::new();
        let mut cur = block;
        while seen.insert(cur) {
            let Some(parent) = self.parent.get(&cur).copied().flatten() else {
                return depth;
            };
            depth = depth.saturating_add(1);
            cur = parent;
        }
        0
    }

    fn lca_all(&self, blocks: &[BlockId]) -> Option<BlockId> {
        let (&first, rest) = blocks.split_first()?;
        let mut acc = first;
        for &block in rest {
            acc = self.lca(acc, block)?;
        }
        Some(acc)
    }

    fn lca(&self, mut left: BlockId, mut right: BlockId) -> Option<BlockId> {
        let mut left_depth = *self.depth.get(&left)?;
        let mut right_depth = *self.depth.get(&right)?;

        while left_depth > right_depth {
            left = self.parent.get(&left).copied().flatten()?;
            left_depth -= 1;
        }
        while right_depth > left_depth {
            right = self.parent.get(&right).copied().flatten()?;
            right_depth -= 1;
        }

        while left != right {
            let left_parent = self.parent.get(&left).copied().flatten();
            let right_parent = self.parent.get(&right).copied().flatten();
            match (left_parent, right_parent) {
                (Some(l), Some(r)) => {
                    left = l;
                    right = r;
                }
                _ => return None,
            }
        }

        Some(left)
    }

    fn is_ancestor(&self, ancestor: BlockId, mut block: BlockId) -> bool {
        loop {
            if ancestor == block {
                return true;
            }
            let Some(parent) = self.parent.get(&block).copied().flatten() else {
                return false;
            };
            block = parent;
        }
    }

    fn depth_of(&self, block: BlockId) -> u32 {
        self.depth.get(&block).copied().unwrap_or(0)
    }
}

fn emit_live_linear_errors(
    func: &Function,
    live: &BTreeSet<InstId>,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    emitted: &mut BTreeSet<InstId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for origin in live {
        if !emitted.insert(*origin) {
            continue;
        }
        let Some((_, inst)) = inst_lookup.get(origin) else {
            continue;
        };
        let name = func
            .local_names
            .get(&origin.0)
            .cloned()
            .unwrap_or_else(|| format!("%{}", origin.0));
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: ErrorCode::RegionLinear,
            message: format!(
                "linear value `{name}` is not consumed before its region closes"
            ),
            primary: SourceLocation {
                file: func.source_file.clone(),
                byte_offset: inst.origin.start,
                byte_len: inst.origin.len,
            },
            secondary: vec![],
            blame: Blame::None,
            hints: vec![format!(
                "consume `{name}` before this scope exits, or return it to transfer the obligation to the caller"
            )],
        });
    }
}

fn edge_region_markers(
    pred: BlockId,
    succ: Option<BlockId>,
    block_regions: &BTreeSet<BlockId>,
    block_tree: &BlockTree,
    refresh_regions_by_edge: &BTreeMap<(BlockId, BlockId), Vec<BlockId>>,
) -> EdgeRegionMarkers {
    let refreshes = succ
        .and_then(|target| refresh_regions_by_edge.get(&(pred, target)).cloned())
        .unwrap_or_default();
    let mut closes = Vec::new();
    for &region_block in block_regions {
        if !block_tree.is_ancestor(region_block, pred) {
            continue;
        }
        let exits_region = succ
            .map(|target| !block_tree.is_ancestor(region_block, target))
            .unwrap_or(true);
        if exits_region && !refreshes.contains(&region_block) {
            closes.push(region_block);
        }
    }
    closes.sort_by(|a, b| {
        block_tree
            .depth_of(*b)
            .cmp(&block_tree.depth_of(*a))
            .then_with(|| b.cmp(a))
    });
    EdgeRegionMarkers { closes, refreshes }
}

fn marker_insts(next_id: &mut u32, markers: &EdgeRegionMarkers, span: Span) -> Vec<Inst> {
    let mut insts = Vec::with_capacity(markers.closes.len() + markers.refreshes.len());
    for &close_block in &markers.closes {
        insts.push(region_marker_inst(
            *next_id,
            Opcode::RegionClose,
            close_block,
            span,
        ));
        *next_id += 1;
    }
    for &refresh_block in &markers.refreshes {
        insts.push(region_marker_inst(
            *next_id,
            Opcode::RegionClose,
            refresh_block,
            span,
        ));
        *next_id += 1;
    }
    insts
}

fn phi_home_blocks(func: &Function) -> BTreeMap<InstId, BlockId> {
    let mut homes = BTreeMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Phi {
                homes.insert(inst.id, block.id);
            }
        }
    }
    homes
}

fn upsilon_target_block(inst: &Inst, phi_homes: &BTreeMap<InstId, BlockId>) -> Option<BlockId> {
    if inst.opcode == Opcode::Upsilon
        && let InstData::PhiTarget(phi_id) = inst.data
    {
        phi_homes.get(&phi_id).copied()
    } else {
        None
    }
}

fn edge_upsilons_for_target(
    insts: &[Inst],
    target: BlockId,
    phi_homes: &BTreeMap<InstId, BlockId>,
) -> Vec<Inst> {
    insts
        .iter()
        .filter(|inst| upsilon_target_block(inst, phi_homes) == Some(target))
        .cloned()
        .collect()
}

fn split_edge_with_markers(
    target: BlockId,
    markers: &EdgeRegionMarkers,
    edge_upsilons: &[Inst],
    span: Span,
    next_id: &mut u32,
    next_block_id: &mut u32,
    split_blocks: &mut Vec<crate::types::BasicBlock>,
) -> BlockId {
    if markers.is_empty() {
        return target;
    }
    let split_id = BlockId(*next_block_id);
    *next_block_id = next_block_id
        .checked_add(1)
        .expect("BlockId overflow while splitting region-marker edge");
    let mut insts = marker_insts(next_id, markers, span);
    for upsilon in edge_upsilons {
        let mut cloned = upsilon.clone();
        cloned.id = InstId(*next_id);
        *next_id += 1;
        insts.push(cloned);
    }
    insts.push(Inst {
        id: InstId(*next_id),
        opcode: Opcode::Jump,
        ty: Ty::Unit,
        args: vec![],
        data: InstData::JumpTarget(target),
        origin: span,
        region: RegionId::Root,
    });
    *next_id += 1;
    split_blocks.push(crate::types::BasicBlock {
        id: split_id,
        insts,
    });
    split_id
}

/// Insert `RegionOpen` / `RegionClose` markers around basic blocks whose
/// region is non-empty (spec §3.5). Must run AFTER `infer_regions` so that
/// every `RegionAlloc` inst carries its inferred `region: RegionId`.
///
/// Emission rules (Phase 4 / S3, criterion 1 of §3.5):
/// - For each function, collect every distinct `BlockId B` appearing as
///   `RegionId::Block(B)` on any inst's `region` field within the function.
/// - For each such `B`, prepend `RegionOpen { region: Block(B) }` to basic
///   block `B`'s instruction list and insert `RegionClose { region:
///   Block(B) }` before every terminator that exits `B`'s block-tree subtree.
///
/// Spec §3.5 criteria 2 (call store-effects) and 3 (call FreshInCaller
/// hidden `target_region` routing) are wired in S4 alongside the call-site
/// hidden-region substitution; until then, those allocations still resolve
/// to `Caller(_)` and never pin a caller block as non-empty.
pub fn insert_region_markers(module: &mut Module) {
    for func in &mut module.functions {
        // Idempotency tripwire: this pass is non-idempotent — calling it
        // twice would insert duplicate `RegionOpen` / `RegionClose` pairs
        // because the scan only sees `RegionId::Block(_)` on existing alloc
        // insts and won't recognise its own previously-inserted markers.
        // The current pipeline calls it exactly once. If a future
        // reorder accidentally runs it twice, this assertion catches it
        // in debug builds before codegen produces a malformed module.
        debug_assert!(
            !func
                .blocks
                .iter()
                .flat_map(|b| b.insts.iter())
                .any(|i| { matches!(i.opcode, Opcode::RegionOpen | Opcode::RegionClose) }),
            "insert_region_markers called twice on function `{}`",
            func.name,
        );
        // Collect all distinct block IDs that participate as a region.
        let mut block_regions: BTreeSet<BlockId> = BTreeSet::new();
        for block in &func.blocks {
            for inst in &block.insts {
                if let RegionId::Block(b) = inst.region {
                    block_regions.insert(b);
                }
            }
        }
        if block_regions.is_empty() {
            continue;
        }

        let block_tree = BlockTree::from_function(func);
        let refresh_regions_by_edge = backedge_refresh_regions_by_edge(func, &block_regions);
        let phi_homes = phi_home_blocks(func);

        let mut next_id = next_inst_id(func);
        let mut next_block_id = next_block_id(func);
        let mut split_blocks = Vec::new();
        for block in &mut func.blocks {
            let old_insts = std::mem::take(&mut block.insts);
            let span = old_insts
                .first()
                .map(|i| i.origin)
                .unwrap_or(Span { start: 0, len: 0 });
            let opens_here = block_regions.contains(&block.id);

            let term_pos = old_insts
                .iter()
                .position(|i| i.opcode.is_terminal())
                .unwrap_or(old_insts.len());
            let term_span = old_insts.get(term_pos).map(|i| i.origin).unwrap_or(span);
            let mut before_term_markers = EdgeRegionMarkers::default();
            let mut rewritten_term: Option<Inst> = None;
            let mut moved_upsilons = BTreeSet::new();
            if let Some(term) = old_insts.get(term_pos) {
                match &term.data {
                    InstData::JumpTarget(target) if term.opcode == Opcode::Jump => {
                        before_term_markers = edge_region_markers(
                            block.id,
                            Some(*target),
                            &block_regions,
                            &block_tree,
                            &refresh_regions_by_edge,
                        );
                    }
                    InstData::BranchTargets {
                        then_block,
                        else_block,
                    } if term.opcode == Opcode::Branch => {
                        let then_markers = edge_region_markers(
                            block.id,
                            Some(*then_block),
                            &block_regions,
                            &block_tree,
                            &refresh_regions_by_edge,
                        );
                        let then_upsilons =
                            edge_upsilons_for_target(&old_insts, *then_block, &phi_homes);
                        let else_markers = edge_region_markers(
                            block.id,
                            Some(*else_block),
                            &block_regions,
                            &block_tree,
                            &refresh_regions_by_edge,
                        );
                        let else_upsilons =
                            edge_upsilons_for_target(&old_insts, *else_block, &phi_homes);
                        let new_then = split_edge_with_markers(
                            *then_block,
                            &then_markers,
                            &then_upsilons,
                            term_span,
                            &mut next_id,
                            &mut next_block_id,
                            &mut split_blocks,
                        );
                        let new_else = split_edge_with_markers(
                            *else_block,
                            &else_markers,
                            &else_upsilons,
                            term_span,
                            &mut next_id,
                            &mut next_block_id,
                            &mut split_blocks,
                        );
                        if new_then != *then_block || new_else != *else_block {
                            if new_then != *then_block {
                                moved_upsilons.extend(then_upsilons.iter().map(|inst| inst.id));
                            }
                            if new_else != *else_block {
                                moved_upsilons.extend(else_upsilons.iter().map(|inst| inst.id));
                            }
                            let mut new_term = term.clone();
                            new_term.data = InstData::BranchTargets {
                                then_block: new_then,
                                else_block: new_else,
                            };
                            rewritten_term = Some(new_term);
                        }
                    }
                    _ => {
                        before_term_markers = edge_region_markers(
                            block.id,
                            None,
                            &block_regions,
                            &block_tree,
                            &refresh_regions_by_edge,
                        );
                    }
                }
            } else {
                before_term_markers = edge_region_markers(
                    block.id,
                    None,
                    &block_regions,
                    &block_tree,
                    &refresh_regions_by_edge,
                );
            }

            if !opens_here && before_term_markers.is_empty() && rewritten_term.is_none() {
                block.insts = old_insts;
                continue;
            }

            let mut new_insts = Vec::with_capacity(
                old_insts.len()
                    + usize::from(opens_here)
                    + before_term_markers.closes.len()
                    + before_term_markers.refreshes.len(),
            );
            if opens_here {
                new_insts.push(region_marker_inst(
                    next_id,
                    Opcode::RegionOpen,
                    block.id,
                    span,
                ));
                next_id += 1;
            }
            new_insts.extend(
                old_insts[..term_pos]
                    .iter()
                    .filter(|inst| !moved_upsilons.contains(&inst.id))
                    .cloned(),
            );
            new_insts.extend(marker_insts(&mut next_id, &before_term_markers, term_span));
            if let Some(term) = rewritten_term {
                new_insts.push(term);
                new_insts.extend(
                    old_insts[term_pos + 1..]
                        .iter()
                        .filter(|inst| !moved_upsilons.contains(&inst.id))
                        .cloned(),
                );
            } else {
                new_insts.extend(old_insts[term_pos..].iter().cloned());
            }
            block.insts = new_insts;
        }
        func.blocks.extend(split_blocks);
    }
}

fn region_marker_inst(id: u32, opcode: Opcode, block: BlockId, origin: Span) -> Inst {
    Inst {
        id: InstId(id),
        opcode,
        ty: Ty::Unit,
        args: vec![],
        data: InstData::None,
        origin,
        region: RegionId::Block(block),
    }
}

/// Smallest unused `InstId` value across `func`'s blocks. Panics on the
/// (4 billion-inst) overflow case rather than silently returning
/// `u32::MAX` — a duplicate ID would corrupt the IR. CLAUDE.md
/// "no shortcuts": impossible cases fail loudly.
fn next_inst_id(func: &Function) -> u32 {
    let mut max_id = 0u32;
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.id.0 > max_id {
                max_id = inst.id.0;
            }
        }
    }
    max_id
        .checked_add(1)
        .expect("InstId overflow in insert_region_markers — function too large")
}

fn next_block_id(func: &Function) -> u32 {
    let mut max_id = 0u32;
    for block in &func.blocks {
        if block.id.0 > max_id {
            max_id = block.id.0;
        }
    }
    max_id
        .checked_add(1)
        .expect("BlockId overflow in insert_region_markers — function too large")
}

fn internal_compiler_error(message: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code: ErrorCode::RegionConflict,
        message: format!("internal compiler error: {message}"),
        primary: SourceLocation {
            file: String::new(),
            byte_offset: 0,
            byte_len: 0,
        },
        secondary: vec![],
        blame: Blame::None,
        hints: vec![
            "this indicates a bug in the region inference pass; please file an issue".to_string(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Internal lattice
// ---------------------------------------------------------------------------

/// Internal summary used during fixed-point iteration. Distinct from the
/// public `RegionSummary` because the public type cannot represent the
/// `Uninit` ⊥ element (spec §4.3 step 1).
#[derive(Debug, Clone, PartialEq, Eq)]
struct InternalSummary {
    return_region: InternalReturnRegion,
    /// Set of (target_param_index, source_constraint) pairs. We use a set
    /// rather than a Vec so set-inclusion is the join.
    store_effects: BTreeSet<(u32, InternalReturnRegion)>,
    n_params: usize,
}

impl InternalSummary {
    fn seed(n_params: usize) -> Self {
        Self {
            return_region: InternalReturnRegion::Uninit,
            store_effects: BTreeSet::new(),
            n_params,
        }
    }

    fn equals(&self, other: &Self) -> bool {
        self == other
    }

    fn to_published(&self, n_params: usize) -> RegionSummary {
        let return_region = match &self.return_region {
            InternalReturnRegion::Uninit => RegionConstraint::ConstantGlobal,
            InternalReturnRegion::Published(c) => c.clone(),
        };
        let mut store_effects: Vec<StoreEffect> = self
            .store_effects
            .iter()
            .map(|(target, source)| StoreEffect {
                target: *target,
                source: match source {
                    InternalReturnRegion::Uninit => RegionConstraint::ConstantGlobal,
                    InternalReturnRegion::Published(c) => c.clone(),
                },
            })
            .collect();
        // Canonicalise: ascending by target, stable for equal targets.
        store_effects.sort_by_key(|e| e.target);
        // param_regions: one RegionVar per parameter, indexed by parameter
        // position (placeholders, spec §4.2).
        let param_regions: Vec<RegionVar> = (0..n_params as u32).map(RegionVar).collect();
        RegionSummary {
            param_regions,
            return_region,
            store_effects,
        }
    }
}

/// Internal return-region with a `Uninit` bottom for SCC seeding.
/// `Ord` is derived so this can sit inside `BTreeSet` for store_effects keys;
/// the order is structural, not semantic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum InternalReturnRegion {
    Uninit,
    Published(RegionConstraint),
}

impl InternalReturnRegion {
    fn is_uninit(&self) -> bool {
        matches!(self, InternalReturnRegion::Uninit)
    }
}

/// Join two internal return-regions per spec §4.3 lattice.
fn join_return(a: &InternalReturnRegion, b: &InternalReturnRegion) -> InternalReturnRegion {
    use InternalReturnRegion::*;
    match (a, b) {
        (Uninit, x) | (x, Uninit) => x.clone(),
        (Published(x), Published(y)) => InternalReturnRegion::Published(join_constraint(x, y)),
    }
}

/// Join two published `RegionConstraint`s per spec §4.3 lattice.
fn join_constraint(a: &RegionConstraint, b: &RegionConstraint) -> RegionConstraint {
    use RegionConstraint::*;
    match (a, b) {
        (FreshInCaller, _) | (_, FreshInCaller) => FreshInCaller,
        (ConstantGlobal, ConstantGlobal) => ConstantGlobal,
        (ConstantGlobal, AliasOf(_))
        | (AliasOf(_), ConstantGlobal)
        | (ConstantGlobal, AliasOfAny(_))
        | (AliasOfAny(_), ConstantGlobal) => FreshInCaller,
        (AliasOf(i), AliasOf(j)) => {
            if i == j {
                AliasOf(*i)
            } else {
                AliasOfAny(canonical_aliases(&[*i, *j]))
            }
        }
        (AliasOf(i), AliasOfAny(s)) | (AliasOfAny(s), AliasOf(i)) => {
            let mut combined = s.clone();
            combined.push(*i);
            AliasOfAny(canonical_aliases(&combined))
        }
        (AliasOfAny(s), AliasOfAny(t)) => {
            let mut combined = s.clone();
            combined.extend_from_slice(t);
            AliasOfAny(canonical_aliases(&combined))
        }
    }
}

fn canonical_aliases(xs: &[u32]) -> Vec<u32> {
    let mut v: Vec<u32> = xs.to_vec();
    v.sort_unstable();
    v.dedup();
    v
}

// ---------------------------------------------------------------------------
// Per-function analysis
// ---------------------------------------------------------------------------

/// Analyse a single function with the current global summaries fixed.
/// Returns the function's tightened internal summary and populates
/// `region_map` with per-inst regions.
fn analyze_function(
    func: &Function,
    summaries: &[InternalSummary],
    region_map: &mut BTreeMap<InstId, RegionId>,
    diagnostics: &mut Vec<Diagnostic>,
) -> InternalSummary {
    let source_file: &str = &func.source_file;
    let mut summary = InternalSummary::seed(func.params.len());

    // Build inst lookup + a flat "all instructions" iterator that records
    // (block_id, inst_id) for must_outlive resolution.
    let mut inst_lookup: BTreeMap<InstId, (BlockId, &Inst)> = BTreeMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            inst_lookup.insert(inst.id, (block.id, inst));
        }
    }

    // must_outlive[InstId] is the set of MustOutliveMarker(s) the value
    // must remain live across.
    let mut must_outlive: BTreeMap<InstId, BTreeSet<MustOutliveMarker>> = BTreeMap::new();

    // Pre-collect Pizlo-SSA Upsilon→Phi arms for deep origin walks.
    let phi_arms = collect_phi_arms(func);
    let block_tree = BlockTree::from_function(func);

    // Forward sweep collecting use-set contributions.
    for block in &func.blocks {
        for inst in &block.insts {
            handle_inst(
                source_file,
                inst,
                block.id,
                &inst_lookup,
                summaries,
                is_scalar_ty(func.return_ty),
                &mut must_outlive,
                &mut summary,
                diagnostics,
            );
        }
    }

    collect_regular_use_markers(func, &mut must_outlive);
    propagate_alias_markers(func, summaries, &phi_arms, &mut must_outlive);

    // Compute return-region contributions in a deep pass, walking Phi/Call
    // origins with the current summaries fixed. This is the canonical
    // path; the in-handle_inst Return shortcut only flags virtual-caller
    // escape on the must_outlive set.
    summary.return_region = compute_return_region(func, &inst_lookup, &phi_arms, summaries);

    // Compute LUB-derived RegionId for every heap-producing inst.
    for block in &func.blocks {
        for inst in &block.insts {
            if !is_heap_producing(inst, summaries) {
                continue;
            }
            let markers = must_outlive.get(&inst.id).cloned().unwrap_or_default();
            let mut region_id = lub_to_region_id(&markers, block.id, &block_tree);
            if inst.opcode == Opcode::Call
                && !matches!(&inst.data, InstData::CallExtern(sym) if heap_producing_extern(sym))
            {
                // Hidden caller-region codegen is still too shallow for
                // unresolved/internal aggregate FreshInCaller call results
                // that are immediately projected and repackaged. Keep those
                // materialisations conservative, while allowing recognised
                // runtime heap producers to route through their explicit
                // arena-aware ABI.
                if matches!(region_id, RegionId::Block(_) | RegionId::Caller(_)) {
                    region_id = RegionId::Root;
                }
            }
            region_map.insert(inst.id, region_id);
        }
    }

    // Codex Option 1.5 (issue #314): semantic store-conflict check that
    // consults the populated region_map. Replaces the old shape-based check
    // that fired in handle_inst purely on IR-opcode shape.
    check_store_conflicts_post_inference(
        func,
        summaries,
        region_map,
        &block_tree,
        &inst_lookup,
        diagnostics,
    );

    // Codex Option 1.5 (issue #314): emit RegionRootEscape notes for
    // allocations that route to the root region. Conservative
    // over-approximation: any Caller(_) inferred region in a function whose
    // summary will publish FreshInCaller (so the alloc CAN escape via the
    // caller chain to root). Severity Note — non-blocking; surfaces silent
    // program-lifetime placement that the spec §4.4 rationale warns about.
    if matches!(
        summary.return_region,
        InternalReturnRegion::Published(RegionConstraint::FreshInCaller)
    ) || summary
        .store_effects
        .iter()
        .any(|(_, c)| matches!(c, InternalReturnRegion::Published(_)))
    {
        emit_root_escape_notes(func, region_map, diagnostics);
    }

    summary
}

/// Codex Option 1.5 (issue #314): emit a `RegionRootEscape` Note for each
/// heap-producing instruction whose inferred region is `Caller(_)`. Only
/// invoked from `analyze_function` for functions that may propagate the
/// allocation to a caller (FreshInCaller return or any published store
/// effect). Conservative over-approximation — false positives are
/// acceptable for a Note severity, false negatives would silently miss
/// program-lifetime placements that §4.4's rationale targets.
fn emit_root_escape_notes(
    func: &Function,
    region_map: &BTreeMap<InstId, RegionId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let source_file: &str = &func.source_file;
    for block in &func.blocks {
        for inst in &block.insts {
            let Some(rgn) = region_map.get(&inst.id) else {
                continue;
            };
            if !matches!(rgn, RegionId::Caller(_)) {
                continue;
            }
            // Skip the value being returned — that's the canonical
            // FreshInCaller return path, not a side-effect escape.
            // Heuristic: if any Return inst in this function references
            // this id, treat it as the return-value path.
            let mut is_return_value = false;
            for blk in &func.blocks {
                for i in &blk.insts {
                    if i.opcode == Opcode::Return && i.args.first() == Some(&inst.id) {
                        is_return_value = true;
                        break;
                    }
                }
                if is_return_value {
                    break;
                }
            }
            if is_return_value {
                continue;
            }
            diagnostics.push(Diagnostic {
                severity: Severity::Note,
                code: ErrorCode::RegionRootEscape,
                message: "allocation may live in the root region: routed via \
                          store-effect chain to a caller whose target_region \
                          ultimately resolves to root"
                    .to_string(),
                primary: SourceLocation {
                    file: source_file.to_string(),
                    byte_offset: inst.origin.start,
                    byte_len: inst.origin.len,
                },
                secondary: vec![],
                blame: Blame::Callee,
                hints: vec![
                    "if intentional (e.g. program-lifetime data), no action \
                     needed; if you want this allocation freed earlier, \
                     restructure so the value is returned rather than stored \
                     into a parameter container"
                        .to_string(),
                ],
            });
        }
    }
}

fn extern_fresh_in_caller(sym: &str) -> bool {
    matches!(
        sym,
        "__vow_string_from_raw_parts_copy" | "__vow_vec_from_raw_parts_copy_val"
    )
}

fn vec_creation_extern(sym: &str) -> bool {
    matches!(sym, "__vow_vec_new" | "__vow_vec_new_val")
}

fn string_creation_extern(sym: &str) -> bool {
    matches!(
        sym,
        "__vow_string_new"
            | "__vow_string_new_in_arena"
            | "__vow_string_from_cstr"
            | "__vow_string_from_cstr_in_arena"
            | "__vow_string_substr"
            | "__vow_string_substr_in_arena"
            | "__vow_string_substring"
            | "__vow_string_substring_in_arena"
            | "__vow_string_from_i64"
            | "__vow_string_from_i64_in_arena"
            | "__vow_string_split"
            | "__vow_string_split_in_arena"
            | "__vow_string_trim"
            | "__vow_string_trim_in_arena"
            | "__vow_string_to_upper"
            | "__vow_string_to_upper_in_arena"
            | "__vow_string_to_lower"
            | "__vow_string_to_lower_in_arena"
            | "__vow_string_replace"
            | "__vow_string_replace_in_arena"
            | "__vow_string_join"
            | "__vow_string_join_in_arena"
    )
}

fn map_creation_extern(sym: &str) -> bool {
    matches!(sym, "__vow_map_new" | "__vow_map_new_in_arena")
}

fn heap_producing_extern(sym: &str) -> bool {
    extern_fresh_in_caller(sym)
        || vec_creation_extern(sym)
        || string_creation_extern(sym)
        || map_creation_extern(sym)
}

fn for_each_extern_store_edge(sym: &str, args: &[InstId], mut visit: impl FnMut(InstId, InstId)) {
    match sym {
        "__vow_vec_push_val" if args.len() >= 2 => visit(args[0], args[1]),
        "__vow_vec_push_val_in_arena" if args.len() >= 3 => visit(args[1], args[2]),
        "__vow_vec_set_val" if args.len() >= 3 => visit(args[0], args[2]),
        "__vow_map_insert" | "__vow_btreemap_insert" if args.len() >= 3 => {
            visit(args[0], args[1]);
            visit(args[0], args[2]);
        }
        "__vow_map_insert_in_arena" if args.len() >= 4 => {
            visit(args[1], args[2]);
            visit(args[1], args[3]);
        }
        _ => {}
    }
}

fn extern_growth_target(sym: &str, args: &[InstId]) -> Option<InstId> {
    match sym {
        "__vow_vec_push" if !args.is_empty() => Some(args[0]),
        "__vow_vec_push_in_arena" | "__vow_vec_reserve_in_arena" if args.len() >= 2 => {
            Some(args[1])
        }
        "__vow_string_push_str" | "__vow_string_push_byte" if !args.is_empty() => Some(args[0]),
        "__vow_string_push_str_in_arena" | "__vow_string_push_byte_in_arena" if args.len() >= 2 => {
            Some(args[1])
        }
        "__vow_map_insert" if !args.is_empty() => Some(args[0]),
        "__vow_map_insert_in_arena" if args.len() >= 2 => Some(args[1]),
        _ => None,
    }
}

fn is_heap_producing(inst: &Inst, summaries: &[InternalSummary]) -> bool {
    matches!(inst.opcode, Opcode::RegionAlloc)
        || matches!(
            (&inst.opcode, &inst.data),
            (Opcode::Call, InstData::CallExtern(sym)) if heap_producing_extern(sym)
        )
        || matches!(
            (&inst.opcode, &inst.data),
            (Opcode::Call, InstData::CallTarget(callee_id))
                if summaries
                    .get(callee_id.0 as usize)
                    .is_some_and(|s| s.return_region
                        == InternalReturnRegion::Published(RegionConstraint::FreshInCaller))
        )
}

/// Handle one instruction: contribute to `must_outlive` and to the
/// function's tightening `summary`.
///
/// `_source_file` and `_diagnostics` were used by the eager-emission shape
/// of the old `check_store_conflict` (Issue #314). After Codex Option 1.5,
/// the conflict check moved to a post-inference pass; the parameters are
/// retained on this signature to preserve the call shape and keep diffs
/// localised, but are unused here.
#[allow(clippy::too_many_arguments)]
fn handle_inst(
    _source_file: &str,
    inst: &Inst,
    _block_id: BlockId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    summaries: &[InternalSummary],
    return_is_scalar: bool,
    must_outlive: &mut BTreeMap<InstId, BTreeSet<MustOutliveMarker>>,
    summary: &mut InternalSummary,
    _diagnostics: &mut Vec<Diagnostic>,
) {
    match inst.opcode {
        Opcode::Return => {
            // The returned value escapes to the virtual caller. Mark it so
            // the inst.region populate pass tags any RegionAlloc that flows
            // into the return as Caller(0).
            if !return_is_scalar && let Some(&arg_id) = inst.args.first() {
                add_marker(must_outlive, arg_id, MustOutliveMarker::VirtualCaller);
            }
            // The actual return-region summary contribution is computed in a
            // separate deep pass that walks Phi/Call origins with summaries
            // fixed (see compute_return_region).
            let _ = summary; // silence unused warning if other arms grow
        }
        Opcode::Store | Opcode::FieldSet
            // Store/FieldSet: source must outlive target's region. We model
            // this as: source's must_outlive includes the region of the
            // target. Unknown/root-pinned targets still fall back to Root;
            // local heap targets and parameter targets get their precise
            // marker.
            if inst.args.len() >= 2 => {
                // IR convention for Store / FieldSet: args = [target, source].
                // - Store: codegen emits `store(value=arg!(1), address=arg!(0))`
                //   (see `vow-codegen/src/cranelift_backend.rs::Opcode::Store`).
                // - FieldSet: lowering emits `vec![ptr_id, new_val]`
                //   (see `vow-ir/src/lower/mod.rs::ExprKind::Assign` field path).
                // So `args[0]` is the target (container) and `args[1]` is the
                // source (value being stored).
                let target_id = inst.args[0];
                let source_id = inst.args[1];
                add_marker(
                    must_outlive,
                    source_id,
                    target_region_marker(target_id, inst_lookup, summaries),
                );
                // If the target traces to a parameter, record a store_effect.
                if let Some(target_param) = trace_param(target_id, inst_lookup) {
                    add_store_effect_source_constraints(
                        summary,
                        target_param,
                        source_id,
                        inst_lookup,
                        summaries,
                    );
                }
            }
        Opcode::Upsilon => {
            if let InstData::PhiTarget(target_phi) = inst.data
                && let Some(&source_id) = inst.args.first()
                && let Some((target_block, _)) = inst_lookup.get(&target_phi)
            {
                add_marker(
                    must_outlive,
                    source_id,
                    MustOutliveMarker::Block(*target_block),
                );
            }
        }
        Opcode::Call => {
            if let InstData::CallExtern(sym) = &inst.data {
                for_each_extern_store_edge(sym, &inst.args, |target_id, source_id| {
                    let marker = if trace_param(target_id, inst_lookup).is_some() {
                        MustOutliveMarker::Root
                    } else {
                        target_region_marker(target_id, inst_lookup, summaries)
                    };
                    add_marker(must_outlive, source_id, marker);
                    if let Some(target_param) = trace_param(target_id, inst_lookup) {
                        add_store_effect_source_constraints(
                            summary,
                            target_param,
                            source_id,
                            inst_lookup,
                            summaries,
                        );
                    }
                });
                if let Some(target_id) = extern_growth_target(sym, &inst.args)
                    && let Some(target_param) = trace_param(target_id, inst_lookup)
                {
                    summary.store_effects.insert((
                        target_param,
                        InternalReturnRegion::Published(RegionConstraint::ConstantGlobal),
                    ));
                }
            }

            // Look up callee summary and apply store-effects + return aliasing.
            let callee_summary: Option<&InternalSummary> = if let InstData::CallTarget(callee_id) =
                &inst.data
            {
                summaries.get(callee_id.0 as usize)
            } else {
                None
            };

            if let Some(cs) = callee_summary {
                // Apply store_effects: for each (target, source) effect on
                // the callee, values constrained by `source` are stored into
                // the region of argument `target`. If `source` aliases a callee
                // parameter, make the corresponding caller source argument
                // inherit the caller target argument's region marker.
                for (target_param, source_constraint) in &cs.store_effects {
                    let target_idx = *target_param as usize;
                    if target_idx >= inst.args.len() {
                        continue;
                    }
                    let target_arg_id = inst.args[target_idx];
                    if let Some(current_target_param) = trace_param(target_arg_id, inst_lookup) {
                        publish_transitive_store_effect(
                            summary,
                            current_target_param,
                            source_constraint,
                            inst,
                            inst_lookup,
                            summaries,
                        );
                    }
                    match source_constraint {
                        InternalReturnRegion::Published(RegionConstraint::AliasOf(p)) => {
                            // The callee writes argument-at-position-p into argument-at-position-target.
                            // Add the must-outlive marker so region inference can widen the
                            // source's region. The conflict diagnostic itself is now emitted
                            // by `check_store_conflicts_post_inference` after the region_map
                            // is populated, so the check can consult the inferred region
                            // (Codex Option 1.5 — issue #314).
                            let p_idx = *p as usize;
                            if p_idx < inst.args.len() {
                                let source_arg_id = inst.args[p_idx];
                                add_marker(
                                    must_outlive,
                                    source_arg_id,
                                    target_region_marker(target_arg_id, inst_lookup, summaries),
                                );
                            }
                        }
                        InternalReturnRegion::Published(RegionConstraint::FreshInCaller) => {}
                        _ => {}
                    }
                }

                // Return aliasing: if callee returns AliasOf(j) and result is used,
                // the result carries the j-th arg into wider regions.
                match &cs.return_region {
                    InternalReturnRegion::Published(RegionConstraint::AliasOf(j)) => {
                        let j_idx = *j as usize;
                        if j_idx < inst.args.len() {
                            // result must_outlive ⊆ arg[j] must_outlive (same value).
                            // Propagate by aliasing: anything added to the result later
                            // also adds to arg[j].
                            propagate_alias(must_outlive, inst.id, inst.args[j_idx]);
                        }
                    }
                    InternalReturnRegion::Published(RegionConstraint::AliasOfAny(s)) => {
                        for j in s {
                            let j_idx = *j as usize;
                            if j_idx < inst.args.len() {
                                propagate_alias(must_outlive, inst.id, inst.args[j_idx]);
                            }
                        }
                    }
                    _ => {}
                }
            }
            // CallExtern (no CallTarget): default ConstantGlobal, no constraints.
        }
        _ => {}
    }
}

fn publish_transitive_store_effect(
    summary: &mut InternalSummary,
    current_target_param: u32,
    source_constraint: &InternalReturnRegion,
    call_inst: &Inst,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    summaries: &[InternalSummary],
) {
    match source_constraint {
        InternalReturnRegion::Published(RegionConstraint::AliasOf(p)) => {
            let p_idx = *p as usize;
            if p_idx < call_inst.args.len() {
                add_store_effect_source_constraints(
                    summary,
                    current_target_param,
                    call_inst.args[p_idx],
                    inst_lookup,
                    summaries,
                );
            }
        }
        InternalReturnRegion::Published(RegionConstraint::AliasOfAny(ps)) => {
            for p in ps {
                let p_idx = *p as usize;
                if p_idx < call_inst.args.len() {
                    add_store_effect_source_constraints(
                        summary,
                        current_target_param,
                        call_inst.args[p_idx],
                        inst_lookup,
                        summaries,
                    );
                }
            }
        }
        InternalReturnRegion::Published(RegionConstraint::FreshInCaller) => {
            summary.store_effects.insert((
                current_target_param,
                InternalReturnRegion::Published(RegionConstraint::FreshInCaller),
            ));
        }
        InternalReturnRegion::Published(RegionConstraint::ConstantGlobal) => {
            summary.store_effects.insert((
                current_target_param,
                InternalReturnRegion::Published(RegionConstraint::ConstantGlobal),
            ));
        }
        InternalReturnRegion::Uninit => {}
    }
}

fn add_marker(
    must_outlive: &mut BTreeMap<InstId, BTreeSet<MustOutliveMarker>>,
    inst_id: InstId,
    marker: MustOutliveMarker,
) {
    must_outlive.entry(inst_id).or_default().insert(marker);
}

fn add_store_effect_source_constraints(
    summary: &mut InternalSummary,
    target_param: u32,
    source_id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    summaries: &[InternalSummary],
) {
    let source_origin = trace_origin(source_id, inst_lookup);
    let source_constraint = origin_to_constraint(&source_origin);
    summary.store_effects.insert((
        target_param,
        InternalReturnRegion::Published(source_constraint),
    ));
    publish_embedded_param_aliases(summary, target_param, source_id, inst_lookup);

    let Some((_, source_inst)) = inst_lookup.get(&source_id) else {
        return;
    };
    let Opcode::Call = source_inst.opcode else {
        return;
    };
    let InstData::CallTarget(callee_id) = &source_inst.data else {
        return;
    };
    let Some(callee) = summaries.get(callee_id.0 as usize) else {
        return;
    };
    if callee.return_region != InternalReturnRegion::Published(RegionConstraint::FreshInCaller) {
        return;
    }

    for &arg_id in &source_inst.args {
        if let Some(param_idx) = trace_param(arg_id, inst_lookup) {
            summary.store_effects.insert((
                target_param,
                InternalReturnRegion::Published(RegionConstraint::AliasOf(param_idx)),
            ));
        }
    }
}

fn publish_embedded_param_aliases(
    summary: &mut InternalSummary,
    target_param: u32,
    aggregate_id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
) {
    for (_, inst) in inst_lookup.values() {
        if !matches!(inst.opcode, Opcode::Store | Opcode::FieldSet) || inst.args.len() < 2 {
            continue;
        }
        if inst.args[0] != aggregate_id {
            continue;
        }
        if let Some(param_idx) = trace_param(inst.args[1], inst_lookup) {
            summary.store_effects.insert((
                target_param,
                InternalReturnRegion::Published(RegionConstraint::AliasOf(param_idx)),
            ));
        }
    }
}

fn collect_regular_use_markers(
    func: &Function,
    must_outlive: &mut BTreeMap<InstId, BTreeSet<MustOutliveMarker>>,
) {
    for block in &func.blocks {
        for inst in &block.insts {
            for &arg_id in &inst.args {
                add_marker(must_outlive, arg_id, MustOutliveMarker::Block(block.id));
            }
        }
    }
}

fn propagate_alias_markers(
    func: &Function,
    summaries: &[InternalSummary],
    phi_arms: &BTreeMap<InstId, Vec<InstId>>,
    must_outlive: &mut BTreeMap<InstId, BTreeSet<MustOutliveMarker>>,
) {
    let mut alias_edges: Vec<(InstId, InstId)> = Vec::new();
    for (phi_id, arms) in phi_arms {
        for &arm_id in arms {
            alias_edges.push((*phi_id, arm_id));
        }
    }
    for block in &func.blocks {
        for inst in &block.insts {
            match inst.opcode {
                Opcode::FieldGet => {
                    if let Some(&source) = inst.args.first() {
                        alias_edges.push((inst.id, source));
                    }
                }
                Opcode::Load if matches!(inst.ty, Ty::Ptr | Ty::LinearPtr) => {
                    if let Some(&source) = inst.args.first() {
                        alias_edges.push((inst.id, source));
                    }
                }
                Opcode::Store | Opcode::FieldSet if inst.args.len() >= 2 => {
                    // Stored values must follow later widening of their target
                    // container. The direct store handler already contributes
                    // the target's origin marker; this edge catches later
                    // use-derived markers such as `Return(target)`.
                    alias_edges.push((inst.args[0], inst.args[1]));
                }
                Opcode::Call => {
                    if let InstData::CallExtern(sym) = &inst.data {
                        for_each_extern_store_edge(sym, &inst.args, |target_id, source_id| {
                            alias_edges.push((target_id, source_id));
                        });
                    }
                    let InstData::CallTarget(callee_id) = &inst.data else {
                        continue;
                    };
                    let Some(summary) = summaries.get(callee_id.0 as usize) else {
                        continue;
                    };
                    for (target_param, source_constraint) in &summary.store_effects {
                        let target_idx = *target_param as usize;
                        if target_idx >= inst.args.len() {
                            continue;
                        }
                        let target_arg = inst.args[target_idx];
                        match source_constraint {
                            InternalReturnRegion::Published(RegionConstraint::AliasOf(p)) => {
                                let p_idx = *p as usize;
                                if p_idx < inst.args.len() {
                                    alias_edges.push((target_arg, inst.args[p_idx]));
                                }
                            }
                            InternalReturnRegion::Published(RegionConstraint::AliasOfAny(ps)) => {
                                for p in ps {
                                    let p_idx = *p as usize;
                                    if p_idx < inst.args.len() {
                                        alias_edges.push((target_arg, inst.args[p_idx]));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    match &summary.return_region {
                        InternalReturnRegion::Published(RegionConstraint::AliasOf(j)) => {
                            let j_idx = *j as usize;
                            if j_idx < inst.args.len() {
                                alias_edges.push((inst.id, inst.args[j_idx]));
                            }
                        }
                        InternalReturnRegion::Published(RegionConstraint::AliasOfAny(js)) => {
                            for j in js {
                                let j_idx = *j as usize;
                                if j_idx < inst.args.len() {
                                    alias_edges.push((inst.id, inst.args[j_idx]));
                                }
                            }
                        }
                        InternalReturnRegion::Published(RegionConstraint::FreshInCaller) => {
                            // A fresh aggregate can still contain borrowed
                            // heap descriptors passed as constructor args
                            // (`IrInst { args: ... }` is the bootstrap
                            // stress case). Any later widening of the call
                            // result must therefore widen the heap-producing
                            // arguments that may have been embedded in it.
                            for &arg_id in &inst.args {
                                alias_edges.push((inst.id, arg_id));
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    let mut changed = true;
    let mut iters = 0usize;
    let bound = alias_edges.len().saturating_add(func.blocks.len()).max(8);
    while changed && iters <= bound {
        changed = false;
        iters += 1;
        for &(result_id, arg_id) in &alias_edges {
            changed |= propagate_alias(must_outlive, result_id, arg_id);
        }
    }
    // The marker lattice is finite and `bound` accounts for the worst-case
    // propagation chain, so non-convergence is unreachable on valid IR.
    // A regression that breaks monotonicity would silently under-propagate
    // markers and could let a block-local alloc qualify for block-region
    // placement when it shouldn't — catch it loudly in debug builds.
    debug_assert!(
        !changed,
        "propagate_alias_markers did not converge within {bound} iterations"
    );
}

/// After a Phi or call returns an alias, the result inst is the same value as
/// its arm/arg from the must_outlive standpoint: markers on the result must
/// also apply to the origin.
fn propagate_alias(
    must_outlive: &mut BTreeMap<InstId, BTreeSet<MustOutliveMarker>>,
    result_id: InstId,
    arg_id: InstId,
) -> bool {
    let result_markers = must_outlive.get(&result_id).cloned().unwrap_or_default();
    if result_markers.is_empty() {
        return false;
    }
    let entry = must_outlive.entry(arg_id).or_default();
    let before = entry.len();
    entry.extend(result_markers);
    entry.len() != before
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(dead_code)] // Root + Rodata appear once the dataflow recognises Root-pin / .rodata flows.
enum MustOutliveMarker {
    Block(BlockId),
    VirtualCaller,
    Root,
    Rodata,
}

/// LUB of marker set per spec §4.1 coercions.
///
/// `defining_block` is the basic block where the allocation lives — used as
/// the default region when the marker set yields no narrower constraint
/// (empty set, or pure block markers reducible to the defining block).
fn lub_to_region_id(
    markers: &BTreeSet<MustOutliveMarker>,
    defining_block: BlockId,
    block_tree: &BlockTree,
) -> RegionId {
    let mut has_caller = false;
    let mut has_root = false;
    let mut has_rodata = false;
    let mut blocks: Vec<BlockId> = Vec::new();
    for m in markers {
        match m {
            MustOutliveMarker::Block(b) => blocks.push(*b),
            MustOutliveMarker::VirtualCaller => has_caller = true,
            MustOutliveMarker::Root => has_root = true,
            MustOutliveMarker::Rodata => has_rodata = true,
        }
    }
    // Rodata ⊔ Root → Root (spec §4.1).
    if has_root {
        return RegionId::Root;
    }
    if has_rodata && !has_caller && blocks.is_empty() {
        return RegionId::Rodata;
    }
    // Rodata ⊔ block → Rodata; Rodata ⊔ caller → Rodata.
    if has_rodata && !has_caller {
        return RegionId::Rodata;
    }
    if has_caller {
        return RegionId::Caller(HiddenRegionIdx(0));
    }
    blocks.push(defining_block);
    blocks.sort_unstable();
    blocks.dedup();
    if blocks.len() == 1 && blocks[0] == defining_block {
        return RegionId::Block(defining_block);
    }
    match block_tree.lca_all(&blocks) {
        Some(block) => RegionId::Block(block),
        None => RegionId::Caller(HiddenRegionIdx(0)),
    }
}

// ---------------------------------------------------------------------------
// Return-region deep computation
// ---------------------------------------------------------------------------

/// Walk every `Opcode::Return` in the function and compute the joined
/// return-region contribution, using the current per-function summaries
/// for any `Opcode::Call` results that flow into the return.
///
/// Pizlo SSA: Phi values are merged via Upsilon. `deep_origin_with_calls`
/// handles the walk, including chains like `phi → upsilon → call(callee)`.
fn compute_return_region(
    func: &Function,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    phi_arms: &BTreeMap<InstId, Vec<InstId>>,
    summaries: &[InternalSummary],
) -> InternalReturnRegion {
    let mut acc = InternalReturnRegion::Uninit;
    let mut found_return = false;
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode != Opcode::Return {
                continue;
            }
            found_return = true;
            // Scalar-typed declared-return functions: §4.3 step 5 rule 3.
            // ConstantGlobal regardless of body. Scalars carry no region.
            if is_scalar_ty(func.return_ty) {
                acc = join_return(
                    &acc,
                    &InternalReturnRegion::Published(RegionConstraint::ConstantGlobal),
                );
                continue;
            }
            let contribution = if let Some(&arg_id) = inst.args.first() {
                origin_to_internal(arg_id, inst_lookup, phi_arms, summaries)
            } else {
                InternalReturnRegion::Published(RegionConstraint::ConstantGlobal)
            };
            acc = join_return(&acc, &contribution);
        }
    }
    if !found_return {
        // No return at all (e.g., diverges) — leave at Uninit; resolve_uninit
        // catches it via §4.3 step 5 default.
        return acc;
    }
    acc
}

/// Resolve an inst back to its return-region contribution. Walks Pizlo Phi
/// arms via Upsilon, and consults `summaries` for `Opcode::Call` results.
fn origin_to_internal(
    id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    phi_arms: &BTreeMap<InstId, Vec<InstId>>,
    summaries: &[InternalSummary],
) -> InternalReturnRegion {
    let mut visiting: VecDeque<InstId> = VecDeque::new();
    origin_to_internal_inner(id, inst_lookup, phi_arms, summaries, &mut visiting)
}

// PERFORMANCE TODO (Phase 5 perf pass, issue #200): `visiting.contains(&id)`
// is O(n). On Phi-heavy IR with deep call resolution, the recursive walk
// makes `origin_to_internal_inner` O(n²) in the visiting-stack depth.
// Fix: keep `VecDeque` for push/pop ordering but maintain a parallel
// `BTreeSet<InstId>` for the membership test (BTreeSet is already imported
// elsewhere in this file). Same shape as the self-hosted port's
// `find_inst_index` perf TODO in `compiler/region.vow`. Acceptable today
// because the integration tests stay well under the quadratic knee.
fn origin_to_internal_inner(
    id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    phi_arms: &BTreeMap<InstId, Vec<InstId>>,
    summaries: &[InternalSummary],
    visiting: &mut VecDeque<InstId>,
) -> InternalReturnRegion {
    if visiting.contains(&id) {
        // Cycle (rare with Phi/Upsilon, but possible in pathological IR).
        // Leave at Uninit so the join with other arms still tightens.
        return InternalReturnRegion::Uninit;
    }
    let (_, inst) = match inst_lookup.get(&id) {
        Some(v) => v,
        None => return InternalReturnRegion::Published(RegionConstraint::ConstantGlobal),
    };
    match inst.opcode {
        Opcode::GetArg => {
            if let InstData::ArgIndex(i) = inst.data {
                InternalReturnRegion::Published(RegionConstraint::AliasOf(i))
            } else {
                InternalReturnRegion::Published(RegionConstraint::ConstantGlobal)
            }
        }
        Opcode::RegionAlloc => InternalReturnRegion::Published(RegionConstraint::FreshInCaller),
        Opcode::ConstStr
        | Opcode::ConstI32
        | Opcode::ConstI64
        | Opcode::ConstU64
        | Opcode::ConstF32
        | Opcode::ConstF64
        | Opcode::ConstBool
        | Opcode::ConstUnit => InternalReturnRegion::Published(RegionConstraint::ConstantGlobal),
        Opcode::Call => {
            if let InstData::CallExtern(sym) = &inst.data
                && heap_producing_extern(sym)
            {
                return InternalReturnRegion::Published(RegionConstraint::FreshInCaller);
            }
            if let InstData::CallTarget(callee_id) = &inst.data
                && let Some(cs) = summaries.get(callee_id.0 as usize)
            {
                // For AliasOf(i) returns, the call result aliases the
                // i-th caller arg. Recurse into that arg's origin to
                // get its true region.
                match &cs.return_region {
                    InternalReturnRegion::Published(RegionConstraint::AliasOf(i)) => {
                        let i_idx = *i as usize;
                        if i_idx < inst.args.len() {
                            visiting.push_back(id);
                            let r = origin_to_internal_inner(
                                inst.args[i_idx],
                                inst_lookup,
                                phi_arms,
                                summaries,
                                visiting,
                            );
                            visiting.pop_back();
                            return r;
                        }
                        return InternalReturnRegion::Published(RegionConstraint::AliasOf(*i));
                    }
                    InternalReturnRegion::Published(RegionConstraint::AliasOfAny(s)) => {
                        // Join the regions of all aliased caller args.
                        visiting.push_back(id);
                        let mut acc = InternalReturnRegion::Uninit;
                        for j in s {
                            let j_idx = *j as usize;
                            if j_idx < inst.args.len() {
                                let r = origin_to_internal_inner(
                                    inst.args[j_idx],
                                    inst_lookup,
                                    phi_arms,
                                    summaries,
                                    visiting,
                                );
                                acc = join_return(&acc, &r);
                            }
                        }
                        visiting.pop_back();
                        return acc;
                    }
                    InternalReturnRegion::Published(c) => {
                        return InternalReturnRegion::Published(c.clone());
                    }
                    InternalReturnRegion::Uninit => {
                        // Callee not yet summarised (intra-SCC); contribute
                        // Uninit so the join with other arms tightens later.
                        return InternalReturnRegion::Uninit;
                    }
                }
            }
            // CallExtern or unresolved: ConstantGlobal default.
            InternalReturnRegion::Published(RegionConstraint::ConstantGlobal)
        }
        Opcode::Phi => {
            visiting.push_back(id);
            let arms = phi_arms.get(&id).cloned().unwrap_or_default();
            let mut acc = InternalReturnRegion::Uninit;
            for arm_id in arms {
                let r =
                    origin_to_internal_inner(arm_id, inst_lookup, phi_arms, summaries, visiting);
                acc = join_return(&acc, &r);
            }
            visiting.pop_back();
            acc
        }
        _ => InternalReturnRegion::Published(RegionConstraint::ConstantGlobal),
    }
}

// ---------------------------------------------------------------------------
// Origin tracing helpers
// ---------------------------------------------------------------------------

/// Lightweight origin enum used by `trace_origin` for the in-handle_inst
/// shallow walks. Deeper walks (Phi/Call) go through `origin_to_internal`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum ValueOrigin {
    Param(u32),
    RegionAlloc,
    Constant,
    Other,
}

fn trace_origin(id: InstId, inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>) -> ValueOrigin {
    let (_, inst) = match inst_lookup.get(&id) {
        Some(v) => v,
        None => return ValueOrigin::Other,
    };
    match inst.opcode {
        Opcode::GetArg => {
            if let InstData::ArgIndex(i) = inst.data {
                ValueOrigin::Param(i)
            } else {
                ValueOrigin::Other
            }
        }
        Opcode::RegionAlloc => ValueOrigin::RegionAlloc,
        Opcode::Call if matches!(&inst.data, InstData::CallExtern(sym) if heap_producing_extern(sym)) => {
            ValueOrigin::RegionAlloc
        }
        Opcode::ConstStr
        | Opcode::ConstI32
        | Opcode::ConstI64
        | Opcode::ConstU64
        | Opcode::ConstF32
        | Opcode::ConstF64
        | Opcode::ConstBool
        | Opcode::ConstUnit => ValueOrigin::Constant,
        _ => ValueOrigin::Other,
    }
}

fn target_region_marker(
    id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    summaries: &[InternalSummary],
) -> MustOutliveMarker {
    let Some((block_id, inst)) = inst_lookup.get(&id) else {
        return MustOutliveMarker::Root;
    };
    match inst.opcode {
        Opcode::GetArg => MustOutliveMarker::VirtualCaller,
        _ if is_heap_producing(inst, summaries) => MustOutliveMarker::Block(*block_id),
        _ => MustOutliveMarker::Root,
    }
}

fn trace_param(id: InstId, inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>) -> Option<u32> {
    let mut visiting = VecDeque::new();
    trace_param_inner(id, inst_lookup, &mut visiting)
}

fn trace_param_inner(
    id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    visiting: &mut VecDeque<InstId>,
) -> Option<u32> {
    if visiting.contains(&id) {
        return None;
    }
    let (_, inst) = inst_lookup.get(&id)?;
    match (&inst.opcode, &inst.data) {
        (Opcode::GetArg, InstData::ArgIndex(i)) => Some(*i),
        (Opcode::FieldGet, _) | (Opcode::Load, _) => {
            let source = *inst.args.first()?;
            visiting.push_back(id);
            let result = trace_param_inner(source, inst_lookup, visiting);
            visiting.pop_back();
            result
        }
        (Opcode::Call, InstData::CallExtern(sym))
            if matches!(sym.as_str(), "__vow_vec_get_val" | "__vow_vec_get") =>
        {
            let source = *inst.args.first()?;
            visiting.push_back(id);
            let result = trace_param_inner(source, inst_lookup, visiting);
            visiting.pop_back();
            result
        }
        _ => None,
    }
}

/// Maps a shallow `ValueOrigin` (no Phi/Call awareness) to a
/// `RegionConstraint`. Used by store-effect contributions which only need
/// to know whether the source is a parameter, alloc, or constant.
fn origin_to_constraint(origin: &ValueOrigin) -> RegionConstraint {
    match origin {
        ValueOrigin::Param(i) => RegionConstraint::AliasOf(*i),
        ValueOrigin::RegionAlloc => RegionConstraint::FreshInCaller,
        ValueOrigin::Constant => RegionConstraint::ConstantGlobal,
        ValueOrigin::Other => RegionConstraint::ConstantGlobal,
    }
}

// ---------------------------------------------------------------------------
// Conflict detection
// ---------------------------------------------------------------------------

/// Phase 5 partial conflict detection (spec §4.4).
///
/// The self-hosted `compiler/region.vow` mirrors this in two phases: it
/// publishes direct `Store`/`FieldSet` effects during fixed-point iteration,
/// then checks call sites once after summaries converge. Keeping diagnostic
/// emission outside the SCC loop avoids duplicate reports while preserving the
/// same source-visible rejection as this Rust path.
///
/// Called during the forward sweep when an `Opcode::Call` site processes
/// a callee store-effect of shape `(target_param, AliasOf(p))`: the callee
/// writes its arg[p] into its arg[target_param]. In the caller's frame,
/// `target_arg_id = inst.args[target_param]` and `source_arg_id =
/// inst.args[p]`.
///
/// The constraint per spec §4.4 is `region(target) ⊇ region(source)`. The
/// clearest unsatisfiable shape we can detect with current data:
///
/// * `source_origin = RegionAlloc` (block-local fresh allocation in the
///   caller) and `target_origin = Param(_)` (caller's parameter region,
///   which strictly outlives any caller block). Storing a block-local
///   value into a parameter container is a use-after-free in the
///   caller's caller after the caller returns, unless the caller's own
///   summary lifts the allocation via a `(target, FreshInCaller)` entry.
///   The lift only materialises if the caller separately publishes that
///   effect (via a direct `Store`/`FieldSet` of a fresh value into the
///   same param), which is its own visible source-level pattern; we
///   reject this call shape unconditionally — the user fix is either to
///   hoist the alloc, copy via `pin_to_root`, or restructure the return
///   flow per §4.4 + issue #200.
///
/// Still outside this partial checker:
///   * Cross-param without published spanning effect (`source = Param(p)`,
///     `target = Param(q)`, `p != q`): needs the caller's full summary,
///     which is built up forward through this same pass.
///   * Phi-of-mixed-origins: descend into upsilon arms and reject when
///     the joined origin set spans incompatible regions.
///
/// Codex Option 1.5 (issue #314): semantic post-inference store-conflict check.
///
/// After `analyze_function` populates `region_map`, walk every CallTarget
/// instruction; for each callee store-effect of kind `AliasOf(p)`, look up
/// the inferred region of the corresponding caller-side argument. Reject
/// only when that inferred region is a concrete block strictly narrower
/// than the target's region. Sources whose inferred region is already
/// `Caller`/`Root`/`Rodata` satisfy any parameter-region target — region
/// inference's must_outlive widening (added by the same call site in
/// `handle_inst`) has already done the right thing; the check must consult
/// it instead of inspecting opcode shape.
fn check_store_conflicts_post_inference(
    func: &Function,
    summaries: &[InternalSummary],
    region_map: &BTreeMap<InstId, RegionId>,
    block_tree: &BlockTree,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let source_file: &str = &func.source_file;
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode != Opcode::Call {
                continue;
            }
            let InstData::CallTarget(callee) = &inst.data else {
                continue;
            };
            let callee_idx = callee.0 as usize;
            if callee_idx >= summaries.len() {
                continue;
            }
            let cs = &summaries[callee_idx];
            for (target, source_constraint) in &cs.store_effects {
                let target_idx = *target as usize;
                if target_idx >= inst.args.len() {
                    continue;
                }
                let target_arg_id = inst.args[target_idx];
                if let InternalReturnRegion::Published(RegionConstraint::AliasOf(p)) =
                    source_constraint
                {
                    let p_idx = *p as usize;
                    if p_idx >= inst.args.len() {
                        continue;
                    }
                    let source_arg_id = inst.args[p_idx];
                    check_store_conflict_semantic(
                        source_file,
                        target_arg_id,
                        source_arg_id,
                        inst,
                        inst_lookup,
                        summaries,
                        region_map,
                        block_tree,
                        diagnostics,
                    );
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_store_conflict_semantic(
    source_file: &str,
    target_arg_id: InstId,
    source_arg_id: InstId,
    call_inst: &Inst,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    summaries: &[InternalSummary],
    region_map: &BTreeMap<InstId, RegionId>,
    block_tree: &BlockTree,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Inline case: target is not a parameter; the inline path handles its own
    // region-LCA check via direct markers, no cross-call conflict.
    let target_origin = trace_origin(target_arg_id, inst_lookup);
    let ValueOrigin::Param(_) = target_origin else {
        return;
    };

    // Arm A/A.5: source is not in the region map (not a heap producer) OR is
    // already widened to caller/root/rodata. Either way, no conflict.
    let Some(src_rgn) = region_map.get(&source_arg_id).copied() else {
        return;
    };
    let src_block = match src_rgn {
        RegionId::Caller(_) | RegionId::Root | RegionId::Rodata => return,
        RegionId::Block(b) => b,
    };

    // Arm B: source is a concrete block. Compute target's region; conflict iff
    // src_block does not enclose tgt_block. We've already established target is
    // a parameter (above), so target_region_marker returns VirtualCaller —
    // hence any concrete block source is strictly narrower. We still go
    // through the full match for clarity / future-proofing.
    let target_marker = target_region_marker(target_arg_id, inst_lookup, summaries);
    let conflict = match target_marker {
        MustOutliveMarker::VirtualCaller | MustOutliveMarker::Root => true,
        MustOutliveMarker::Rodata => true,
        MustOutliveMarker::Block(tgt_block) => {
            // src outlives tgt iff src_block is an ancestor of tgt_block in the
            // block tree (a node is its own ancestor for this purpose).
            !block_tree.is_ancestor(src_block, tgt_block)
        }
    };
    if !conflict {
        return;
    }

    let source_span = inst_lookup
        .get(&source_arg_id)
        .map(|(_, i)| i.origin)
        .unwrap_or(call_inst.origin);
    let target_span = inst_lookup
        .get(&target_arg_id)
        .map(|(_, i)| i.origin)
        .unwrap_or(call_inst.origin);

    diagnostics.push(Diagnostic {
        severity: Severity::Error,
        code: ErrorCode::RegionConflict,
        message: "value's region cannot satisfy target container's region: \
                  block-local allocation stored into a parameter region"
            .to_string(),
        primary: SourceLocation {
            file: source_file.to_string(),
            byte_offset: source_span.start,
            byte_len: source_span.len,
        },
        secondary: vec![
            SourceLocation {
                file: source_file.to_string(),
                byte_offset: target_span.start,
                byte_len: target_span.len,
            },
            SourceLocation {
                file: source_file.to_string(),
                byte_offset: call_inst.origin.start,
                byte_len: call_inst.origin.len,
            },
        ],
        blame: Blame::Callee,
        hints: vec![
            "hoist the allocation to a wider scope, copy the value into the \
             outer arena, or restructure the return flow so the value \
             escapes to the caller"
                .to_string(),
        ],
    });
}

// ---------------------------------------------------------------------------
// §4.3 step 5: Uninit termination resolution
// ---------------------------------------------------------------------------

/// §4.3 step 5: terminal resolution. Called for any function still left
/// at internal `Uninit` after the SCC fixed-point converges. The
/// computed return-region used summaries that resolved to Uninit at
/// that point — re-walk now with all summaries final, falling back to
/// `ConstantGlobal` (benign default) when nothing identifiable is found.
fn resolve_uninit_return(func: &Function) -> RegionConstraint {
    // Build inst lookup + phi arms locally (no summaries available here —
    // any Uninit-still call result resolves to ConstantGlobal default).
    let mut inst_lookup: BTreeMap<InstId, (BlockId, &Inst)> = BTreeMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            inst_lookup.insert(inst.id, (block.id, inst));
        }
    }
    let phi_arms = collect_phi_arms(func);

    let mut found_return = false;
    let mut acc = InternalReturnRegion::Uninit;
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode != Opcode::Return {
                continue;
            }
            found_return = true;
            if is_scalar_ty(func.return_ty) {
                acc = join_return(
                    &acc,
                    &InternalReturnRegion::Published(RegionConstraint::ConstantGlobal),
                );
                continue;
            }
            let contribution = if let Some(&arg) = inst.args.first() {
                // Pass an empty summaries slice — Call results default to
                // ConstantGlobal at this terminal stage.
                origin_to_internal(arg, &inst_lookup, &phi_arms, &[])
            } else {
                InternalReturnRegion::Published(RegionConstraint::ConstantGlobal)
            };
            acc = join_return(&acc, &contribution);
        }
    }

    if !found_return {
        return RegionConstraint::ConstantGlobal;
    }
    match acc {
        InternalReturnRegion::Uninit => RegionConstraint::ConstantGlobal,
        InternalReturnRegion::Published(c) => c,
    }
}

fn is_scalar_ty(ty: Ty) -> bool {
    matches!(
        ty,
        Ty::I32 | Ty::I64 | Ty::U64 | Ty::F32 | Ty::F64 | Ty::Bool | Ty::Unit
    )
    // Ptr and LinearPtr are heap-typed and fall through to origin-based rules.
}

/// Collect Upsilon→Phi mappings: phi_id → list of Upsilon source-value insts.
fn collect_phi_arms(func: &Function) -> BTreeMap<InstId, Vec<InstId>> {
    let mut arms: BTreeMap<InstId, Vec<InstId>> = BTreeMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if inst.opcode == Opcode::Upsilon
                && let InstData::PhiTarget(target) = inst.data
                && let Some(&value_id) = inst.args.first()
            {
                arms.entry(target).or_default().push(value_id);
            }
        }
    }
    arms
}

// ---------------------------------------------------------------------------
// Call graph + Tarjan SCC
// ---------------------------------------------------------------------------

fn build_call_graph(module: &Module) -> Vec<Vec<u32>> {
    let n = module.functions.len();
    let mut graph: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (i, func) in module.functions.iter().enumerate() {
        for block in &func.blocks {
            for inst in &block.insts {
                if inst.opcode == Opcode::Call
                    && let InstData::CallTarget(target) = &inst.data
                {
                    let t = target.0;
                    if (t as usize) < n {
                        graph[i].push(t);
                    }
                }
            }
        }
        // Deduplicate adjacency for determinism.
        graph[i].sort_unstable();
        graph[i].dedup();
    }
    graph
}

/// Tarjan's SCC algorithm — iterative to avoid stack-depth limits on large
/// programs. Returns SCCs in reverse topological order (callees first).
fn tarjan_sccs(graph: &[Vec<u32>]) -> Vec<Vec<u32>> {
    let n = graph.len();
    let mut indices: Vec<i64> = vec![-1; n];
    let mut lowlinks: Vec<i64> = vec![0; n];
    let mut on_stack: Vec<bool> = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut sccs: Vec<Vec<u32>> = Vec::new();
    let mut index_counter: i64 = 0;

    // Iterative DFS frame: (node, child_iter_position).
    enum Action {
        Visit(u32),
        Process(u32, usize),
    }

    for start in 0..n {
        if indices[start] != -1 {
            continue;
        }
        let mut work: Vec<Action> = vec![Action::Visit(start as u32)];
        while let Some(action) = work.pop() {
            match action {
                Action::Visit(v) => {
                    indices[v as usize] = index_counter;
                    lowlinks[v as usize] = index_counter;
                    index_counter += 1;
                    stack.push(v);
                    on_stack[v as usize] = true;
                    work.push(Action::Process(v, 0));
                }
                Action::Process(v, child_idx) => {
                    let succs = &graph[v as usize];
                    if child_idx < succs.len() {
                        let w = succs[child_idx];
                        // Schedule revisit of (v, child_idx + 1).
                        work.push(Action::Process(v, child_idx + 1));
                        if indices[w as usize] == -1 {
                            work.push(Action::Visit(w));
                        } else if on_stack[w as usize] {
                            lowlinks[v as usize] = lowlinks[v as usize].min(indices[w as usize]);
                        }
                    } else {
                        // All children processed: update lowlinks from finished children.
                        for &w in succs {
                            if on_stack[w as usize] {
                                lowlinks[v as usize] =
                                    lowlinks[v as usize].min(lowlinks[w as usize]);
                            }
                        }
                        if lowlinks[v as usize] == indices[v as usize] {
                            let mut component: Vec<u32> = Vec::new();
                            loop {
                                let w = stack.pop().expect("stack non-empty");
                                on_stack[w as usize] = false;
                                component.push(w);
                                if w == v {
                                    break;
                                }
                            }
                            // Sort within an SCC for deterministic iteration.
                            component.sort_unstable();
                            sccs.push(component);
                        }
                    }
                }
            }
        }
    }

    // Tarjan emits SCCs in reverse topological order naturally — leaves first.
    // That's what we want.
    sccs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::HiddenRegionIdx;
    use crate::types::{
        BasicBlock, BlockId, FuncId, Function, Inst, InstData, InstId, Module, Opcode,
        RegionConstraint, RegionId, RegionSummary, Ty, VowEntry, VowId,
    };
    use std::collections::HashMap;
    use vow_diag::{ErrorCode, Severity};
    use vow_syntax::ast::Effect;
    use vow_syntax::span::Span;

    fn span() -> Span {
        Span { start: 0, len: 0 }
    }

    fn inst(id: u32, opcode: Opcode, ty: Ty, args: Vec<u32>, data: InstData) -> Inst {
        Inst {
            id: InstId(id),
            opcode,
            ty,
            args: args.into_iter().map(InstId).collect(),
            data,
            origin: span(),
            region: RegionId::Root,
        }
    }

    fn block(id: u32, insts: Vec<Inst>) -> BasicBlock {
        BasicBlock {
            id: BlockId(id),
            insts,
        }
    }

    fn jump_inst(id: u32, target: u32) -> Inst {
        inst(
            id,
            Opcode::Jump,
            Ty::Unit,
            vec![],
            InstData::JumpTarget(BlockId(target)),
        )
    }

    fn branch_inst(id: u32, then_block: u32, else_block: u32) -> Inst {
        inst(
            id,
            Opcode::Branch,
            Ty::Unit,
            vec![],
            InstData::BranchTargets {
                then_block: BlockId(then_block),
                else_block: BlockId(else_block),
            },
        )
    }

    fn return_unit_inst(id: u32) -> Inst {
        inst(id, Opcode::Return, Ty::Unit, vec![], InstData::None)
    }

    fn marker_set(markers: &[MustOutliveMarker]) -> BTreeSet<MustOutliveMarker> {
        markers.iter().cloned().collect()
    }

    fn function(
        id: u32,
        name: &str,
        params: Vec<Ty>,
        return_ty: Ty,
        blocks: Vec<BasicBlock>,
    ) -> Function {
        let n_params = params.len();
        Function {
            id: FuncId(id),
            name: name.to_string(),
            param_names: (0..n_params).map(|i| format!("p{i}")).collect(),
            params,
            return_ty,
            effects: vec![] as Vec<Effect>,
            vows: vec![] as Vec<VowEntry>,
            blocks,
            local_names: HashMap::new(),
            summary: RegionSummary::default(),
            source_file: "test.vow".to_string(),
        }
    }

    fn module(functions: Vec<Function>) -> Module {
        Module {
            name: "test".to_string(),
            functions,
            strings: vec![],
            struct_layouts: vec![],
            enum_layouts: vec![],
            warnings: vec![],
        }
    }

    fn _unused_vow_id() -> VowId {
        VowId(0)
    }

    // ---------- Helpers for the tests ----------

    /// Build `fn id(s: String) -> String { s }`. Pizlo IR: GetArg(0); Return(GetArg).
    fn build_identity_fn() -> Function {
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        function(0, "id", vec![Ty::Ptr], Ty::Ptr, vec![block(0, insts)])
    }

    /// Build `fn lit() -> i64 { "literal" }` returning a constant string.
    fn build_const_str_return() -> Function {
        let insts = vec![
            inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
            inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        function(0, "lit", vec![], Ty::Ptr, vec![block(0, insts)])
    }

    /// Build `fn allocs() -> Ptr { RegionAlloc(...) }` — a fresh alloc returned to caller.
    fn build_returning_alloc() -> Function {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        function(0, "allocs", vec![], Ty::Ptr, vec![block(0, insts)])
    }

    /// Build `fn scalar() -> i64 { 3 }`.
    fn build_scalar_return() -> Function {
        let insts = vec![
            inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(3)),
            inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        function(0, "scalar", vec![], Ty::I64, vec![block(0, insts)])
    }

    // ---------- Tests ----------

    #[test]
    fn alias_of_parameter_pass_through() {
        // `fn id(s) -> s` has no heap-producing inst. §4.3 step 5 catches it
        // and resolves to AliasOf(0). A buggy implementation seeded at
        // ConstantGlobal would silently mis-summarise as ConstantGlobal.
        let mut m = module(vec![build_identity_fn()]);
        infer_regions(&mut m);
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::AliasOf(0)
        );
    }

    #[test]
    fn rodata_literal_return() {
        let mut m = module(vec![build_const_str_return()]);
        infer_regions(&mut m);
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::ConstantGlobal
        );
    }

    #[test]
    fn returned_alloc_escapes_to_caller() {
        let mut m = module(vec![build_returning_alloc()]);
        infer_regions(&mut m);
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::FreshInCaller
        );
        // The alloc inst should be tagged Caller(0), not Root.
        let inst0 = &m.functions[0].blocks[0].insts[0];
        assert_eq!(inst0.region, RegionId::Caller(HiddenRegionIdx(0)));
    }

    #[test]
    fn scalar_returns_are_constantglobal() {
        let mut m = module(vec![build_scalar_return()]);
        infer_regions(&mut m);
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::ConstantGlobal
        );
    }

    #[test]
    fn ptr_return_not_treated_as_scalar() {
        // `fn id(p: Ptr) -> Ptr { p }` — Ptr is heap-typed, must NOT trigger
        // the scalar rule in §4.3 step 5. Expected: AliasOf(0).
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        let f = function(0, "id_ptr", vec![Ty::Ptr], Ty::Ptr, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::AliasOf(0)
        );
    }

    #[test]
    fn parallel_branch_join_produces_freshincaller() {
        // Pizlo SSA: phi receives values via Upsilon. Two upsilons feed the
        // phi: one with a ConstStr (.rodata), one with a GetArg (parameter).
        // Per §4.3 lattice, AliasOf(0) ⊔ ConstantGlobal = FreshInCaller.
        //
        // Block 0: GetArg(0); Branch(...) → block 1 / block 2
        // Block 1: ConstStr; Upsilon(ConstStr, target=phi); Jump → 3
        // Block 2: Upsilon(GetArg, target=phi); Jump → 3
        // Block 3: Phi; Return(Phi)
        //
        // We simplify by skipping the Branch (no condition needed for region inference).
        let phi_id = 10u32;

        let b0_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                1,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(1)),
            ),
        ];
        let b1_insts = vec![
            inst(2, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
            inst(
                3,
                Opcode::Upsilon,
                Ty::Unit,
                vec![2],
                InstData::PhiTarget(InstId(phi_id)),
            ),
            inst(
                4,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(3)),
            ),
        ];
        let b2_insts = vec![
            inst(
                5,
                Opcode::Upsilon,
                Ty::Unit,
                vec![0],
                InstData::PhiTarget(InstId(phi_id)),
            ),
            inst(
                6,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(3)),
            ),
        ];
        let b3_insts = vec![
            inst(phi_id, Opcode::Phi, Ty::Ptr, vec![], InstData::None),
            inst(11, Opcode::Return, Ty::Unit, vec![phi_id], InstData::None),
        ];
        let f = function(
            0,
            "branchy",
            vec![Ty::Ptr],
            Ty::Ptr,
            vec![
                block(0, b0_insts),
                block(1, b1_insts),
                block(2, b2_insts),
                block(3, b3_insts),
            ],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        // The function has no RegionAlloc and no Call — its return path falls
        // through to §4.3 step 5 deep_origin walk, which sees Phi merging
        // Param(0) + Constant. Origin merge → FreshInCaller per §4.3 join.
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::FreshInCaller
        );
    }

    #[test]
    fn aliasofany_is_canonical_sorted() {
        // Phi merges GetArg(2), GetArg(0) → AliasOfAny([0, 2]) ascending+dedup.
        let phi_id = 10u32;
        let b0_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(2, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(2)),
            inst(
                3,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(1)),
            ),
        ];
        let b1_insts = vec![
            inst(
                4,
                Opcode::Upsilon,
                Ty::Unit,
                vec![2],
                InstData::PhiTarget(InstId(phi_id)),
            ),
            inst(
                5,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(2)),
            ),
        ];
        let b2_insts = vec![
            inst(
                6,
                Opcode::Upsilon,
                Ty::Unit,
                vec![0],
                InstData::PhiTarget(InstId(phi_id)),
            ),
            inst(
                7,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(3)),
            ),
        ];
        let b3_insts = vec![
            inst(phi_id, Opcode::Phi, Ty::Ptr, vec![], InstData::None),
            inst(11, Opcode::Return, Ty::Unit, vec![phi_id], InstData::None),
        ];
        let f = function(
            0,
            "any",
            vec![Ty::Ptr, Ty::Ptr, Ty::Ptr],
            Ty::Ptr,
            vec![
                block(0, b0_insts),
                block(1, b1_insts),
                block(2, b2_insts),
                block(3, b3_insts),
            ],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        match &m.functions[0].summary.return_region {
            RegionConstraint::AliasOfAny(v) => {
                assert_eq!(v, &vec![0, 2], "aliases must be ascending+deduplicated");
            }
            other => panic!("expected AliasOfAny([0,2]), got {other:?}"),
        }
    }

    #[test]
    fn uninit_never_serialized() {
        // After infer_regions, every function's summary.return_region must
        // be a published RegionConstraint variant — Uninit is internal-only.
        // We check this structurally: serialise + deserialise the module and
        // assert the return_region is identifiable as one of the four
        // published variants for every function.
        let mut m = module(vec![
            build_identity_fn(),
            build_const_str_return(),
            build_returning_alloc(),
            build_scalar_return(),
        ]);
        // Give every function a unique id & name to satisfy module invariants.
        for (i, f) in m.functions.iter_mut().enumerate() {
            f.id = FuncId(i as u32);
            f.name = format!("f{i}");
        }
        infer_regions(&mut m);

        let bytes = crate::encode_module(&m);
        let decoded = crate::decode_module(&bytes).expect("decode round-trips");

        for f in &decoded.functions {
            match &f.summary.return_region {
                RegionConstraint::FreshInCaller
                | RegionConstraint::AliasOf(_)
                | RegionConstraint::AliasOfAny(_)
                | RegionConstraint::ConstantGlobal => {}
                #[allow(unreachable_patterns)]
                _ => panic!("unexpected variant: {:?}", f.summary.return_region),
            }
        }
    }

    #[test]
    fn scc_seed_uninit_not_constantglobal() {
        // Recursive alias-only function: f(s) calls f(s) and returns the result.
        // Seeded at Uninit, it converges to AliasOf(0). Seeded at
        // ConstantGlobal (the bug), the join AliasOf(0) ⊔ ConstantGlobal =
        // FreshInCaller and the function gets a spurious hidden parameter.
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                1,
                Opcode::Call,
                Ty::Ptr,
                vec![0],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(2, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        let f = function(0, "f", vec![Ty::Ptr], Ty::Ptr, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::AliasOf(0)
        );
    }

    #[test]
    fn scc_fixedpoint_mutual_recursion_alloc() {
        // Two mutually recursive functions returning fresh allocs.
        // Both seeded Uninit → both converge to FreshInCaller.
        let f0_insts = vec![
            inst(
                0,
                Opcode::Call,
                Ty::Ptr,
                vec![],
                InstData::CallTarget(FuncId(1)),
            ),
            inst(1, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        let f1_insts = vec![
            inst(
                2,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 8, align: 8 },
            ),
            inst(
                3,
                Opcode::Call,
                Ty::Ptr,
                vec![],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(4, Opcode::Return, Ty::Unit, vec![2], InstData::None),
        ];
        let f0 = function(0, "f0", vec![], Ty::Ptr, vec![block(0, f0_insts)]);
        let f1 = function(1, "f1", vec![], Ty::Ptr, vec![block(0, f1_insts)]);
        let mut m = module(vec![f0, f1]);
        infer_regions(&mut m);
        assert_eq!(
            m.functions[1].summary.return_region,
            RegionConstraint::FreshInCaller,
            "f1 returns a fresh alloc — must be FreshInCaller"
        );
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::FreshInCaller,
            "f0 returns f1()'s fresh alloc — must propagate to FreshInCaller"
        );
    }

    #[test]
    fn block_tree_lub_of_siblings_routes_to_parent_block() {
        let f = function(
            0,
            "siblings",
            vec![],
            Ty::Unit,
            vec![
                block(0, vec![branch_inst(0, 1, 2)]),
                block(1, vec![return_unit_inst(1)]),
                block(2, vec![return_unit_inst(2)]),
            ],
        );
        let tree = BlockTree::from_function(&f);
        let markers = marker_set(&[
            MustOutliveMarker::Block(BlockId(1)),
            MustOutliveMarker::Block(BlockId(2)),
        ]);

        assert_eq!(
            lub_to_region_id(&markers, BlockId(0), &tree),
            RegionId::Block(BlockId(0))
        );
    }

    #[test]
    fn block_tree_lub_of_block_and_descendant_routes_to_ancestor_block() {
        let f = function(
            0,
            "descendant",
            vec![],
            Ty::Unit,
            vec![
                block(0, vec![jump_inst(0, 1)]),
                block(1, vec![jump_inst(1, 2)]),
                block(2, vec![return_unit_inst(2)]),
            ],
        );
        let tree = BlockTree::from_function(&f);
        let markers = marker_set(&[
            MustOutliveMarker::Block(BlockId(1)),
            MustOutliveMarker::Block(BlockId(2)),
        ]);

        assert_eq!(
            lub_to_region_id(&markers, BlockId(1), &tree),
            RegionId::Block(BlockId(1))
        );
    }

    #[test]
    fn block_tree_lub_of_branch_and_merge_routes_to_common_parent() {
        let f = function(
            0,
            "diamond",
            vec![],
            Ty::Unit,
            vec![
                block(0, vec![branch_inst(0, 1, 2)]),
                block(1, vec![jump_inst(1, 3)]),
                block(2, vec![jump_inst(2, 3)]),
                block(3, vec![return_unit_inst(3)]),
            ],
        );
        let tree = BlockTree::from_function(&f);
        let markers = marker_set(&[
            MustOutliveMarker::Block(BlockId(1)),
            MustOutliveMarker::Block(BlockId(3)),
        ]);

        assert_eq!(
            lub_to_region_id(&markers, BlockId(1), &tree),
            RegionId::Block(BlockId(0))
        );
    }

    #[test]
    fn block_tree_lub_of_disconnected_roots_routes_to_virtual_caller() {
        let f = function(
            0,
            "disconnected",
            vec![],
            Ty::Unit,
            vec![
                block(0, vec![return_unit_inst(0)]),
                block(10, vec![return_unit_inst(10)]),
                block(20, vec![return_unit_inst(20)]),
            ],
        );
        let tree = BlockTree::from_function(&f);
        let markers = marker_set(&[
            MustOutliveMarker::Block(BlockId(10)),
            MustOutliveMarker::Block(BlockId(20)),
        ]);

        assert_eq!(
            lub_to_region_id(&markers, BlockId(10), &tree),
            RegionId::Caller(HiddenRegionIdx(0))
        );
    }

    #[test]
    fn block_tree_lub_single_defining_block_marker_stays_in_defining_block() {
        let f = function(
            0,
            "single",
            vec![],
            Ty::Unit,
            vec![block(0, vec![return_unit_inst(0)])],
        );
        let tree = BlockTree::from_function(&f);
        let markers = marker_set(&[MustOutliveMarker::Block(BlockId(0))]);

        assert_eq!(
            lub_to_region_id(&markers, BlockId(0), &tree),
            RegionId::Block(BlockId(0))
        );
    }

    #[test]
    fn block_tree_lub_root_marker_forces_root() {
        let f = function(
            0,
            "root",
            vec![],
            Ty::Unit,
            vec![block(0, vec![return_unit_inst(0)])],
        );
        let tree = BlockTree::from_function(&f);
        let markers = marker_set(&[
            MustOutliveMarker::Root,
            MustOutliveMarker::Block(BlockId(0)),
        ]);

        assert_eq!(
            lub_to_region_id(&markers, BlockId(0), &tree),
            RegionId::Root
        );
    }

    #[test]
    fn local_alloc_used_only_locally() {
        // Allocation that does NOT escape (no return, no store-into-param):
        // its LUB stays in the local block. We assert the inst.region was
        // set (not Root/default).
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            // Returning a constant scalar — alloc does not escape.
            inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
        ];
        let f = function(0, "local", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        let inst0 = &m.functions[0].blocks[0].insts[0];
        // A heap value with no escaping uses can be scoped to its defining
        // block; this is the no-alloc-block elision prerequisite from #204.
        assert_eq!(inst0.region, RegionId::Block(BlockId(0)));
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::ConstantGlobal,
            "scalar return must be ConstantGlobal even when function allocates"
        );
    }

    #[test]
    fn markers_inserted_for_non_empty_block_region() {
        // The marker insertion pass keys off `inst.region == Block(_)`. We
        // hand-tag the alloc to exercise the marker pass directly without
        // depending on the inference pass shape.
        let mut alloc = inst(
            0,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        alloc.region = RegionId::Block(BlockId(0));
        let insts = vec![
            alloc,
            inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
        ];
        let f = function(0, "local", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        // Skip infer_regions so this test stays focused on marker insertion
        // rather than on the inference pass shape.
        insert_region_markers(&mut m);

        let block_insts = &m.functions[0].blocks[0].insts;
        let opens: Vec<_> = block_insts
            .iter()
            .filter(|i| i.opcode == Opcode::RegionOpen)
            .collect();
        let closes: Vec<_> = block_insts
            .iter()
            .filter(|i| i.opcode == Opcode::RegionClose)
            .collect();
        assert_eq!(opens.len(), 1, "exactly one RegionOpen for Block(0)");
        assert_eq!(closes.len(), 1, "exactly one RegionClose for Block(0)");
        assert_eq!(opens[0].region, RegionId::Block(BlockId(0)));
        assert_eq!(closes[0].region, RegionId::Block(BlockId(0)));
        assert_eq!(
            block_insts.first().unwrap().opcode,
            Opcode::RegionOpen,
            "RegionOpen must be the first instruction of the block"
        );
        let close_pos = block_insts
            .iter()
            .position(|i| i.opcode == Opcode::RegionClose)
            .unwrap();
        let term_pos = block_insts
            .iter()
            .position(|i| i.opcode.is_terminal())
            .unwrap();
        assert_eq!(
            close_pos + 1,
            term_pos,
            "RegionClose must immediately precede the block's terminator"
        );
    }

    #[test]
    fn backedge_self_loop_refreshes_header_region() {
        let mut alloc = inst(
            0,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        alloc.region = RegionId::Block(BlockId(0));
        let f = function(
            0,
            "self_loop",
            vec![],
            Ty::Unit,
            vec![block(0, vec![alloc, jump_inst(1, 0)])],
        );
        let mut m = module(vec![f]);

        insert_region_markers(&mut m);

        let block_insts = &m.functions[0].blocks[0].insts;
        let opens: Vec<_> = block_insts
            .iter()
            .filter(|i| i.opcode == Opcode::RegionOpen && i.region == RegionId::Block(BlockId(0)))
            .collect();
        assert_eq!(
            opens.len(),
            1,
            "self-loop header should reopen only when control reaches the block entry"
        );
        let jump_pos = block_insts
            .iter()
            .position(|i| i.opcode == Opcode::Jump)
            .expect("self-loop should end in a back-edge jump");
        assert_eq!(
            block_insts[jump_pos - 1].opcode,
            Opcode::RegionClose,
            "self-loop backedge should close the header-owned region before jumping"
        );
        assert_eq!(
            block_insts[jump_pos - 1].region,
            RegionId::Block(BlockId(0))
        );
    }

    #[test]
    fn backedge_single_loop_refreshes_body_region() {
        let b0_insts = vec![jump_inst(0, 1)];
        let b1_insts = vec![
            inst(5, Opcode::Phi, Ty::Ptr, vec![], InstData::None),
            branch_inst(1, 2, 3),
        ];
        let mut alloc = inst(
            2,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        alloc.region = RegionId::Block(BlockId(2));
        let b2_insts = vec![alloc, jump_inst(3, 1)];
        let b3_insts = vec![return_unit_inst(4)];
        let f = function(
            0,
            "single_loop",
            vec![],
            Ty::Unit,
            vec![
                block(0, b0_insts),
                block(1, b1_insts),
                block(2, b2_insts),
                block(3, b3_insts),
            ],
        );
        let mut m = module(vec![f]);

        insert_region_markers(&mut m);

        let body = &m.functions[0].blocks[2].insts;
        let jump_pos = body
            .iter()
            .position(|i| i.opcode == Opcode::Jump)
            .expect("body should end in a back-edge jump");
        assert_eq!(
            body[jump_pos - 1].opcode,
            Opcode::RegionClose,
            "back-edge predecessor should close the body region before jumping to the header"
        );
        assert_eq!(body[jump_pos - 1].region, RegionId::Block(BlockId(2)));
        assert!(
            !body[jump_pos..]
                .iter()
                .any(|i| i.opcode == Opcode::RegionOpen && i.region == RegionId::Block(BlockId(2))),
            "back-edge refresh must not reopen the body region before the header can exit"
        );
    }

    #[test]
    fn backedge_nested_loops_refresh_inner_and_outer_regions() {
        let b0_insts = vec![jump_inst(0, 1)];
        let b1_insts = vec![branch_inst(1, 2, 7)];
        let mut outer_alloc = inst(
            2,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        outer_alloc.region = RegionId::Block(BlockId(2));
        let b2_insts = vec![outer_alloc, jump_inst(3, 3)];
        let b3_insts = vec![branch_inst(4, 4, 6)];
        let mut inner_alloc = inst(
            5,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        inner_alloc.region = RegionId::Block(BlockId(4));
        let b4_insts = vec![inner_alloc, jump_inst(6, 3)];
        let b6_insts = vec![jump_inst(7, 1)];
        let b7_insts = vec![return_unit_inst(8)];
        let f = function(
            0,
            "nested_loops",
            vec![],
            Ty::Unit,
            vec![
                block(0, b0_insts),
                block(1, b1_insts),
                block(2, b2_insts),
                block(3, b3_insts),
                block(4, b4_insts),
                block(6, b6_insts),
                block(7, b7_insts),
            ],
        );
        let mut m = module(vec![f]);

        insert_region_markers(&mut m);

        let inner_body = &m.functions[0].blocks[4].insts;
        let inner_jump = inner_body
            .iter()
            .position(|i| i.opcode == Opcode::Jump)
            .expect("inner body should jump back to inner header");
        assert_eq!(inner_body[inner_jump - 1].opcode, Opcode::RegionClose);
        assert_eq!(
            inner_body[inner_jump - 1].region,
            RegionId::Block(BlockId(4))
        );

        let outer_backedge = &m.functions[0].blocks[5].insts;
        let outer_jump = outer_backedge
            .iter()
            .position(|i| i.opcode == Opcode::Jump)
            .expect("outer body should jump back to outer header");
        assert_eq!(outer_backedge[outer_jump - 1].opcode, Opcode::RegionClose);
        assert_eq!(
            outer_backedge[outer_jump - 1].region,
            RegionId::Block(BlockId(2))
        );
        assert!(
            !outer_backedge
                .iter()
                .any(|i| i.opcode == Opcode::RegionClose && i.region == RegionId::Block(BlockId(4))),
            "outer back-edge must not refresh the inner loop body's region"
        );
    }

    #[test]
    fn backedge_break_predecessor_does_not_refresh_body_region() {
        let b0_insts = vec![jump_inst(0, 1)];
        let b1_insts = vec![branch_inst(1, 2, 4)];
        let mut alloc = inst(
            2,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        alloc.region = RegionId::Block(BlockId(2));
        let b2_insts = vec![alloc, branch_inst(3, 3, 5)];
        let b3_insts = vec![jump_inst(4, 4)];
        let b4_insts = vec![return_unit_inst(5)];
        let b5_insts = vec![jump_inst(6, 1)];
        let f = function(
            0,
            "break_loop",
            vec![],
            Ty::Unit,
            vec![
                block(0, b0_insts),
                block(1, b1_insts),
                block(2, b2_insts),
                block(3, b3_insts),
                block(4, b4_insts),
                block(5, b5_insts),
            ],
        );
        let mut m = module(vec![f]);

        insert_region_markers(&mut m);

        let break_block = &m.functions[0].blocks[3].insts;
        assert!(
            !break_block
                .iter()
                .any(|i| i.opcode == Opcode::RegionOpen && i.region == RegionId::Block(BlockId(2))),
            "break edge exits the loop and must not reopen the body region"
        );
        assert!(
            break_block
                .iter()
                .any(|i| i.opcode == Opcode::RegionClose && i.region == RegionId::Block(BlockId(2))),
            "break edge still keeps the ordinary exit close"
        );

        let backedge_block = &m.functions[0].blocks[5].insts;
        let jump_pos = backedge_block
            .iter()
            .position(|i| i.opcode == Opcode::Jump)
            .expect("natural loop path should jump back to header");
        assert_eq!(backedge_block[jump_pos - 1].opcode, Opcode::RegionClose);
        assert_eq!(
            backedge_block[jump_pos - 1].region,
            RegionId::Block(BlockId(2))
        );
    }

    #[test]
    fn backedge_mixed_branch_splits_exit_and_refresh_edges() {
        let b0_insts = vec![jump_inst(0, 1)];
        let b1_insts = vec![branch_inst(1, 2, 3)];
        let mut alloc = inst(
            2,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        alloc.region = RegionId::Block(BlockId(2));
        let b2_insts = vec![
            alloc,
            inst(
                4,
                Opcode::Upsilon,
                Ty::Unit,
                vec![2],
                InstData::PhiTarget(InstId(5)),
            ),
            branch_inst(3, 1, 3),
        ];
        let b3_insts = vec![
            inst(5, Opcode::Phi, Ty::Ptr, vec![], InstData::None),
            return_unit_inst(6),
        ];
        let f = function(
            0,
            "mixed_backedge_exit",
            vec![],
            Ty::Unit,
            vec![
                block(0, b0_insts),
                block(1, b1_insts),
                block(2, b2_insts),
                block(3, b3_insts),
            ],
        );
        let mut m = module(vec![f]);

        insert_region_markers(&mut m);

        let pred = m.functions[0]
            .blocks
            .iter()
            .find(|block| block.id == BlockId(2))
            .expect("predecessor block should remain present");
        let branch = pred
            .insts
            .iter()
            .find(|inst| inst.opcode == Opcode::Branch)
            .expect("predecessor should keep a conditional branch");
        let InstData::BranchTargets {
            then_block,
            else_block,
        } = branch.data
        else {
            panic!("branch should carry targets");
        };
        assert_ne!(
            then_block,
            BlockId(1),
            "backedge must route via split block"
        );
        assert_ne!(
            else_block,
            BlockId(3),
            "exit edge must route via split block"
        );
        assert!(
            !pred.insts
                .iter()
                .any(|i| i.opcode == Opcode::RegionClose && i.region == RegionId::Block(BlockId(2))),
            "mixed branch predecessor must not emit one block-wide close before both edges"
        );
        assert!(
            !pred
                .insts
                .iter()
                .any(|i| i.opcode == Opcode::Upsilon && i.data == InstData::PhiTarget(InstId(5))),
            "exit phi feed should move from the predecessor onto the split exit edge"
        );

        let then_split = m.functions[0]
            .blocks
            .iter()
            .find(|block| block.id == then_block)
            .expect("backedge split block should exist");
        assert_eq!(then_split.insts[0].opcode, Opcode::RegionClose);
        assert_eq!(then_split.insts[0].region, RegionId::Block(BlockId(2)));
        assert!(
            !then_split
                .insts
                .iter()
                .any(|i| i.opcode == Opcode::RegionOpen && i.region == RegionId::Block(BlockId(2))),
            "backedge split must not reopen before the header can exit"
        );
        assert!(
            !then_split
                .insts
                .iter()
                .any(|i| i.opcode == Opcode::Upsilon && i.data == InstData::PhiTarget(InstId(5))),
            "backedge split must not steal exit phi feeds"
        );
        let then_jump = then_split
            .insts
            .last()
            .expect("split block should not be empty");
        assert_eq!(then_jump.opcode, Opcode::Jump);
        assert_eq!(then_jump.data, InstData::JumpTarget(BlockId(1)));

        let else_split = m.functions[0]
            .blocks
            .iter()
            .find(|block| block.id == else_block)
            .expect("exit split block should exist");
        assert_eq!(else_split.insts[0].opcode, Opcode::RegionClose);
        assert_eq!(else_split.insts[0].region, RegionId::Block(BlockId(2)));
        assert!(
            !else_split
                .insts
                .iter()
                .any(|i| i.opcode == Opcode::RegionOpen),
            "exit split block must not reopen the region it is leaving"
        );
        assert!(
            else_split.insts.iter().any(|i| i.opcode == Opcode::Upsilon
                && i.id != InstId(4)
                && i.args == vec![InstId(2)]
                && i.data == InstData::PhiTarget(InstId(5))),
            "exit split should preserve pre-branch phi feeds with fresh instruction ids"
        );
        let else_jump = else_split
            .insts
            .last()
            .expect("split block should not be empty");
        assert_eq!(else_jump.opcode, Opcode::Jump);
        assert_eq!(else_jump.data, InstData::JumpTarget(BlockId(3)));
    }

    #[test]
    fn backedge_continue_predecessors_each_refresh_body_region() {
        let b0_insts = vec![jump_inst(0, 1)];
        let b1_insts = vec![branch_inst(1, 2, 6)];
        let mut alloc = inst(
            2,
            Opcode::RegionAlloc,
            Ty::Ptr,
            vec![],
            InstData::AllocSize { size: 16, align: 8 },
        );
        alloc.region = RegionId::Block(BlockId(2));
        let b2_insts = vec![alloc, branch_inst(3, 3, 4)];
        let b3_insts = vec![jump_inst(4, 1)];
        let b4_insts = vec![jump_inst(5, 1)];
        let b6_insts = vec![return_unit_inst(6)];
        let f = function(
            0,
            "continue_loop",
            vec![],
            Ty::Unit,
            vec![
                block(0, b0_insts),
                block(1, b1_insts),
                block(2, b2_insts),
                block(3, b3_insts),
                block(4, b4_insts),
                block(6, b6_insts),
            ],
        );
        let mut m = module(vec![f]);

        insert_region_markers(&mut m);

        for block_idx in [3usize, 4usize] {
            let backedge_block = &m.functions[0].blocks[block_idx].insts;
            let jump_pos = backedge_block
                .iter()
                .position(|i| i.opcode == Opcode::Jump)
                .expect("continue path should jump back to header");
            assert_eq!(backedge_block[jump_pos - 1].opcode, Opcode::RegionClose);
            assert_eq!(
                backedge_block[jump_pos - 1].region,
                RegionId::Block(BlockId(2))
            );
        }
    }

    #[test]
    fn no_markers_for_empty_block_region() {
        // Empty-region elision (spec §3.5): a function whose only alloc
        // escapes (`Caller(0)` summary) has no `Block(_)` region in itself,
        // so no RegionOpen/Close must be inserted.
        let f = build_returning_alloc();
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        insert_region_markers(&mut m);
        let any_marker = m.functions[0]
            .blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .any(|i| matches!(i.opcode, Opcode::RegionOpen | Opcode::RegionClose));
        assert!(
            !any_marker,
            "function with no Block(_) regions must not gain Open/Close markers"
        );
    }

    #[test]
    fn local_alloc_used_only_in_defining_block_routes_to_block_region() {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(1, Opcode::Load, Ty::I64, vec![0], InstData::None),
            inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
        ];
        let f = function(0, "local", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        let inst0 = &m.functions[0].blocks[0].insts[0];
        assert_eq!(inst0.region, RegionId::Block(BlockId(0)));
        insert_region_markers(&mut m);
        assert_eq!(
            m.functions[0].blocks[0].insts[0].opcode,
            Opcode::RegionOpen,
            "non-empty block region should open at block entry"
        );
        assert!(
            m.functions[0].blocks[0]
                .insts
                .iter()
                .any(|i| i.opcode == Opcode::RegionClose),
            "non-empty block region should close before the terminator"
        );
    }

    #[test]
    fn alloc_used_from_descendant_block_routes_to_lca_block() {
        let b0_insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                1,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(1)),
            ),
        ];
        let b1_insts = vec![
            inst(2, Opcode::Load, Ty::I64, vec![0], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
        ];
        let f = function(
            0,
            "cross_block",
            vec![],
            Ty::I64,
            vec![block(0, b0_insts), block(1, b1_insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        let inst0 = &m.functions[0].blocks[0].insts[0];
        assert_eq!(
            inst0.region,
            RegionId::Block(BlockId(0)),
            "a value defined in an ancestor and used in a descendant should stay in the ancestor block"
        );
    }

    #[test]
    fn ancestor_block_region_closes_at_subtree_exit() {
        let b0_insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            jump_inst(1, 1),
        ];
        let b1_insts = vec![
            inst(2, Opcode::Load, Ty::I64, vec![0], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
        ];
        let f = function(
            0,
            "cross_block",
            vec![],
            Ty::I64,
            vec![block(0, b0_insts), block(1, b1_insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        insert_region_markers(&mut m);

        let block0 = &m.functions[0].blocks[0].insts;
        let block1 = &m.functions[0].blocks[1].insts;
        assert_eq!(block0[0].opcode, Opcode::RegionOpen);
        assert_eq!(block0[0].region, RegionId::Block(BlockId(0)));
        assert!(
            !block0.iter().any(|i| i.opcode == Opcode::RegionClose),
            "ancestor region must not close before jumping into its subtree"
        );
        let close_pos = block1
            .iter()
            .position(|i| i.opcode == Opcode::RegionClose)
            .expect("subtree exit should close ancestor region");
        let return_pos = block1
            .iter()
            .position(|i| i.opcode == Opcode::Return)
            .expect("test block has return");
        assert_eq!(close_pos + 1, return_pos);
        assert_eq!(block1[close_pos].region, RegionId::Block(BlockId(0)));
    }

    #[test]
    fn branch_alloc_used_at_merge_opens_at_common_parent() {
        let b0_insts = vec![branch_inst(0, 1, 2)];
        let b1_insts = vec![
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            jump_inst(2, 3),
        ];
        let b2_insts = vec![jump_inst(3, 3)];
        let b3_insts = vec![
            inst(4, Opcode::Load, Ty::I64, vec![1], InstData::None),
            inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
        ];
        let f = function(
            0,
            "diamond_merge_use",
            vec![],
            Ty::I64,
            vec![
                block(0, b0_insts),
                block(1, b1_insts),
                block(2, b2_insts),
                block(3, b3_insts),
            ],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);
        assert_eq!(
            m.functions[0].blocks[1].insts[0].region,
            RegionId::Block(BlockId(0)),
            "a branch allocation used after merge must use the merge dominator"
        );

        insert_region_markers(&mut m);
        let block0 = &m.functions[0].blocks[0].insts;
        let block1 = &m.functions[0].blocks[1].insts;
        let block2 = &m.functions[0].blocks[2].insts;
        let block3 = &m.functions[0].blocks[3].insts;
        assert_eq!(block0[0].opcode, Opcode::RegionOpen);
        assert_eq!(block0[0].region, RegionId::Block(BlockId(0)));
        assert!(
            !block1.iter().any(|i| i.opcode == Opcode::RegionOpen),
            "the non-dominating branch must not own the region open"
        );
        assert!(
            !block2.iter().any(|i| i.opcode == Opcode::RegionClose),
            "the sibling branch must not close a region it did not open"
        );
        let close_pos = block3
            .iter()
            .position(|i| i.opcode == Opcode::RegionClose)
            .expect("merge exit should close the common-parent region");
        assert_eq!(block3[close_pos].region, RegionId::Block(BlockId(0)));
    }

    #[test]
    fn vec_push_val_routes_source_to_local_vector_region() {
        let insts = vec![
            inst(
                0,
                Opcode::Call,
                Ty::Ptr,
                vec![],
                InstData::CallExtern("__vow_vec_new".to_string()),
            ),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                2,
                Opcode::Call,
                Ty::Unit,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
        ];
        let f = function(0, "vec_push", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[0].blocks[0].insts[1].region,
            RegionId::Block(BlockId(0)),
            "a value copied into a local vector should inherit the vector target's region"
        );
    }

    #[test]
    fn vec_new_local_result_routes_to_block_region() {
        let insts = vec![
            inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
            inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
            inst(
                2,
                Opcode::Call,
                Ty::Ptr,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_new".to_string()),
            ),
            inst(
                3,
                Opcode::Call,
                Ty::I64,
                vec![2],
                InstData::CallExtern("__vow_vec_len".to_string()),
            ),
            inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
        ];
        let f = function(0, "vec_local", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[0].blocks[0].insts[2].region,
            RegionId::Block(BlockId(0))
        );
    }

    #[test]
    fn returned_vec_new_routes_to_caller_region() {
        let insts = vec![
            inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
            inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
            inst(
                2,
                Opcode::Call,
                Ty::Ptr,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_new".to_string()),
            ),
            inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
        ];
        let f = function(0, "vec_return", vec![], Ty::Ptr, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::FreshInCaller
        );
        assert_eq!(
            m.functions[0].blocks[0].insts[2].region,
            RegionId::Caller(HiddenRegionIdx(0))
        );
    }

    #[test]
    fn vec_push_into_returned_vec_lifts_source_to_outer_region() {
        let insts = vec![
            inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
            inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(8)),
            inst(
                2,
                Opcode::Call,
                Ty::Ptr,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_new".to_string()),
            ),
            inst(
                3,
                Opcode::Call,
                Ty::Ptr,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_new".to_string()),
            ),
            inst(
                4,
                Opcode::Call,
                Ty::Unit,
                vec![2, 3],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(5, Opcode::Return, Ty::Unit, vec![2], InstData::None),
        ];
        let f = function(0, "vec_push_escape", vec![], Ty::Ptr, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[0].blocks[0].insts[2].region,
            RegionId::Caller(HiddenRegionIdx(0))
        );
        assert_eq!(
            m.functions[0].blocks[0].insts[3].region,
            RegionId::Caller(HiddenRegionIdx(0))
        );
    }

    #[test]
    fn vec_push_val_into_param_vector_routes_source_to_root_without_conflict() {
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                2,
                Opcode::Call,
                Ty::Unit,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f = function(
            0,
            "vec_push_param",
            vec![Ty::Ptr],
            Ty::Unit,
            vec![block(0, insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert_eq!(m.functions[0].blocks[0].insts[1].region, RegionId::Root);
        assert!(
            m.warnings
                .iter()
                .all(|d| d.code != ErrorCode::RegionConflict),
            "extern vector stores into parameter vectors should widen to Root, not reject"
        );
    }

    #[test]
    fn fresh_in_caller_call_result_used_locally_remains_root_until_aggregate_codegen() {
        let callee = build_returning_alloc();
        let caller_insts = vec![
            inst(
                10,
                Opcode::Call,
                Ty::Ptr,
                vec![],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(11, Opcode::Load, Ty::I64, vec![10], InstData::None),
            inst(12, Opcode::Return, Ty::Unit, vec![11], InstData::None),
        ];
        let caller = function(1, "caller", vec![], Ty::I64, vec![block(0, caller_insts)]);
        let mut m = module(vec![callee, caller]);
        infer_regions(&mut m);
        let call = &m.functions[1].blocks[0].insts[0];
        assert_eq!(
            call.region,
            RegionId::Root,
            "FreshInCaller call result materialization remains conservative until aggregate hidden-region codegen is complete"
        );
    }

    #[test]
    fn fresh_aggregate_call_widens_argument_stored_in_result() {
        let callee_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                2,
                Opcode::FieldSet,
                Ty::Unit,
                vec![1, 0],
                InstData::FieldIndex(0),
            ),
            inst(3, Opcode::Return, Ty::Unit, vec![1], InstData::None),
        ];
        let callee = function(
            0,
            "wrap_arg",
            vec![Ty::Ptr],
            Ty::Ptr,
            vec![block(0, callee_insts)],
        );
        let caller_insts = vec![
            inst(
                10,
                Opcode::Call,
                Ty::Ptr,
                vec![],
                InstData::CallExtern("__vow_vec_new".to_string()),
            ),
            inst(
                11,
                Opcode::Call,
                Ty::Ptr,
                vec![10],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(12, Opcode::Return, Ty::Unit, vec![11], InstData::None),
        ];
        let caller = function(1, "caller", vec![], Ty::Ptr, vec![block(0, caller_insts)]);
        let mut m = module(vec![callee, caller]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[1].blocks[0].insts[0].region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "argument embedded in a returned fresh aggregate must follow the aggregate escape"
        );
    }

    #[test]
    fn fresh_aggregate_store_effect_preserves_embedded_argument_alias() {
        let wrap_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                2,
                Opcode::FieldSet,
                Ty::Unit,
                vec![1, 0],
                InstData::FieldIndex(0),
            ),
            inst(3, Opcode::Return, Ty::Unit, vec![1], InstData::None),
        ];
        let wrap = function(
            0,
            "wrap_arg",
            vec![Ty::Ptr],
            Ty::Ptr,
            vec![block(0, wrap_insts)],
        );
        let store_insts = vec![
            inst(10, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(11, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(
                12,
                Opcode::Call,
                Ty::Ptr,
                vec![11],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(
                13,
                Opcode::Call,
                Ty::Unit,
                vec![10, 12],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(14, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let store = function(
            1,
            "store_wrapped_arg",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, store_insts)],
        );
        let mut m = module(vec![wrap, store]);
        infer_regions(&mut m);

        assert!(
            m.functions[1].summary.store_effects.iter().any(|effect| {
                effect.target == 0 && effect.source == RegionConstraint::AliasOf(1)
            }),
            "storing a fresh aggregate into a target must also publish aliases embedded in that aggregate"
        );
    }

    #[test]
    fn nested_parameter_container_store_publishes_store_effect() {
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(
                2,
                Opcode::FieldGet,
                Ty::Ptr,
                vec![0],
                InstData::FieldIndex(0),
            ),
            inst(
                3,
                Opcode::Call,
                Ty::Unit,
                vec![2, 1],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(4, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f = function(
            0,
            "store_nested",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert!(
            m.functions[0].summary.store_effects.iter().any(|effect| {
                effect.target == 0 && effect.source == RegionConstraint::AliasOf(1)
            }),
            "stores through parameter-owned fields must publish a store effect for the owning parameter"
        );
    }

    #[test]
    fn vec_new_through_callee_store_effect_lifts_without_conflict() {
        let callee_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(
                2,
                Opcode::Call,
                Ty::Unit,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let callee = function(
            0,
            "push_arg",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, callee_insts)],
        );
        let caller_insts = vec![
            inst(10, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                11,
                Opcode::Call,
                Ty::Ptr,
                vec![],
                InstData::CallExtern("__vow_vec_new".to_string()),
            ),
            inst(
                12,
                Opcode::Call,
                Ty::Unit,
                vec![10, 11],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(13, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let caller = function(
            1,
            "caller",
            vec![Ty::Ptr],
            Ty::Unit,
            vec![block(0, caller_insts)],
        );
        let mut m = module(vec![callee, caller]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[1].blocks[0].insts[1].region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "Vec creation passed through a store-effecting callee should lift to the target parameter region"
        );
        assert!(
            m.warnings
                .iter()
                .all(|d| d.code != ErrorCode::RegionConflict),
            "routable Vec creation should not trip the block-local RegionAlloc conflict"
        );
    }

    #[test]
    fn string_from_cstr_non_escaping_allocates_in_block_region() {
        let insts = vec![
            inst(0, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
            inst(
                1,
                Opcode::Call,
                Ty::Ptr,
                vec![0],
                InstData::CallExtern("__vow_string_from_cstr".to_string()),
            ),
            inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(3, Opcode::Return, Ty::Unit, vec![2], InstData::None),
        ];
        let f = function(0, "make_string", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[0].blocks[0].insts[1].region,
            RegionId::Block(BlockId(0)),
            "String::from_cstr should be treated as a fresh heap producer"
        );
    }

    #[test]
    fn fresh_string_runtime_helpers_non_escaping_allocate_in_block_region() {
        let cases = [
            ("__vow_string_split", 2),
            ("__vow_string_trim", 1),
            ("__vow_string_to_upper", 1),
            ("__vow_string_to_lower", 1),
            ("__vow_string_replace", 3),
            ("__vow_string_join", 2),
        ];

        for (sym, arity) in cases {
            let mut insts = vec![
                inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
                inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
                inst(2, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(2)),
            ];
            let args: Vec<u32> = (0..arity).collect();
            insts.push(inst(
                3,
                Opcode::Call,
                Ty::Ptr,
                args,
                InstData::CallExtern(sym.to_string()),
            ));
            insts.push(inst(
                4,
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(0),
            ));
            insts.push(inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None));

            let f = function(
                0,
                "fresh_string_helper",
                vec![Ty::Ptr, Ty::Ptr, Ty::Ptr],
                Ty::I64,
                vec![block(0, insts)],
            );
            let mut m = module(vec![f]);
            infer_regions(&mut m);

            assert_eq!(
                m.functions[0].blocks[0].insts[3].region,
                RegionId::Block(BlockId(0)),
                "{sym} should be treated as a fresh heap producer"
            );
        }
    }

    #[test]
    fn string_substring_return_allocates_in_caller_region() {
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(2, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(1)),
            inst(
                3,
                Opcode::Call,
                Ty::Ptr,
                vec![0, 1, 2],
                InstData::CallExtern("__vow_string_substring".to_string()),
            ),
            inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
        ];
        let f = function(
            0,
            "slice_string",
            vec![Ty::Ptr],
            Ty::Ptr,
            vec![block(0, insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[0].blocks[0].insts[3].region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "returned String::substring result should be caller-region allocated"
        );
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::FreshInCaller,
            "String::substring return should publish FreshInCaller"
        );
    }

    #[test]
    fn internal_call_republishes_callee_store_effect_for_parameter_target() {
        let callee_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(
                2,
                Opcode::Call,
                Ty::Unit,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let callee = function(
            0,
            "push_arg",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, callee_insts)],
        );
        let wrapper_insts = vec![
            inst(10, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(11, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(
                12,
                Opcode::Call,
                Ty::Unit,
                vec![10, 11],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(13, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let wrapper = function(
            1,
            "wrapper",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, wrapper_insts)],
        );
        let mut m = module(vec![callee, wrapper]);
        infer_regions(&mut m);

        assert!(
            m.functions[1].summary.store_effects.iter().any(|effect| {
                effect.target == 0 && effect.source == RegionConstraint::AliasOf(1)
            }),
            "internal wrappers must republish callee store effects for their own parameter targets"
        );
    }

    #[test]
    fn fresh_aggregate_stored_into_param_publishes_field_aliases() {
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(
                2,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                3,
                Opcode::FieldSet,
                Ty::Unit,
                vec![2, 1],
                InstData::FieldIndex(0),
            ),
            inst(
                4,
                Opcode::Call,
                Ty::Unit,
                vec![0, 2],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(5, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f = function(
            0,
            "store_aggregate_with_param_field",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert!(
            m.functions[0].summary.store_effects.iter().any(|effect| {
                effect.target == 0 && effect.source == RegionConstraint::AliasOf(1)
            }),
            "fresh aggregates stored into parameter containers must publish parameter aliases in their fields"
        );
    }

    #[test]
    fn field_set_into_local_alloc_keeps_source_in_target_block() {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                2,
                Opcode::FieldSet,
                Ty::Unit,
                vec![0, 1],
                InstData::FieldIndex(0),
            ),
            inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
        ];
        let f = function(0, "field_store", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        let source = &m.functions[0].blocks[0].insts[1];
        assert_eq!(
            source.region,
            RegionId::Block(BlockId(0)),
            "source stored into a local target should inherit the target's block region"
        );
    }

    #[test]
    fn store_into_local_alloc_keeps_source_in_target_block() {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
            inst(3, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
        ];
        let f = function(0, "store_local", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        let source = &m.functions[0].blocks[0].insts[1];
        assert_eq!(
            source.region,
            RegionId::Block(BlockId(0)),
            "source stored into a local target should inherit the target's block region"
        );
    }

    #[test]
    fn store_into_escaping_local_target_widens_source_to_caller_region() {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        let f = function(0, "store_escape", vec![], Ty::Ptr, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        let target = &m.functions[0].blocks[0].insts[0];
        let source = &m.functions[0].blocks[0].insts[1];
        assert_eq!(
            target.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "returned target should be caller-region allocated"
        );
        assert_eq!(
            source.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "source stored into an escaping target must inherit the target's caller region"
        );
    }

    #[test]
    fn field_get_escape_widens_container_and_stored_source() {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                2,
                Opcode::FieldSet,
                Ty::Unit,
                vec![0, 1],
                InstData::FieldIndex(0),
            ),
            inst(
                3,
                Opcode::FieldGet,
                Ty::Ptr,
                vec![0],
                InstData::FieldIndex(0),
            ),
            inst(4, Opcode::Return, Ty::Unit, vec![3], InstData::None),
        ];
        let f = function(0, "field_escape", vec![], Ty::Ptr, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert_eq!(
            m.functions[0].blocks[0].insts[0].region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "container read by an escaping FieldGet must be widened"
        );
        assert_eq!(
            m.functions[0].blocks[0].insts[1].region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "stored source must follow the widened container"
        );
    }

    #[test]
    fn store_into_param_routes_source_to_caller_region() {
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f = function(
            0,
            "store_param",
            vec![Ty::Ptr],
            Ty::Unit,
            vec![block(0, insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        let source = &m.functions[0].blocks[0].insts[1];
        assert_eq!(
            source.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "source stored into a parameter target must outlive this function"
        );
    }

    #[test]
    fn upsilon_source_inherits_phi_target_block_lca() {
        let phi_id = 10u32;
        let b0_insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                1,
                Opcode::Upsilon,
                Ty::Unit,
                vec![0],
                InstData::PhiTarget(InstId(phi_id)),
            ),
            inst(
                2,
                Opcode::Jump,
                Ty::Unit,
                vec![],
                InstData::JumpTarget(BlockId(1)),
            ),
        ];
        let b1_insts = vec![
            inst(phi_id, Opcode::Phi, Ty::Ptr, vec![], InstData::None),
            inst(11, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(12, Opcode::Return, Ty::Unit, vec![11], InstData::None),
        ];
        let f = function(
            0,
            "upsilon_target",
            vec![],
            Ty::I64,
            vec![block(0, b0_insts), block(1, b1_insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        let source = &m.functions[0].blocks[0].insts[0];
        assert_eq!(
            source.region,
            RegionId::Block(BlockId(0)),
            "source feeding a Phi in a descendant block should use the concrete LCA"
        );
    }

    #[test]
    fn store_into_root_pinned_target_routes_source_to_root() {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                1,
                Opcode::Call,
                Ty::Ptr,
                vec![0],
                InstData::CallExtern("__vow_string_pin_to_root".to_string()),
            ),
            inst(
                2,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(3, Opcode::Store, Ty::Unit, vec![1, 2], InstData::None),
            inst(4, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(5, Opcode::Return, Ty::Unit, vec![4], InstData::None),
        ];
        let f = function(0, "store_root", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        let source = &m.functions[0].blocks[0].insts[2];
        assert_eq!(
            source.region,
            RegionId::Root,
            "source stored into a root-pinned target must route to Root"
        );
    }

    #[test]
    fn callee_store_effect_routes_source_arg_to_target_marker() {
        let callee_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let callee = function(
            0,
            "copy_param",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, callee_insts)],
        );

        let caller_insts = vec![
            inst(
                4,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                5,
                Opcode::Call,
                Ty::Ptr,
                vec![4],
                InstData::CallExtern("__vow_string_pin_to_root".to_string()),
            ),
            inst(
                6,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                7,
                Opcode::Call,
                Ty::Unit,
                vec![5, 6],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(8, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let caller = function(1, "caller", vec![], Ty::Unit, vec![block(0, caller_insts)]);
        let mut m = module(vec![callee, caller]);
        infer_regions(&mut m);

        let source = &m.functions[1].blocks[0].insts[2];
        assert_eq!(
            source.region,
            RegionId::Root,
            "caller source arg should inherit the callee store-effect target marker"
        );
    }

    #[test]
    fn callee_store_effect_into_escaping_target_widens_source_to_caller_region() {
        let callee_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let callee = function(
            0,
            "copy_param",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, callee_insts)],
        );

        let caller_insts = vec![
            inst(
                4,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                5,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                6,
                Opcode::Call,
                Ty::Unit,
                vec![4, 5],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(7, Opcode::Return, Ty::Unit, vec![4], InstData::None),
        ];
        let caller = function(1, "caller", vec![], Ty::Ptr, vec![block(0, caller_insts)]);
        let mut m = module(vec![callee, caller]);
        infer_regions(&mut m);

        let target = &m.functions[1].blocks[0].insts[0];
        let source = &m.functions[1].blocks[0].insts[1];
        assert_eq!(
            target.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "returned target should be caller-region allocated"
        );
        assert_eq!(
            source.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "caller source arg should inherit later escapes from the store-effect target"
        );
    }

    #[test]
    fn extern_vec_push_into_escaping_target_widens_source_to_caller_region() {
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                2,
                Opcode::Call,
                Ty::Unit,
                vec![0, 1],
                InstData::CallExtern("__vow_vec_push_val".to_string()),
            ),
            inst(3, Opcode::Return, Ty::Unit, vec![0], InstData::None),
        ];
        let f = function(
            0,
            "extern_vec_push_escape",
            vec![],
            Ty::Ptr,
            vec![block(0, insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        let target = &m.functions[0].blocks[0].insts[0];
        let source = &m.functions[0].blocks[0].insts[1];
        assert_eq!(
            target.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "returned extern-mutated target should be caller-region allocated"
        );
        assert_eq!(
            source.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "source stored by an extern container mutator must inherit target escapes"
        );
    }

    #[test]
    fn explicit_arena_vec_push_val_into_escaping_target_widens_source_to_caller_region() {
        let insts = vec![
            inst(0, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(
                1,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                2,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                3,
                Opcode::Call,
                Ty::Unit,
                vec![0, 1, 2],
                InstData::CallExtern("__vow_vec_push_val_in_arena".to_string()),
            ),
            inst(4, Opcode::Return, Ty::Unit, vec![1], InstData::None),
        ];
        let f = function(
            0,
            "extern_arena_vec_push_escape",
            vec![],
            Ty::Ptr,
            vec![block(0, insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        let target = &m.functions[0].blocks[0].insts[1];
        let source = &m.functions[0].blocks[0].insts[2];
        assert_eq!(
            target.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "returned explicit-arena extern-mutated target should be caller-region allocated"
        );
        assert_eq!(
            source.region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "source stored by explicit-arena Vec::push_val must inherit target escapes"
        );
    }

    #[test]
    fn string_push_byte_parameter_receiver_publishes_growth_store_effect() {
        let insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                1,
                Opcode::ConstI64,
                Ty::I64,
                vec![],
                InstData::ConstI64(120),
            ),
            inst(
                2,
                Opcode::Call,
                Ty::Unit,
                vec![0, 1],
                InstData::CallExtern("__vow_string_push_byte".to_string()),
            ),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f = function(
            0,
            "grow_string_param",
            vec![Ty::Ptr],
            Ty::Unit,
            vec![block(0, insts)],
        );
        let mut m = module(vec![f]);
        infer_regions(&mut m);

        assert!(
            m.functions[0].summary.store_effects.iter().any(|effect| {
                effect.target == 0 && effect.source == RegionConstraint::ConstantGlobal
            }),
            "String::push_byte on a parameter receiver must request that receiver's hidden arena"
        );
    }

    #[test]
    fn diagnostic_code_is_region_conflict_pascal() {
        // External-schema guard: Debug-format MUST yield "RegionConflict"
        // exactly. The DiagnosticJson layer relies on this.
        assert_eq!(format!("{:?}", ErrorCode::RegionConflict), "RegionConflict");
        let _ = Severity::Error; // unused-import suppressor
    }

    #[test]
    fn empty_module_does_not_panic() {
        let mut m = module(vec![]);
        infer_regions(&mut m);
        assert!(m.functions.is_empty());
        assert!(m.warnings.is_empty());
    }

    #[test]
    fn build_call_graph_deduplicates_and_ignores_out_of_range_targets() {
        let f0 = function(
            0,
            "caller",
            vec![],
            Ty::Unit,
            vec![block(
                0,
                vec![
                    inst(
                        0,
                        Opcode::Call,
                        Ty::Unit,
                        vec![],
                        InstData::CallTarget(FuncId(1)),
                    ),
                    inst(
                        1,
                        Opcode::Call,
                        Ty::Unit,
                        vec![],
                        InstData::CallTarget(FuncId(1)),
                    ),
                    inst(
                        2,
                        Opcode::Call,
                        Ty::Unit,
                        vec![],
                        InstData::CallTarget(FuncId(99)),
                    ),
                    return_unit_inst(3),
                ],
            )],
        );
        let f1 = function(
            1,
            "callee",
            vec![],
            Ty::Unit,
            vec![block(0, vec![return_unit_inst(4)])],
        );
        let graph = build_call_graph(&module(vec![f0, f1]));
        assert_eq!(graph, vec![vec![1], vec![]]);
    }

    #[test]
    fn tarjan_sccs_groups_cycles_and_leaves_first() {
        let mut sccs = tarjan_sccs(&[vec![1], vec![2], vec![0, 3], vec![]]);
        assert_eq!(sccs.remove(0), vec![3]);
        assert_eq!(sccs.remove(0), vec![0, 1, 2]);
        assert!(sccs.is_empty());
    }

    /// A callee that stores its second arg into its first arg publishes
    /// `(0, AliasOf(1))` store-effect. A caller passing `(some_param,
    /// fresh_alloc)` exhibits the alloc→param-via-callee shape that
    /// `check_store_conflict` rejects (Phase 5 partial detection).
    #[test]
    /// Codex Option 1.5 (issue #314): a fresh `RegionAlloc` passed through a
    /// callee whose store-effect routes it into a parameter container MUST
    /// NOT trip `RegionConflict`. Region inference's must_outlive widening
    /// (`call_store_effects_collect_regions` equivalent) places the alloc at
    /// the caller's region; `check_store_conflicts_post_inference` consults
    /// that inferred region rather than the IR opcode.
    ///
    /// Mirrors the existing `vec_new_through_callee_store_effect_lifts_without_conflict`
    /// test for the `Vec::new()` case; before Codex 1.5, the two cases gave
    /// different verdicts because the old check filtered on `RegionAlloc`
    /// opcode while the routing semantics were identical.
    fn region_alloc_through_callee_store_effect_lifts_without_conflict() {
        let f0_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f0 = function(
            0,
            "copy_param",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, f0_insts)],
        );

        let f1_insts = vec![
            inst(4, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(
                5,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(
                6,
                Opcode::Call,
                Ty::Unit,
                vec![4, 5],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(7, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f1 = function(
            1,
            "caller",
            vec![Ty::Ptr],
            Ty::Unit,
            vec![block(0, f1_insts)],
        );

        let mut m = module(vec![f0, f1]);
        infer_regions(&mut m);

        // No RegionConflict — the routing through the callee's store-effect
        // satisfies the constraint by widening the alloc's region.
        assert!(
            m.warnings
                .iter()
                .all(|d| d.code != ErrorCode::RegionConflict),
            "routable RegionAlloc should not trip the conflict check; warnings: {:?}",
            m.warnings
        );
        // The alloc's inferred region must lift to Caller(0).
        assert_eq!(
            m.functions[1].blocks[0].insts[1].region,
            RegionId::Caller(HiddenRegionIdx(0)),
            "RegionAlloc passed through a store-effecting callee should lift to caller"
        );
    }

    // Removed by Codex Option 1.5 (issue #314): the test
    // `region_conflict_uses_callee_function_source_file` asserted that the
    // `source_file` field of a RegionConflict diagnostic points at the
    // analyzing caller's file. After Option 1.5, the conflict path is
    // reached only when region inference's must_outlive marker propagation
    // does NOT widen the source — but that propagation runs unconditionally
    // for every AliasOf store-effect at every call site (see handle_inst
    // around line 1696 and check_store_conflicts_post_inference). The path
    // is therefore unreachable from synthetic per-function IR and the test
    // could not be re-fixtured. The source_file plumbing remains exercised
    // by other diagnostics (Linear*, Mismatch*) and by integration tests.

    /// Same callee shape as the conflict test, but caller passes two
    /// parameters (no fresh alloc). Cross-param store stays Phase-5-deferred,
    /// so no diagnostic should fire.
    #[test]
    fn region_conflict_not_emitted_for_param_to_param_store() {
        let f0_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f0 = function(
            0,
            "copy_param",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, f0_insts)],
        );

        let f1_insts = vec![
            inst(4, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(5, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(
                6,
                Opcode::Call,
                Ty::Unit,
                vec![4, 5],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(7, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f1 = function(
            1,
            "caller",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, f1_insts)],
        );

        let mut m = module(vec![f0, f1]);
        infer_regions(&mut m);

        let conflicts: Vec<_> = m
            .warnings
            .iter()
            .filter(|d| d.code == ErrorCode::RegionConflict)
            .filter(|d| !d.message.starts_with("internal compiler error"))
            .collect();
        assert!(
            conflicts.is_empty(),
            "did not expect RegionConflict for param→param store, got: {:?}",
            conflicts
        );
    }

    /// Boundary: source is `ConstStr` (ConstantGlobal). Even when target is a
    /// param, this must not emit — `.rodata`-backed values outlive any
    /// caller-frame region.
    #[test]
    fn region_conflict_not_emitted_for_constant_into_param() {
        let f0_insts = vec![
            inst(0, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(1, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(1)),
            inst(2, Opcode::Store, Ty::Unit, vec![0, 1], InstData::None),
            inst(3, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f0 = function(
            0,
            "copy_param",
            vec![Ty::Ptr, Ty::Ptr],
            Ty::Unit,
            vec![block(0, f0_insts)],
        );

        let f1_insts = vec![
            inst(4, Opcode::GetArg, Ty::Ptr, vec![], InstData::ArgIndex(0)),
            inst(5, Opcode::ConstStr, Ty::Ptr, vec![], InstData::ConstStr(0)),
            inst(
                6,
                Opcode::Call,
                Ty::Unit,
                vec![4, 5],
                InstData::CallTarget(FuncId(0)),
            ),
            inst(7, Opcode::Return, Ty::Unit, vec![], InstData::None),
        ];
        let f1 = function(
            1,
            "caller",
            vec![Ty::Ptr],
            Ty::Unit,
            vec![block(0, f1_insts)],
        );

        let mut m = module(vec![f0, f1]);
        infer_regions(&mut m);

        let conflicts: Vec<_> = m
            .warnings
            .iter()
            .filter(|d| d.code == ErrorCode::RegionConflict)
            .filter(|d| !d.message.starts_with("internal compiler error"))
            .collect();
        assert!(
            conflicts.is_empty(),
            "did not expect RegionConflict for ConstStr→param store, got: {:?}",
            conflicts
        );
    }
}
