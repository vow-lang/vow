#!/usr/bin/env python3
"""Z3 cross-check of the native verifier's Bitwuzla queries (#1431).

`vowc verify --backend native` decides each claim with one Bitwuzla query. A
single solver is a single point of trust: a Bitwuzla bug, or an encoder that
leans on behaviour only Bitwuzla has, turns into a false proof with nothing in
the pipeline to notice. This sweep puts a second, independent solver behind it.

How it works, per fixture:

  1. Run `vowc verify --backend native` with a `bitwuzla` shim first on PATH.
     The shim copies the `.smt2` file the product hands it and then `exec`s the
     real solver with the same argv, so the product's own watchdog, output and
     exit code are untouched. (The shape of `parity_c.py`'s fake `esbmc`.)
  2. Re-run every captured query offline under the real Bitwuzla and under Z3.
  3. Compare the verdicts.

Disagreements, and why they fail differently:

  SOUNDNESS  Bitwuzla `unsat`, Z3 `sat`. The proof was false.
  PRECISION  Bitwuzla `sat`, Z3 `unsat`. The counterexample is spurious.

`unknown` and `timeout` on either side are `inconclusive`: counted, never a
disagreement, and bounded from below by `--min-agreed` so a sweep that decided
nothing cannot read as clean.

Only standalone queries are captured: the runs set `VOW_VERIFY_NO_INCREMENTAL=1`
so every claim is one self-contained file. A session invocation (stdin commands,
no file) is logged and is a harness failure rather than a silent gap.

Exit codes follow verify_diff.py: 0 clean, 1 a disagreement, 2 the harness
cannot judge (no queries, a solver error, an unrunnable verifier).
"""

import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

import verifier_runtime

REPO_ROOT = verifier_runtime.REPO_ROOT

CLASSES = ("agree", "SOUNDNESS", "PRECISION", "inconclusive", "harness")
DISAGREEMENTS = ("SOUNDNESS", "PRECISION")

BITWUZLA_ARGS = ["--bv-output-format", "2"]

SHIM = """#!/usr/bin/env bash
set -u
if [ -z "${VOW_REAL_BITWUZLA:-}" ]; then
    echo "bitwuzla shim: VOW_REAL_BITWUZLA is not set" >&2
    exit 127
fi
last=""
for arg in "$@"; do
    case "$arg" in
        --version|--help|-h) exec "$VOW_REAL_BITWUZLA" "$@" ;;
    esac
    last="$arg"
done
case "$last" in
    *.smt2)
        if [ -f "$last" ]; then
            dest=$(mktemp "$VOW_QUERY_CAPTURE_DIR/q.XXXXXX")
            cp "$last" "$dest"
            mv "$dest" "$dest.smt2"
        fi
        ;;
    *)
        echo "$*" >> "$VOW_QUERY_CAPTURE_DIR/sessions.log"
        ;;
esac
exec "$VOW_REAL_BITWUZLA" "$@"
"""


def install_shim(directory):
    """Write the capturing `bitwuzla` into `directory` and return its path."""
    path = Path(directory) / "bitwuzla"
    path.write_text(SHIM)
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return path


def parse_verdict(stdout):
    """The first non-empty line decides, as `vc_parse_check_sat` does."""
    for line in stdout.splitlines():
        line = line.strip()
        if line:
            return line if line in ("sat", "unsat", "unknown", "timeout") else "error"
    return "error"


def classify(bitwuzla, z3):
    if z3 == "error":
        return "harness"
    if bitwuzla == "unsat" and z3 == "sat":
        return "SOUNDNESS"
    if bitwuzla == "sat" and z3 == "unsat":
        return "PRECISION"
    if bitwuzla == z3 and bitwuzla in ("sat", "unsat"):
        return "agree"
    return "inconclusive"


def exit_status(counts, min_agreed):
    if counts["harness"] or counts["agree"] < min_agreed:
        return 2
    if any(counts[c] for c in DISAGREEMENTS):
        return 1
    return 0


def _run_solver(argv, timeout_s):
    try:
        proc = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
            timeout=timeout_s,
        )
    except subprocess.TimeoutExpired:
        return "timeout"
    except OSError:
        return "error"
    return parse_verdict(proc.stdout)


def run_bitwuzla(binary, query, timeout_s):
    return _run_solver([binary, *BITWUZLA_ARGS, str(query)], timeout_s)


