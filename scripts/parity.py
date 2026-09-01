#!/usr/bin/env python3
"""Compare structured output from the Rust and self-hosted compilers."""

import json
import re
import sys
from collections import Counter
from pathlib import Path

KNOWN_CEX_DIVERGENCE = re.compile(
    (
        r'^// TEST: known-cex-divergence ([0-9]+) "(.*)" '
        r"rust-name=([A-Za-z_][A-Za-z0-9_]*) "
        r"self-name=([A-Za-z_][A-Za-z0-9_]*)$"
    ),
    re.MULTILINE,
)
KNOWN_CEX_COUNT_DIVERGENCE = re.compile(
    r'^// TEST: known-cex-count-divergence ([0-9]+) "(.*)"$', re.MULTILINE
)

# Labels that a suppression policy keys off. Both the comparator that emits the
# message and the predicate that matches it read them from here, so a reworded
# message can never silently disable a suppression.
VALUES_LABEL = "values"
ERROR_CODES_LABEL = "error codes"
COUNTEREXAMPLE_COUNT_LABEL = "counterexamples count"
ESBMC_INTERNAL_VALUE_PREFIX = "$esbmc$"

# `vow test`'s authoritative output contract. Read back out of the schema
# rather than restated so a field added there is compared by this blocking
# gate automatically, instead of drifting out of coverage unnoticed.
TEST_RESULT_SCHEMA = json.loads(
    (
        Path(__file__).resolve().parent.parent
        / "docs/spec/schemas/test-result.schema.json"
    ).read_text()
)
_TEST_ENTRY_SCHEMA = TEST_RESULT_SCHEMA["$defs"]["TestEntry"]
# Wall-clock only. Every other field the schema declares is a function of the
# sources, so the two compilers must agree on it.
TEST_NONDETERMINISTIC_FIELDS = frozenset({"duration_ms"})
# Which entry fields name a test rather than describe its outcome. The membership
# delta keys off these; the rest are compared per shared entry.
TEST_IDENTITY_FIELDS = ("file", "name", "status")
TEST_PAYLOAD_FIELDS = tuple(
    field
    for field in _TEST_ENTRY_SCHEMA["properties"]
    if field not in TEST_IDENTITY_FIELDS and field not in TEST_NONDETERMINISTIC_FIELDS
)
# Top-level counters: everything but the fields with their own comparison below.
TEST_COUNT_FIELDS = tuple(
    field
    for field in TEST_RESULT_SCHEMA["properties"]
    if field not in ("status", "tests", "contract_density")
)


def _mismatch(label, rust_value, self_value):
    """The one parity error for a single observable, or none if it agrees."""
    if rust_value == self_value:
        return []
    return [f"{label}: {rust_value} vs {self_value}"]


def _field_multiset(entries, *fields):
    """Count each entry's `fields` tuple, tolerating absent fields."""
    return Counter(tuple(entry.get(field) for field in fields) for entry in entries)


def _ordered(multiset):
    # A field may be absent, so the tuples mix None with str and need an order
    # that tolerates both.
    return sorted(multiset.elements(), key=repr)


def _diagnostic_multiset(diagnostics):
    return _ordered(_field_multiset(diagnostics, "error_code", "blame"))


def _error_codes(diagnostics):
    # Normalised exactly as equivalence.error_codes does it: both harnesses key
    # off the ledger's `error_code` observable and must agree on what diverged.
    return sorted(diagnostic.get("error_code", "") for diagnostic in diagnostics)


def _counterexample_values(counterexample):
    return {
        name: value
        for name, value in counterexample.get("values", {}).items()
        if not name.startswith(ESBMC_INTERNAL_VALUE_PREFIX)
    }


def _counterexample_fields(base, rust_cex, self_cex):
    """Include contract text only when both counterexamples matched a vow."""
    fields = list(base)
    # With blame "none", neither compiler matched a contract clause and each
    # currently emits a different fallback placeholder (#1144). Contract-backed
    # violations have stable source text and must remain parity checked (#1113).
    if rust_cex.get("blame") != "none" and self_cex.get("blame") != "none":
        fields.append("violation")
    return fields


