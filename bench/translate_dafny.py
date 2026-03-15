#!/usr/bin/env python3
"""Generate draft Vow benchmark files from Dafny HumanEval tasks."""

from __future__ import annotations

import json
import re
import sys
import tomllib
from pathlib import Path

ALREADY_TRANSLATED = {3, 5, 9, 13, 25, 31, 41, 42, 49, 60}
ROOT = Path(__file__).resolve().parent.parent
JSONL_PATH = Path("/tmp/dafny_tasks.jsonl")


def extract_he_number(source_id: str) -> int:
    parts = source_id.split("_")
    return int(parts[1][:3])


def extract_name(source_id: str) -> str:
    """Extract name from source-id like 'humaneval_042_incr_list'."""
    parts = source_id.split("_")
    if len(parts) > 2:
        return "_".join(parts[2:])
    return f"he{parts[1][:3]}"


def dafny_type_to_vow(dtype: str) -> str | None:
    dtype = dtype.strip()
    if dtype in ("int", "nat"):
        return "i64"
    if dtype == "bool":
        return "i64"  # Vow uses i64 for bool returns
    if dtype in ("seq<int>", "seq<nat>"):
        return "Vec<i64>"
    return None


def parse_dafny_signature(spec: str) -> dict | None:
    """Parse Dafny method signature into components."""
    m = re.match(
        r"method\s+(\w+)\(([^)]*)\)\s+returns\s*\(([^)]*)\)",
        spec.split("\n")[0].strip(),
    )
    if not m:
        return None

    name = m.group(1)
    params_str = m.group(2).strip()
    returns_str = m.group(3).strip()

    # Parse params
    params = []
    if params_str:
        for p in params_str.split(","):
            p = p.strip()
            pm = re.match(r"(\w+)\s*:\s*(.+)", p)
            if pm:
                pname = pm.group(1)
                ptype = pm.group(2).strip()
                vtype = dafny_type_to_vow(ptype)
                if vtype is None:
                    return None
                params.append((pname, ptype, vtype))

    # Parse return
    ret_parts = returns_str.split(",")
    if len(ret_parts) > 1:
        return None  # multi-return
    rm = re.match(r"(\w+)\s*:\s*(.+)", ret_parts[0].strip())
    if not rm:
        return None
    ret_name = rm.group(1)
    ret_type = rm.group(2).strip()
    vret = dafny_type_to_vow(ret_type)
    if vret is None:
        return None

    return {
        "name": name,
        "params": params,
        "ret_name": ret_name,
        "ret_dafny_type": ret_type,
        "ret_vow_type": vret,
    }


def snake_case(name: str) -> str:
    s = re.sub(r"(?<=[a-z])(?=[A-Z])", "_", name)
    return s.lower()


def classify_difficulty(he_num: int, spec: str) -> str:
    """Heuristic difficulty classification."""
    has_loop = "while" in spec or "for" in spec or "seq<int>" in spec
    has_nested = spec.count("while") > 1 or spec.count("forall") > 1
    if has_nested:
        return "hard"
    if has_loop:
        return "medium"
    return "easy"


def classify_fidelity(spec: str, helpers: str) -> str:
    """Heuristic fidelity classification based on Dafny spec complexity."""
    combined = spec + "\n" + helpers
    has_forall = "forall" in combined
    has_exists = "exists" in combined
    has_spec_fn = bool(re.search(r"function\s+\w+", combined))

    if has_forall or has_exists:
        return "partial"
    if has_spec_fn:
        return "partial"
    return "exact"


def generate_vow_signature(sig: dict) -> str:
    params = ", ".join(f"{snake_case(p[0])}: {p[2]}" for p in sig["params"])
    return f"fn {snake_case(sig['name'])}({params}) -> {sig['ret_vow_type']}"


def generate_requires(sig: dict) -> list[str]:
    """Generate requires clauses for bounded verification."""
    reqs = []
    for pname, dtype, vtype in sig["params"]:
        sname = snake_case(pname)
        if vtype == "i64":
            if dtype == "nat":
                reqs.append(f"  requires: {sname} >= 0")
                reqs.append(f"  requires: {sname} <= 100")
            else:
                reqs.append(f"  requires: {sname} >= 0")
                reqs.append(f"  requires: {sname} <= 100")
        elif vtype == "Vec<i64>":
            reqs.append(f"  requires: {sname}.len() >= 0")
            reqs.append(f"  requires: {sname}.len() <= 8")
    return reqs


def generate_ensures(sig: dict) -> list[str]:
    """Generate placeholder ensures clauses."""
    ens = []
    if sig["ret_vow_type"] == "i64":
        if sig["ret_dafny_type"] == "bool":
            ens.append("  ensures: result >= 0")
            ens.append("  ensures: result <= 1")
        else:
            ens.append("  ensures: result >= 0")
    elif sig["ret_vow_type"] == "Vec<i64>":
        ens.append("  ensures: result.len() >= 0")
    return ens


