# Vericoding Comparison: Vow vs Dafny / Verus / Lean

## Provenance

| Field | Value |
|-------|-------|
| **Date** | 2026-03-22 |
| **Commit** | ac51718 |
| **Model** | claude-sonnet-4-20250514 (Anthropic) |
| **Temperature** | 0.0 |
| **Max tokens** | 8192 |
| **Compiler** | Self-hosted (`./vowc`, verified fixed-point binary) |
| **Max CEGIS iterations** | 5 (HumanEval), 2 (original suite) |
| **ESBMC version** | 8.0.0 (64-bit x86_64 linux) |
| **Suites** | Vow original (40 benchmarks) + HumanEval (67 benchmarks) |

## Reproducing

```bash
# Prerequisites: ANTHROPIC_API_KEY in .env, ESBMC on PATH
set -a && source .env && set +a

# Validate all 103 non-stretch references
uv run --project bench bench/run.py validate-references --compiler self-hosted

# Run all benchmarks (original Vow suite + HumanEval)
uv run --project bench bench/run.py run \
  --model claude-sonnet-4-20250514 --compiler self-hosted

# Generate report from results
uv run --project bench bench/run.py report --run-id <run-id>
```

---

## Summary

### Original Vow Suite (40 benchmarks, 36 non-Stretch)

| Metric | Value |
|--------|-------|
| **Total** | **36/36 (100%)** |
| Easy (15) | 15/15 (100%) |
| Medium (15) | 15/15 (100%) |
| Hard (6) | 6/6 (100%) |
| Mean CEGIS iterations | 1.0 |
| Stretch (4) | 2/4 verified |

### HumanEval Suite (67 benchmarks)

| Metric | Value |
|--------|-------|
| **Total** | **66/67 (98.5%)** |
| Easy (19) | 19/19 (100%) |
| Medium (38) | 37/38 (97.4%) |
| Hard (10) | 10/10 (100%) |
| Mean CEGIS iterations | 1.32 |
| Multi-iteration benchmarks | 12/67 (17.9%) |

### Combined

| Metric | Value |
|--------|-------|
| **Total** | **102/103 (99.0%)** |
| Mean CEGIS iterations | 1.21 |

---

## Comparison Table

Dafny, Verus/Rust, and Lean pass rates are from the Vericoding benchmark paper
(arxiv.org/abs/2509.22908). These tools were evaluated on HumanEval-162 with
Dafny-native specifications. The Vow numbers below are on 67 HumanEval
benchmarks translated from the same source, with Vow-native contracts.

| Language/Model | HumanEval Rate | Original Suite Rate | Combined Rate |
|----------------|---------------|-------------------|--------------|
| **Vow (Sonnet 4)** | **66/67 (98.5%)** | **36/36 (100%)** | **102/103 (99.0%)** |
| Dafny (paper) | 82% | — | — |
| Verus/Rust (paper) | 44% | — | — |
| Lean (paper) | 27% | — | — |

---

## Contract Fidelity Breakdown (HumanEval)

Contract fidelity measures how precisely the Vow specification captures the
intended correctness property:

- **Exact**: the ensures clause fully specifies the function's output for all
  valid inputs. A verified implementation is provably correct.
- **Partial**: the ensures clause captures key properties (bounds, ordering,
  element membership) but does not fully determine the output.
- **Weak**: the ensures clause captures only basic properties (e.g., output
  length). Verification proves something, but less than full correctness.

| Fidelity | Verified | Total | Rate |
|----------|----------|-------|------|
| **Exact** | **31** | **32** | **96.9%** |
| **Partial** | **34** | **34** | **100%** |
| **Weak** | **1** | **1** | **100%** |
| **All** | **66** | **67** | **98.5%** |

The single failure (HE062 derivative, exact fidelity) is an ESBMC convergence
issue on loop invariants involving Vec mutation, not an algorithmic error.

---

## CEGIS Iteration Analysis

The CEGIS (Counterexample-Guided Inductive Synthesis) loop feeds ESBMC
counterexamples back to the LLM for iterative repair. Most benchmarks verify
on the first attempt.

| CEGIS Iterations | Count | Percentage |
|-----------------|-------|-----------|
| 1 (first attempt) | 55 | 82.1% |
| 2 | 4 | 6.0% |
| 3 | 5 | 7.5% |
| 4 | 1 | 1.5% |
| 5 | 2 | 3.0% |

Total wall-clock time for the HumanEval suite: 511s (8.5 minutes).
Token usage: ~1.1M input tokens, ~30K output tokens.

