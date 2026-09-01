#!/usr/bin/env python3
"""Differential equivalence runner: Rust bootstrap vs self-hosted compiler.

Cross-checks the two compilers over a corpus of `.vow` files on four
observables (#1081):

  accept/reject  both compile, or both reject
  error_code     when both reject, the multiset of diagnostic codes agrees
  runtime        when both compile, the two binaries' stdout agrees
  runtime_exit   ... and so do their exit codes, tracked separately so a known
                 wrong-output gap cannot also hide a wrong exit status
  exit_code      the two compiler PROCESSES agree on their own exit status,
                 which docs/spec/cli.md defines as part of the CLI contract
  verify_status  for `// TEST: verify-only` fixtures, the two verifiers agree
                 on the verification verdict
  fixture_error  the corpus itself is broken (e.g. a declared stdin-file is
                 missing) — a finding about the fixture, not either compiler
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
import copy
import hashlib
import json
import os
import re
import resource
import subprocess
import sys
import time
from datetime import datetime, timezone
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
DIRECTIVE_SKIP = re.compile(r'^// TEST: skip "(.*)"$', re.MULTILINE)
DIRECTIVE_STDIN = re.compile(r'^// TEST: stdin "(.*)"$', re.MULTILINE)
DIRECTIVE_STDIN_FILE = re.compile(r"^// TEST: stdin-file (.*)$", re.MULTILINE)
DIRECTIVE_VERIFY_ONLY = re.compile(r"^// TEST: verify-only$", re.MULTILINE)
DIRECTIVE_EXIT = re.compile(r"^// TEST: exit ([0-9]+)$", re.MULTILINE)


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


class MissingStdinFile(Exception):
    """A fixture declares `// TEST: stdin-file` naming a file that is absent."""


def stdin_bytes(directives):
    """The stdin a fixture declares.

    Args:
        directives: The parsed `// TEST:` directives for one fixture.

    Returns:
        bytes: The declared stdin, empty when none is declared.

    Raises:
        MissingStdinFile: The declared stdin-file does not exist. Substituting
            empty stdin would let both binaries agree only because both got the
            wrong input, hiding the behaviour the fixture exists to exercise —
            and it would still count as a completed comparison. full_test.sh
            treats this as a hard fixture error, so this does too.
    """
    if directives["stdin_file"]:
        p = Path(directives["stdin_file"])
        if not p.exists():
            raise MissingStdinFile(directives["stdin_file"])
        return p.read_bytes()
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
            check=False,
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
            check=False,
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


def signal_of(result):
    """The signal a process died on, or None if it exited normally.

    Args:
        result: A run_binary or run_compiler result dict.

    Returns:
        int | None: The signal number, or None.
    """
    code = result["exit"]
    return -code if code is not None and code < 0 else None


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
                    "rust": rc,
                    "self_hosted": sc,
                }
            )

    # Executable parity agreeing does not mean the verdicts agree: `Unverified`
    # vs `Verified` (both exit 0, both with an executable) and `CompileFailed`
    # vs `VerifyFailed` (both exit 1, matching diagnostics) are distinct CLI
    # outcomes per docs/spec/cli.md.
    r_status, s_status = status_of(rust), status_of(slf)
    if r_status != s_status:
        div.append(
            {
                "observable": "accept_reject",
                "detail": (
                    f"same executable parity but status differs: "
                    f"{r_status} vs {s_status}"
                ),
            }
        )

    # The two compilers reached the same verdict; their PROCESS exit status is
    # a separate promise. docs/spec/cli.md pins an exit code per outcome and
    # full_test.sh::compare_json already enforces it, so a compiler returning 0
    # for CompileFailed (or nonzero for a clean build) is a CLI-contract
    # regression this sweep would otherwise pass.
    if rust["exit"] != slf["exit"]:
        div.append(
            {
                "observable": "exit_code",
                "detail": (
                    f"same verdict but process exit differs: "
                    f"{rust['exit']} vs {slf['exit']}"
                ),
            }
        )
    return div


def verify_outcome(result):
    """The backend verification outcome behind an aggregate status.

    `VerifyFailed` covers both a real counterexample and a soft backend failure
    (timeout, unknown, tool error, panic), which cli.md distinguishes through
    `verify_status` and `counterexamples[]` rather than the status alone.

    Args:
        result: A run_compiler result dict.

    Returns:
        tuple: (verify_status, sorted (function, vow_id, blame) identities).
    """
    j = result["json"] or {}
    # Identity, not count: two runs each reporting one counterexample are not
    # in agreement when they name different functions, vow_ids or blame, and
    # full_test.sh::compare_json already treats those fields as significant.
    cexs = sorted(
        (
            str(c.get("function")),
            str(c.get("vow_id")),
            str(c.get("blame")),
        )
        for c in (j.get("counterexamples") or [])
    )
    return (j.get("verify_status"), cexs)


def compare_verify(rust, slf):
    """Verification-verdict parity for `// TEST: verify-only` fixtures.

    These fixtures are library modules with no `main`, so `build` yields no
    executable on either side and the build observables can only ever report
    "both rejected". The verifier verdict is the whole point of the directive,
    so it is compared directly instead.

    Args:
        rust: run_compiler result for the Rust compiler.
        slf: run_compiler result for the self-hosted compiler.

    Returns:
        list: Divergence dicts, empty when the two verifiers agree.
    """
    div = []
    r_status, s_status = status_of(rust), status_of(slf)
    if r_status != s_status:
        div.append(
            {
                "observable": "verify_status",
                "detail": f"verification verdict differs: {r_status} vs {s_status}",
            }
        )
    # A shared `VerifyFailed` is not agreement: one side may have produced a
    # counterexample while the other merely timed out or errored. cli.md makes
    # verify_status ("timeout"/"unknown"/"error"/"tool_not_found"/"panicked")
    # and counterexamples[] the fields that distinguish those, and both cases
    # commonly carry exit 1 and no diagnostics.
    r_backend, s_backend = verify_outcome(rust), verify_outcome(slf)
    if r_backend != s_backend:
        div.append(
            {
                "observable": "verify_status",
                "detail": (f"verifier outcome differs: {r_backend} vs {s_backend}"),
            }
        )

    rc, sc = error_codes(rust), error_codes(slf)
    if rc != sc:
        div.append(
            {
                "observable": "error_code",
                "detail": f"verify diagnostics differ: {rc} vs {sc}",
                "rust": rc,
                "self_hosted": sc,
            }
        )
    if rust["exit"] != slf["exit"]:
        div.append(
            {
                "observable": "exit_code",
                "detail": (
                    f"verify process exit differs: {rust['exit']} vs {slf['exit']}"
                ),
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
    rust_hung = r1["timeout"] or r2["timeout"]
    if not rust_hung and (r1["stdout"] != r2["stdout"] or r1["exit"] != r2["exit"]):
        return [], "nondeterministic"

    # The peer runs even when the reference hung: "one hangs, one terminates"
    # is itself a difference between the implementations, in either direction.
    #
    # Deliberately run ONCE, against the reference's two runs. The double-run
    # exists to spot a program whose own output varies, which would make any
    # cross-compiler comparison meaningless; establishing that on one side is
    # enough. If the self-hosted binary is the unstable one, a single
    # mismatching run against a stable r1/r2 is reported as a runtime
    # divergence — which is the right answer, since self-hosted-only
    # instability is itself a miscompile. Three runs, not four.
    s = run_binary(self_bin, stdin_data, timeout, limit_memory=True)

    if rust_hung and s["timeout"]:
        # Neither side finished, so nothing distinguishes them.
        return [], "runtime-timeout"
    if rust_hung != s["timeout"]:
        hung, ran = ("rust", "self-hosted") if rust_hung else ("self-hosted", "rust")
        ran_exit = s["exit"] if rust_hung else r1["exit"]
        return [
            {
                "observable": "runtime",
                "detail": (
                    f"{hung} binary timed out after {timeout}s; {ran} exited {ran_exit}"
                ),
            }
        ], None

    div = []

    # Memory unsafety is a property of ONE emitted binary, so each side is
    # judged independently and first. A SIGSEGV on one side only would
    # otherwise surface as a plain exit difference.
    crashed = set()
    for name, res in (("rust", r1), ("self-hosted", s)):
        crash = signal_of(res)
        if crash in UNSAFE_SIGNALS:
            crashed.add(name)
            div.append(
                {
                    "observable": "fail_closed",
                    "detail": (
                        f"{name} binary died on {UNSAFE_SIGNALS[crash]} "
                        f"({crash}) — memory unsafety, not a trap"
                    ),
                }
            )

    # Exit parity gets its OWN observable, separate from stdout. A ledger entry
    # documenting a wrong-output `runtime` gap must not also suppress the file
    # later returning the wrong exit status — and a normal nonzero exit is
    # ordinary Vow behaviour (tests/run/short_circuit.vow returns 1), so
    # singling out aborts would have left that half open.
    #
    # Skipped when a per-side crash above already explains the difference:
    # check_fail_closed's rule is that one bug yields one finding.
    if r1["exit"] != s["exit"] and not crashed:
        div.append(
            {
                "observable": "runtime_exit",
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
    # a deliberate trap is the language working. Memory unsafety was handled
    # per-side above and is never suppressed by a declared exit.
    both = {signal_of(r1), signal_of(s)}
    if len(both) == 1:
        signal = both.pop()
        if (
            signal is not None
            and signal not in UNSAFE_SIGNALS
            and signal != expect_signal
            and signal not in TRAP_SIGNALS
        ):
            div.append(
                {
                    "observable": "fail_closed",
                    "detail": f"both binaries died on unclassified signal {signal}",
                }
            )
    return div, None


# ---------------------------------------------------------------------------
# Per-file driver
# ---------------------------------------------------------------------------


# What `read_directives` returns for a file that declares nothing. A candidate
# a model wrote is not a corpus fixture, so `--no-directives` hands this back
# instead of letting the input steer the comparison that judges it.
NO_DIRECTIVES = {
    "skip": None,
    "expected_exit": None,
    "verify_only": False,
    "stdin": None,
    "stdin_file": None,
}


def check_file(
    vow_file, rust, slf, outdir, timeout, honour_directives=True, verify_only=False
):
    rel = (
        str(Path(vow_file).relative_to(REPO_ROOT))
        if str(vow_file).startswith(str(REPO_ROOT))
        else str(vow_file)
    )
    directives = read_directives(vow_file) if honour_directives else NO_DIRECTIVES
    if verify_only:
        # Which comparison to run is the harness's choice, not the input's, so
        # this is selected out of band and survives --no-directives.
        directives = {**directives, "verify_only": True}
    record = {"file": rel, "divergences": [], "skipped": None}

    if directives["skip"]:
        record["skipped"] = f"directive: {directives['skip']}"
        return record

    stem = hashlib.sha256(rel.encode()).hexdigest()[:16]
    rust_out = outdir / f"rust_{stem}"
    self_out = outdir / f"self_{stem}"

    try:
        # `--no-cache` is mandatory, not hygiene: the compile-object cache is keyed
        # on dependency content + mode + a hand-bumped ABI string, NOT on the
        # compiler binary, and both compilers share $VOW_CACHE_DIR. Without it a
        # cached object from the peer compiler can be linked in and the runtime
        # observable silently compares a binary to itself.
        # A `// TEST: verify-only` fixture is a library module with no main.
        # Building it produces no executable on either side, so the build path
        # could only ever record "both rejected (no runtime check)" — and the
        # verifier, which is the only thing the directive asks about, never ran.
        if directives["verify_only"]:
            rust_args = self_args = ["verify", "--no-cache", str(vow_file)]
        else:
            args = ["build", "--no-verify", "--no-cache", str(vow_file)]
            rust_args = args + ["-o", str(rust_out)]
            self_args = args + ["-o", str(self_out)]
        r = run_compiler(rust, rust_args, timeout, False)
        s = run_compiler(slf, self_args, timeout, True)

        record["divergences"] += check_fail_closed("rust", r)
        record["divergences"] += check_fail_closed("self-hosted", s)

        if r["timeout"] and s["timeout"]:
            record["skipped"] = "compile timeout (both)"
            return record
        if r["timeout"] or s["timeout"]:
            # Only a timeout on BOTH sides is inconclusive. One compiler
            # hanging on an input the other compiles is a finding: otherwise a
            # regression that makes the self-hosted compiler loop forever rides
            # out the sweep as a skip.
            hung, ran = (
                ("rust", "self-hosted") if r["timeout"] else ("self-hosted", "rust")
            )
            record["divergences"].append(
                {
                    "observable": "fail_closed",
                    "detail": (
                        f"{hung} compiler timed out after {timeout}s; {ran} completed"
                    ),
                }
            )
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

        if directives["verify_only"]:
            # Agreement here is a real comparison, so the record stays
            # unskipped and counts toward the coverage floor.
            record["divergences"] += compare_verify(r, s)
            return record

        record["divergences"] += compare_build(r, s)

        if compiled_ok(r) and compiled_ok(s) and not record["divergences"]:
            try:
                declared_stdin = stdin_bytes(directives)
            except MissingStdinFile as exc:
                # A broken fixture is a finding about the CORPUS, not about
                # either compiler, so it gets its own observable rather than
                # being dressed up as a compiler divergence.
                record["divergences"].append(
                    {
                        "observable": "fixture_error",
                        "detail": (f"declared stdin-file does not exist: {exc}"),
                    }
                )
                return record
            rt_div, why = compare_runtime(
                rust_out,
                self_out,
                declared_stdin,
                timeout,
                expect_signal=expected_signal(directives),
            )
            record["divergences"] += rt_div
            if why:
                record["skipped"] = why
        # Both rejected and agreed on why: every applicable build observable
        # was compared, so this is a COMPLETED comparison, not a skip. Labelling
        # it skipped excluded error fixtures from `compared` — enough that a
        # single error fixture under the default --min-compared 1 exited 2 for
        # insufficient coverage despite having compared everything there was.

    finally:
        # Codegen leaves a sibling .o next to each executable, and a full-corpus
        # sweep would otherwise accumulate one per file per compiler. This runs
        # on EVERY exit path: the compile-timeout and unparseable-JSON returns
        # above can each leave a partially written binary behind, and those are
        # exactly the paths a sweep hits when something is going wrong.
        for out in (rust_out, self_out):
            for victim in (out, out.with_suffix(".o")):
                try:
                    os.unlink(victim)
                except OSError:
                    pass
    return record


# ---------------------------------------------------------------------------
# Ledger
# ---------------------------------------------------------------------------

LEDGER_PATH = REPO_ROOT / "docs" / "equivalence" / "ledger.json"


def load_ledger_document(path=None):
    """Load the complete ledger document, or ``None`` when it is unusable."""
    ledger_path = Path(path) if path else LEDGER_PATH
    if not ledger_path.exists():
        return None
    try:
        document = json.loads(ledger_path.read_text())
    except (json.JSONDecodeError, OSError):
        return None
    return document if isinstance(document, dict) else None


def load_ledger(path=None):
    """Known divergences, keyed by repo-relative path.

    A tracked divergence is not news. Without this the nightly reports the same
    13 known-gap fixtures every run, and a genuinely new finding is lost in the
    noise — the failure mode that makes recurring jobs get ignored.
    """
    document = load_ledger_document(path)
    return document.get("corpus", {}) if document is not None else {}


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


def divergence_matches_entry(divergence, entry, tracked):
    """Whether a finding is the exact observable payload the ledger records."""
    if divergence["observable"] not in tracked:
        return False
    if divergence["observable"] != "error_code":
        return True
    expected_rust = entry.get("rust_error_codes")
    expected_self = entry.get("self_hosted_error_codes")
    return (
        expected_rust is not None
        and expected_self is not None
        and divergence.get("rust") == expected_rust
        and divergence.get("self_hosted") == expected_self
    )


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
    real findings. Only a file that was actually compared this run can be
    reported `fixed`; a skipped file proves nothing either way.

    Args:
        records: Per-file result records from check_file.
        ledger: Known divergences keyed by repo-relative path.

    Returns:
        tuple: (new, known, fixed) — new/known are record lists carrying only
        their untracked/tracked divergences respectively; fixed is a list of
        {"file", "observables"} dicts naming the tracked observables that no
        longer reproduce.
    """
    new, known, fixed = [], [], []
    for rec in records:
        entry = ledger.get(rec["file"])
        tracked = tracked_observables(entry)
        # Only `open` and `expected` entries suppress anything. A `fixed` entry
        # is retained precisely so a reappearance reads as a regression, so its
        # observables must NOT be folded into `known` — that would let the very
        # regression the entry was kept to catch exit the run successfully.
        suppresses = entry is not None and entry.get("status") in ("open", "expected")
        matched = [
            d for d in rec["divergences"] if divergence_matches_entry(d, entry, tracked)
        ]
        untracked = [
            d
            for d in rec["divergences"]
            if not divergence_matches_entry(d, entry, tracked)
        ]

        as_new = untracked if suppresses else rec["divergences"]
        if as_new:
            new.append({**rec, "divergences": as_new})
        if suppresses and matched:
            known.append({**rec, "divergences": matched})

        # Per-observable, not all-or-nothing: an entry tracking `error_code`
        # and `runtime` where only `runtime` still reproduces has a stale half,
        # and leaving it listed would suppress that observable's next
        # recurrence as known.
        #
        # A file that was not actually compared this run (a skip directive, a
        # compile timeout, a nondeterministic binary) is never reported: it
        # collected no evidence either way, and calling that "fixed" fails the
        # run and demands a ledger edit over infra flakiness.
        if suppresses and not rec.get("skipped"):
            gone = sorted(tracked - {d["observable"] for d in matched})
            if gone:
                fixed.append({"file": rec["file"], "observables": gone})
    return new, known, fixed


