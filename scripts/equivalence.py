#!/usr/bin/env python3
"""Differential equivalence runner: Rust bootstrap vs self-hosted compiler.

Cross-checks the two compilers over a corpus of `.vow` files on four
observables (#1081):

  accept/reject  both compile, or both reject
  error_code     when both reject, the multiset of diagnostic codes agrees
  runtime        when both compile, stdout + exit code of the two binaries agree
  fail_closed    neither compiler may panic, and neither emitted binary may die
                 on a signal — a clean `error[...]` is always acceptable

The binary fixed point proves the self-hosted compiler reproduces *itself* over
the exact source of the compiler. It says nothing about inputs outside that
source. This runner covers that gap, and is the shared execution layer the
fuzzer (#905), the adversarial pair review (#1083), and the verifier-model axis
(#1084) all build on rather than each growing its own harness.

Exit status is 0 only when there are no divergences AND the run cleared its
coverage floor (see --min-compared): a sweep that skipped everything is a
failure to measure, not a pass.

Which self-hosted binary is compared matters, and the results file records its
digest. `build/vowc` is the verified fixed point and is the default. A stage-1
binary (Rust-compiled `compiler/main.vow`) is a legitimate target too, but a
divergence found against it may live either in the self-hosted source or in the
Rust compiler's lowering of that source — the two are only distinguishable by
re-running against the fixed point.
"""

import argparse
import hashlib
import json
import os
import re
import resource
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# `ulimit -v` equivalent for self-hosted binaries, in bytes (2 GB).
SELF_MEM_LIMIT = 2_000_000 * 1024

# Signals Vow uses deliberately: a checked-arithmetic overflow, a vow violation
# in debug mode, and a division guard all terminate the process on purpose. When
# BOTH compilers produce the same one, that is agreement about designed
# behaviour, not a divergence.
TRAP_SIGNALS = {4: "SIGILL", 5: "SIGTRAP", 6: "SIGABRT", 8: "SIGFPE"}

# Memory-unsafety signals. Worth reporting even when both compilers agree: #905
# names "no input may produce a binary that dies on SIGSEGV" as an invariant
# that holds independently of equivalence.
UNSAFE_SIGNALS = {7: "SIGBUS", 11: "SIGSEGV"}

# A compiler that panics is always a bug, however the panic is spelled.
PANIC_MARKERS = (
    "thread 'main' panicked",
    "panicked at",
    "not yet implemented",
    "internal compiler error",
)


# ---------------------------------------------------------------------------
# Provenance
# ---------------------------------------------------------------------------


def fingerprint(path):
    """Identify a compiler binary so a report can never be misattributed.

    A stale `build/vowc` silently turns this whole runner into a test of last
    week's compiler, so the digest goes in the results file and the summary.
    """
    p = Path(path)
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return {
        "path": str(p),
        "sha256": h.hexdigest(),
        "size": p.stat().st_size,
        "mtime": int(p.stat().st_mtime),
    }


# ---------------------------------------------------------------------------
# Corpus
# ---------------------------------------------------------------------------

# `// TEST:` directives are the existing corpus convention (full_test.sh
# Section 4); honouring them keeps this runner's view of a fixture identical to
# the suite's, so a divergence here is never an artifact of feeding a fixture
# the wrong stdin.
DIRECTIVE_SKIP = re.compile(r'^// TEST: skip "(.*)"$', re.M)
DIRECTIVE_STDIN = re.compile(r'^// TEST: stdin "(.*)"$', re.M)
DIRECTIVE_STDIN_FILE = re.compile(r"^// TEST: stdin-file (.*)$", re.M)
DIRECTIVE_VERIFY_ONLY = re.compile(r"^// TEST: verify-only$", re.M)
DIRECTIVE_EXIT = re.compile(r"^// TEST: exit ([0-9]+)$", re.M)


