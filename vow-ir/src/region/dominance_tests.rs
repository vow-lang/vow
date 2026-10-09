// Differential pin for the block dominator tree (#658).
//
// `BlockTree::from_function` computes immediate dominators by intersecting
// dominator sets over the DFS-forward graph. The self-hosted compiler's
// `build_block_parent` (compiler/region.vow) reaches the same tree with an
// incremental-LCA fixpoint, which keeps O(n) memory where this pass keeps
// O(n^2). The two only agree because the forward graph is a DAG: parents start
// as DFS-discovery predecessors, so every dominator of a block is always one
// of its tree ancestors, and at the fixpoint every ancestor dominates it.
//
// `compiler/tests/test_region_dominance.vow` runs the same corpus through the
// self-hosted pass. The golden tables below are duplicated there verbatim and
// the exhaustive / LCG corpora are tied together by the pinned digests, so a
// divergence on either side fails a test instead of silently moving a region
// open marker.

use super::BlockTree;
use crate::types::{
    BasicBlock, BlockId, FuncId, Function, Inst, InstData, InstId, Opcode, RegionId, RegionSummary,
    Ty,
};
use std::collections::HashMap;
use vow_syntax::span::Span;

const DIGEST_MOD: i64 = 72_057_594_037_927_936;
const DIGEST_SEED: i64 = 7;
const LCG_SEED: i64 = 20_240_607;

const DIGEST_EXHAUSTIVE_4: i64 = 46_900_383_860_154_851;
const DIGEST_LCG_5: i64 = 7_220_846_967_842_058;
const DIGEST_LCG_6: i64 = 3_581_780_127_454_291;

#[derive(Clone, Copy)]
enum Term {
    Ret,
    Jump(usize),
    Branch(usize, usize),
}

fn terminator(id: u32, term: Term, ids: &[u32]) -> Inst {
    let (opcode, data) = match term {
        Term::Ret => (Opcode::Return, InstData::None),
        Term::Jump(t) => (Opcode::Jump, InstData::JumpTarget(BlockId(ids[t]))),
        Term::Branch(a, b) => (
            Opcode::Branch,
            InstData::BranchTargets {
                then_block: BlockId(ids[a]),
                else_block: BlockId(ids[b]),
            },
        ),
    };
    Inst {
        id: InstId(id),
        opcode,
        ty: Ty::Unit,
        args: vec![],
        data,
        origin: Span { start: 0, len: 0 },
        region: RegionId::Root,
    }
}

// `terms[i]` is the terminator of logical block `i`, which carries block id
// `ids[i]`; block 0 is the entry. Targets are logical indices.
fn build(terms: &[Term], ids: &[u32]) -> Function {
    let blocks = terms
        .iter()
        .enumerate()
        .map(|(i, &term)| BasicBlock {
            id: BlockId(ids[i]),
            insts: vec![terminator(u32::try_from(i).unwrap(), term, ids)],
        })
        .collect();
    Function {
        id: FuncId(0),
        name: "f".to_string(),
        param_names: vec![],
        params: vec![],
        return_ty: Ty::Unit,
        effects: vec![],
        vows: vec![],
        blocks,
        local_names: HashMap::new(),
        summary: RegionSummary::default(),
        source_file: "test.vow".to_string(),
    }
}

// Parent of every block id in `0..n`: `-1` for a root, `-2` for an absent id.
fn parent_table(func: &Function, n: usize) -> Vec<i64> {
    let tree = BlockTree::from_function(func);
    (0..n)
        .map(|id| {
            let id = BlockId(u32::try_from(id).unwrap());
            match tree.parent.get(&id) {
                Some(Some(parent)) => i64::from(parent.0),
                Some(None) => -1,
                None => -2,
            }
        })
        .collect()
}

// Every present block must reach a root through existing parents, without a
// cycle; a broken tree is reported here instead of as an opaque digest drift.
fn assert_tree_shape(table: &[i64], context: &str) {
    for (id, &parent) in table.iter().enumerate() {
        let mut cur = parent;
        let mut steps = 0;
        while cur >= 0 {
            let next = table[usize::try_from(cur).unwrap()];
            assert_ne!(next, -2, "{context}: block {id} has absent ancestor {cur}");
            cur = next;
            steps += 1;
            assert!(steps <= table.len(), "{context}: block {id} parent cycle");
        }
    }
}

fn fold(digest: i64, parent: i64) -> i64 {
    (digest * 61 + (parent + 3)) % DIGEST_MOD
}

// ---------- Named CFG shapes -------------------------------------------------