---

## Known Limitations and Caveats

The following caveats should be considered when interpreting these results.

### 1. ESBMC Model Bounds

The verification backend models collection types with fixed-size arrays:

| Type | C Model | Max Elements |
|------|---------|-------------|
| `Vec<T>` | `int64_t data[128]` + `int len` | 128 |
| `String` | `int8_t data[256]` + `int len` | 256 bytes |
| `HashMap<K,V>` | `int64_t keys[64]` + `int64_t vals[64]` + `int len` | 64 entries |

Loop unwind bound: 10 iterations (configurable per-benchmark via `--unwind N`).

The 99.0% pass rate is on benchmarks that operate within these bounds. Programs
requiring larger collections or deeper loop nesting may exceed the verifier's
model capacity. This is inherent to bounded model checking — ESBMC proves
properties up to a bound, not for all possible inputs.

### 2. Spec Expressiveness

Vow contracts cannot express quantifiers (`forall`, `exists`). Ensures clauses
use bounded expressions over concrete values. Phase 21.1 added spec function
calls (pure user-defined functions in ensures clauses), which significantly
improved contract expressiveness for algorithmic properties.

The contract fidelity categories acknowledge this: "Partial" and "Weak"
benchmarks have specifications that could be tighter with quantifiers. The 32
"Exact" benchmarks have fully deterministic specifications despite this
limitation.

### 3. Benchmark Provenance

- **Original suite (40 benchmarks)**: authored specifically for Vow. These
  exercise Vow's type system and verification features directly.
- **HumanEval suite (67 benchmarks)**: translated from HumanEval-162 Dafny
  benchmarks (the same source used by the Vericoding paper). Translation was
  semi-automated: `bench/translate_dafny.py` handles type mapping
  (`int`→`i64`, `seq<int>`→`Vec<i64>`), then specifications and skeletons
  were manually reviewed.
- **Coverage**: 67/162 HumanEval tasks are represented. The 95 excluded tasks
  require types not in Vow (strings, reals, sequences of strings,
  multi-return) or specifications beyond bounded model checking.

### 4. Nested Vec Nondeterminism

The C model for Vec uses `__VERIFIER_nondet_long()` for reads, meaning two
reads of the same index may return different values in the ESBMC model. This
affects benchmarks with nested loops over the same Vec. The nondeterminism is
conservative (ESBMC may reject correct programs) but means some verification
failures are false negatives, not true bugs.

---

## Failure Analysis

### HE062 (derivative) — exact fidelity, medium difficulty

The model produced correct algorithmic code (`result.push(i * xs[i])` for `i`
from 1 to `n`) but could not converge on loop invariants that ESBMC could
verify within 5 iterations. Each attempt tried different invariant formulations
(`i <= xs.len()`, `i < xs.len()`, `i <= n` with cached length), but the
verifier found counterexamples for the upper bound invariant. The root cause is
that `xs.len()` after `result.push()` mutations is not trivially provable as
stable in the C model.

---

## Per-Benchmark Results — HumanEval Suite