def read_directives(path):
    text = Path(path).read_text(errors="replace")
    skip = DIRECTIVE_SKIP.search(text)
    stdin_inline = DIRECTIVE_STDIN.search(text)
    stdin_file = DIRECTIVE_STDIN_FILE.search(text)
    exit_code = DIRECTIVE_EXIT.search(text)
    return {
        "skip": skip.group(1) if skip else None,
        "expected_exit": int(exit_code.group(1)) if exit_code else None,
        "verify_only": bool(DIRECTIVE_VERIFY_ONLY.search(text)),
        "stdin": (
            stdin_inline.group(1).encode().decode("unicode_escape")
            if stdin_inline
            else None
        ),
        "stdin_file": (
            str(Path(path).parent / stdin_file.group(1)) if stdin_file else None
        ),
    }


def stdin_bytes(directives):
    if directives["stdin_file"]:
        p = Path(directives["stdin_file"])
        return p.read_bytes() if p.exists() else b""
    if directives["stdin"] is not None:
        return directives["stdin"].encode()
    return b""


def collect_corpus(roots, exclude):
    """Expand roots (files or directories) into a sorted, deduplicated list.

    Sorted so a sharded run is reproducible: shard k of n must always mean the
    same set of files for a given corpus.
    """
    out = []
    for root in roots:
        p = Path(root)
        if p.is_file():
            out.append(p)
        elif p.is_dir():
            out.extend(sorted(p.rglob("*.vow")))
    seen, result = set(), []
    for p in out:
        rp = p.resolve()
        if rp in seen:
            continue
        seen.add(rp)
        if any(pat in str(p) for pat in exclude):
            continue
        result.append(p)
    return sorted(result, key=str)


# ---------------------------------------------------------------------------
# Execution
# ---------------------------------------------------------------------------


def _limit_memory():
    resource.setrlimit(resource.RLIMIT_AS, (SELF_MEM_LIMIT, SELF_MEM_LIMIT))


def run_compiler(binary, args, timeout, limit_memory):
    """Run one compiler. Never raises: a crash is data, not an error."""
    try:
        proc = subprocess.run(
            [str(binary)] + args,
            capture_output=True,
            cwd=REPO_ROOT,
            timeout=timeout,
            preexec_fn=_limit_memory if limit_memory else None,
        )
    except subprocess.TimeoutExpired:
        return {"timeout": True, "exit": None, "stdout": "", "stderr": "", "json": None}
    stdout = proc.stdout.decode(errors="replace")
    stderr = proc.stderr.decode(errors="replace")
    parsed = None
    if stdout.strip():
        try:
            parsed = json.loads(stdout.strip())
        except json.JSONDecodeError:
            parsed = None
    return {
        "timeout": False,
        "exit": proc.returncode,
        "stdout": stdout,
        "stderr": stderr,
        "json": parsed,
    }


def run_binary(path, stdin_data, timeout, limit_memory):
    try:
        proc = subprocess.run(
            [str(path)],
            input=stdin_data,
            capture_output=True,
            cwd=REPO_ROOT,
            timeout=timeout,
            preexec_fn=_limit_memory if limit_memory else None,
        )
    except subprocess.TimeoutExpired:
        return {"timeout": True, "exit": None, "stdout": b""}
    return {"timeout": False, "exit": proc.returncode, "stdout": proc.stdout}


# ---------------------------------------------------------------------------
# Observables
# ---------------------------------------------------------------------------


def error_codes(result):
    j = result["json"] or {}
    return sorted(d.get("error_code", "") for d in (j.get("diagnostics") or []))


def status_of(result):
    return (result["json"] or {}).get("status")


def compiled_ok(result):
    """Did this compiler produce an executable?"""
    j = result["json"] or {}
    return bool(j.get("executable"))


def panic_markers(result):
    hay = result["stderr"]
    return [m for m in PANIC_MARKERS if m in hay]


def check_fail_closed(name, result):
    """A panic or a signal death is a bug regardless of what the peer did.

    One panic yields one finding even when several markers match it (a Rust
    panic line contains both "thread 'main' panicked" and "panicked at"), so
    the divergence count stays a count of distinct bugs.
    """
    out = []
    markers = panic_markers(result)
    if markers:
        out.append(
            {
                "observable": "fail_closed",
                "detail": f"{name} compiler panicked: {markers[0]!r}",
            }
        )
    if result["exit"] is not None and result["exit"] < 0:
        out.append(
            {
                "observable": "fail_closed",
                "detail": f"{name} compiler died on signal {-result['exit']}",
            }
        )
    return out