def _values_match_after_rename(
    rust_counterexamples, self_counterexamples, rust_name, self_name
):
    """Whether one declared source-label rename explains every value map."""
    if not rust_counterexamples or not self_counterexamples:
        return False
    saw_rename = False
    for rust_cex, self_cex in zip(rust_counterexamples, self_counterexamples):
        renamed = {}
        for name, value in _counterexample_values(rust_cex).items():
            mapped_name = self_name if name == rust_name else name
            if mapped_name in renamed:
                return False
            renamed[mapped_name] = value
            saw_rename = saw_rename or name == rust_name
        if renamed != _counterexample_values(self_cex):
            return False
    return saw_rename


def _compare_counterexamples(rust_counterexamples, self_counterexamples, fields):
    """Count and per-index parity errors for the two counterexample lists."""
    errors = _mismatch(
        COUNTEREXAMPLE_COUNT_LABEL,
        len(rust_counterexamples),
        len(self_counterexamples),
    )
    for index, (rust_cex, self_cex) in enumerate(
        zip(rust_counterexamples, self_counterexamples)
    ):
        for field in _counterexample_fields(fields, rust_cex, self_cex):
            rust_value = rust_cex.get(field)
            self_value = self_cex.get(field)
            # An unknown vow_id is spelled 0, -1, or absent depending on which
            # emitter produced it; all three mean the same "no id".
            if field == "vow_id" and (
                rust_value in (0, -1, None) and self_value in (0, -1, None)
            ):
                continue
            errors += _mismatch(
                f"counterexample[{index}].{field}", rust_value, self_value
            )
        errors += _mismatch(
            f"counterexample[{index}].{VALUES_LABEL}",
            _counterexample_values(rust_cex),
            _counterexample_values(self_cex),
        )
    return errors


def compare_json(rust, self_hosted, rust_exit, self_exit):
    """Return parity errors for a general compiler invocation."""
    errors = _mismatch("exit code", rust_exit, self_exit)

    rust_status = rust.get("status", "")
    self_status = self_hosted.get("status", "")
    errors += _mismatch("status", rust_status, self_status)

    if rust_status != "VerifyFailed":
        errors += _mismatch(
            "diagnostics",
            _diagnostic_multiset(rust.get("diagnostics", [])),
            _diagnostic_multiset(self_hosted.get("diagnostics", [])),
        )

    rust_counterexamples = rust.get("counterexamples", [])
    self_counterexamples = self_hosted.get("counterexamples", [])
    both_verify_failed = rust_status == self_status == "VerifyFailed"
    # A VerifyFailed with a non-empty verify_status is a 'soft' ESBMC outcome
    # (timeout / unknown / error / tool_not_found) — ESBMC produced no
    # counterexample by design, so the parity check must not require one.
    rust_verify_status = rust.get("verify_status") or ""
    self_verify_status = self_hosted.get("verify_status") or ""
    if both_verify_failed and rust_verify_status and self_verify_status:
        errors += _mismatch("verify_status", rust_verify_status, self_verify_status)
        if len(rust_counterexamples) != 0:
            errors.append(
                "rust soft VerifyFailed has "
                f"{len(rust_counterexamples)} counterexamples"
            )
        if len(self_counterexamples) != 0:
            errors.append(
                "self soft VerifyFailed has "
                f"{len(self_counterexamples)} counterexamples"
            )
        # For deterministic inputs the same function should trigger the soft
        # fail on both compilers. ESBMC's verify_message remains intentionally
        # unexamined because its text is nondeterministic.
        errors += _mismatch(
            "function", rust.get("function") or "", self_hosted.get("function") or ""
        )
    elif both_verify_failed:
        if len(rust_counterexamples) == 0:
            errors.append("rust has no counterexamples for VerifyFailed")
        if len(self_counterexamples) == 0:
            errors.append("self has no counterexamples for VerifyFailed")
        errors += _compare_counterexamples(
            rust_counterexamples, self_counterexamples, ("function", "blame")
        )
    else:
        errors += _compare_counterexamples(
            rust_counterexamples,
            self_counterexamples,
            ("function", "vow_id", "blame"),
        )

    return errors


