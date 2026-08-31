#!/usr/bin/env python3
"""Classify a commit range for the CI job gates.

Emits two independent outputs, `code` and `arena`, each guarding a different
set of expensive jobs. Both fail open: anything the classifier cannot
positively rule out runs the jobs it guards.

`code` -- documentation-only detection, for ci.yml's build/bootstrap gate.

CI's expensive jobs exist to catch regressions in compiled artifacts: the Rust
workspace build, the ESBMC verifier-evaluation corpus, and the three-stage
`scripts/bootstrap.sh` fixed point. Together they are several hours of runner
time per pull request, and a commit that changes nothing but prose cannot
regress any of them. This module decides when that is the case so `ci.yml` can
skip them.

The classification is deliberately narrow. A path counts as prose only if it is
a Markdown file outside the two roots that hold Markdown the *build* consumes:

  * `docs/spec/` is the canonical spec, and `scripts/generate_help.py` copies it
    into the compiler sources -- `vow/src/skill.rs` and `compiler/main.vow` each
    embed a verbatim copy. `scripts/check_help_coverage.py` fails the suite when
    `grammar.md` and the generated `--help` drift apart, and the
    `docs/spec/schemas/*.json` beside it are `include_str!`-ed into `vow-diag`
    and `vow`, so editing one changes a compiled binary.
  * `skills/` is the on-disk mirror `generate_help.py` writes for
    `npx skills add`. A `cargo test` in `vow/src/skill.rs` asserts it matches
    the compiler-embedded skill byte for byte, so editing a file there alone
    fails `cargo test`.

Add a root here if another generated mirror of compiler-embedded content is
ever checked in; a generated file that reads as prose is how this gate would
skip the suite that guards it.

Everything the classifier does not positively recognise as prose counts as
code, so the failure mode of an unanticipated path -- a new file extension, an
unreadable commit range, a first push with no parent -- is to run the full
suite, never to skip it.

`arena` -- arena-proof relevance, for arena-verify.yml's gate.

`vow-runtime/verify/arena.c` is a standalone C harness: ESBMC proves it on its
own, with no compiler and no Rust build involved. That single proof is the
longest step in CI by a wide margin (~800s of a ~1800s pull request), and its
inputs are narrow, so it runs only when the changeset can actually affect its
outcome, plus nightly regardless. An input is anything that changes what is
proved, how it is proved, or the headroom it is proved within:

  * `vow-runtime/` -- both the harness under `verify/` and the
    `vow-runtime/src/` implementation whose semantics it mirrors. The harness
    is only meaningful while the two agree.
  * The runner and its test, which fix the memory cap and the ESBMC flags.
  * This module and its tests. The gate decides whether the proof runs, so a
    change that narrows it must run the job it narrows -- otherwise a mistake
    here disables the proof and the same commit hides the evidence, leaving
    only the nightly run to notice.
  * `.github/actions/install-esbmc/`, which holds the ESBMC pin for the whole
    repository. Solver and version drift move the proof's memory ceiling by
    hundreds of MiB under a 2 GB cap, so a version bump has to re-run the
    proof that guards the headroom (#546, #747).

An unresolvable commit range reports `arena=true`, which is also what a
`schedule` or `workflow_dispatch` event produces: neither carries a range, so
the nightly run needs no separate condition to opt itself in.
"""

import argparse
import subprocess
import sys

# Markdown under these prefixes is build input, not documentation: the compiler
# sources embed a copy, and a test asserts the two agree. See the module
# docstring for the specific mechanisms.
BUILD_INPUT_PREFIXES = ("docs/spec/", "skills/")

PROSE_SUFFIX = ".md"

# Paths that can change the arena proof's outcome or its memory headroom. See
# the module docstring for why each one is in the list. Split by kind so each
# is matched the way it should be: a directory by prefix, a file exactly. A
# bare `startswith` over both would let `verify_arena.sh.bak` read as an input.
ARENA_INPUT_DIRS = (
    "vow-runtime/",
    ".github/actions/install-esbmc/",
)