def compare_build(rust, slf):
    """Accept/reject and error_code parity. Returns a divergence list."""
    div = []
    r_ok, s_ok = compiled_ok(rust), compiled_ok(slf)
    if r_ok != s_ok:
        div.append(
            {
                "observable": "accept_reject",
                "detail": (
                    f"rust {'accepted' if r_ok else 'rejected'} but "
                    f"self-hosted {'accepted' if s_ok else 'rejected'} "
                    f"(status {status_of(rust)} vs {status_of(slf)})"
                ),
            }
        )
        return div

    if not r_ok:
        rc, sc = error_codes(rust), error_codes(slf)
        if rc != sc:
            div.append(
                {
                    "observable": "error_code",
                    "detail": f"both rejected but codes differ: {rc} vs {sc}",
                }
            )
    return div


def expected_signal(directives):
    """The signal a fixture DECLARES it dies on, if any.

    A checked-arithmetic overflow trap is the feature working: such fixtures
    carry `// TEST: exit 132` (128 + SIGILL). Reporting that as a fail_closed
    finding would make the runner cry wolf on its own test corpus, so a
    declared 128+N exit converts to an expected signal N.
    """
    want = directives.get("expected_exit")
    if want is not None and want > 128:
        return want - 128
    return None


def compare_runtime(rust_bin, self_bin, stdin_data, timeout, expect_signal=None):
    """Runtime parity, with a nondeterminism guard.

    A program whose own output varies between two runs of the SAME binary
    (clock reads, pids, hash iteration order) cannot yield a meaningful
    cross-compiler comparison — reporting it as a divergence would be a false
    positive, so it is reported as skipped-nondeterministic instead.
    """
    # Both binaries run under the SAME limits. Limiting only the self-hosted
    # side (the repo's `ulimit -v` convention, which exists for running the
    # memory-hungry self-hosted *compiler*) would make any program needing more
    # than the cap look like a miscompile.
    r1 = run_binary(rust_bin, stdin_data, timeout, limit_memory=True)
    r2 = run_binary(rust_bin, stdin_data, timeout, limit_memory=True)
    if r1["timeout"] or r2["timeout"]:
        # The reference side never finished, so there is nothing to compare
        # against: genuinely inconclusive.
        return [], "runtime-timeout"
    if r1["stdout"] != r2["stdout"] or r1["exit"] != r2["exit"]:
        return [], "nondeterministic"

    s = run_binary(self_bin, stdin_data, timeout, limit_memory=True)
    if s["timeout"]:
        # One-sided: the Rust binary terminated deterministically and the
        # self-hosted one did not. That distinguishes the two implementations,
        # so it is a finding — a codegen regression turning a terminating
        # program into an infinite loop must not read as merely "skipped".
        return [
            {
                "observable": "runtime",
                "detail": (
                    f"self-hosted binary timed out after {timeout}s; "
                    f"rust exited {r1['exit']}"
                ),
            }
        ], None

    div = []
    if r1["exit"] != s["exit"]:
        div.append(
            {
                "observable": "runtime",
                "detail": f"exit code {r1['exit']} vs {s['exit']}",
            }
        )
    if r1["stdout"] != s["stdout"]:
        div.append(
            {
                "observable": "runtime",
                "detail": (
                    f"stdout differs ({len(r1['stdout'])} vs {len(s['stdout'])} bytes)"
                ),
            }
        )
    # A signal death the two compilers DISAGREE on is already reported above as
    # an exit-code divergence. What is left to judge is a signal both produced:
    # a deliberate trap is the language working, memory unsafety never is.
    both = {
        -r1["exit"] if r1["exit"] is not None and r1["exit"] < 0 else None,
        -s["exit"] if s["exit"] is not None and s["exit"] < 0 else None,
    }
    if len(both) == 1:
        signal = both.pop()
        if signal is not None:
            # Memory unsafety is classified BEFORE the declared-exit check. A
            # fixture may carry `// TEST: exit 139`, but #905 makes "no input
            # produces a binary that dies on SIGSEGV" an invariant that holds
            # independently of equivalence and independently of what the
            # fixture declares — a declaration cannot license the finding away.
            if signal in UNSAFE_SIGNALS:
                div.append(
                    {
                        "observable": "fail_closed",
                        "detail": (
                            f"both binaries died on {UNSAFE_SIGNALS[signal]} "
                            f"({signal}) — memory unsafety, not a trap"
                        ),
                    }
                )
            elif signal != expect_signal and signal not in TRAP_SIGNALS:
                div.append(
                    {
                        "observable": "fail_closed",
                        "detail": (
                            f"both binaries died on unclassified signal {signal}"
                        ),
                    }
                )
    return div, None