def compare_error(rust, self_hosted, rust_exit, self_exit):
    """Return parity errors for an invocation expected to fail compilation."""
    errors = []
    if rust_exit == 0:
        errors.append("rust exited 0, expected failure")
    if self_exit == 0:
        errors.append("self exited 0, expected failure")
    for name, document in (("rust", rust), ("self", self_hosted)):
        if document.get("status") != "CompileFailed":
            errors.append(
                f"{name} status={document.get('status')}, expected CompileFailed"
            )
        if len(document.get("diagnostics", [])) < 1:
            errors.append(f"{name} has no diagnostics")
    errors += _mismatch(
        ERROR_CODES_LABEL,
        _error_codes(rust.get("diagnostics", [])),
        _error_codes(self_hosted.get("diagnostics", [])),
    )
    return errors


def _missing_test_fields(name, document):
    """Required `vow test` keys one document omits, counted not enumerated.

    Parity alone cannot see a field both compilers dropped, so the shape is
    asserted against the schema in absolute terms. Per-entry omissions are
    tallied rather than listed: one broken emitter would otherwise print a line
    per test for a suite the size of `compiler/`.
    """
    errors = [
        f"{name} is missing {field}"
        for field in TEST_RESULT_SCHEMA["required"]
        if field not in document
    ]
    entry_gaps = Counter(
        field
        for entry in document.get("tests", [])
        for field in _TEST_ENTRY_SCHEMA["required"]
        if field not in entry
    )
    errors += [
        f"{name} tests: {count} entries missing {field}"
        for field, count in sorted(entry_gaps.items())
    ]
    density = document.get("contract_density")
    if isinstance(density, dict):
        errors += [
            f"{name} contract_density is missing {field}"
            for field in TEST_RESULT_SCHEMA["$defs"]["ContractDensity"]["required"]
            if field not in density
        ]
    return errors


def _by_test_identity(entries):
    grouped = {}
    for entry in entries:
        key = tuple(entry.get(field) for field in TEST_IDENTITY_FIELDS)
        grouped.setdefault(key, []).append(entry)
    return grouped


def _compare_test_entries(rust_tests, self_tests):
    """Membership delta first, then every stable field of a shared entry."""
    # Report only the delta, so a one-test disagreement reads as one line
    # however far the two suites drift apart. Counted rather than set-compared:
    # suites differing only in how often a name repeats must still name the
    # entry they disagree on instead of printing an empty delta.
    rust_ids = _field_multiset(rust_tests, *TEST_IDENTITY_FIELDS)
    self_ids = _field_multiset(self_tests, *TEST_IDENTITY_FIELDS)
    errors = []
    if rust_ids != self_ids:
        only_rust = _ordered(rust_ids - self_ids)
        only_self = _ordered(self_ids - rust_ids)
        errors.append(f"tests: rust-only {only_rust} vs self-only {only_self}")

    rust_by_id = _by_test_identity(rust_tests)
    self_by_id = _by_test_identity(self_tests)
    for key in sorted(rust_by_id.keys() & self_by_id.keys(), key=repr):
        for rust_entry, self_entry in zip(rust_by_id[key], self_by_id[key]):
            for field in TEST_PAYLOAD_FIELDS:
                errors += _mismatch(
                    f"test {key[0]}.{field}",
                    rust_entry.get(field),
                    self_entry.get(field),
                )
    return errors


