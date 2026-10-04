# Developer Benchmarks

This directory documents repository-local benchmark harnesses. These are development tools, not `vow` CLI subcommands, so they do not belong in `docs/spec/cli.md`.

## Memory Characterization

`bench/memory/run.sh` builds the programs under `bench/memory/programs/` with `target/release/vow build --no-verify`, runs them under `/usr/bin/time -v`, and checks maximum RSS against each source file's `// BENCH: max-rss-kb N` annotation.

Use it when changing arena, container, string, or allocation behavior:

```bash
cargo build --release -p vow
cargo build --release -p vow-runtime
bench/memory/run.sh
```

To refresh characterization baselines after a measured local improvement or a deliberate baseline reset:

```bash
bench/memory/run.sh --record
```

`--record` rewrites `bench/memory/expected.toml` and the source annotations using fresh measurements plus a fixed 4096 KiB cushion. Lower bounds only when a real implementation improvement has been measured; do not turn one noisy low run into a tighter regression gate.

### CI gate

`scripts/check_memory_bounds.py --compiler <vow|vowc>` runs the same programs and the same `// BENCH:` bounds without GNU `time` and against either compiler. It samples `VmHWM` from `/proc/<pid>/status` while the program runs (a `wait4` `ru_maxrss` would include the resident size of the forking parent), so it is Linux only. `scripts/full_test.sh` runs it once per compiler (Section 8a, `memory/bounds-rust` and `memory/bounds-self`), so a per-call leak in region inference, in either lowering, or in the runtime fails CI, not just a developer's local `run.sh`. Its unit tests are `scripts/test_check_memory_bounds.py`.

A program that must stay flat has to loop long enough that a per-iteration leak is far beyond the 4096 KiB cushion: 10^5 iterations of a 16-byte leak is already 1.6 MB, so size the loop so the leak per iteration times the iteration count is tens of MB.