// Spec: `id>succ,succ;...`, entry first. Expected: `id>idom;...`, `r` = root.
fn rows(spec: &str) -> Vec<Vec<i64>> {
    spec.split(';')
        .map(|row| {
            row.split(['>', ','])
                .filter(|tok| !tok.is_empty())
                .map(|tok| if tok == "r" { -1 } else { tok.parse().unwrap() })
                .collect()
        })
        .collect()
}

fn check_named(name: &str, cfg: &str, expected: &str) {
    let rows = rows(cfg);
    let ids: Vec<u32> = rows.iter().map(|r| u32::try_from(r[0]).unwrap()).collect();
    let by_id: HashMap<i64, usize> = rows.iter().enumerate().map(|(i, r)| (r[0], i)).collect();
    let terms: Vec<Term> = rows
        .iter()
        .map(|row| {
            let target = |t: i64| by_id.get(&t).copied().unwrap_or(usize::MAX);
            match row.len() {
                1 => Term::Ret,
                2 => Term::Jump(target(row[1])),
                3 => Term::Branch(target(row[1]), target(row[2])),
                other => panic!("{name}: unsupported row width {other}"),
            }
        })
        .collect();
    let func = build_with_dangling(&terms, &ids);
    let tree = BlockTree::from_function(&func);
    let expected = self::rows(expected);
    assert_eq!(tree.parent.len(), ids.len(), "{name}: block count");
    for row in &expected {
        let want = (row[1] >= 0).then(|| BlockId(u32::try_from(row[1]).unwrap()));
        let got = tree.parent.get(&BlockId(u32::try_from(row[0]).unwrap()));
        assert_eq!(got, Some(&want), "{name}: parent of block {}", row[0]);
    }
}

// A target that names no block (`usize::MAX`) is emitted as block id 9999 so
// the dangling-edge case reaches the pass unchanged.
fn build_with_dangling(terms: &[Term], ids: &[u32]) -> Function {
    let mut padded: Vec<u32> = ids.to_vec();
    let dangling = padded.len();
    padded.push(9999);
    let remap = |t: usize| if t == usize::MAX { dangling } else { t };
    let terms: Vec<Term> = terms
        .iter()
        .map(|&term| match term {
            Term::Ret => Term::Ret,
            Term::Jump(t) => Term::Jump(remap(t)),
            Term::Branch(a, b) => Term::Branch(remap(a), remap(b)),
        })
        .collect();
    build(&terms, &padded)
}

#[test]
fn golden_straight_line() {
    check_named("straight", "0>1;1>2;2>3;3", "0>r;1>0;2>1;3>2");
}

#[test]
fn golden_diamond() {
    check_named("diamond", "0>1,2;1>3;2>3;3", "0>r;1>0;2>0;3>0");
}

#[test]
fn golden_triangle() {
    check_named("triangle", "0>1,2;1>2;2", "0>r;1>0;2>0");
}

#[test]
fn golden_nested_diamond() {
    check_named(
        "nested diamond",
        "0>1,2;1>3,4;3>5;4>5;5>6;2>6;6",
        "0>r;1>0;2>0;3>1;4>1;5>1;6>0",
    );
}

#[test]
fn golden_join_with_unequal_predecessor_depths() {
    check_named(
        "unequal arms",
        "0>1,4;1>2;2>3;3>5;4>5;5",
        "0>r;1>0;2>1;3>2;4>0;5>0",
    );
    check_named(
        "unequal arms with inner branch",
        "0>1,5;1>2,3;2>4;3>4;4>6;5>6;6",
        "0>r;1>0;2>1;3>1;4>1;5>0;6>0",
    );
}

#[test]
fn golden_while_loop() {
    check_named("while", "0>1;1>2,3;2>1;3", "0>r;1>0;2>1;3>1");
}

#[test]
fn golden_nested_loops() {
    check_named(
        "nested loops",
        "0>1;1>2,6;2>3;3>4,5;4>3;5>1;6",
        "0>r;1>0;2>1;3>2;4>3;5>3;6>1",
    );
}

#[test]
fn golden_loop_with_break() {
    check_named("break", "0>1;1>2,4;2>3,4;3>1;4", "0>r;1>0;2>1;3>2;4>1");
}

#[test]
fn golden_loop_with_continue() {
    check_named(
        "continue",
        "0>1;1>2,5;2>3,4;3>1;4>1;5",
        "0>r;1>0;2>1;3>2;4>2;5>1",
    );
}

#[test]
fn golden_self_loop() {
    check_named("self loop", "0>1,2;1>1,2;2", "0>r;1>0;2>0");
}

#[test]
fn golden_multi_return() {
    check_named("multi return", "0>1,2;1;2", "0>r;1>0;2>0");
}

#[test]
fn golden_same_target_branch_and_dangling_target() {
    check_named("same target", "0>1,1;1", "0>r;1>0");
    check_named("dangling target", "0>1,9999;1", "0>r;1>0");
}

