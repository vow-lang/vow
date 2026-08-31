#!/usr/bin/env python3
"""Compare structured output from the Rust and self-hosted compilers."""

import json
import os
import re
import sys
from pathlib import Path

from equivalence import REPO_ROOT, load_ledger, tracked_observables


KNOWN_CEX_DIVERGENCE = re.compile(
    r'^// TEST: known-cex-divergence ([0-9]+) "(.*)"$', re.MULTILINE
)


def _diagnostic_multiset(document):
    return sorted(
        (
            (diagnostic.get("error_code"), diagnostic.get("blame"))
            for diagnostic in document
        ),
        key=repr,
    )


def _counterexample_values(counterexample):
    return {
        name: value
        for name, value in counterexample.get("values", {}).items()
        if not name.startswith("_esbmc")
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


def compare_json(rust, self_hosted, rust_exit, self_exit):
    """Return parity errors for a general compiler invocation."""
    errors = []

    if rust_exit != self_exit:
        errors.append(f"exit code: {rust_exit} vs {self_exit}")

    rust_status = rust.get("status", "")
    self_status = self_hosted.get("status", "")
    if rust_status != self_status:
        errors.append(f"status: {rust_status} vs {self_status}")

    if rust_status != "VerifyFailed":
        rust_diagnostics = _diagnostic_multiset(rust.get("diagnostics", []))
        self_diagnostics = _diagnostic_multiset(self_hosted.get("diagnostics", []))
        if rust_diagnostics != self_diagnostics:
            errors.append(f"diagnostics: {rust_diagnostics} vs {self_diagnostics}")

    rust_counterexamples = rust.get("counterexamples", [])
    self_counterexamples = self_hosted.get("counterexamples", [])
    # A VerifyFailed with a non-empty verify_status is a 'soft' ESBMC outcome
    # (timeout / unknown / error / tool_not_found) — ESBMC produced no
    # counterexample by design, so the parity check must not require one.
    rust_verify_status = rust.get("verify_status") or ""
    self_verify_status = self_hosted.get("verify_status") or ""
    soft_fail = (
        rust_status == "VerifyFailed"
        and self_status == "VerifyFailed"
        and rust_verify_status
        and self_verify_status
    )
    if soft_fail:
        if rust_verify_status != self_verify_status:
            errors.append(
                f"verify_status: {rust_verify_status} vs {self_verify_status}"
            )
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
        rust_function = rust.get("function") or ""
        self_function = self_hosted.get("function") or ""
        if rust_function != self_function:
            errors.append(f"function: {rust_function} vs {self_function}")
    elif rust_status == "VerifyFailed" and self_status == "VerifyFailed":
        if len(rust_counterexamples) == 0:
            errors.append("rust has no counterexamples for VerifyFailed")
        if len(self_counterexamples) == 0:
            errors.append("self has no counterexamples for VerifyFailed")
        if rust_counterexamples and self_counterexamples:
            for field in _counterexample_fields(
                ("function", "blame"),
                rust_counterexamples[0],
                self_counterexamples[0],
            ):
                rust_value = rust_counterexamples[0].get(field)
                self_value = self_counterexamples[0].get(field)
                if rust_value != self_value:
                    errors.append(
                        f"counterexample[0].{field}: {rust_value} vs {self_value}"
                    )
            rust_values = _counterexample_values(rust_counterexamples[0])
            self_values = _counterexample_values(self_counterexamples[0])
            if rust_values != self_values:
                errors.append(
                    f"counterexample[0].values: {rust_values} vs {self_values}"
                )
    else:
        if len(rust_counterexamples) != len(self_counterexamples):
            errors.append(
                "counterexamples count: "
                f"{len(rust_counterexamples)} vs {len(self_counterexamples)}"
            )
        else:
            for index, (rust_cex, self_cex) in enumerate(
                zip(rust_counterexamples, self_counterexamples)
            ):
                for field in _counterexample_fields(
                    ("function", "vow_id", "blame"), rust_cex, self_cex
                ):
                    rust_value = rust_cex.get(field)
                    self_value = self_cex.get(field)
                    if field == "vow_id" and (
                        rust_value in (0, -1, None) and self_value in (0, -1, None)
                    ):
                        continue
                    if rust_value != self_value:
                        errors.append(
                            f"counterexample[{index}].{field}: "
                            f"{rust_value} vs {self_value}"
                        )
                rust_values = _counterexample_values(rust_cex)
                self_values = _counterexample_values(self_cex)
                if rust_values != self_values:
                    errors.append(
                        f"counterexample[{index}].values: "
                        f"{rust_values} vs {self_values}"
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
    rust_codes = sorted(
        (diagnostic.get("error_code") for diagnostic in rust.get("diagnostics", [])),
        key=repr,
    )
    self_codes = sorted(
        (
            diagnostic.get("error_code")
            for diagnostic in self_hosted.get("diagnostics", [])
        ),
        key=repr,
    )
    if rust_codes != self_codes:
        errors.append(f"error codes: {rust_codes} vs {self_codes}")
    return errors


def _load_documents(rust_path, self_path):
    with open(rust_path) as rust_file:
        rust = json.load(rust_file)
    with open(self_path) as self_file:
        self_hosted = json.load(self_file)
    return rust, self_hosted


def _known_cex_divergence(fixture_path):
    if not fixture_path:
        return None
    match = KNOWN_CEX_DIVERGENCE.search(Path(fixture_path).read_text(errors="replace"))
    return f"#{match.group(1)}: {match.group(2)}" if match else None


def _is_values_error(error):
    return error.startswith("counterexample[") and "].values:" in error


def _ledger_entry(fixture_path):
    if not fixture_path:
        return None
    relative_path = os.path.relpath(os.path.abspath(fixture_path), REPO_ROOT)
    if relative_path == ".." or relative_path.startswith(".." + os.sep):
        return None
    return load_ledger().get(relative_path)


def main(argv=None):
    """Run a comparator over two JSON files for scripts/full_test.sh."""
    args = sys.argv[1:] if argv is None else argv
    if len(args) not in (5, 6) or args[0] not in ("json", "error"):
        print(
            "usage: parity.py {json,error} RUST_JSON SELF_JSON "
            "RUST_EXIT SELF_EXIT [FIXTURE]",
            file=sys.stderr,
        )
        return 2

    mode, rust_path, self_path, rust_exit, self_exit = args[:5]
    try:
        rust, self_hosted = _load_documents(rust_path, self_path)
    except (json.JSONDecodeError, OSError) as error:
        print(f"FAIL: JSON parse error: {error}")
        return 1

    comparator = compare_json if mode == "json" else compare_error
    errors = comparator(rust, self_hosted, int(rust_exit), int(self_exit))
    fixture_path = args[5] if len(args) == 6 else None
    if mode == "json":
        try:
            known_cex = _known_cex_divergence(fixture_path)
        except OSError as error:
            print(f"FAIL: fixture read error: {error}")
            return 1
        if known_cex and errors and all(_is_values_error(error) for error in errors):
            print(f"SKIP: known counterexample divergence ({known_cex})")
            return 0
        if known_cex and not errors:
            print(
                f"FAIL: known-cex-divergence ({known_cex}) no longer "
                "reproduces — remove the directive"
            )
            return 1
    else:
        entry = _ledger_entry(fixture_path)
        tracks_active_error_code = (
            entry
            and entry.get("status") in ("open", "expected")
            and "error_code" in tracked_observables(entry)
        )
        code_errors = [error for error in errors if error.startswith("error codes:")]
        if tracks_active_error_code and code_errors and len(code_errors) == len(errors):
            print(
                f"SKIP: known error-code divergence (#{entry.get('issue', 'unfiled')})"
            )
            return 0
        if tracks_active_error_code and not errors:
            print(
                "FAIL: error_code divergence tracked by "
                f"#{entry.get('issue', 'unfiled')} no longer diverges — "
                "update docs/equivalence/ledger.json"
            )
            return 1
    if errors:
        print("FAIL: " + "; ".join(errors))
        return 1

    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