| ID | Name | Difficulty | Fidelity | Status | Iters | Time (s) |
|----|------|------------|----------|--------|-------|----------|
| HE003 | below_zero | medium | weak | pass | 1 | 3.8 |
| HE005 | insert_delimiter | medium | partial | pass | 3 | 14.9 |
| HE009 | rolling_max | medium | partial | pass | 2 | 9.8 |
| HE013 | gcd | medium | partial | pass | 1 | 39.5 |
| HE024 | largest_divisor | medium | partial | pass | 1 | 6.3 |
| HE025 | factorize | hard | partial | pass | 1 | 12.8 |
| HE026 | count | medium | exact | pass | 1 | 4.2 |
| HE030 | get_positive | medium | exact | pass | 1 | 3.0 |
| HE031 | is_prime | medium | exact | pass | 1 | 5.2 |
| HE034 | sort_seq | medium | partial | pass | 1 | 5.2 |
| HE035 | find_max_element | medium | exact | pass | 1 | 3.3 |
| HE036 | count7 | easy | exact | pass | 1 | 3.4 |
| HE038 | decode_cyclic | hard | partial | pass | 1 | 4.1 |
| HE040 | triples_sum_to_zero | medium | exact | pass | 1 | 3.6 |
| HE041 | car_race_collision | easy | exact | pass | 1 | 2.2 |
| HE042 | incr_list | medium | partial | pass | 1 | 4.0 |
| HE043 | pairs_sum_to_zero | medium | exact | pass | 1 | 3.8 |
| HE046 | fib4 | easy | partial | pass | 1 | 4.3 |
| HE049 | modp | medium | exact | pass | 1 | 4.8 |
| HE052 | check_below_threshold | medium | exact | pass | 1 | 3.8 |
| HE053 | add | easy | exact | pass | 1 | 2.1 |
| HE055 | fib | easy | partial | pass | 1 | 6.0 |
| HE057 | is_monotonic | medium | exact | pass | 1 | 3.8 |
| HE059 | largest_prime_factor | medium | partial | pass | 2 | 19.7 |
| HE060 | sum_to_n | easy | exact | pass | 1 | 2.6 |
| HE062 | derivative | medium | exact | **FAIL** | 5 | 14.3 |
| HE063 | fibfib | easy | partial | pass | 1 | 4.4 |
| HE068 | pluck | hard | partial | pass | 3 | 12.2 |
| HE069 | search | medium | exact | pass | 1 | 5.5 |
| HE072 | will_it_fly | medium | exact | pass | 4 | 21.4 |
| HE073 | smallest_change | medium | partial | pass | 1 | 3.8 |
| HE075 | is_multiply_prime | easy | partial | pass | 3 | 28.6 |
| HE076 | is_simple_power | easy | partial | pass | 1 | 4.3 |
| HE077 | cube_root | easy | exact | pass | 1 | 3.0 |
| HE083 | starts_one_ends | easy | partial | pass | 1 | 7.7 |
| HE084 | solve | easy | exact | pass | 1 | 4.5 |
| HE085 | add | medium | exact | pass | 1 | 3.8 |
| HE088 | sort_array | medium | exact | pass | 3 | 18.9 |
| HE094 | skjkasdkd | hard | partial | pass | 1 | 6.6 |
| HE096 | count_up_to | hard | partial | pass | 1 | 4.3 |
| HE097 | multiply | easy | exact | pass | 1 | 2.9 |
| HE100 | make_a_pile | medium | partial | pass | 1 | 3.4 |
| HE102 | choose_num | easy | exact | pass | 1 | 3.8 |
| HE104 | unique_digits | hard | partial | pass | 1 | 6.1 |
| HE106 | f | easy | partial | pass | 1 | 5.9 |
| HE108 | count_nums | medium | exact | pass | 1 | 5.3 |
| HE109 | move_one_ball | hard | partial | pass | 5 | 25.1 |
| HE114 | min_sub_array_sum | medium | partial | pass | 1 | 4.2 |
| HE116 | sort_array | medium | partial | pass | 1 | 7.2 |
| HE120 | sort_seq | hard | partial | pass | 1 | 5.1 |
| HE121 | solution | medium | exact | pass | 1 | 3.9 |
| HE122 | add_elements | medium | exact | pass | 1 | 5.8 |
| HE123 | get_odd_collatz | hard | partial | pass | 1 | 7.1 |
| HE126 | check_valid_list | medium | exact | pass | 3 | 19.0 |
| HE130 | tribonacci | medium | partial | pass | 1 | 19.2 |
| HE132 | is_nested | medium | partial | pass | 1 | 5.6 |
| HE135 | can_arrange | medium | partial | pass | 1 | 3.8 |
| HE138 | is_equal_to_sum_even | easy | exact | pass | 1 | 2.9 |
| HE139 | special_factorial | easy | partial | pass | 1 | 4.3 |
| HE142 | sum_squares | medium | exact | pass | 1 | 4.0 |
| HE145 | order_by_points | medium | partial | pass | 1 | 8.2 |
| HE146 | special_filter | medium | partial | pass | 1 | 5.7 |
| HE147 | get_max_triples | easy | exact | pass | 2 | 12.0 |
| HE150 | x_or_y | easy | exact | pass | 1 | 4.8 |
| HE152 | compare | medium | exact | pass | 1 | 4.6 |
| HE159 | eat | medium | exact | pass | 1 | 3.5 |
| HE163 | generate_integers | hard | partial | pass | 2 | 8.0 |

## Per-Benchmark Results — Original Vow Suite

