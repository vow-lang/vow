# HumanEval Vericoding Benchmark Results — 2026-03-15

## Provenance

- **Date:** 2026-03-15
- **Commit:** 741407dc874ac3a4bed3e3e3164717f53b0785dd
- **Model:** claude-sonnet-4-20250514 (Anthropic)
- **Temperature:** 0.0
- **Max tokens:** 8192
- **Compiler:** Self-hosted (`./vowc`, verified fixed-point binary)
- **Suite:** HumanEval (67 benchmarks adapted from HumanEval-162)
- **Max CEGIS iterations:** 5
- **Runner:** `uv run --project bench bench/run.py run --model claude-sonnet-4-20250514 --compiler self-hosted --suite humaneval --run-id humaneval-2026-03-15`
- **Results file:** `bench/results/humaneval-2026-03-15/claude-sonnet-4-20250514.json`

## Reproducing

```bash
# 1. Ensure API key
set -a && source .env && set +a

# 2. Validate references (all 103 should pass)
uv run --project bench bench/run.py validate-references --compiler self-hosted

# 3. Run the full HumanEval suite
uv run --project bench bench/run.py run \
  --model claude-sonnet-4-20250514 \
  --compiler self-hosted \
  --suite humaneval \
  --run-id humaneval-2026-03-15

# 4. Generate report
uv run --project bench bench/run.py report --run-id humaneval-2026-03-15
```

## Summary

| Metric | Value |
|--------|-------|
| **Total** | **66/67 (98.5%)** |
| Easy (19) | 19/19 (100%) |
| Medium (38) | 37/38 (97.4%) |
| Hard (10) | 10/10 (100%) |
| Exact fidelity | 31/32 (96.9%) |
| Partial fidelity | 34/34 (100%) |
| Weak fidelity | 1/1 (100%) |
| Mean CEGIS iterations | 1.32 |
| Max CEGIS iterations | 5 |

## Comparison Table

| Language/Model | Easy | Medium | Hard | Total | Rate |
|----------------|------|--------|------|-------|------|
| Dafny (paper) | — | — | — | — | 82% |
| **Vow (Sonnet 4, HumanEval)** | **19/19** | **37/38** | **10/10** | **66/67** | **98.5%** |
| Vow (Sonnet 4, original suite) | 15/15 | 15/15 | 6/6 | 36/36 | 100% |
| Verus/Rust (paper) | — | — | — | — | 44% |
| Lean (paper) | — | — | — | — | 27% |

## Contract Fidelity Breakdown

| Fidelity | Verified | Total | Rate |
|----------|----------|-------|------|
| Exact | 31 | 32 | 96.9% |
| Partial | 34 | 34 | 100% |
| Weak | 1 | 1 | 100% |

## Failure Analysis

**HE062 (derivative)** — exact fidelity, medium difficulty. Failed after 5 CEGIS iterations.

The model produced correct algorithmic code (`result.push(i * xs[i])` for `i` from 1 to `n`) but
could not converge on loop invariants that ESBMC could verify. Each iteration attempted different
invariant formulations (`i <= xs.len()`, `i < xs.len()`, `i <= n` with cached length), but the
verifier consistently found counterexamples for the upper bound invariant. The underlying issue is
that `xs.len()` after `result.push()` mutations is not trivially provable as stable across
iterations in the C model.

## Per-Benchmark Results

