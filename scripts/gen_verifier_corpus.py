#!/usr/bin/env python3
"""Generated corpus for the verifier-versus-runtime differential (#1473).

`gen_corpus.py` (#905) feeds the compiler-equivalence sweep and has no soundness
signal for the verifier: its `main` never calls the functions it generates, so a
false proof in them is never observable at runtime. This module is the other
axis. Every program it emits carries contracts over a collection or a loop whose
real behaviour exceeds any bound a verifier might quietly assume, and its `main`
calls each generated function with concrete inputs that satisfy the `requires`,
so a debug-mode run is an independent oracle for the proof.

Two classes, told apart by a `// TEST: gen-class` directive that
`scripts/verifier_runtime.py` reads:

  exceeds-bound  the contract is false, but only when the collection grows (or
                 the loop runs) past a boundary `B`. `main` drives the function
                 past `B`, so the debug binary reports a `VowViolation`. A
                 verifier that unrolls, or assumes a collection length, up to
                 `B` and then proves the property is exposed as a SOUNDNESS
                 finding; `Violated`/`unknown` are both fine.
  control        the same shape with the true contract. It carries
                 `// TEST: expect-verified`: it must stay `Verified` and clean
                 at runtime, so a verifier that "fixes" the first class by
                 giving up on everything shows up as a lost control instead of
                 as a green run.

Known disagreements are tracked by issue number in `KNOWN_DISAGREEMENTS`, keyed
by shape, class and regime (`within-unwind` or `beyond-unwind`, relative to the
native verifier's internal unrolling bound): the generator stamps the program
with the matching `known-*` directive, which the runner reports but does not
fail, and which it fails on once the disagreement no longer reproduces. Nothing
is skipped silently, and an entry without an issue number is rejected.

Determinism is a hard requirement (the seed is an explicit argument, never a
clock read): a finding nobody can reproduce is not actionable.
"""

import re

# The `Vec` length the ESBMC pipeline assumed (`VEC_MODEL_CAP` in
# vow-verify/src/c_emitter.rs; Strings were 256, maps 64). The native verifier
# has no collection cap (ADR-2026-10-08-1422: collections are an array plus a
# symbolic length), so this is the boundary an unsound port of the old model
# would reintroduce. Pinned against the emitter by `test_gen_verifier_corpus.py`.
ESBMC_VEC_CAP = 128

# Boundaries a bounded checker is likely to hard-code: the ESBMC capacities and
# the native unwind bound, powers of two and round numbers, with the neighbours
# on both sides because an off-by-one in an unwind or a length assumption lives
# exactly there.
BOUNDARIES = (
    7, 8, 16, 17, 31, 32, 33, 63, 64, 65, 100, 127, 128, 129, 255, 256, 257,
)  # fmt: skip

CLASS_BUG = "exceeds-bound"
CLASS_CONTROL = "control"

# The native verifier's internal loop-unrolling bound (compiler/vc_*.vow,
# reported as "a loop may run more than 64 iterations"). A loop that runs past it
# is only decided by an inductive invariant. Mirrored here so a generated
# program knows which side of it it sits on; `test_gen_verifier_corpus.py`
# pins the value against the compiler source.
UNWIND_BOUND = 64

WITHIN = "within-unwind"
BEYOND = "beyond-unwind"

# (shape, class, regime) -> "#issue". The entry is the tracking record of a
# disagreement a generated program exposes; it is added with the issue, never
# without one.
KNOWN_DISAGREEMENTS = {
    # A Vec filled by a loop that runs past the unrolling bound is never cut for
    # induction, so a true length contract stays `unknown` (completeness gap).
    (shape, CLASS_CONTROL, BEYOND): "#1645"
    for shape in (
        "vec_len_literal",
        "vec_len_cap",
        "vec_last_element",
        "vec_pop_all",
    )
}

