# Vow CLI Reference

## Commands

### `vow build` (default)

Compile source to native executable. Verifies contracts by default.

```
vow build [OPTIONS] <source.vow>
vow [OPTIONS] <source.vow>          # bare form (identical to `vow build`)
```

The bare `vow <source.vow>` form is exactly `vow build`: it verifies by default, accepts the same flags, prints the same build-result JSON and fails closed (non-zero exit and `VerifyFailed` when verification fails). The former legacy-only `--emit-c` and `--verify` flags are not accepted and are rejected with the usage error (exit 2) like any unknown flag; `--no-verify` is the opt-out and `--dump-ir` prints the IR. Without `-o` the executable is written to `build/<stem>`.

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `build/<stem>` | Output executable path                  |
| `--mode <debug\|release\|profile\|sanitize>` | `release` | Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + use-after-region-close detection for Vecs |
| `--no-verify`     | (off)       | Skip static verification                  |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |
| `--debug-trace <off\|calls\|full>` | `off` | Emit JSON trace lines to stderr at runtime |
| `--no-cache`    | (off)       | Disable verification result caching, and (for `--no-verify` builds) the compile-object cache. See "Compile-object cache behavior" below |
| `--max-k-step <N>` | `50`     | ESBMC backend only; a usage error unless `--backend esbmc` is given. ESBMC incremental BMC max iterations |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC backend only; a usage error unless `--backend esbmc` is given. ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC backend only; a usage error unless `--backend esbmc` is given. ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | Per-function verification timeout in seconds; the only resource knob of the native backend, where the default is a fixed `300`. ESBMC backend: under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is an immediate watchdog kill under ESBMC and a zero budget (no solver is spawned; statically decided claims still resolve) under `--backend native` |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent verification jobs (ESBMC processes, or `verify-worker` subprocesses under `--backend native`) |
| `--backend <esbmc\|native>` | `native` | Verification backend of the self-hosted `vowc`; the Rust `vow` has no `--backend` and always verifies with ESBMC. `native` (the default) is the symbolic checker with Bitwuzla, run in `verify-worker` subprocesses (epic #1398, see "Native backend" below); `esbmc` is the opt-out and is accepted by every command. An explicit `--backend native` together with `--no-verify` is a usage error (exit 1), as are `--max-k-step`, `--solver` and `--encoding` unless `--backend esbmc` is given. Any value other than `esbmc` or `native` is a usage error |
| `--replay-cex`    | (off)       | Differential test the verifier model against runtime semantics (same as `vow verify --replay-cex`; see below) |
| `--perfetto <path>` | (off) | Write a gzipped Chrome Trace Event Format trace of this compilation to `<path>` (load directly at ui.perfetto.dev). Captures per-phase spans, codegen/link and, under the ESBMC backend, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Distinct from `--mode profile`, which instruments the *compiled program*. Pure side artifact: never affects codegen, the build JSON, or the cache. |

**Compile-object cache behavior.** The on-disk compile-object cache (`$VOW_CACHE_DIR` or `~/.cache/vow/`, where each entry is a `<key>.o` artifact keyed by a content hash of all dependencies, mode, and trace settings) is automatically disabled whenever static verification is active. This guarantees the linked binary always comes from the same codegen run whose IR was verified, closing the integrity gap where a stale or attacker-supplied `.o` could be linked against freshly-verified IR. Concretely the cache only activates on `vow build --no-verify` invocations; it is bypassed on the default `vow build` path. `--no-cache` additionally disables the cache for `--no-verify` builds.

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching; under `--backend native` it disables the native result cache of `verify` (see "Result cache" below) |
| `--max-k-step <N>` | `50`       | ESBMC backend only; a usage error unless `--backend esbmc` is given. ESBMC incremental BMC max iterations |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC backend only; a usage error unless `--backend esbmc` is given. ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC backend only; a usage error unless `--backend esbmc` is given. ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | Per-function verification timeout in seconds; the only resource knob of the native backend, where the default is a fixed `300`. ESBMC backend: under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is an immediate watchdog kill under ESBMC and a zero budget (no solver is spawned; statically decided claims still resolve) under `--backend native` |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent verification jobs (ESBMC processes, or `verify-worker` subprocesses under `--backend native`) |
| `--backend <esbmc\|native>` | `native` | Verification backend of the self-hosted `vowc`; the Rust `vow` has no `--backend` and always verifies with ESBMC. `native` (the default) is the symbolic checker with Bitwuzla, run in `verify-worker` subprocesses (epic #1398), see "Native backend" below; `esbmc` is the opt-out, kept for two releases after the flip (issue #1863), and is accepted by every command. `contracts` and `test` without `--verify` reject an explicit `--backend native` with a usage error (exit 1) instead of silently ignoring it; the same holds for `build` with `--no-verify`. `--max-k-step`, `--solver` and `--encoding` are usage errors unless `--backend esbmc` is given. Any value other than `esbmc` or `native` is a usage error |
| `--replay-cex`    | (off)       | Differential test of the verifier model against runtime semantics. After the verifier reports a counterexample, build a `--mode debug` harness that calls the failing function with the counterexample's concrete inputs and check that the runtime `VowViolation` agrees (same `vow_id` and blame). Adds a `replay` field to each counterexample (see "Counterexample replay" below). Opt-in, off by default; also accepted by `vow build`. |
| `--perfetto <path>` | (off) | Write a gzipped Chrome Trace Event Format trace of this verification run to `<path>` (load directly at ui.perfetto.dev). Captures frontend phase spans and, under the ESBMC backend, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process; under `--backend native` it records the frontend phase spans (`module-load`, `parse`, `typecheck`, `lower`, ...) and one `verification` span (no per-claim worker or solver spans, no per-worker RSS yet). Accepted only by `build` and `verify`. Pure side artifact. |