| ID | Name | Difficulty | Status | Iters | Time (s) |
|----|------|------------|--------|-------|----------|
| E01 | absolute_value | easy | pass | 1 | 2.6 |
| E02 | max_of_two | easy | pass | 1 | 2.5 |
| E03 | min_of_two | easy | pass | 1 | 3.7 |
| E04 | clamp | easy | pass | 1 | 3.0 |
| E05 | safe_divide | easy | pass | 1 | 3.2 |
| E06 | safe_subtract | easy | pass | 1 | 2.4 |
| E07 | bounded_add | easy | pass | 1 | 4.5 |
| E08 | sign | easy | pass | 1 | 3.9 |
| E09 | is_even | easy | pass | 1 | 3.7 |
| E10 | safe_modulo | easy | pass | 1 | 5.1 |
| E11 | double | easy | pass | 1 | 2.4 |
| E12 | power_of_two | easy | pass | 1 | 3.6 |
| E13 | fibonacci_bounded | easy | pass | 1 | 3.2 |
| E14 | midpoint | easy | pass | 1 | 3.3 |
| E15 | checked_multiply | easy | pass | 1 | 2.7 |
| M01 | binary_search | medium | pass | 1 | 40.7 |
| M02 | vec_fill | medium | pass | 1 | 3.1 |
| M03 | vec_sum | medium | pass | 1 | 4.3 |
| M04 | vec_find | medium | pass | 1 | 3.5 |
| M05 | vec_count | medium | pass | 1 | 4.9 |
| M06 | vec_max | medium | pass | 1 | 4.1 |
| M07 | count_steps | medium | pass | 1 | 3.5 |
| M08 | map_insert_lookup | medium | pass | 1 | 2.6 |
| M09 | map_update | medium | pass | 1 | 3.9 |
| M10 | map_multi_insert | medium | pass | 1 | 3.8 |
| M11 | bounded_counter | medium | pass | 1 | 6.0 |
| M12 | swap_pair | medium | pass | 1 | 3.8 |
| M13 | gcd | medium | pass | 1 | 20.5 |
| M14 | selection_min | medium | pass | 1 | 3.7 |
| M15 | collatz_bounded | medium | pass | 1 | 3.4 |
| H01 | stack_push_pop | hard | pass | 1 | 4.6 |
| H02 | geometry_area | hard | pass | 1 | 4.5 |
| H03 | bounded_queue | hard | pass | 1 | 5.1 |
| H05 | state_machine | hard | pass | 1 | 4.2 |
| H06 | matrix_ops | hard | pass | 1 | 5.7 |
| H08 | interval_overlap | hard | pass | 1 | 6.1 |

### Stretch Benchmarks (not counted in rates)

| ID | Name | Status | Iters |
|----|------|--------|-------|
| H04 | sorted_insert | verified | 2 |
| H07 | ring_buffer | verify_failed | 5 |
| H09 | tokenizer | verified | 2 |
| H10 | expression_eval | verify_failed | 5 |

---

## Methodology

### Benchmark Translation

67 HumanEval benchmarks were translated from the Vericoding paper's
HumanEval-162 Dafny benchmark set. The translation pipeline:

1. **Triage** (`bench/triage_humaneval.py`): 162 tasks classified by type
   compatibility — 73 translatable (int/bool/seq<int> signatures only),
   34 maybe (string/char), 45 skip (real, seq<string>, multi-return).
2. **Auto-translate** (`bench/translate_dafny.py`): Dafny method signatures
   converted to Vow (`int`→`i64`, `seq<int>`→`Vec<i64>`), specifications
   translated to Vow contracts.
3. **Manual review**: each benchmark's spec, skeleton, and reference
   implementation reviewed for correctness.
4. **Verification**: all 103 non-stretch reference implementations verified
   with `./vowc verify`.

### CEGIS Protocol

Each benchmark is a single LLM conversation:
1. **System prompt**: ~35KB of Vow skill documentation (grammar, contracts,
   CLI, errors, examples).
2. **User prompt**: benchmark specification (`spec.md`) + incomplete skeleton
   (`skeleton.vow`).
3. **LLM response**: complete Vow implementation.
4. **Verification**: `./vowc verify` runs ESBMC on the implementation.
5. **If failed**: ESBMC counterexample JSON fed back as next user message.
6. **Repeat** steps 3–5 up to 5 iterations.

Temperature: 0.0 for reproducibility. No agent tools — direct API calls only.

### Verification Backend

ESBMC 8.0.0 with bounded model checking. The verification pipeline:
- Vow IR → C source emission (with ESBMC intrinsics)
- ESBMC runs with `--unwind 10 --no-bounds-check --no-pointer-check --64`
- Counterexamples mapped back to source via `Origin` metadata
- Structured JSON output with blame (Caller/Callee) and captured variable values

Collection type models use fixed-size arrays (see Caveats section).
Verification caching (FNV-1a content hash) avoids redundant ESBMC invocations.