def compare_test(rust, self_hosted, rust_exit, self_exit):
    """Return parity errors for two ``vow test`` suite results."""
    errors = []
    for name, document, exit_code in (
        ("rust", rust, rust_exit),
        ("self", self_hosted, self_exit),
    ):
        if exit_code != 0:
            errors.append(f"{name} exited {exit_code}, expected 0")
        if document.get("status") != "TestsPassed":
            errors.append(
                f"{name} status={document.get('status')}, expected TestsPassed"
            )
        errors += _missing_test_fields(name, document)

    # Equality alone is satisfied by two suites that discovered nothing, which
    # is how a blocking gate goes vacuous exactly when it matters most.
    if not rust.get("total"):
        errors.append(f"rust total={rust.get('total')}, expected a non-empty suite")

    for field in TEST_COUNT_FIELDS:
        errors += _mismatch(field, rust.get(field), self_hosted.get(field))
    # Derived from the same sources by each compiler's own frontend, so a
    # disagreement here is a parity bug the test list alone cannot expose.
    errors += _mismatch(
        "contract_density",
        rust.get("contract_density"),
        self_hosted.get("contract_density"),
    )

    errors += _compare_test_entries(rust.get("tests", []), self_hosted.get("tests", []))
    return errors


def _load_documents(rust_path, self_path):
    with open(rust_path) as rust_file:
        rust = json.load(rust_file)
    with open(self_path) as self_file:
        self_hosted = json.load(self_file)
    return rust, self_hosted


def _suppress(errors, covered, exercised, reason, stale_reason):
    """Verdict for a divergence some registry already tracks, or None.

    A tracked divergence is a loud SKIP while it still reproduces and a hard
    FAIL once it stops, so a suppression cannot outlive the gap it documents.
    Only a run that `exercised` the observable may call the suppression stale:
    the same fixture is reachable through invocations that never compare it (a
    --no-verify build emits no counterexamples), and failing there would fail a
    run that measured nothing.
    """
    if errors and all(covered(error) for error in errors):
        return 0, f"SKIP: {reason}"
    if not errors and exercised:
        return 1, f"FAIL: {stale_reason}"
    return None


def _known_cex_policies(rust, self_hosted, fixture_text):
    """Active-observable policies declared by one counterexample fixture."""
    policies = []
    match = KNOWN_CEX_DIVERGENCE.search(fixture_text)
    if match:
        known = f"#{match.group(1)}: {match.group(2)}"
        expected_gap = _values_match_after_rename(
            rust.get("counterexamples", []),
            self_hosted.get("counterexamples", []),
            match.group(3),
            match.group(4),
        )
        policies.append(
            (
                lambda error: (
                    expected_gap
                    and error.startswith("counterexample[")
                    and f"].{VALUES_LABEL}:" in error
                ),
                bool(
                    rust.get("counterexamples") and self_hosted.get("counterexamples")
                ),
                f"known counterexample divergence ({known})",
                (
                    f"known-cex-divergence ({known}) no longer reproduces — "
                    "remove the directive"
                ),
            )
        )

    match = KNOWN_CEX_COUNT_DIVERGENCE.search(fixture_text)
    if not match:
        return policies
    known = f"#{match.group(1)}: {match.group(2)}"
    both_hard_failed = rust.get("status") == self_hosted.get(
        "status"
    ) == "VerifyFailed" and not (
        rust.get("verify_status") and self_hosted.get("verify_status")
    )
    policies.append(
        (
            lambda error: error.startswith(f"{COUNTEREXAMPLE_COUNT_LABEL}: "),
            both_hard_failed,
            f"known counterexample-count divergence ({known})",
            (
                f"known-cex-count-divergence ({known}) no longer reproduces — "
                "remove the directive"
            ),
        )
    )
    return policies


