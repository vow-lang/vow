# `vow complexity` gate calibration

_Generated 2026-06-18 by scripts/complexity_calibrate.py over `compiler/*.vow` (25 files)._

_Retention: `current-baseline` for the `complexity-calibration` stream. Replace
this snapshot in the same PR as the next committed complexity calibration
snapshot unless a reviewer reclassifies it as `release-evidence`._

> Validation target is comprehensibility / refactor priority, NOT correctness
> (docs/design Part 4). Correctness is the job of contracts + tests.

## Threshold calibration

- Functions scored: **1183** (from 21/25 files; 4 skipped — don't compile standalone)
- Over threshold (`score > 80`): **166** (14.0%) — target 5–15% → **OK**
- complexity_score p50/p90/max: 11 / 90 / 100
- cognitive p50/p90/max: 0 / 14 / 634
- nloc p50/p90/max: 6 / 47 / 4021

Anchors: cog=15, nloc=60. If the rate is far off target, adjust these (not the 0-100 scale).

## Beats-size check

- Spearman(score, nloc) = **0.990** → score is **size-dominated (cognitive differentiates the tail)**.
- Median cognitive is **0**: ~half the functions have no control flow, so size correctly drives their score. The cognitive factor only reorders the complex tail (below); judge the metric there, not on the global correlation.

Functions the score prioritizes ABOVE their size rank (tangled, not just long):

| function | line | score | cognitive | nloc |
|---|--:|--:|--:|--:|
| `map_model_receiver_arg` | 556 | 46 | 7 | 10 |
| `cq_byte_is_alpha` | 10610 | 19 | 3 | 3 |
| `c_nondet_suffix` | 511 | 51 | 8 | 11 |
| `vec_model_receiver_arg` | 567 | 51 | 8 | 11 |
| `extern_store_edge_count` | 4165 | 40 | 6 | 9 |
| `write_effects` | 275 | 34 | 5 | 8 |
| `status_str` | 10 | 34 | 5 | 8 |
| `extern_store_target_pos` | 4175 | 34 | 5 | 8 |

Functions the score DEPRIORITIZES below their size rank (long but flat):

| function | line | score | cognitive | nloc |
|---|--:|--:|--:|--:|
| `ir_function_new` | 366 | 27 | 0 | 20 |
| `skill_support_paths` | 6416 | 24 | 0 | 18 |
| `skill_support_contents` | 10397 | 24 | 0 | 18 |
| `arena_new` | 109 | 24 | 0 | 18 |
| `ir_inst_set_region` | 341 | 23 | 0 | 17 |
| `branch_inst_with_targets` | 3992 | 20 | 0 | 15 |
| `ir_inst_new` | 325 | 20 | 0 | 15 |
| `parse_const_def` | 196 | 20 | 0 | 15 |

## Worst functions by score

| function | line | score | cyclomatic | cognitive | nloc |
|---|--:|--:|--:|--:|--:|
| `is_modelable` | 209 | 100 | 136 | 61 | 129 |
| `collect_modelled_vars` | 689 | 100 | 35 | 103 | 94 |
| `emit_inst` | 922 | 100 | 149 | 166 | 372 |
| `emit_string_op` | 1474 | 100 | 59 | 116 | 296 |
| `emit_c_function` | 1961 | 100 | 84 | 216 | 369 |
| `check_expr_inner` | 956 | 100 | 190 | 488 | 719 |
| `collect_calls_in_expr` | 1676 | 100 | 30 | 40 | 161 |
| `clif_routed_extern_symbol` | 194 | 100 | 46 | 65 | 134 |
| `cog_expr` | 449 | 100 | 35 | 53 | 133 |
| `hal_expr` | 1011 | 100 | 35 | 43 | 153 |
| `ec_name` | 170 | 100 | 26 | 350 | 79 |
| `opcode_name` | 84 | 100 | 116 | 115 | 118 |
| `lex` | 163 | 100 | 66 | 190 | 269 |
| `lower_expr` | 841 | 100 | 272 | 634 | 2048 |
| `lower_function_vow` | 3271 | 100 | 19 | 52 | 134 |
