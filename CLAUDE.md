# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Agent session setup

Before implementation, provision and verify this worktree's hooks:

```bash
./scripts/ensure-hooks.sh
# Launch an agent after provisioning its selected worktree:
./scripts/agent-session.sh --worktree /path/to/worktree claude
./scripts/agent-session.sh codex
```

Install `pre-commit` with `uv tool install pre-commit` first. Provision from the
host before mounting Git metadata read-only; set `PRE_COMMIT_HOME` to a writable
cache. Existing hooks are preserved and chained; provisioned hooks are reused on
resume. Claude SessionStart provisions hooks and PreToolUse blocks shell/edit
operations if either hook is missing, disabled, or overridden, including after
startup. For other agents use the launcher. Do not bypass failures with `SKIP`,
`--no-verify`, or `core.hooksPath`; repair provisioning before continuing.

Changed authored text files must fit **20,000 bytes** (5,000 estimated tokens at
bytes/4). The hook checks staged blobs, including partial staging and rename
destinations. CI checks committed blobs and exemptions from the head revision,
even for docs-only changes. An existing oversized file may change only if it does
not grow past its size at the base commit (a ratchet), so split it as you touch it;
new files and files under the limit must stay at or below it.
Use cohesive Vow modules and explicit `use` declarations, keep semantic contracts
and ownership intact, and honor dual-compiler and C-emitter parity. Never weaken
contracts to make splitting or verification easier. Only complete generated
outputs and lockfiles explicitly listed in `.gitattributes` are exempt; a
partially generated compiler source file is still authored code.

Read the linked guidance before work in the corresponding area; these files are
part of the repository instructions:

- [Compiler architecture and self-hosted compiler](docs/agents/compiler-architecture.md)
- [Operation Catalogue and generation rules](docs/agents/operation-catalogue.md)
- [Test harnesses, C parity, memory bounds, mutation testing, and benchmarks](docs/agents/validation.md)

## Language Design Principles

Vow is a language for agents, not humans. New language features should only be added if they meet **all three** criteria:
1. **Does not make verification harder** — the ESBMC pipeline and C model must not become more complex or fragile.
2. **Eliminates a class of agent bugs** — the feature must make it harder for agents to generate incorrect code, not just shorter code.
3. **Makes agentic coding easier** — the feature must help agents produce correct, verified programs more reliably (not merely reduce verbosity).

Syntactic convenience for humans (string interpolation, pattern matching sugar, etc.) is not sufficient justification. Every feature added expands the compiler, C emitter, and verification surface area.

**Crisp rule:** add surface sugar only when it desugars to today's core semantics with near-zero verifier impact; reject anything that introduces a new type-system axis. This preserves Vow's identity.

## Production Quality

Vow is a serious, production-grade project. All implementation decisions must reflect this:

- **No shortcuts.** Do not assume programs will be small, short-lived, or low-allocation. Vow programs may be long-running services, compile large codebases, or process substantial data.
- **Memory management matters.** Every allocation must have a clear ownership story and a path to deallocation. "The process exits soon anyway" is never an acceptable justification for leaking memory.
- **Scalability is a requirement.** Data structures, algorithms, and compiler passes must be chosen for reasonable asymptotic behavior, not just correctness on small inputs.
- **No "experimental" excuses.** Do not defer correctness, robustness, or resource discipline with the rationale that the project is early-stage or experimental. Treat every change as if it will run in production.

## Development Discipline

These principles apply to all work in this repo — compiler code, self-hosted compiler, tooling, and benchmarks. They also apply to Vow programs written by agents (see `docs/spec/index.md`, which embeds the same guidance into the skill).

- **Deep modules.** Favor deep modules (lots of functionality, simple interface, hides complexity) over shallow ones (thin wrappers, complex interface, surface complexity). When a module's interface is nearly as wide as its implementation, collapse it or widen its responsibilities. Many exported symbols and pass-through functions are a shallow-module smell.
- **Surgical changes.** Many small changes beat one large change. They are easier to review, debug, `git bisect`, and revert. Do not bundle refactors, formatting, or unrelated cleanups into a bug fix. If a task grows, split it.
- **Small files, smaller functions.** Context is precious — every file read consumes budget. Keep files and functions short enough that an agent can load the relevant unit without displacing other context. Split by responsibility as soon as a unit stops fitting a single coherent idea.

