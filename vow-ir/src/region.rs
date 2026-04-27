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
//!    sets. Heap-producing instructions (today: `Opcode::RegionAlloc`) get
//!    their `region` field set from the LUB.
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
/// On `RegionConflict`, iteration completes anyway so internal `Uninit`
/// state never leaks into `Function.summary` (spec §4.3).
pub fn infer_regions(module: &mut Module, source_file: &str) {
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
                    source_file,
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

    check_linear_regions(module, source_file, &mut diagnostics);

    module.warnings.extend(diagnostics);
}

fn check_linear_regions(module: &Module, source_file: &str, diagnostics: &mut Vec<Diagnostic>) {
    for func in &module.functions {
        check_function_linear_regions(func, source_file, diagnostics);
    }
}

fn check_function_linear_regions(
    func: &Function,
    source_file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
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
                    emit_live_linear_errors(
                        func,
                        source_file,
                        &live,
                        &inst_lookup,
                        &mut emitted,
                        diagnostics,
                    );
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
        // A LinearPtr-typed Phi is its own fresh origin: each predecessor
        // arm transferred its origin into the Phi via Upsilon (handled below),
        // so leaks must be reported against the merged Phi value rather than
        // the per-arm origins.
        live.insert(inst.id);
    }
    if inst.opcode == Opcode::Upsilon
        && let Some(&arg) = inst.args.first()
        && inst_lookup
            .get(&arg)
            .is_some_and(|(_, a)| a.ty == Ty::LinearPtr)
    {
        // Path-local transfer: the Upsilon hands the arm's origin to the
        // target Phi. Removing it here keeps the analysis path-conservative —
        // if a sibling arm fails to transfer, the unmatched origin survives
        // the merge and is reported as RegionLinear. Note: the Upsilon's
        // own `ty` is `Ty::Unit` (it produces no value), so we test the
        // arg's type instead.
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
    // A LinearPtr Phi is treated as its own leaf origin (see
    // `apply_linear_transfer`): per-arm origins are transferred into the Phi
    // by their Upsilons, and only the Phi itself is left in `live`. Tracing
    // back through Phi/Upsilon arms here would re-introduce the
    // path-insensitive double-removal bug (returning a Phi would consume
    // every possible source origin, masking a leak on the unselected arm).
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

fn emit_live_linear_errors(
    func: &Function,
    source_file: &str,
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
                file: source_file.to_string(),
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

/// Insert `RegionOpen` / `RegionClose` markers around basic blocks whose
/// region is non-empty (spec §3.5). Must run AFTER `infer_regions` so that
/// every `RegionAlloc` inst carries its inferred `region: RegionId`.
///
/// Emission rules (Phase 4 / S3, criterion 1 of §3.5):
/// - For each function, collect every distinct `BlockId B` appearing as
///   `RegionId::Block(B)` on any inst's `region` field within the function.
/// - For each such `B`, prepend `RegionOpen { region: Block(B) }` to basic
///   block `B`'s instruction list and insert `RegionClose { region:
///   Block(B) }` immediately before the block's terminator.
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

        let mut next_id = next_inst_id(func);
        for block in &mut func.blocks {
            if !block_regions.contains(&block.id) {
                continue;
            }
            // Pick a span for the synthesised markers from a real inst in
            // the block, falling back to the empty span.
            let span = block
                .insts
                .first()
                .map(|i| i.origin)
                .unwrap_or(Span { start: 0, len: 0 });

            let open = Inst {
                id: InstId(next_id),
                opcode: Opcode::RegionOpen,
                ty: Ty::Unit,
                args: vec![],
                data: InstData::None,
                origin: span,
                region: RegionId::Block(block.id),
            };
            next_id += 1;
            let close = Inst {
                id: InstId(next_id),
                opcode: Opcode::RegionClose,
                ty: Ty::Unit,
                args: vec![],
                data: InstData::None,
                origin: span,
                region: RegionId::Block(block.id),
            };
            next_id += 1;

            // Insert RegionClose just before the terminator. Every
            // well-formed block ends with a terminal opcode; spec §12.3
            // requires close on every exit edge, which inside a single
            // basic block reduces to "right before the terminator."
            let term_pos = block
                .insts
                .iter()
                .position(|i| i.opcode.is_terminal())
                .unwrap_or(block.insts.len());
            block.insts.insert(term_pos, close);
            // RegionOpen at the very start of the block.
            block.insts.insert(0, open);
        }
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
    source_file: &str,
    summaries: &[InternalSummary],
    region_map: &mut BTreeMap<InstId, RegionId>,
    diagnostics: &mut Vec<Diagnostic>,
) -> InternalSummary {
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

    // Forward sweep collecting use-set contributions.
    for block in &func.blocks {
        for inst in &block.insts {
            handle_inst(
                source_file,
                inst,
                block.id,
                &inst_lookup,
                summaries,
                &mut must_outlive,
                &mut summary,
                diagnostics,
            );
        }
    }

    // Compute return-region contributions in a deep pass, walking Phi/Call
    // origins with the current summaries fixed. This is the canonical
    // path; the in-handle_inst Return shortcut only flags virtual-caller
    // escape on the must_outlive set.
    summary.return_region = compute_return_region(func, &inst_lookup, &phi_arms, summaries);

    // Compute LUB-derived RegionId for every heap-producing inst.
    for block in &func.blocks {
        for inst in &block.insts {
            if !is_heap_producing(inst) {
                continue;
            }
            let markers = must_outlive.get(&inst.id).cloned().unwrap_or_default();
            let region_id = lub_to_region_id(&markers, block.id);
            region_map.insert(inst.id, region_id);
        }
    }

    summary
}

fn is_heap_producing(inst: &Inst) -> bool {
    matches!(inst.opcode, Opcode::RegionAlloc)
}

/// Handle one instruction: contribute to `must_outlive` and to the
/// function's tightening `summary`.
#[allow(clippy::too_many_arguments)]
fn handle_inst(
    source_file: &str,
    inst: &Inst,
    _block_id: BlockId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    summaries: &[InternalSummary],
    must_outlive: &mut BTreeMap<InstId, BTreeSet<MustOutliveMarker>>,
    summary: &mut InternalSummary,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match inst.opcode {
        Opcode::Return => {
            // The returned value escapes to the virtual caller. Mark it so
            // the inst.region populate pass tags any RegionAlloc that flows
            // into the return as Caller(0).
            if let Some(&arg_id) = inst.args.first() {
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
            // target. For Phase 3 the target's region is approximated by
            // tracing it back to its definition (parameter, alloc, etc.).
            //
            // For now we conservatively add the target's *containing block*
            // region as a marker on the source, surfacing
            // RegionConflict only when the source's region statically
            // strictly outlives the target.
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
                if let Some(target_block) = trace_target_block(target_id, inst_lookup) {
                    add_marker(must_outlive, source_id, MustOutliveMarker::Block(target_block));
                }
                // If the target traces to a parameter, record a store_effect.
                if let Some(target_param) = trace_param(target_id, inst_lookup) {
                    let source_origin = trace_origin(source_id, inst_lookup);
                    let source_constraint = origin_to_constraint(&source_origin);
                    summary
                        .store_effects
                        .insert((target_param, InternalReturnRegion::Published(source_constraint)));
                }
            }
        Opcode::Call => {
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
                // the callee, the caller's argument at position `target`
                // receives values constrained by `source`. If `source` aliases
                // a callee parameter, propagate the corresponding caller
                // argument's must_outlive contribution back to the target arg.
                for (target_param, source_constraint) in &cs.store_effects {
                    let target_idx = *target_param as usize;
                    if target_idx >= inst.args.len() {
                        continue;
                    }
                    let target_arg_id = inst.args[target_idx];
                    match source_constraint {
                        InternalReturnRegion::Published(RegionConstraint::AliasOf(p)) => {
                            // The callee writes argument-at-position-p into argument-at-position-target.
                            let p_idx = *p as usize;
                            if p_idx < inst.args.len() {
                                let source_arg_id = inst.args[p_idx];
                                // target must outlive source — if conflict, emit.
                                check_store_conflict(
                                    source_file,
                                    target_arg_id,
                                    source_arg_id,
                                    inst,
                                    inst_lookup,
                                    diagnostics,
                                );
                            }
                        }
                        InternalReturnRegion::Published(RegionConstraint::FreshInCaller) => {
                            // Callee allocates fresh and stores into target. Caller
                            // must place the alloc in target's region; mark target
                            // for downstream LUB.
                            add_marker(must_outlive, target_arg_id, MustOutliveMarker::VirtualCaller);
                        }
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

fn add_marker(
    must_outlive: &mut BTreeMap<InstId, BTreeSet<MustOutliveMarker>>,
    inst_id: InstId,
    marker: MustOutliveMarker,
) {
    must_outlive.entry(inst_id).or_default().insert(marker);
}

/// After a call returns AliasOf(j), the result inst is *the same value* as
/// arg[j] from the must_outlive standpoint: any marker on the result must
/// also apply to arg[j], and vice versa.
///
/// **Currently inert in the forward sweep.** `handle_inst` invokes this at
/// the point a `Call` is processed, but no downstream consumer (e.g., a
/// `Return` that marks the call result `VirtualCaller`) has run yet — so
/// `must_outlive[result_id]` is always empty here and the function returns
/// without doing anything. The return-region summary is correctly
/// computed by the separate `compute_return_region` deep pass below, so
/// this no-op does not affect Phase 3 outputs.
///
/// TODO(Phase 5 / issue #200): once store-effect propagation lands, run
/// `propagate_alias` either as a backward sweep after the must_outlive
/// forward pass completes, or as a dedicated pass that walks
/// `Opcode::Call` results once their downstream uses are visible. Either
/// approach makes the function load-bearing for AliasOf-driven
/// arena-routing decisions on call results.
fn propagate_alias(
    must_outlive: &mut BTreeMap<InstId, BTreeSet<MustOutliveMarker>>,
    result_id: InstId,
    arg_id: InstId,
) {
    let result_markers = must_outlive.get(&result_id).cloned().unwrap_or_default();
    if result_markers.is_empty() {
        return;
    }
    must_outlive
        .entry(arg_id)
        .or_default()
        .extend(result_markers);
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
fn lub_to_region_id(markers: &BTreeSet<MustOutliveMarker>, defining_block: BlockId) -> RegionId {
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
    // ── Phase 4 (CURRENT BEHAVIOUR) ────────────────────────────────────
    // All pure-block-marker / empty-set cases are routed to `Root`.
    // The classification logic below (Phase 9 target) is NOT active
    // yet — we cannot safely emit `Block(_)` until the `must_outlive`
    // pass tracks every use of the value (regular `FieldGet`/`Load`,
    // non-store-effect call args, Pizlo-Phi uses). An untracked read
    // in a sibling block would consume freed memory after
    // `RegionClose`. `Root` is conservatively safe — process-lifetime
    // storage strictly outlives any concrete block LUB.
    //
    // ── Phase 9 (#204) target classification, FOR REFERENCE ────────────
    // Once `must_outlive` is complete, the bullets below describe the
    // intended behaviour:
    //   - Empty set → defining block (its narrowest possible region).
    //   - Single marker == defining block → that block.
    //   - Anything else (different single block, multiple blocks) →
    //     `Root` (no block-tree dominance info; returning
    //     `Block(other)` for an alloc in a different basic block
    //     would risk use-after-free when `other`'s arena closes).
    //
    // The match below is the Phase 9 dispatch, retained as a
    // structural placeholder; for now we discard `blocks` and return
    // `Root` unconditionally.
    let _ = (blocks, defining_block);
    RegionId::Root
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

fn trace_target_block(
    id: InstId,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
) -> Option<BlockId> {
    inst_lookup.get(&id).map(|(b, _)| *b)
}

fn trace_param(id: InstId, inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>) -> Option<u32> {
    let (_, inst) = inst_lookup.get(&id)?;
    if let (Opcode::GetArg, InstData::ArgIndex(i)) = (inst.opcode, &inst.data) {
        Some(*i)
    } else {
        None
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
/// **Self-hosted gap (#204):** the self-hosted `compiler/region.vow` has
/// no equivalent of this function today. The gap is currently masked
/// because the self-hosted port also defers store-effect *inference*
/// (see `compiler/region.vow` `analyze_function`'s "Phase 3 minimal"
/// note), so `store_effects` is always empty on the self-hosted path
/// and no call site reaches this check. Both pieces — the inference
/// and the conflict emission — must land together on the self-hosted
/// side before the differential gate stays meaningful for programs
/// that exercise this diagnostic.
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
/// Deferred to Phase 9 (#204):
///   * Full block-tree LUB requires a dominator tree (`lub_to_region_id`
///     currently routes undecidable block-marker sets to `Root` — the
///     pre-existing §4.4 compliance gap shipped with Phase 4).
///   * Cross-param without published spanning effect (`source = Param(p)`,
///     `target = Param(q)`, `p != q`): needs the caller's full summary,
///     which is built up forward through this same pass.
///   * Phi-of-mixed-origins: descend into upsilon arms and reject when
///     the joined origin set spans incompatible regions.
// Multi-module limitation (#254): `source_file` is the *root* file path,
// not the per-function source. `module_loader::merge_modules` collects
// items from every imported module into a single AST without rebasing
// spans, and `lower_module` tags the whole merged IR with the root path,
// so a function from `lib.vow` whose span points into `lib.vow` gets
// labelled as `main.vow` here. Tracked in #254 for the structural fix
// (per-Function `source_file` field). Single-module builds — including
// every test on the corpus today — get the correct file label.
fn check_store_conflict(
    source_file: &str,
    target_arg_id: InstId,
    source_arg_id: InstId,
    call_inst: &Inst,
    inst_lookup: &BTreeMap<InstId, (BlockId, &Inst)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let source_origin = trace_origin(source_arg_id, inst_lookup);
    let target_origin = trace_origin(target_arg_id, inst_lookup);

    if !matches!(source_origin, ValueOrigin::RegionAlloc) {
        return;
    }
    let ValueOrigin::Param(_) = target_origin else {
        return;
    };

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
        // Fault is in the analysing function's body: it passes a block-local
        // alloc where the callee's store-effect demands a longer-lived
        // region. `Blame::Caller` would implicate the *caller of the
        // analysing function*, which isn't right here.
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
        infer_regions(&mut m, "test.vow");
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::AliasOf(0)
        );
    }

    #[test]
    fn rodata_literal_return() {
        let mut m = module(vec![build_const_str_return()]);
        infer_regions(&mut m, "test.vow");
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::ConstantGlobal
        );
    }

    #[test]
    fn returned_alloc_escapes_to_caller() {
        let mut m = module(vec![build_returning_alloc()]);
        infer_regions(&mut m, "test.vow");
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
        infer_regions(&mut m, "test.vow");
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
        infer_regions(&mut m, "test.vow");
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
        infer_regions(&mut m, "test.vow");
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
        infer_regions(&mut m, "test.vow");
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
        infer_regions(&mut m, "test.vow");

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
        infer_regions(&mut m, "test.vow");
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
        infer_regions(&mut m, "test.vow");
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
    fn local_alloc_used_only_locally() {
        // Allocation that does NOT escape (no return, no store-into-param):
        // its LUB stays in the local block. Conservative Phase 3 leaves it
        // at Block(0). We assert the inst.region was set (not Root/default).
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
        infer_regions(&mut m, "test.vow");
        let inst0 = &m.functions[0].blocks[0].insts[0];
        // The alloc has no must_outlive contributions in Phase 3 minimal —
        // it falls back to Root. This is conservative; Phase 4 will tighten.
        assert!(matches!(inst0.region, RegionId::Root | RegionId::Block(_)));
        assert_eq!(
            m.functions[0].summary.return_region,
            RegionConstraint::ConstantGlobal,
            "scalar return must be ConstantGlobal even when function allocates"
        );
    }

    #[test]
    fn markers_inserted_for_non_empty_block_region() {
        // The marker insertion pass keys off `inst.region == Block(_)`. We
        // hand-tag the alloc to exercise the marker pass directly — the
        // region pass's own `Block(_)` emission is currently disabled (see
        // `local_alloc_assigned_to_root_until_use_set_is_complete`).
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
        // Skip infer_regions — it would overwrite the hand-set region with
        // `Root` while Block emission is deferred.
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
    fn no_markers_for_empty_block_region() {
        // Empty-region elision (spec §3.5): a function whose only alloc
        // escapes (`Caller(0)` summary) has no `Block(_)` region in itself,
        // so no RegionOpen/Close must be inserted.
        let f = build_returning_alloc();
        let mut m = module(vec![f]);
        infer_regions(&mut m, "test.vow");
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
    fn local_alloc_assigned_to_root_until_use_set_is_complete() {
        // Phase 4 / S3 (deferred to Phase 9 / #204): non-escaping local
        // allocs would ideally land in `Block(defining_block)`, but
        // `must_outlive` currently misses `FieldGet` / `Load` / non-store-
        // effect call args. Without a complete use-set we cannot safely
        // close the block's arena while uses might survive. The pass falls
        // back to `Root` until `must_outlive` is extended.
        let insts = vec![
            inst(
                0,
                Opcode::RegionAlloc,
                Ty::Ptr,
                vec![],
                InstData::AllocSize { size: 16, align: 8 },
            ),
            inst(1, Opcode::ConstI64, Ty::I64, vec![], InstData::ConstI64(0)),
            inst(2, Opcode::Return, Ty::Unit, vec![1], InstData::None),
        ];
        let f = function(0, "local", vec![], Ty::I64, vec![block(0, insts)]);
        let mut m = module(vec![f]);
        infer_regions(&mut m, "test.vow");
        let inst0 = &m.functions[0].blocks[0].insts[0];
        assert_eq!(inst0.region, RegionId::Root);
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
        infer_regions(&mut m, "test.vow");
        assert!(m.functions.is_empty());
        assert!(m.warnings.is_empty());
    }

    /// A callee that stores its second arg into its first arg publishes
    /// `(0, AliasOf(1))` store-effect. A caller passing `(some_param,
    /// fresh_alloc)` exhibits the alloc→param-via-callee shape that
    /// `check_store_conflict` rejects (Phase 5 partial detection).
    #[test]
    fn region_conflict_alloc_into_param_via_callee_store_effect() {
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
        infer_regions(&mut m, "test.vow");

        let conflicts: Vec<_> = m
            .warnings
            .iter()
            .filter(|d| d.code == ErrorCode::RegionConflict)
            .filter(|d| !d.message.starts_with("internal compiler error"))
            .collect();
        // Exactly one diagnostic per violating call site — `infer_regions`
        // runs `analyze_function` in an SCC fixed-point loop and would
        // accumulate one Diagnostic per round without per-iteration
        // bucketing. See the `iter_diagnostics` mechanism in
        // `infer_regions`.
        assert_eq!(
            conflicts.len(),
            1,
            "expected exactly one RegionConflict (no SCC-iteration dupes), \
             got {} from warnings: {:?}",
            conflicts.len(),
            m.warnings
        );
        let c = conflicts[0];
        assert_eq!(c.severity, Severity::Error);
        assert_eq!(c.blame, Blame::Callee);
        assert!(!c.hints.is_empty(), "expected at least one hint");
        // RegionConflict diagnostics are user-visible; the file field
        // must be populated, not the historical `String::new()` placeholder.
        assert_eq!(c.primary.file, "test.vow");
        for s in &c.secondary {
            assert_eq!(s.file, "test.vow");
        }
    }

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
        infer_regions(&mut m, "test.vow");

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
        infer_regions(&mut m, "test.vow");

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
