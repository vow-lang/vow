"""What explains a differential row that would otherwise read as `weaker`.

Two explanations, both written next to the fixture they excuse:

  esbmc-model-artifact   one ESBMC counterexample comes from ESBMC's own model
                         (`verify_eval.artifact_directives`); here it excuses the
                         extra counterexample of a fixture both backends refute.
  native-gap             native is deliberately less conclusive than ESBMC on the
                         fixture and an open issue tracks closing it.

Neither makes a row `match`: the first is `more_precise`, the second `tracked`, a
class of its own that stays in every report and never fails the exit status.
"""

import re

MORE_PRECISE = "more_precise"
WEAKER = "weaker"
MATCH = "match"
TRACKED = "tracked"

# What a `native-gap` may excuse: native declined to decide, which is `Skipped`
# (no `verify_status`) or `unknown`. A native refutation of a program the corpus
# labels correct, a crash, a panic, an error and a timeout are never a gap.
LESS_CONCLUSIVE = ("skipped", "inconclusive")
DECLINED_STATUS = (None, "unknown")

NATIVE_GAP_LINE = re.compile(r"^[ \t]*//\s*TEST:\s*native-gap\b(.*)$", re.MULTILINE)
NATIVE_GAP_BODY = re.compile(r'^\s+"([^"]+)"\s+#(\d+)\s*$')


def native_gap(path):
    """`(why, issue)` of the fixture's `native-gap` directive, or None.

    `// TEST: native-gap "<why>" #<issue>`. The reason and the issue number are
    mandatory and a malformed line is an error, not a missing directive, so a typo
    cannot leave a row looking unexplained and cannot excuse one without an issue.
    """
    try:
        with open(path, "r", encoding="utf-8") as fh:
            text = fh.read()
    except OSError:
        return None
    gaps = []
    for m in NATIVE_GAP_LINE.finditer(text):
        body = NATIVE_GAP_BODY.match(m.group(1))
        if not body:
            raise ValueError(
                f"{path}: malformed native-gap directive "
                f'(expected: native-gap "<why>" #<issue>): {m.group(0)!r}'
            )
        gaps.append((body.group(1), int(body.group(2))))
    if len(gaps) > 1:
        raise ValueError(f"{path}: more than one native-gap directive")
    return gaps[0] if gaps else None


def compare_cex(esbmc_keys, native_keys, artifacts=None):
    """Return (class, detail) for two refutations of the same fixture.

    `artifacts` maps the `(fn, blame, vow_id)` of ESBMC counterexamples declared
    to be ESBMC model artefacts to their reason. A declared counterexample native
    does not report is not a lost one when native refutes the same function
    through its own counterexample: the row is `more_precise`. Any undeclared
    counterexample native lacks, and a declared one in a function native does
    not refute, keeps the row `weaker`.
    """
    declared = esbmc_keys & set(artifacts or {})
    missing = esbmc_keys - native_keys - declared
    extra = native_keys - esbmc_keys
    if missing:
        return (
            WEAKER,
            f"native lacks ESBMC counterexample(s) {sorted(missing, key=repr)}",
        )
    lost = declared - native_keys
    if lost and {k[0] for k in lost} - {k[0] for k in native_keys}:
        return (
            WEAKER,
            f"native does not refute the function(s) of ESBMC's declared artefact(s) {sorted(lost, key=repr)}",
        )
    if lost:
        reasons = "; ".join(artifacts[k] for k in sorted(lost, key=repr))
        return (
            MORE_PRECISE,
            f"ESBMC adds counterexample(s) only through its own model ({reasons}); "
            "native refutes the program through its own",
        )
    if extra:
        return (
            MORE_PRECISE,
            f"native reports extra counterexample(s) {sorted(extra, key=repr)}",
        )
    return MATCH, None


def track(cls, detail, native_verdict, native_status, path):
    """`(class, detail)` with a `weaker` row the fixture documents as `tracked`."""
    if (
        cls != WEAKER
        or native_verdict not in LESS_CONCLUSIVE
        or native_status not in DECLINED_STATUS
    ):
        return cls, detail
    gap = native_gap(path)
    if not gap:
        return cls, detail
    why, issue = gap
    return TRACKED, f"tracked in #{issue} ({why}); {detail}"