# ---------------------------------------------------------------------------
# Per-file driver
# ---------------------------------------------------------------------------


def check_file(vow_file, rust, slf, outdir, timeout):
    rel = (
        str(Path(vow_file).relative_to(REPO_ROOT))
        if str(vow_file).startswith(str(REPO_ROOT))
        else str(vow_file)
    )
    directives = read_directives(vow_file)
    record = {"file": rel, "divergences": [], "skipped": None}

    if directives["skip"]:
        record["skipped"] = f"directive: {directives['skip']}"
        return record

    stem = hashlib.sha256(rel.encode()).hexdigest()[:16]
    rust_out = outdir / f"rust_{stem}"
    self_out = outdir / f"self_{stem}"

    # `--no-cache` is mandatory, not hygiene: the compile-object cache is keyed
    # on dependency content + mode + a hand-bumped ABI string, NOT on the
    # compiler binary, and both compilers share $VOW_CACHE_DIR. Without it a
    # cached object from the peer compiler can be linked in and the runtime
    # observable silently compares a binary to itself.
    args = ["build", "--no-verify", "--no-cache", str(vow_file)]
    r = run_compiler(rust, args + ["-o", str(rust_out)], timeout, False)
    s = run_compiler(slf, args + ["-o", str(self_out)], timeout, True)

    record["divergences"] += check_fail_closed("rust", r)
    record["divergences"] += check_fail_closed("self-hosted", s)

    if r["timeout"] or s["timeout"]:
        which = "rust" if r["timeout"] else "self-hosted"
        record["skipped"] = f"compile timeout ({which})"
        return record

    # No parseable JSON from a compiler that did not panic means the contract
    # "always emit structured output" was broken — that is itself a finding,
    # not a reason to skip.
    for name, res in (("rust", r), ("self-hosted", s)):
        if res["json"] is None and not panic_markers(res):
            record["divergences"].append(
                {
                    "observable": "fail_closed",
                    "detail": (
                        f"{name} emitted no parseable JSON (exit {res['exit']})"
                    ),
                }
            )
    if r["json"] is None or s["json"] is None:
        record["status"] = {"rust": status_of(r), "self": status_of(s)}
        return record

    record["status"] = {"rust": status_of(r), "self": status_of(s)}
    record["divergences"] += compare_build(r, s)

    if compiled_ok(r) and compiled_ok(s) and not record["divergences"]:
        rt_div, why = compare_runtime(
            rust_out,
            self_out,
            stdin_bytes(directives),
            timeout,
            expect_signal=expected_signal(directives),
        )
        record["divergences"] += rt_div
        if why:
            record["skipped"] = why
    elif not record["divergences"] and not compiled_ok(r):
        # Both rejected and agreed on why: the build observables did their job,
        # there is simply no binary to compare. Not a divergence, and not an
        # unexamined file either. A file that DID diverge is never labelled
        # skipped — that would understate coverage in the skip histogram.
        record["skipped"] = record["skipped"] or "both rejected (no runtime check)"

    # Codegen leaves a sibling .o next to each executable; a full-corpus sweep
    # would otherwise accumulate one per file per compiler.
    for p in (rust_out, self_out):
        for victim in (p, p.with_suffix(".o")):
            try:
                os.unlink(victim)
            except OSError:
                pass
    return record