def run_z3(binary, query, timeout_s):
    # `-T` is Z3's own soft limit, in seconds; the subprocess deadline is the
    # backstop for a solver that ignores it.
    return _run_solver([binary, "-smt2", f"-T:{timeout_s}", str(query)], timeout_s + 5)


def solver_version(binary):
    try:
        proc = subprocess.run(
            [binary, "--version"], capture_output=True, text=True, timeout=30
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    return proc.stdout.strip().splitlines()[0] if proc.stdout.strip() else None


def capture_fixture(vowc, fixture, shim_dir, capture_dir, real_bitwuzla, timeout):
    """Run native `verify` on `fixture` and return what the shim captured.

    Returns `{"exit_code", "queries": [Path], "sessions": int, "timed_out"}`.
    The verify cache is disabled twice over (`--no-cache` and a fresh
    `VOW_CACHE_DIR`): a cache hit sends no query and would read as nothing
    to check.
    """
    capture_dir = Path(capture_dir)
    env = dict(os.environ)
    env["PATH"] = f"{shim_dir}{os.pathsep}{env.get('PATH', '')}"
    env["VOW_REAL_BITWUZLA"] = str(real_bitwuzla)
    env["VOW_QUERY_CAPTURE_DIR"] = str(capture_dir)
    env["VOW_VERIFY_NO_INCREMENTAL"] = "1"
    env["VOW_CACHE_DIR"] = str(capture_dir / "cache")
    (capture_dir / "cache").mkdir()
    timed_out, exit_code = False, None
    try:
        proc = subprocess.run(
            [
                str(vowc),
                "verify",
                "--backend",
                "native",
                "--no-cache",
                "--verify-jobs",
                "1",
                str(fixture),
            ],
            env=env,
            cwd=REPO_ROOT,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=timeout,
        )
        exit_code = proc.returncode
    except subprocess.TimeoutExpired:
        timed_out = True
    except OSError:
        exit_code = 127
    sessions_log = capture_dir / "sessions.log"
    sessions = (
        len(sessions_log.read_text().splitlines()) if sessions_log.exists() else 0
    )
    return {
        "exit_code": exit_code,
        "timed_out": timed_out,
        "queries": sorted(capture_dir.glob("q.*.smt2")),
        "sessions": sessions,
    }


def _parse_args(argv):
    ap = argparse.ArgumentParser(
        description="Re-run the native verifier's Bitwuzla queries under Z3 (#1431)"
    )
    ap.add_argument("roots", nargs="*", default=None)
    ap.add_argument("--vowc", default="build/vowc")
    ap.add_argument("--bitwuzla", default=None, help="default: first on PATH")
    ap.add_argument("--z3", default=None, help="default: first on PATH")
    ap.add_argument(
        "--extra-root",
        action="append",
        default=[],
        metavar="PATH",
        help="also sweep PATH (repeatable), e.g. a generated corpus",
    )
    ap.add_argument("--output-dir", default="z3-crosscheck.out")
    ap.add_argument("--timeout", type=int, default=300, help="per fixture, seconds")
    ap.add_argument("--solver-timeout", type=int, default=60, help="per query")
    ap.add_argument("--shard", default=None, metavar="K/N")
    ap.add_argument(
        "--min-agreed",
        type=int,
        default=1,
        help="fail (exit 2) if fewer than N queries were decided identically",
    )
    return ap.parse_args(argv)


def _resolve(explicit, name):
    found = explicit or shutil.which(name)
    if found and (Path(found).exists() or shutil.which(found)):
        return str(Path(shutil.which(found) or found).resolve())
    return None


def main(argv=None):
    args = _parse_args(argv)
    vowc = Path(args.vowc)
    vowc = vowc if vowc.is_absolute() else (REPO_ROOT / vowc)
    problems = []
    if not vowc.exists():
        problems.append(f"vowc not found: {vowc}")
    bitwuzla = _resolve(args.bitwuzla, "bitwuzla")
    if not bitwuzla:
        problems.append("bitwuzla not found")
    z3 = _resolve(args.z3, "z3")
    if not z3:
        problems.append("z3 not found")
    if problems:
        for p in problems:
            print(f"error: {p}", file=sys.stderr)
        return 2

    corpus = verifier_runtime.collect_corpus(
        verifier_runtime.resolve_roots(args.roots, args.extra_root, "native"),
        args.shard,
    )
    outdir = Path(args.output_dir)
    outdir.mkdir(parents=True, exist_ok=True)
    saved = outdir / "queries"
    saved.mkdir(exist_ok=True)

    versions = {"bitwuzla": solver_version(bitwuzla), "z3": solver_version(z3)}
    print(f"=== Z3 cross-check of native queries: {len(corpus)} fixtures ===")
    print(f"  vowc     : {vowc}")
    print(f"  bitwuzla : {versions['bitwuzla']}")
    print(f"  z3       : {versions['z3']}")
    print()

    counts = {c: 0 for c in CLASSES}
    records, disagreements, harness = [], [], []
    queries = sessions = 0
    with tempfile.TemporaryDirectory(prefix="vow-z3-crosscheck-") as work:
        shim_dir = Path(work) / "bin"
        shim_dir.mkdir()
        install_shim(shim_dir)
        for i, fixture in enumerate(corpus, 1):
            rel = verifier_runtime.relative_to_repo(fixture)
            text = fixture.read_text(errors="replace")
            skip = verifier_runtime.DIRECTIVE_SKIP.search(text)
            if skip:
                records.append({"file": rel, "skipped": skip.group(1)})
                continue
            capture_dir = Path(work) / f"f{i}"
            capture_dir.mkdir()
            cap = capture_fixture(
                vowc, fixture, shim_dir, capture_dir, bitwuzla, args.timeout
            )
            sessions += cap["sessions"]
            if cap["sessions"]:
                harness.append(
                    f"{rel}: {cap['sessions']} incremental session(s) bypassed capture"
                )
            fixture_counts = {c: 0 for c in CLASSES}
            for n, query in enumerate(cap["queries"]):
                b = run_bitwuzla(bitwuzla, query, args.solver_timeout)
                z = run_z3(z3, query, args.solver_timeout)
                verdict = classify(b, z)
                fixture_counts[verdict] += 1
                if verdict in DISAGREEMENTS or verdict == "harness":
                    dest = saved / f"{_slug(rel)}.{n}.smt2"
                    shutil.copyfile(query, dest)
                    entry = {
                        "fixture": rel,
                        "class": verdict,
                        "bitwuzla": b,
                        "z3": z,
                        "query": str(dest.relative_to(outdir)),
                    }
                    if verdict == "harness":
                        harness.append(f"{rel}: z3 gave no verdict on {entry['query']}")
                        continue
                    disagreements.append(entry)
                    print(f"  {verdict} {rel}")
                    print(f"            bitwuzla={b} z3={z}  {entry['query']}")
            for c in CLASSES:
                counts[c] += fixture_counts[c]
            queries += len(cap["queries"])
            records.append(
                {
                    "file": rel,
                    "exit_code": cap["exit_code"],
                    "timed_out": cap["timed_out"],
                    "queries": len(cap["queries"]),
                    "counts": fixture_counts,
                }
            )
            if i % 20 == 0:
                print(f"  ... {i}/{len(corpus)}")
            shutil.rmtree(capture_dir, ignore_errors=True)

    if queries == 0:
        harness.append("no query was captured: nothing was cross-checked")
    status = exit_status({**counts, "harness": len(harness)}, args.min_agreed)
    results = {
        "schema_version": 1,
        "vowc": str(vowc),
        "solvers": versions,
        "corpus_size": len(corpus),
        "queries": queries,
        "sessions": sessions,
        "counts": counts,
        "disagreements": disagreements,
        "harness": harness,
        "records": records,
    }
    (outdir / "results.json").write_text(json.dumps(results, indent=2) + "\n")

    print()
    print("=== Summary ===")
    print(f"  fixtures      : {len(corpus)}")
    print(f"  queries       : {queries}")
    print(f"  agree         : {counts['agree']}")
    print(f"  inconclusive  : {counts['inconclusive']}  (unknown/timeout)")
    print(f"  SOUNDNESS     : {counts['SOUNDNESS']}  (bitwuzla unsat, z3 sat)")
    print(f"  PRECISION     : {counts['PRECISION']}  (bitwuzla sat, z3 unsat)")
    for h in harness:
        print(f"  HARNESS       : {h}")
    print(f"  results       : {outdir / 'results.json'}")
    if status == 2 and not harness:
        print(
            f"\nFAIL: only {counts['agree']} queries agreed, "
            f"need >= {args.min_agreed}.",
            file=sys.stderr,
        )
    return status


def _slug(rel):
    return rel.replace("/", "__")


if __name__ == "__main__":
    sys.exit(main())
