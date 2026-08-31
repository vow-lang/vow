#!/usr/bin/env python3
"""Classify a commit range as documentation-only, for ci.yml's job gate.

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
"""

import argparse
import subprocess
import sys

# Markdown under these prefixes is build input, not documentation: the compiler
# sources embed a copy, and a test asserts the two agree. See the module
# docstring for the specific mechanisms.
BUILD_INPUT_PREFIXES = ("docs/spec/", "skills/")

PROSE_SUFFIX = ".md"

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
    """Print `code=true` or `code=false` for a GitHub Actions job output.

    Args:
        argv: Command-line arguments, defaulting to `sys.argv[1:]`.

    Returns:
        int: Process exit status. Always 0 -- an unresolvable range is reported
            as `code=true`, not as a failure, because refusing to classify must
            not itself break the pipeline.
    """
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--base", default="", help="older end of the commit range")
    ap.add_argument("--head", default="", help="newer end of the commit range")
    args = ap.parse_args(argv)

    paths = changed_paths(args.base, args.head)
    if paths is None:
        docs_only = False
    else:
        docs_only = is_docs_only(paths)
        for p in paths:
            print(f"{'prose' if is_prose(p) else 'code '}  {p}", file=sys.stderr)

    print(f"code={'false' if docs_only else 'true'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