## Contract Authoring

Contracts express **semantic correctness** — what is mathematically required for a function to be correct. They must never be weakened or artificially bounded to satisfy ESBMC's verification limits.

- **Write the true contract.** `gcd(a, b)` requires `a >= 0, b >= 0, a + b > 0`. It does not require `a <= 50` — that is a verifier limitation, not a property of Euclid's algorithm.
- **ESBMC bounds are not contracts.** Bounds like `n <= 10` (to fit within `--unwind 10`) or `a <= 100` (to help the SMT solver) are verification artifacts. They do not belong in `requires`/`ensures` clauses.
- **Collection and representation facts follow the same rule.** Lengths, capacities, indices, and struct fields belong in contracts only when they express the function's real domain, result, or representation invariant. Never cap a collection, struct, or capacity merely to fit the verifier's model or reduce its state space.
- **Postconditions should be tight.** `min(a, b)` must ensure `result == a || result == b`, not just `result <= a && result <= b`. A weak postcondition that admits incorrect implementations is a bad contract.
- **If ESBMC can't prove a correct contract, that's ESBMC's problem.** Mark the function as unverifiable or skip it in the verification pass. Do not distort the contract to accommodate the tool.
- **Only add bounds that reflect genuine semantic constraints.** Overflow guards (e.g., `requires: x > -9223372036854775807` for `abs`) are legitimate — they prevent undefined behavior in the implementation. Loop iteration caps for ESBMC are not.
- **Use the backend-independence test.** Replacing ESBMC with a stronger verifier must not require editing a source contract.

## Commit Messages