| ID | Name | Difficulty | Fidelity | Status | Iters | Time (s) |
|----|------|------------|----------|--------|-------|----------|
| HE003 | below_zero | easy | weak | pass | 1 | 3.8 |
| HE005 | insert_delimiter | easy | partial | pass | 3 | 14.9 |
| HE009 | rolling_max | easy | partial | pass | 2 | 9.8 |
| HE013 | gcd | easy | partial | pass | 1 | 39.5 |
| HE024 | largest_divisor | medium | partial | pass | 1 | 6.3 |
| HE025 | factorize | easy | partial | pass | 1 | 12.8 |
| HE026 | count | medium | exact | pass | 1 | 4.2 |
| HE030 | get_positive | medium | exact | pass | 1 | 3.0 |
| HE031 | is_prime | easy | exact | pass | 1 | 5.2 |
| HE034 | sort_seq | medium | partial | pass | 1 | 5.2 |
| HE035 | find_max_element | medium | exact | pass | 1 | 3.3 |
| HE036 | count7 | medium | exact | pass | 1 | 3.4 |
| HE038 | decode_cyclic | medium | partial | pass | 1 | 4.1 |
| HE040 | triples_sum_to_zero | medium | exact | pass | 1 | 3.6 |
| HE041 | car_race_collision | easy | exact | pass | 1 | 2.2 |
| HE042 | incr_list | easy | partial | pass | 1 | 4.0 |
| HE043 | pairs_sum_to_zero | medium | exact | pass | 1 | 3.8 |
| HE046 | fib4 | medium | partial | pass | 1 | 4.3 |
| HE049 | modp | easy | exact | pass | 1 | 4.8 |
| HE052 | check_below_threshold | medium | exact | pass | 1 | 3.8 |
| HE053 | add | easy | exact | pass | 1 | 2.1 |
| HE055 | fib | medium | partial | pass | 1 | 6.0 |
| HE057 | is_monotonic | medium | exact | pass | 1 | 3.8 |
| HE059 | largest_prime_factor | medium | partial | pass | 2 | 19.7 |
| HE060 | sum_to_n | easy | exact | pass | 1 | 2.6 |
| HE062 | derivative | medium | exact | **FAIL** | 5 | 14.3 |
| HE063 | fibfib | medium | partial | pass | 1 | 4.4 |
| HE068 | pluck | medium | partial | pass | 3 | 12.2 |
| HE069 | search | medium | exact | pass | 1 | 5.5 |
| HE072 | will_it_fly | medium | exact | pass | 4 | 21.4 |
| HE073 | smallest_change | medium | partial | pass | 1 | 3.8 |
| HE075 | is_multiply_prime | medium | partial | pass | 3 | 28.6 |
| HE076 | is_simple_power | medium | partial | pass | 1 | 4.3 |
| HE077 | cube_root | medium | exact | pass | 1 | 3.0 |
| HE083 | starts_one_ends | hard | partial | pass | 1 | 7.7 |
| HE084 | solve | hard | exact | pass | 1 | 4.5 |
| HE085 | add | hard | exact | pass | 1 | 3.8 |
| HE088 | sort_array | hard | exact | pass | 3 | 18.9 |
| HE094 | skjkasdkd | hard | partial | pass | 1 | 6.6 |
| HE096 | count_up_to | hard | partial | pass | 1 | 4.3 |
| HE097 | multiply | easy | exact | pass | 1 | 2.9 |
| HE100 | make_a_pile | easy | partial | pass | 1 | 3.4 |
| HE102 | choose_num | easy | exact | pass | 1 | 3.8 |
| HE104 | unique_digits | hard | partial | pass | 1 | 6.1 |
| HE106 | f | medium | partial | pass | 1 | 5.9 |
| HE108 | count_nums | medium | exact | pass | 1 | 5.3 |
| HE109 | move_one_ball | hard | partial | pass | 5 | 25.1 |
| HE114 | min_sub_array_sum | medium | partial | pass | 1 | 4.2 |
| HE116 | sort_array | medium | partial | pass | 1 | 7.2 |
| HE120 | sort_seq | medium | partial | pass | 1 | 5.1 |
| HE121 | solution | medium | exact | pass | 1 | 3.9 |
| HE122 | add_elements | medium | exact | pass | 1 | 5.8 |
| HE123 | get_odd_collatz | medium | partial | pass | 1 | 7.1 |
| HE126 | check_valid_list | hard | exact | pass | 3 | 19.0 |
| HE130 | tribonacci | medium | partial | pass | 1 | 19.2 |
| HE132 | is_nested | medium | partial | pass | 1 | 5.6 |
| HE135 | can_arrange | medium | partial | pass | 1 | 3.8 |
| HE138 | is_equal_to_sum_even | easy | exact | pass | 1 | 2.9 |
| HE139 | special_factorial | medium | partial | pass | 1 | 4.3 |
| HE142 | sum_squares | medium | exact | pass | 1 | 4.0 |
| HE145 | order_by_points | hard | partial | pass | 1 | 8.2 |
| HE146 | special_filter | medium | partial | pass | 1 | 5.7 |
| HE147 | get_max_triples | medium | exact | pass | 2 | 12.0 |
| HE150 | x_or_y | easy | exact | pass | 1 | 4.8 |
| HE152 | compare | easy | exact | pass | 1 | 4.6 |
| HE159 | eat | easy | exact | pass | 1 | 3.5 |
| HE163 | generate_integers | medium | partial | pass | 2 | 8.0 |
