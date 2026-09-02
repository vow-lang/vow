# Plan: Make arena verification solver flags overridable

## 1. Problem restated

The arena verification target already lets callers override the ESBMC executable and unwind bound through `ESBMC ?=` and `UNWIND ?=`, but it hard-codes `--boolector` in the recipe, so supplying solver flags on the `make` command line has no effect. Add an overridable `SOLVER_FLAGS` variable whose default preserves the resource-safe Boolector invocation, and cover both the default and override expansions without changing the arena model or claiming that alternate solvers can prove this harness efficiently.

## 2. Files to touch

- `scripts/full_test.sh` — in Section 11 (`Arena Primitive Verification`), add a fast Make dry-run regression check that runs even when ESBMC is unavailable and verifies both the default solver flag and a command-line override.
- `vow-runtime/verify/Makefile` — define `SOLVER_FLAGS ?= --boolector` beside `UNWIND ?= 5` and replace the recipe's literal `--boolector` with the unquoted `$(SOLVER_FLAGS)` expansion so one or more flags, including an explicitly empty value, can be supplied by the caller.
- No Rust crate files (the top-level `vow-*` crates) and no files under `compiler/` are touched: this is not a compiler or self-hosted-compiler behavior change.
- No `docs/spec/*.md` files are touched: this is an internal developer harness variable, not Vow syntax, semantics, a builtin, an effect, or a `vowc` CLI flag.

## 3. TDD slices

1. **Red — pin the Make recipe's default and override behavior.**
   - **Test location:** `scripts/full_test.sh`, Section 11 immediately before the existing `command -v esbmc` guard.
   - **Behavior under test:** capture `make -s -C vow-runtime/verify -n verify` and require the default command to contain `--boolector`; capture the same dry run with `SOLVER_FLAGS=--z3` and require it to contain `--z3` and not contain `--boolector`. Use the existing `pass`/`fail` helpers and a single focused test label such as `arena/solver-flags` so the check is visible in the normal full-test summary. The override assertion is red against the current hard-coded recipe, while the default assertion protects current behavior.
   - **Production code that will make it pass:** `vow-runtime/verify/Makefile` lines 17-22.

2. **Green — route solver selection through the overridable variable.**
   - **Test location:** rerun the Section 11 dry-run assertions from slice 1 directly, without requiring an ESBMC installation.
   - **Behavior under test:** the no-override command remains byte-for-byte equivalent in its effective ESBMC arguments and ends in `--boolector`; `SOLVER_FLAGS=--z3` replaces that flag; a multi-token value remains separate shell arguments because the recipe expansion is not quoted.
   - **Production change:** in `vow-runtime/verify/Makefile`, add `SOLVER_FLAGS ?= --boolector` after `UNWIND ?= 5`, then use `$(SOLVER_FLAGS)` at the existing solver-flag position in the `verify` recipe. Keep the existing solver-performance explanation because it documents why Boolector remains the default.
   - **Refactor:** none; retain the current target, flag ordering, line continuation, and comments.

3. **Regression verification — exercise the unchanged default proof.**
   - **Test location:** `vow-runtime/verify/Makefile` target `verify`, as invoked by `scripts/full_test.sh` Section 11.
   - **Behavior under test:** when ESBMC is installed, `(ulimit -v 2000000; make -C vow-runtime/verify verify)` still reports successful verification with `--unwind 5` and the default Boolector solver. Run `bash -n scripts/full_test.sh` first, then run `scripts/full_test.sh` for repository-level integration coverage when the prepared toolchain is available.
   - **Production code:** no additional change should be needed after slice 2; any failure here must be investigated rather than addressed by weakening the harness assertions, changing the unwind bound, or silently selecting another solver.

## 4. Verification surface

This change does not alter contracts, compiler codegen, `vow-runtime/verify/arena.c`, or its C model, so ESBMC has no new property to prove. With no override, the effective command remains `arena.c --unwind 5 --no-bounds-check --no-pointer-check --64 --boolector`, and all existing arena assertions must continue to verify under the 2 GB virtual-memory limit. An explicit override only changes the caller-selected solver arguments; Bitwuzla or Z3 may still time out or exceed the documented resource envelope, and this issue does not promise otherwise. No fixtures under `tests/run/` or `examples/` need to grow.

## 5. Risk areas

- **Default solver regression:** omitting the `?=` default or dropping `--boolector` from the no-override command would restore the high-memory ESBMC default described in the Makefile. The dry-run default assertion and the real capped verification guard against this.
- **Override ignored or partially passed:** leaving any literal solver flag in the recipe would defeat the new variable, while quoting `"$(SOLVER_FLAGS)"` would collapse multiple flags into one argument. The override and multi-token dry runs should inspect the expanded command.
- **Shell/Make syntax:** preserve the recipe tab and backslash continuation; validate the edited shell harness with `bash -n` and the Make recipe with `make -n` before the expensive proof.
- **Alternate solver behavior:** caller overrides are intentionally allowed even though the existing comment documents poor Bitwuzla/Z3 results. Do not reinterpret an alternate solver's timeout or OOM as a regression in the default path.
- **Fixed point, canonical printer, and lint:** no `compiler/`, `vow-clif-shim`, parser/printer, Rust, or generated files change, so binary fixed-point ordering, stack-slot layout, `parse → print → parse` idempotency, and `cargo clippy --all -- -D warnings` are not exposed by this patch. The added Bash remains subject only to the existing shell harness behavior.

## 6. Out of scope

- Changing `UNWIND`, incremental-BMC settings, pointer/bounds checks, architecture flags, or any assertion in `arena.c`.
- Benchmarking, fixing, or certifying Bitwuzla, Z3, or other solver combinations for the arena formula.
- Adding a new `vowc` CLI solver option, changing existing compiler solver selection, or updating language/CLI specifications.
- Modifying either compiler, runtime allocation behavior, contracts, codegen, test fixtures, examples, generated help, or files under `build/`.
- Refactoring `scripts/full_test.sh`, reorganizing Make variables, broad formatting changes, or addressing adjacent arena CI/resource-drift work such as issue #747.
- Committing, pushing, opening, or merging a PR during this planning stage; the implementation stage opens the PR and the orchestrator squash-merges it, so the PR title alone becomes the squash subject and must be a valid, sentence-case-avoiding conventional-commit summary.