Every commit message and every PR title MUST follow [Conventional Commits](https://www.conventionalcommits.org/):
`type(optional-scope): subject`. `commitlint.config.cjs` (`@commitlint/config-conventional`) is the
single ruleset, enforced in three places: the `commit-msg` pre-commit hook locally, `Lint PR title`
on every PR, and `Lint commits` on pushes to `main`. All three run the same commitlint config, so a
message that passes locally cannot fail in CI.

Install the hooks once per clone — without this the local gate is simply absent:

```bash
./scripts/ensure-hooks.sh   # installs/verifies both hooks and prepares environments
```

Rules that are easy to get wrong:

- **Subject must be lower-case.** `subject-case` rejects sentence-case, start-case, PascalCase, and
  UPPER-CASE subjects. Write `feat(codegen): add Cranelift I128 codegen`, not
  `feat(codegen): Cranelift I128 codegen` — the leading capital fails even when the capitalized word
  is a proper noun. Start the subject with a lower-case verb and the problem disappears. Capitals
  later in the subject (`Vec`, `VowViolation`, `I128`) are fine.
- **No trailing period.** `subject-full-stop` rejects a subject ending in `.`. Imperative mood
  (`count`, not `counts`/`counted`) is convention rather than a lint rule, but follow it:
  `fix(vow-perf): count Vec sort runtime work`.
- **Type must be one of** `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`,
  `style`, `test`. `@semantic-release/commit-analyzer` derives the release version from these, so the
  type is load-bearing, not decoration.
- **Header ≤ 100 characters, including the ` (#N)` GitHub appends.** The squash subject is
  `<PR title> (#<number>)`, so a PR title has roughly 92 characters to work with, not 100.
  `Lint PR title` checks the title with that suffix attached for this reason. Detail belongs
  in the body.

## Pull Requests

The repository allows **squash merges only** (`gh pr merge --squash`); merge commits and rebase
merges are disabled in the repo settings. The squash subject is taken verbatim from the **PR title**,
so the PR title is the commit message that lands on `main` — it must satisfy the Conventional Commits
rules above, and `Lint PR title` checks it with commitlint for exactly that reason. Fix a rejected
title by editing the PR title, not by amending commits.

## Vow Compiler

When implementing changes across Vow compilers, always modify BOTH the Rust compiler and the self-hosted compiler in the same session. Run the full test suite (`cargo test` and self-hosted tests) after changes to both.

This applies to behaviour-preserving refactors and unattended architecture-deepening runs too. "Rust-only, so no new drift" is not an exemption: the self-hosted compiler is the primary one, and a Rust-only refactor widens the gap between the two. If a change cannot be expressed in Vow (no generics, traits, or closures), do not land the Rust half alone — drop or re-scope the change. Earlier Rust-only landings are outstanding debt, not precedent.

**Scoped exception: the native verifier.** The checker of epic #1398 (`compiler/vc_*.vow`, `vowc verify-worker`) lives only in the self-hosted compiler; it gets no Rust twin and no parity requirement. The dominance validator, which exists only to feed it, is exempt on the same terms. This exempts the verifier and nothing else: the lexer, parser, type checker, lowering, codegen, runtime builtins and non-verifier CLI surface stay dual-compiler, and a Rust-only or Vow-only change outside the verifier remains drift debt. Once the seed lands, Rust stage 0 will delegate verification to a pinned seed `vowc verify` (`scripts/seed.toml`). Bootstrap's fixed point then guards codegen only, and Stage 1 verification depends on the seed pin, so a "green locally" claim must record the seed pin as well as the head SHA. ESBMC and `c_emitter.{rs,vow}` stay in stage 0 and the C parity rule below applies unchanged until both emitters are deleted (epic #1398, P6). See [ADR-2026-10-08-1421](docs/adr/2026-10-08-1421-verifier-lives-only-in-compiler.md).

## Bootstrap Commands (Rust Stage 0)

These Rust workspace commands build the stage 0 bootstrap compiler only. For day-to-day development, use `build/vowc` (see below).

```bash
cargo build --all          # build all crates
cargo test --all           # run all tests
cargo test -p vow-syntax   # run tests for a single crate
cargo test lexer::tests::lex_keywords  # run a single test by path
cargo clippy --all --all-targets -- -D warnings  # lint incl. tests (CI enforces zero warnings)
cargo fmt --all            # format all code
```

## Day-to-Day Usage (Self-Hosted Compiler)

`build/vowc` is the primary compiler for all Vow development. It is a self-hosted, verified fixed-point binary produced by `scripts/bootstrap.sh`.

```bash
build/vowc build examples/divide.vow                   # compile + verify (default)
build/vowc build --no-verify examples/divide.vow       # compile, skip verification
build/vowc build --mode debug examples/divide.vow      # compile with runtime vow checks
build/vowc verify examples/divide.vow                  # verify contracts only (no executable)

# Help
build/vowc --help                                      # JSON capability description (for agents)
build/vowc --help --human                              # human-readable capability description
```

Debug mode is required to see runtime `VowViolation` output. Release omits all vow checks.
`vowc build` verifies by default with the native verifier (Bitwuzla); `--no-verify` skips verification and `--backend esbmc` selects ESBMC instead. The Rust `vow` has no `--backend` and always uses ESBMC.
`vowc verify` runs only the frontend + verification pipeline (no codegen, no executable output).

## Bootstrap (Rust Compiler)

To get `build/vowc` in the first place, bootstrap from the Rust compiler:

```bash
scripts/bootstrap.sh                      # full bootstrap with verification
scripts/bootstrap.sh --skip-cargo         # skip cargo build if already built
```

This builds `./target/release/vow` (stage 0), then uses it to compile and verify the self-hosted compiler, producing `build/vowc`. The Rust compiler (`./target/release/vow`) is only needed for this bootstrap step.

`scripts/bootstrap.sh --native-verify` adds a native pass: the Stage 1 and Stage 2 `vowc` each verify all of `compiler/*.vow` with `--backend native` and must agree. Every function native still skips is listed, with its tracking issue, in `scripts/native_compiler_skips.toml` (a new skip, or a stale entry, fails). `scripts/verify_compiler.py` is the same check plus a native-vs-ESBMC comparison; see `docs/verifier-eval.md`.

### "Green locally" checklist claims must be pinned to the PR's final head SHA

`.github/workflows/bootstrap.yml` only runs on push to `main` and nightly — it never runs on pull
requests. No PR-time CI corroborates a "bootstrap is green" checklist claim; the first time a PR's
actual final tree is verified by CI can be *after* it has already merged.

For migration-epic (#1104/#1116) seam PRs and any other PR whose checklist asserts a green
`scripts/bootstrap.sh` run, re-run `scripts/bootstrap.sh --skip-cargo --no-cache` against the PR's
actual final head SHA — after the last push, not an earlier commit you happened to be looking at —
immediately before ticking that checklist box, and record the checked SHA in the checklist line. Once
Stage 1 verification is delegated to a pinned seed (see "Scoped exception: the native verifier"
above), record the `scripts/seed.toml` pin there too. A checklist claim written against an earlier
commit and never re-checked after later commits land is not a green run of the tree that actually
merges.

`--no-cache` here is cheap defense in depth, not a fix for a confirmed cache bug: `VerifyCache`
(`vow/src/cache.rs`) only ever persists `FAILED` verdicts, never `PROVEN` — a stale cache entry can
make a build wrongly red, never wrongly green. Issue #1307 investigated a prior "green locally, red
in CI" incident (#1291, a `span_len` postcondition violation caught by the first-ever post-merge
Bootstrap run) and found the actual cause to be a PR description written before the contract-breaking
commit landed and never revisited, not a cache defect.

The native verifier's result cache (`compiler/vc_cache.vow`, #1429) is different: it does store
`unsat` answers, so a forged entry can make a run wrongly green. `--no-cache` therefore stays
mandatory for "green locally" claims about the native verifier too.

## Canonical Source of Truth

`docs/spec/` contains the authoritative specification for the Vow language and CLI:

- **`index.md`** — overview and reference table
- **`grammar.md`** — complete grammar: types, operators, control flow, structs, enums, match, modules, methods, effects, builtins
- **`cli.md`** — CLI commands, flags, output JSON schema, exit codes, trace/error formats
- **`contracts.md`** — vow blocks, requires/ensures/invariant, blame semantics
- **`errors.md`** — diagnostic error codes and their meanings
- **`examples.md`** — worked examples

**Any change to Vow syntax, semantics, types, builtins, operators, effects, or CLI flags MUST include a corresponding update to the relevant `docs/spec/*.md` file.** The spec files are the spec — compiler code implements them, not the other way around. If you add a type, update `grammar.md`. If you change a builtin signature, update `grammar.md`. If you add a CLI flag, update `cli.md`. If you change contract semantics, update `contracts.md`.

`cli.md`, `grammar.md`, `errors.md`, `contracts.md`, `contracts-methodology.md` and `stdlib.md` are assembled files: edit the authored parts in
`docs/spec/src/<doc>/NN-*.md`, then run `python3 scripts/spec_assemble.py` (`generate_help.py` does it
too). If you edited an assembled file by mistake, `python3 scripts/spec_assemble.py --import` ports the
edit into the parts; `--split PART` halves a part that grew past 20,000 bytes.

The Claude Code skill document is embedded in the compiler and generated on demand:
```bash
build/vowc skill print                            # print skill to stdout
build/vowc skill install                          # install to .claude/skills/vow/SKILL.md
```

After updating a spec file, regenerate `--help` / embedded skill and rebuild:
```bash
uv run python scripts/generate_help.py          # regenerate --help and skill in both compilers
cargo build --release -p vow                     # rebuild Rust compiler
scripts/bootstrap.sh --skip-cargo                 # rebuild build/vowc
```

The staleness detector `scripts/check_help_coverage.py` (run in `full_test.sh`) will catch drift between `grammar.md` and `--help`.