#[test]
fn golden_branch_point_numbered_above_the_join() {
    check_named(
        "high branch point",
        "5>3,4;3>1;4>1;1>2;2",
        "5>r;3>5;4>5;1>5;2>1",
    );
    check_named(
        "high entry, two joins",
        "9>1,2;1>3;2>3;3>4,5;4>6;5>6;6",
        "9>r;1>9;2>9;3>9;4>3;5>3;6>3",
    );
}

#[test]
fn golden_non_contiguous_ids() {
    check_named(
        "sparse ids",
        "10>20,30;20>40;30>40;40",
        "10>r;20>10;30>10;40>10",
    );
}

#[test]
fn golden_unreachable_blocks_become_their_own_roots() {
    check_named("unreachable root", "0;1>2;2", "0>r;1>r;2>1");
    check_named(
        "unreachable into reachable",
        "0>3;1>2;2>3;3",
        "0>r;1>r;2>1;3>0",
    );
    check_named(
        "two unreachable roots share a target",
        "0;1>3;2>3;3",
        "0>r;1>r;2>r;3>1",
    );
}

#[test]
fn golden_irreducible_cycle_follows_dfs_order() {
    check_named("irreducible", "0>1,2;1>2;2>1;3", "0>r;1>0;2>0;3>r");
    check_named(
        "irreducible with join",
        "0>1,2;1>3;2>3;3>1,2",
        "0>r;1>0;2>0;3>1",
    );
}

// ---------- Corpora tied to the self-hosted pass by digest -------------------

fn pairs(n: usize) -> Vec<(usize, usize)> {
    (0..n)
        .flat_map(|a| ((a + 1)..n).map(move |b| (a, b)))
        .collect()
}

fn decode(n: usize, digit: usize, pairs: &[(usize, usize)]) -> Term {
    match digit {
        0 => Term::Ret,
        d if d <= n => Term::Jump(d - 1),
        d => {
            let (a, b) = pairs[d - n - 1];
            Term::Branch(a, b)
        }
    }
}

fn base_for(n: usize) -> usize {
    1 + n + n * (n - 1) / 2
}

// Folds the parent table of one graph under identity ids, then under reversed
// ids (`n - 1 - logical`, entry stays first in block order).
fn fold_graph(digest: i64, terms: &[Term], identity: &[u32], reversed: &[u32]) -> i64 {
    let n = terms.len();
    let mut digest = digest;
    for ids in [identity, reversed] {
        let table = parent_table(&build(terms, ids), n);
        assert_tree_shape(&table, "corpus graph");
        digest = table.into_iter().fold(digest, fold);
    }
    digest
}

fn id_maps(n: usize) -> (Vec<u32>, Vec<u32>) {
    let identity: Vec<u32> = (0..n).map(|i| u32::try_from(i).unwrap()).collect();
    let reversed: Vec<u32> = identity.iter().rev().copied().collect();
    (identity, reversed)
}

fn exhaustive_digest(n: usize) -> i64 {
    let pairs = pairs(n);
    let base = base_for(n);
    let (identity, reversed) = id_maps(n);
    let total = base.pow(u32::try_from(n).unwrap());
    let mut digest = DIGEST_SEED;
    for graph in 0..total {
        let mut rest = graph;
        let terms: Vec<Term> = (0..n)
            .map(|_| {
                let digit = rest % base;
                rest /= base;
                decode(n, digit, &pairs)
            })
            .collect();
        digest = fold_graph(digest, &terms, &identity, &reversed);
    }
    digest
}

fn lcg_digest(n: usize, count: usize) -> i64 {
    let pairs = pairs(n);
    let base = i64::try_from(base_for(n)).unwrap();
    let (identity, reversed) = id_maps(n);
    let mut state = LCG_SEED;
    let mut digest = DIGEST_SEED;
    for _ in 0..count {
        let terms: Vec<Term> = (0..n)
            .map(|_| {
                state = (state * 1_103_515_245 + 12_345) % 2_147_483_648;
                let digit = usize::try_from((state / 65_536) % base).unwrap();
                decode(n, digit, &pairs)
            })
            .collect();
        digest = fold_graph(digest, &terms, &identity, &reversed);
    }
    digest
}

#[test]
fn exhaustive_four_block_graphs_match_the_pinned_digest() {
    assert_eq!(exhaustive_digest(4), DIGEST_EXHAUSTIVE_4);
}

#[test]
fn random_five_block_graphs_match_the_pinned_digest() {
    assert_eq!(lcg_digest(5, 1000), DIGEST_LCG_5);
}

#[test]
fn random_six_block_graphs_match_the_pinned_digest() {
    assert_eq!(lcg_digest(6, 1000), DIGEST_LCG_6);
}