# (shape, class) -> why the replay harness cannot confirm a counterexample of
# that shape, with its issue. Rendered as `// TEST: replay-skipped`, which the
# runner already reports instead of failing.
REPLAY_SKIPS = {
    ("vec_param_clamp", CLASS_BUG): (
        "Vec parameter, not reported in native counterexamples (#1613)"
    ),
    ("vec_param_last", CLASS_BUG): (
        "Vec parameter, not reported in native counterexamples (#1613)"
    ),
}

_ISSUE = re.compile(r"^#\d+$")


def validate_known(known):
    for key, issue in known.items():
        shape, cls, regime = key
        if shape not in SHAPES:
            raise ValueError(f"known disagreement {key}: unknown shape")
        if cls not in (CLASS_BUG, CLASS_CONTROL):
            raise ValueError(f"known disagreement {key}: unknown class")
        if regime not in (WITHIN, BEYOND):
            raise ValueError(f"known disagreement {key}: unknown regime")
        if not _ISSUE.match(issue):
            raise ValueError(
                f"known disagreement {key}: {issue!r} is not an issue number"
            )


def _vec_fill(count, push="0"):
    """A function body building a Vec of `count` elements with a counted loop."""
    return (
        "  let v: Vec<i64> = Vec::new();\n"
        "  let mut i: u64 = 0;\n"
        f"  while i < {count} vow {{\n"
        f"    invariant: i <= {count} && v.len() == i\n"
        "  } {\n"
        f"    v.push({push});\n"
        "    i = i + 1;\n"
        "  }\n"
        "  v\n"
    )


# Each shape returns `(function source, `main` statements, longest loop trip
# count)`. The trip count decides the regime: a loop past UNWIND_BOUND is decided
# by its invariant, not by unrolling.


def shape_vec_len_literal(rng, honest, bound):
    """A literal-count fill whose `len` equation is off by one when false."""
    n = bound + rng.randint(0, 3)
    claimed = n if honest else n - rng.randint(1, 2)
    src = (
        f"fn fill() -> Vec<i64> vow {{\n"
        f"  ensures: result.len() == {claimed}\n"
        "} {\n" + _vec_fill(n) + "}\n"
    )
    return src, ["print_u64(fill().len());"], n


def shape_vec_len_cap(rng, honest, bound):
    """`len <= cap` after a fill that is only too long past the cap."""
    n = bound + rng.randint(1, 4)
    cap = n if honest else bound
    src = (
        "fn fill_cap() -> Vec<i64> vow {\n"
        f"  ensures: result.len() <= {cap}\n"
        "} {\n" + _vec_fill(n) + "}\n"
    )
    return src, ["print_u64(fill_cap().len());"], n


def shape_vec_last_element(rng, honest, bound):
    """The last element written, read back by an `ensures` over `result[i]`."""
    n = bound + rng.randint(1, 3)
    claimed = 7 if honest else 8
    src = (
        "fn fill_last() -> Vec<i64> vow {\n"
        f"  ensures: result.len() == {n} && result[{n - 1}] == {claimed}\n"
        "} {\n"
        "  let v: Vec<i64> = Vec::new();\n"
        "  let mut i: u64 = 0;\n"
        f"  while i < {n} vow {{\n"
        f"    invariant: i <= {n} && v.len() == i\n"
        "  } {\n"
        f"    if i + 1 == {n} {{\n"
        "      v.push(7);\n"
        "    } else {\n"
        "      v.push(0);\n"
        "    }\n"
        "    i = i + 1;\n"
        "  }\n"
        "  v\n"
        "}\n"
    )
    return src, ["print_u64(fill_last().len());"], n