ARENA_INPUT_FILES = (
    "scripts/verify_arena.sh",
    "scripts/test_verify_arena.py",
    # This module and its tests: they decide whether the proof runs at all, so
    # a change that narrows the gate has to run the job it is narrowing.
    "scripts/ci_docs_only.py",
    "scripts/test_ci_docs_only.py",
    ".github/workflows/arena-verify.yml",
)

# git's "no such commit" sentinel. `github.event.before` is all zeroes on the
# first push to a branch, which leaves no range to diff.
NULL_SHA = "0" * 40


def is_prose(path):
    """Whether one repository path is inert with respect to the build.

    Args:
        path: A repository-relative path, as `git diff --name-only` prints it.

    Returns:
        bool: True when changing this path cannot change a build artifact or a
            test outcome.
    """
    return path.endswith(PROSE_SUFFIX) and not path.startswith(BUILD_INPUT_PREFIXES)


def is_arena_input(path):
    """Whether one repository path feeds the arena proof.

    Args:
        path: A repository-relative path, as `git diff --name-only` prints it.

    Returns:
        bool: True when changing this path can change what the arena proof
            proves, how it is proved, whether it runs, or the memory headroom
            it needs.
    """
    return path.startswith(ARENA_INPUT_DIRS) or path in ARENA_INPUT_FILES


def touches_arena(paths):
    """Whether a changeset reaches the arena proof.

    Args:
        paths: The repository-relative paths a commit range touched.

    Returns:
        bool: True when any path is an arena input. An empty changeset returns
            False -- there is nothing that could have moved the proof.
    """
    return any(is_arena_input(p) for p in paths if p)


def is_docs_only(paths):
    """Whether a changeset consists entirely of prose.

    Args:
        paths: The repository-relative paths a commit range touched.

    Returns:
        bool: True only when there is at least one path and every one of them
            is prose. An empty changeset returns False: there is nothing to
            prove the change is inert, so the full suite runs.
    """
    paths = [p for p in paths if p]
    return bool(paths) and all(is_prose(p) for p in paths)


def changed_paths(base, head):
    """The paths a commit range touched.

    Uses a three-dot range so the comparison is against the merge base, which
    keeps a pull request from being judged on commits that landed on its base
    branch after it was opened.

    Args:
        base: The older end of the range, or a falsy/NULL_SHA value when the
            range has no parent.
        head: The newer end of the range.

    Returns:
        list[str] | None: The changed paths, or None when the range cannot be
            resolved -- an absent endpoint, or a git invocation that failed.
    """
    if not base or not head or base == NULL_SHA or head == NULL_SHA:
        return None
    try:
        out = subprocess.run(
            ["git", "diff", "--no-renames", "--name-only", f"{base}...{head}"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (subprocess.CalledProcessError, OSError) as exc:
        print(f"cannot resolve {base}...{head}: {exc}", file=sys.stderr)
        return None
    return out.splitlines()


def main(argv=None):
    """Print the `code` and `arena` job outputs for GitHub Actions.

    Args:
        argv: Command-line arguments, defaulting to `sys.argv[1:]`.

    Returns:
        int: Process exit status. Always 0 -- an unresolvable range is reported
            as `code=true arena=true`, not as a failure, because refusing to
            classify must not itself break the pipeline.
    """
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--base", default="", help="older end of the commit range")
    ap.add_argument("--head", default="", help="newer end of the commit range")
    args = ap.parse_args(argv)

    paths = changed_paths(args.base, args.head)
    if paths is None:
        docs_only = False
        arena = True
    else:
        docs_only = is_docs_only(paths)
        arena = touches_arena(paths)
        for p in paths:
            labels = "prose" if is_prose(p) else "code "
            if is_arena_input(p):
                labels += " arena"
            print(f"{labels}  {p}", file=sys.stderr)

    print(f"code={'false' if docs_only else 'true'}")
    print(f"arena={'true' if arena else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
