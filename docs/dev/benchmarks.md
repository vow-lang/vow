# Developer Benchmarks

This directory documents repository-local benchmark harnesses. These are development tools, not `vow` CLI subcommands, so they do not belong in `docs/spec/cli.md`.

## Memory Characterization

`bench/memory/run.sh` builds the programs under `bench/memory/programs/` with `target/release/vow build --no-verify`, runs them under `ulimit -v 2000000` and `/usr/bin/time -v`, and checks maximum RSS against each source file's `// BENCH: max-rss-kb N` annotation.

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