def shape_scalar_loop_sum(rng, honest, bound):
    """A counted accumulation: the closed form only fails past the unwind."""
    n = bound + rng.randint(1, 3)
    step = rng.randint(1, 4)
    claimed = n * step if honest else n * step - step
    src = (
        "fn accumulate() -> u64 vow {\n"
        f"  ensures: result == {claimed}\n"
        "} {\n"
        "  let mut acc: u64 = 0;\n"
        "  let mut i: u64 = 0;\n"
        f"  while i < {n} vow {{\n"
        f"    invariant: i <= {n} && acc == i * {step}\n"
        "  } {\n"
        f"    acc = acc + {step};\n"
        "    i = i + 1;\n"
        "  }\n"
        "  acc\n"
        "}\n"
    )
    return src, ["print_u64(accumulate());"], n


def shape_vec_param_clamp(rng, honest, bound):
    """A Vec parameter longer than the bound the contract speaks about.

    The honest version clamps the length; the buggy one returns it unclamped, so
    `ensures: result <= bound` holds for every vector a verifier that assumes
    short collections would consider and fails for the one `main` builds.
    """
    n = bound + rng.randint(1, 5)
    ret = (
        f"if v.len() > {bound} {{ {bound} }} else {{ v.len() }}"
        if honest
        else "v.len()"
    )
    src = (
        "fn clamp_len(v: Vec<i64>) -> u64 vow {\n"
        f"  ensures: result <= {bound}\n"
        "} {\n"
        f"  {ret}\n"
        "}\n"
    )
    main_lines = [
        "let w: Vec<i64> = Vec::new();",
        "let mut j: u64 = 0;",
        f"while j < {n} {{",
        "  w.push(0);",
        "  j = j + 1;",
        "}",
        "print_u64(clamp_len(w));",
    ]
    return src, main_lines, 0


def shape_vec_pop_all(rng, honest, bound):
    """Push past the bound, pop a few: the surviving length is exact."""
    n = bound + rng.randint(2, 6)
    k = rng.randint(1, 2)
    claimed = n - k if honest else n - k + 1
    src = (
        "fn push_pop() -> u64 vow {\n"
        f"  ensures: result == {claimed}\n"
        "} {\n"
        "  let v: Vec<i64> = Vec::new();\n"
        "  let mut i: u64 = 0;\n"
        f"  while i < {n} vow {{\n"
        f"    invariant: i <= {n} && v.len() == i\n"
        "  } {\n"
        "    v.push(1);\n"
        "    i = i + 1;\n"
        "  }\n" + "  v.pop();\n" * k + "  v.len()\n"
        "}\n"
    )
    return src, ["print_u64(push_pop());"], n


def shape_vec_push_chain(rng, honest, bound):
    """Straight-line pushes: no loop, so only the collection itself is long."""
    n = bound + rng.randint(1, 4)
    claimed = n if honest else n - 1
    pushes = "".join(f"  v.push({k});\n" for k in range(n))
    src = (
        "fn chain() -> Vec<i64> vow {\n"
        f"  ensures: result.len() == {claimed}\n"
        "} {\n"
        "  let v: Vec<i64> = Vec::new();\n" + pushes + "  v\n"
        "}\n"
    )
    return src, ["print_u64(chain().len());"], 0


def shape_vec_nested_fill(rng, honest, bound):
    """Two nested counted loops: `a * b` pushes, each loop short on its own."""
    b = rng.choice((4, 8, 16))
    a = bound // b + rng.randint(1, 3)
    total = a * b
    claimed = total if honest else total - 1
    src = (
        "fn grid() -> Vec<i64> vow {\n"
        f"  ensures: result.len() == {claimed}\n"
        "} {\n"
        "  let v: Vec<i64> = Vec::new();\n"
        "  let mut i: u64 = 0;\n"
        f"  while i < {a} vow {{\n"
        f"    invariant: i <= {a} && v.len() == i * {b}\n"
        "  } {\n"
        "    let mut j: u64 = 0;\n"
        f"    while j < {b} vow {{\n"
        f"      invariant: j <= {b} && v.len() == i * {b} + j\n"
        "    } {\n"
        "      v.push(1);\n"
        "      j = j + 1;\n"
        "    }\n"
        "    i = i + 1;\n"
        "  }\n"
        "  v\n"
        "}\n"
    )
    return src, ["print_u64(grid().len());"], max(a, b)


