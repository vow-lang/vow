# Memory Characterization Harness

`bench/memory/` is a standalone RSS characterization harness for Vow programs that exercise arena and allocation behavior. It uses the Rust stage-0 compiler at `target/release/vow` so it can run before the self-hosted fixed point exists in a worktree.

Build the compiler and runtime first if needed:

```bash
cargo build --release -p vow
cargo build --release -p vow-runtime
```

Run the harness normally:

```bash
bench/memory/run.sh
```

Each program in `programs/` starts with:

```vow
// BENCH: max-rss-kb N
```

Normal runs build each program with `target/release/vow build --no-verify`, execute the binary under `ulimit -v 2000000` and `/usr/bin/time -v`, then compare maximum RSS against that annotation. A run exits nonzero when a build fails, a binary exits nonzero, or RSS exceeds the recorded bound.

For each program the harness prints a human `PASS` or `FAIL` line and a machine-readable JSON object:

```json
{"program":"string_scope_churn","source":"bench/memory/programs/string_scope_churn.vow","binary":"/tmp/tmp.x/string_scope_churn","build_exit":0,"run_exit":0,"max_rss_kb":1234,"bound_kb":5678,"wall_seconds":0.04,"status":"pass"}
```

If a binary is killed by the `ulimit -v` memory cap, the harness reports `status = "run_failed"` and preserves the process exit code in `run_exit`. Automation that needs to distinguish OOM-shaped exits from ordinary nonzero program exits should inspect `run_exit`; Linux `SIGKILL` exits commonly appear as `137`.

`expected.toml` mirrors the annotations with one table per program:

```toml
[string_scope_churn]
file = "programs/string_scope_churn.vow"
max_rss_kb = 5678
expected = "pass"
```

Refresh local characterization bounds with:

```bash
bench/memory/run.sh --record
```

`--record` reruns every program, rewrites the `// BENCH:` annotations and `expected.toml`, and adds a fixed 4096 KiB cushion to each measured maximum RSS. It still fails without rewriting if any program fails to build or exits nonzero.

Later memory-precision slices should lower a bound only after measuring a real improvement locally. Do not lower limits just because a single noisy run happens to scrape under a smaller number; these checked-in values are characterization baselines until the allocation behavior itself improves.