def generate_benchmark(task: dict, he_num: int) -> dict | None:
    """Generate benchmark files for a single task."""
    sig = parse_dafny_signature(task["vc-spec"])
    if sig is None:
        return None

    name = snake_case(sig["name"])
    bench_id = f"HE{he_num:03d}"
    bench_dir = ROOT / "benchmarks" / "humaneval" / f"{bench_id}_{name}"

    difficulty = classify_difficulty(he_num, task["vc-spec"] + "\n" + task.get("vc-helpers", ""))
    fidelity = classify_fidelity(task["vc-spec"], task.get("vc-helpers", ""))

    vow_sig = generate_vow_signature(sig)
    reqs = generate_requires(sig)
    ens = generate_ensures(sig)
    module_name = "".join(w.title() for w in name.split("_"))

    vow_block = ",\n".join(reqs + ens)

    skeleton = f"""module {module_name}

{vow_sig} vow {{
{vow_block}
}} {{
  {"Vec::new()" if sig["ret_vow_type"] == "Vec<i64>" else "0"}
}}

fn main() -> i32 [io] {{
  0
}}
"""

    spec_md = f"""# {bench_id}: {name.replace("_", " ").title()}

**Origin:** HumanEval-{he_num:03d} from the Vericoding benchmark (arxiv.org/abs/2509.22908)

## Problem

{task["vc-description"]}

## Signature

```vow
{vow_sig}
```

## Contracts

{chr(10).join(f"- `{r.strip()}`" for r in reqs + ens)}

## Contract Fidelity

**{fidelity.upper()}** — TODO: classify against Dafny spec.

## Dafny Spec

```dafny
{task["vc-spec"]}
```

## Hints

- TODO: add implementation hints
"""

    meta = f"""[benchmark]
id = "{bench_id}"
name = "{name}"
difficulty = "{difficulty}"
tags = ["humaneval"]
unwind = 10
modules = 1
expected_status = "Verified"
max_cegis_iterations = 5
contract_fidelity = "{fidelity}"
"""

    return {
        "bench_id": bench_id,
        "name": name,
        "difficulty": difficulty,
        "fidelity": fidelity,
        "dir": bench_dir,
        "skeleton": skeleton,
        "spec_md": spec_md,
        "meta": meta,
        "reference": skeleton,  # placeholder — same as skeleton
        "manifest_entry": {
            "id": bench_id,
            "name": name,
            "difficulty": difficulty,
            "path": f"humaneval/{bench_id}_{name}",
            "expected_status": "Verified",
        },
    }


def main() -> None:
    if not JSONL_PATH.exists():
        print("Error: /tmp/dafny_tasks.jsonl not found", file=sys.stderr)
        sys.exit(1)

    with open(JSONL_PATH) as f:
        all_tasks = [json.loads(line) for line in f]

    he_tasks = [t for t in all_tasks if t["source"] == "humaneval"]

    # Load triage to filter to translatable only
    triage_path = ROOT / "benchmarks" / "humaneval" / "triage.toml"
    with open(triage_path, "rb") as f:
        triage = tomllib.load(f)

    translatable_nums = set()
    for entry in triage["tasks"]:
        if entry["status"] == "translatable":
            translatable_nums.add(entry["humaneval_num"])

    generated = 0
    failed = 0
    manifest_entries = []
    seen_nums: set[int] = set()

    for task in he_tasks:
        he_num = extract_he_number(task["source-id"])
        if he_num in ALREADY_TRANSLATED:
            continue
        if he_num not in translatable_nums:
            continue
        if he_num in seen_nums:
            continue
        seen_nums.add(he_num)

        result = generate_benchmark(task, he_num)
        if result is None:
            failed += 1
            continue

        # Write files
        result["dir"].mkdir(parents=True, exist_ok=True)
        (result["dir"] / "skeleton.vow").write_text(result["skeleton"])
        (result["dir"] / "spec.md").write_text(result["spec_md"])
        (result["dir"] / "meta.toml").write_text(result["meta"])
        (result["dir"] / "reference.vow").write_text(result["reference"])
        manifest_entries.append(result["manifest_entry"])
        generated += 1

    print(f"Generated: {generated}")
    print(f"Failed to parse: {failed}")

    # Print manifest entries to add
    if manifest_entries:
        print("\n--- Add to benchmarks/manifest.toml ---")
        for e in sorted(manifest_entries, key=lambda x: x["id"]):
            print(f"""
[[benchmarks]]
id = "{e['id']}"
name = "{e['name']}"
difficulty = "{e['difficulty']}"
path = "{e['path']}"
expected_status = "Verified"
""".strip())
            print()


if __name__ == "__main__":
    main()