def _known_cex_verdict(rust, self_hosted, errors, fixture_path):
    """Compose a fixture's counterexample suppressions by observable."""
    if not fixture_path:
        return None
    fixture_text = Path(fixture_path).read_text(errors="replace")
    policies = _known_cex_policies(rust, self_hosted, fixture_text)
    active = [policy for policy in policies if policy[1]]

    if any(not any(covered(error) for covered, _, _, _ in active) for error in errors):
        return None

    stale = [
        stale_reason
        for covered, _, _, stale_reason in active
        if not any(covered(error) for error in errors)
    ]
    if stale:
        return 1, "FAIL: " + "; ".join(stale)

    reproduced = [
        reason
        for covered, _, reason, _ in active
        if any(covered(error) for error in errors)
    ]
    if reproduced:
        return 0, "SKIP: " + "; ".join(reproduced)
    return None


def _ledger_entry(fixture_path):
    """The equivalence-ledger entry for a fixture, with its tracked observables."""
    if not fixture_path:
        return None, frozenset()
    # Deferred import: `equivalence` drags in argparse/subprocess/hashlib, ~12ms
    # of interpreter startup that the far more numerous json-mode invocations
    # would otherwise pay for a ledger they never read.
    from equivalence import REPO_ROOT, load_ledger, tracked_observables

    path = Path(fixture_path).resolve()
    if not path.is_relative_to(REPO_ROOT):
        return None, frozenset()
    entry = load_ledger().get(str(path.relative_to(REPO_ROOT)))
    return entry, tracked_observables(entry)


def _ledger_verdict(rust, self_hosted, errors, fixture_path):
    """Verdict for an active `error_code` entry in the equivalence ledger."""
    entry, observables = _ledger_entry(fixture_path)
    if not entry or entry.get("status") not in ("open", "expected"):
        return None
    if "error_code" not in observables:
        return None
    issue = entry.get("issue", "unfiled")
    expected_gap = _error_codes(rust.get("diagnostics", [])) == entry.get(
        "rust_error_codes"
    ) and _error_codes(self_hosted.get("diagnostics", [])) == entry.get(
        "self_hosted_error_codes"
    )
    return _suppress(
        errors,
        lambda error: expected_gap and error.startswith(f"{ERROR_CODES_LABEL}: "),
        # compare_error already fails when either side emitted no diagnostics,
        # so an empty error list means the code multisets were really compared.
        exercised=True,
        reason=f"known error-code divergence (#{issue})",
        stale_reason=(
            f"error_code divergence tracked by #{issue} no longer diverges — "
            "update docs/equivalence/ledger.json"
        ),
    )


def main(argv=None):
    """Run a comparator over two JSON files for scripts/full_test.sh."""
    args = sys.argv[1:] if argv is None else argv
    if len(args) not in (5, 6) or args[0] not in ("json", "error", "test"):
        print(
            "usage: parity.py {json,error,test} RUST_JSON SELF_JSON "
            "RUST_EXIT SELF_EXIT [FIXTURE]",
            file=sys.stderr,
        )
        return 2

    mode, rust_path, self_path, rust_exit, self_exit = args[:5]
    fixture_path = args[5] if len(args) == 6 else None
    try:
        rust, self_hosted = _load_documents(rust_path, self_path)
    except (json.JSONDecodeError, OSError) as error:
        print(f"FAIL: JSON parse error: {error}")
        return 1

    if mode == "test":
        errors = compare_test(rust, self_hosted, int(rust_exit), int(self_exit))
        verdict = None
    elif mode == "json":
        errors = compare_json(rust, self_hosted, int(rust_exit), int(self_exit))
        try:
            verdict = _known_cex_verdict(rust, self_hosted, errors, fixture_path)
        except OSError as error:
            print(f"FAIL: fixture read error: {error}")
            return 1
    else:
        errors = compare_error(rust, self_hosted, int(rust_exit), int(self_exit))
        verdict = _ledger_verdict(rust, self_hosted, errors, fixture_path)

    if verdict is not None:
        code, message = verdict
        print(message)
        return code
    if errors:
        print("FAIL: " + "; ".join(errors))
        return 1

    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