def shape_vec_param_last(rng, honest, bound):
    """Read the last element of a Vec parameter longer than the bound.

    The buggy version returns the first element; `main` fills the vector with
    distinct values, so only the last position tells the two apart.
    """
    n = bound + rng.randint(1, 5)
    idx = "v[v.len() - 1]" if honest else "v[0]"
    src = (
        "fn last_of(v: Vec<i64>) -> i64 vow {\n"
        "  requires: v.len() > 0,\n"
        "  ensures: result == v[v.len() - 1]\n"
        "} {\n"
        f"  {idx}\n"
        "}\n"
    )
    main_lines = [
        "let w: Vec<i64> = Vec::new();",
        "let mut j: u64 = 0;",
        f"while j < {n} {{",
        "  w.push(j as i64);",
        "  j = j + 1;",
        "}",
        "print_i64(last_of(w));",
    ]
    return src, main_lines, 0


def shape_vec_set_overwrite(rng, honest, bound):
    """Overwrite one slot of a long straight-line vector and read it back."""
    n = bound + rng.randint(1, 4)
    k = rng.randint(0, n - 1)
    claimed = 9 if honest else 10
    pushes = "".join("  v.push(0);\n" for _ in range(n))
    src = (
        "fn overwrite() -> i64 vow {\n"
        f"  ensures: result == {claimed}\n"
        "} {\n"
        "  let v: Vec<i64> = Vec::new();\n" + pushes + f"  v[{k}] = 9;\n"
        f"  v[{k}]\n"
        "}\n"
    )
    return src, ["print_i64(overwrite());"], 0


SHAPES = {
    "vec_len_literal": shape_vec_len_literal,
    "vec_len_cap": shape_vec_len_cap,
    "vec_last_element": shape_vec_last_element,
    "scalar_loop_sum": shape_scalar_loop_sum,
    "vec_param_clamp": shape_vec_param_clamp,
    "vec_pop_all": shape_vec_pop_all,
    "vec_push_chain": shape_vec_push_chain,
    "vec_nested_fill": shape_vec_nested_fill,
    "vec_param_last": shape_vec_param_last,
    "vec_set_overwrite": shape_vec_set_overwrite,
}


def directives(shape, cls, regime, known):
    lines = [
        "// Generated by scripts/gen_verifier_corpus.py (#1473). Do not edit.",
        f"// TEST: gen-class {cls} {shape} {regime}",
    ]
    if cls == CLASS_CONTROL:
        lines.append("// TEST: expect-verified")
    issue = known.get((shape, cls, regime))
    if issue:
        kind = "known-control-gap" if cls == CLASS_CONTROL else "known-soundness-gap"
        lines.append(f"// TEST: {kind} {issue}")
    return lines


def gen_program(rng, index, known=None):
    """One program. The draw order is fixed, so a seed names a corpus forever."""
    known = KNOWN_DISAGREEMENTS if known is None else known
    shape = rng.choice(sorted(SHAPES))
    honest = rng.random() < 0.4
    bound = rng.choice(BOUNDARIES)
    cls = CLASS_CONTROL if honest else CLASS_BUG
    fn_src, main_lines, trips = SHAPES[shape](rng, honest, bound)
    regime = BEYOND if trips > UNWIND_BOUND else WITHIN
    body = "\n".join(f"  {line}" for line in main_lines)
    main = "fn main() -> i32 [io] {\n" + body + "\n  0\n}\n"
    head = "\n".join(directives(shape, cls, regime, known))
    text = f"{head}\nmodule GenV{index}\n\n{fn_src}\n{main}"
    skip = REPLAY_SKIPS.get((shape, cls))
    # The replay directive goes last so no source span moves.
    return text + (f"// TEST: replay-skipped {skip}\n" if skip else "")