# ---------------------------------------------------------------------------
# Ledger
# ---------------------------------------------------------------------------

LEDGER_PATH = REPO_ROOT / "docs" / "equivalence" / "ledger.json"


def load_ledger(path=None):
    """Known divergences, keyed by repo-relative path.

    A tracked divergence is not news. Without this the nightly reports the same
    13 known-gap fixtures every run, and a genuinely new finding is lost in the
    noise — the failure mode that makes recurring jobs get ignored.
    """
    p = Path(path) if path else LEDGER_PATH
    if not p.exists():
        return {}
    try:
        return json.loads(p.read_text()).get("corpus", {})
    except (json.JSONDecodeError, OSError):
        return {}


def tracked_observables(entry):
    """The observable(s) a ledger entry documents.

    A list is accepted alongside the schema's single string so an entry can
    grow a second tracked observable without a schema migration.

    Args:
        entry: A ledger entry dict, or None for an untracked file.

    Returns:
        frozenset: The observable names this entry documents.
    """
    obs = (entry or {}).get("observable")
    if obs is None:
        return frozenset()
    return frozenset([obs] if isinstance(obs, str) else obs)


def reconcile(records, ledger):
    """Split findings into new, known, and disappeared.

    Matching is on (file, observable) — never on the path alone. A ledger entry
    documents one specific asymmetry, so a file tracked for an `error_code` gap
    that ALSO starts dying on SIGSEGV has produced a genuinely new finding.
    Folding that into `known` because the path happens to be listed would
    reintroduce the exact suppression the ledger exists to prevent, one level
    down. A record therefore contributes to both lists when it carries a mix.

    A tracked divergence that stopped reproducing is reported as `fixed` and
    treated as a failure, mirroring verify_eval.py's GAP_FIXED: a welcome change
    must force the ledger to be updated rather than silently drifting out of
    date. A ledger nobody maintains is worse than none, because it suppresses
    real findings.

    Args:
        records: Per-file result records from check_file.
        ledger: Known divergences keyed by repo-relative path.

    Returns:
        tuple: (new, known, fixed) — new/known are record lists carrying only
        their untracked/tracked divergences respectively; fixed is a list of
        paths whose tracked observable no longer reproduces.
    """
    new, known, fixed = [], [], []
    for rec in records:
        entry = ledger.get(rec["file"])
        tracked = tracked_observables(entry)
        matched = [d for d in rec["divergences"] if d["observable"] in tracked]
        untracked = [d for d in rec["divergences"] if d["observable"] not in tracked]
        if untracked:
            new.append({**rec, "divergences": untracked})
        if matched:
            known.append({**rec, "divergences": matched})
        if entry and entry.get("status") in ("open", "expected") and not matched:
            fixed.append(rec["file"])
    return new, known, fixed


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main():
    ap = argparse.ArgumentParser(
        description="Differential equivalence runner: Rust vs self-hosted (#1081)"
    )
    ap.add_argument(
        "roots",
        nargs="*",
        default=None,
        help="files or directories to sweep (default: the standard corpus)",
    )
    ap.add_argument("--rust", default="target/release/vow")
    ap.add_argument("--self", dest="self_bin", default="build/vowc")
    ap.add_argument("--output-dir", default="equivalence.out")
    ap.add_argument(
        "--timeout",
        type=int,
        default=300,
        help="per-invocation timeout in seconds (default: 300)",
    )
    ap.add_argument(
        "--shard",
        default=None,
        metavar="K/N",
        help="run only shard K of N (round-robin over the sorted corpus)",
    )
    ap.add_argument(
        "--exclude", action="append", default=[], help="substring filter; repeatable"
    )
    ap.add_argument(
        "--min-compared",
        type=int,
        default=1,
        help="fail the run if fewer than N files were actually compared",
    )
    ap.add_argument(
        "--ledger",
        default=None,
        help="path to ledger.json (default: docs/equivalence/ledger.json)",
    )
    ap.add_argument(
        "--no-ledger",
        action="store_true",
        help="report every divergence as new, ignoring tracked ones",
    )
    args = ap.parse_args()

    roots = args.roots or [
        REPO_ROOT / "tests",
        REPO_ROOT / "examples",
        REPO_ROOT / "stdlib",
        REPO_ROOT / "benchmarks",
        REPO_ROOT / "euler",
    ]

    rust = Path(args.rust)
    slf = Path(args.self_bin)
    for p in (rust, slf):
        if not p.exists():
            print(f"error: compiler not found: {p}", file=sys.stderr)
            return 2

    corpus = collect_corpus(roots, args.exclude)
    if args.shard:
        k, n = (int(x) for x in args.shard.split("/"))
        corpus = [f for i, f in enumerate(corpus) if i % n == k]

    outdir = Path(args.output_dir)
    outdir.mkdir(parents=True, exist_ok=True)

    print(f"=== Differential equivalence sweep: {len(corpus)} files ===")
    print(f"  rust: {rust}")
    print(f"  self: {slf}")
    print()

    ledger = {} if args.no_ledger else load_ledger(args.ledger)
    started = time.time()
    records, diverged, skipped, compared = [], [], [], 0
    for i, f in enumerate(corpus, 1):
        rec = check_file(f, rust, slf, outdir, args.timeout)
        records.append(rec)
        if rec["divergences"]:
            diverged.append(rec)
            print(f"  DIVERGE {rec['file']}")
            for d in rec["divergences"]:
                print(f"          [{d['observable']}] {d['detail']}")
        elif rec["skipped"]:
            skipped.append(rec)
        else:
            compared += 1
        if i % 25 == 0:
            print(f"  ... {i}/{len(corpus)}")

    new, known, fixed = reconcile(records, ledger)
    elapsed = int(time.time() - started)
    results = {
        "schema_version": 1,
        "compilers": {"rust": fingerprint(rust), "self_hosted": fingerprint(slf)},
        "corpus_size": len(corpus),
        "compared": compared,
        "diverged": len(diverged),
        "new_divergences": [r["file"] for r in new],
        "known_divergences": [r["file"] for r in known],
        "no_longer_diverging": fixed,
        "skipped": len(skipped),
        "elapsed_secs": elapsed,
        "records": records,
    }
    (outdir / "results.json").write_text(json.dumps(results, indent=2))

    # Report what was NOT covered. A sweep that silently skipped most of the
    # corpus reads as "all clear" when it measured almost nothing.
    print()
    print("=== Summary ===")
    print(f"  compared : {compared}")
    print(f"  diverged : {len(diverged)}  (new: {len(new)}, tracked: {len(known)})")
    print(f"  skipped  : {len(skipped)}")
    print(f"  elapsed  : {elapsed}s")
    if skipped:
        reasons = {}
        for rec in skipped:
            key = rec["skipped"].split("(")[0].strip()
            reasons[key] = reasons.get(key, 0) + 1
        print("  skip reasons:")
        for reason, count in sorted(reasons.items(), key=lambda kv: -kv[1]):
            print(f"    {count:5d}  {reason}")
    print(f"  results  : {outdir / 'results.json'}")

    if fixed:
        print()
        print("  NO LONGER DIVERGING — update docs/equivalence/ledger.json:")
        for f in fixed:
            entry = ledger.get(f, {})
            issue = entry.get("issue")
            print(f"    {f}" + (f"  (issue #{issue})" if issue else ""))

    if compared < args.min_compared:
        print(
            f"\nFAIL: only {compared} files compared, need >= {args.min_compared}"
            " — the sweep did not measure enough to be meaningful.",
            file=sys.stderr,
        )
        return 2
    # Tracked divergences do not fail the run; new ones and stale ledger
    # entries do.
    return 1 if (new or fixed) else 0


if __name__ == "__main__":
    sys.exit(main())