def _observable_field(observables):
    """Collapse a set of observables to the schema's string-or-array form."""
    ordered = sorted(observables)
    return ordered[0] if len(ordered) == 1 else ordered


def propose_ledger(document, new, fixed, today):
    """Return a deterministic corpus-ledger update proposed by a sweep.

    The corpus runner has no module-pair hashes, so this deliberately preserves
    ``pairs`` byte-for-byte at the value level. Tier 3 owns pair review state.
    ``today`` is injected by the caller to keep this transformation pure and
    reproducible in tests.
    """
    proposed = copy.deepcopy(document)
    proposed["updated"] = today
    corpus = proposed.setdefault("corpus", {})

    # Apply disappearances first. A changed payload can make reconcile report
    # the old observable fixed and the replacement new in the same run; the
    # later new-finding pass must win and leave that entry open.
    for finding in fixed:
        entry = corpus.get(finding["file"])
        if entry is None:
            continue
        remaining = tracked_observables(entry) - set(finding["observables"])
        if remaining:
            entry["observable"] = _observable_field(remaining)
            if "error_code" not in remaining:
                entry.pop("rust_error_codes", None)
                entry.pop("self_hosted_error_codes", None)
        else:
            # Retain the observable and its metadata so a reappearance remains
            # recognizable as a regression rather than a first-time finding.
            entry["status"] = "fixed"

    for record in new:
        path = record["file"]
        entry = corpus.setdefault(path, {"first_seen": today})
        # A `fixed` entry retains its observable so a reappearance reads as a
        # regression, but nothing observed that observable this run. Carrying
        # it into a reopen driven by an unrelated new finding would mark it
        # live again — suppressing its next real recurrence as `known`, then
        # reporting it `fixed` all over again. The full-fix branch above sets
        # this status for an observable fixed by *this* sweep; one fixed by an
        # *earlier* sweep never reaches `fixed` at all (reconcile only reports
        # `open`/`expected` entries), so the status is what distinguishes both.
        carried = (
            frozenset()
            if entry.get("status") == "fixed"
            else tracked_observables(entry)
        )
        observables = carried | {
            divergence["observable"] for divergence in record["divergences"]
        }
        entry["observable"] = _observable_field(observables)
        entry["status"] = "open"

        error_code = next(
            (
                divergence
                for divergence in record["divergences"]
                if divergence["observable"] == "error_code"
            ),
            None,
        )
        if error_code is not None:
            entry["rust_error_codes"] = sorted(error_code.get("rust", []))
            entry["self_hosted_error_codes"] = sorted(error_code.get("self_hosted", []))
        elif "error_code" not in observables:
            # Mirrors the partial-fix branch: a multiset that no longer pins an
            # active error_code divergence must not linger on a reopened entry.
            entry.pop("rust_error_codes", None)
            entry.pop("self_hosted_error_codes", None)

    proposed["corpus"] = dict(sorted(corpus.items()))
    return proposed


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
    # A proposal is an edit to the ledger this run deliberately ignored, so the
    # two flags are exclusive; argparse both enforces that and shows it in --help.
    ledger_use = ap.add_mutually_exclusive_group()
    ledger_use.add_argument(
        "--no-ledger",
        action="store_true",
        help="report every divergence as new, ignoring tracked ones",
    )
    ledger_use.add_argument(
        "--emit-ledger-update",
        action="store_true",
        help="write a proposed corpus-ledger update into the output directory",
    )
    ap.add_argument(
        "--today",
        default=datetime.now(timezone.utc).date().isoformat(),
        help="ISO date stamped into the proposal's `updated` field (default: UTC today)",
    )
    # Not a ledger flag: this is about whose input steers the run, so it is
    # outside the mutually exclusive group above.
    ap.add_argument(
        "--no-directives",
        action="store_true",
        help="ignore `// TEST:` directives — the input is a candidate, not a fixture",
    )
    ap.add_argument(
        "--verify-only",
        action="store_true",
        help="compare `verify` rather than `build` for every file, whatever it declares",
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
    # Neither artifact may outlive the sweep that produced it. In a reused
    # --output-dir both are read as claims about THIS run: equivalence.yml
    # treats results.json's presence as proof the sweep completed (so a stale
    # one turns a crash into a divergence verdict), and an operator applies
    # ledger.proposed.json wholesale. Cleared up front rather than before each
    # write, because a sweep that dies inside check_file never reaches either.
    proposal_path = outdir / "ledger.proposed.json"
    results_path = outdir / "results.json"
    for sentinel in (proposal_path, results_path):
        sentinel.unlink(missing_ok=True)

    print(f"=== Differential equivalence sweep: {len(corpus)} files ===")
    print(f"  rust: {rust}")
    print(f"  self: {slf}")
    print()

    ledger_document = None if args.no_ledger else load_ledger_document(args.ledger)
    if args.emit_ledger_update and ledger_document is None:
        print(
            "error: cannot propose an update from a missing or invalid ledger",
            file=sys.stderr,
        )
        return 2
    ledger = {} if ledger_document is None else ledger_document.get("corpus", {})
    started = time.time()
    records, diverged, skipped, compared = [], [], [], 0
    for i, f in enumerate(corpus, 1):
        rec = check_file(
            f,
            rust,
            slf,
            outdir,
            args.timeout,
            honour_directives=not args.no_directives,
            verify_only=args.verify_only,
        )
        records.append(rec)
        if rec["divergences"]:
            diverged.append(rec)
            print(f"  DIVERGE {rec['file']}")
            for d in rec["divergences"]:
                print(f"          [{d['observable']}] {d['detail']}")
        # Coverage counts files that reached a real comparison, which includes
        # every file that diverged. Counting only agreeing files made a
        # divergence-heavy shard — or a single-reproducer run under the default
        # --min-compared 1 — fail as "did not measure enough" instead of
        # reporting the divergence it did measure.
        if rec["skipped"]:
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
    # Written before results.json, and only for a run that met its coverage
    # floor. results.json must stay the last artifact produced, because
    # equivalence.yml keys off its presence to tell a divergence verdict from a
    # crash; and a shard that measured too little to be meaningful must not ship
    # a proposal that looks applicable.
    proposed = args.emit_ledger_update and compared >= args.min_compared
    if proposed:
        proposal = propose_ledger(ledger_document, new, fixed, args.today)
        proposal_path.write_text(json.dumps(proposal, indent=2) + "\n")
    results_path.write_text(json.dumps(results, indent=2))

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
    print(f"  results  : {results_path}")
    if proposed:
        print(f"  ledger proposal: {proposal_path}")
    elif args.emit_ledger_update:
        print("  ledger proposal: none — coverage floor not met")

    if fixed:
        print()
        print("  NO LONGER DIVERGING — update docs/equivalence/ledger.json:")
        for f in fixed:
            entry = ledger.get(f["file"], {})
            issue = entry.get("issue")
            obs = ", ".join(f["observables"])
            print(
                f"    {f['file']}  [{obs}]" + (f"  (issue #{issue})" if issue else "")
            )
        if proposed:
            print(f"    proposed update: {proposal_path}")

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
