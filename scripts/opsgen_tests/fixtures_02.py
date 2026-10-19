"""Operation catalogue test fixtures."""

import json
from pathlib import Path

from opsgen_tests.fixtures_01 import (
    ENV_OPS,
    FS_STDIN_ARGS_STDERR_OPS,
    HASH_OPS,
    PRINT_OPS,
    PROCESS_OPS,
    _query_op,
)

# Query and utility operations migrated in #1271: memory and time queries,
# hex helpers, parse and string-conversion helpers, and the String helpers.
# (name, symbol, params, ret, doc_signature, effects, arena_routing, c_model)
_QUERY_UTILITY_ROWS = [
    (
        "memory_root_arena_bytes",
        "__vow_memory_root_arena_bytes",
        [],
        "u64",
        "fn() -> u64",
        "[io]",
    ),
    ("memory_peak_bytes", "__vow_memory_peak_bytes", [], "u64", "fn() -> u64", "[io]"),
    (
        "memory_alloc_count_since_start",
        "__vow_memory_alloc_count_since_start",
        [],
        "u64",
        "fn() -> u64",
        "[io]",
    ),
    ("time_unix", "__vow_time_unix", [], "i64", "fn() -> i64", "[io]"),
    ("time_unix_ms", "__vow_time_unix_ms", [], "i64", "fn() -> i64", "[io]"),
    ("time_micros", "__vow_time_micros", [], "i64", "fn() -> i64", "[io]"),
    ("num_cpus", "__vow_num_cpus", [], "i64", "fn() -> i64", "[io]"),
    (
        "proc_sample",
        "__vow_proc_sample",
        [],
        "ptr",
        "fn() -> String",
        "[io]",
        True,
    ),
    (
        "gzip_write_file",
        "__vow_gzip_write_file",
        ["ptr", "ptr"],
        "i64",
        "fn(path: String, data: String) -> i64",
        "[io]",
    ),
    (
        "hex_encode",
        "__vow_hex_encode",
        ["ptr"],
        "ptr",
        "fn(data: Vec<u8>) -> String",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "hex_decode",
        "__vow_hex_decode",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Vec<u8>",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "parse_i64",
        "__vow_string_parse_i64_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<i64>",
        "[]",
        True,
        "base_and_arena",
    ),
    (
        "parse_i8",
        "__vow_string_parse_i8_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<i8>",
        "[]",
        True,
        "base",
    ),
    (
        "parse_i16",
        "__vow_string_parse_i16_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<i16>",
        "[]",
        True,
        "base",
    ),
    (
        "parse_i32",
        "__vow_string_parse_i32_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<i32>",
        "[]",
        True,
        "base",
    ),
    (
        "parse_i128",
        "__vow_string_parse_i128_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<i128>",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "parse_u8",
        "__vow_string_parse_u8_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<u8>",
        "[]",
        True,
        "base",
    ),
    (
        "parse_u16",
        "__vow_string_parse_u16_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<u16>",
        "[]",
        True,
        "base",
    ),
    (
        "parse_u32",
        "__vow_string_parse_u32_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<u32>",
        "[]",
        True,
        "base",
    ),
    (
        "parse_u128",
        "__vow_string_parse_u128_opt",
        ["ptr"],
        "ptr",
        "fn(s: String) -> Option<u128>",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "parse_f64_bits",
        "__vow_parse_f64_bits",
        ["ptr"],
        "u64",
        "fn(s: String) -> u64",
        "[]",
        False,
        "unmodeled",
    ),
    (
        "format_f64_bits",
        "__vow_format_f64_bits",
        ["u64"],
        "ptr",
        "fn(bits: u64) -> String",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "int_to_string",
        "__vow_string_from_i64",
        ["i64"],
        "ptr",
        "fn(v: i64) -> String",
        "[]",
        True,
        "base_and_arena",
    ),
    (
        "uint_to_string",
        "__vow_string_from_u64",
        ["u64"],
        "ptr",
        "fn(v: u64) -> String",
        "[]",
        True,
        "base_and_arena",
    ),
    (
        "int128_to_string",
        "__vow_string_from_i128",
        ["i128"],
        "ptr",
        "fn(v: i128) -> String",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "uint128_to_string",
        "__vow_string_from_u128",
        ["u128"],
        "ptr",
        "fn(v: u128) -> String",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "string_substr",
        "__vow_string_substr",
        ["ptr", "u64", "u64"],
        "ptr",
        "fn(s: String, start: u64, len: u64) -> String",
        "[]",
        True,
        "base_and_arena",
    ),
    (
        "string_split",
        "__vow_string_split",
        ["ptr", "ptr"],
        "ptr",
        "fn(s: String, delim: String) -> Vec<String>",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "string_starts_with",
        "__vow_string_starts_with",
        ["ptr", "ptr"],
        "i64",
        "fn(s: String, prefix: String) -> i64",
        "[]",
        False,
        "unmodeled",
    ),
    (
        "string_ends_with",
        "__vow_string_ends_with",
        ["ptr", "ptr"],
        "i64",
        "fn(s: String, suffix: String) -> i64",
        "[]",
        False,
        "unmodeled",
    ),
    (
        "string_trim",
        "__vow_string_trim",
        ["ptr"],
        "ptr",
        "fn(s: String) -> String",
        "[]",
        True,
        "base_and_arena",
    ),
    (
        "string_to_upper",
        "__vow_string_to_upper",
        ["ptr"],
        "ptr",
        "fn(s: String) -> String",
        "[]",
        True,
        "base_and_arena",
    ),
    (
        "string_to_lower",
        "__vow_string_to_lower",
        ["ptr"],
        "ptr",
        "fn(s: String) -> String",
        "[]",
        True,
        "base_and_arena",
    ),
    (
        "string_replace",
        "__vow_string_replace",
        ["ptr", "ptr", "ptr"],
        "ptr",
        "fn(s: String, from: String, to: String) -> String",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "string_join",
        "__vow_string_join",
        ["ptr", "ptr"],
        "ptr",
        "fn(parts: Vec<String>, sep: String) -> String",
        "[]",
        True,
        "unmodeled",
    ),
    (
        "vec_sort",
        "__vow_vec_sort",
        ["ptr"],
        "ptr",
        "fn(v: Vec<i64>) -> Vec<i64>",
        "[]",
        True,
        "unmodeled",
    ),
]


QUERY_UTILITY_OPS = [_query_op(*row) for row in _QUERY_UTILITY_ROWS]


def _pure_c_model(op):
    if op["effects"] == "[]" and "c_model" not in op:
        return {"c_model": "unmodeled"}
    return {}


# The native verifier models the String slice builtins by runtime symbol
# (`vc_str`, #1426), the float text hooks (`vc_hooks`, #1646), the numeric
# builtins (`vc_numeric`, #1654) and the folds, codecs and opaque total functions
# of #1654 (`vc_strfold`, `vc_hooks`), and none of the others yet. The pure hash
# builtins still say whether the ESBMC C model knows them.
VERIFIER_KNOWN_NAMES = {
    "string_substr",
    "string_starts_with",
    "string_ends_with",
    "parse_f64_bits",
    "format_f64_bits",
    "hash_u64",
    "int_to_string",
    "uint_to_string",
    "int128_to_string",
    "uint128_to_string",
    "hash_str",
    "hex_encode",
    "hex_decode",
    "string_trim",
    "string_to_upper",
    "string_to_lower",
    "string_replace",
    "string_split",
    "string_join",
    "parse_i8",
    "parse_i16",
    "parse_i32",
    "parse_i64",
    "parse_i128",
    "parse_u8",
    "parse_u16",
    "parse_u32",
    "parse_u128",
    "vec_sort",
}


def _reasoned(op):
    """`op` with the `verifier_skip_reason` a pure unmodeled op must carry."""
    if op["effects"] == "[]" and op.get("verifier_model") == "unmodeled":
        return dict(op, verifier_skip_reason="not modelled by the test fixture")
    return op


def _without_skip_reason(ops):
    return [{k: v for k, v in op.items() if k != "verifier_skip_reason"} for op in ops]


KNOWN_OPS = [
    dict(
        op,
        verifier_model="known" if op["name"] in VERIFIER_KNOWN_NAMES else "unmodeled",
        **_pure_c_model(op),
    )
    for op in (
        PRINT_OPS
        + FS_STDIN_ARGS_STDERR_OPS
        + PROCESS_OPS
        + HASH_OPS
        + ENV_OPS
        + QUERY_UTILITY_OPS
    )
]


GRAMMAR_FIXTURE = """\
### Builtin Function Signatures

#### FFI Wrapper Intrinsics

| Function | Signature | Effects |
|---|---|---|
| `pin_to_root` | `fn(value: String) -> String` | `[]` |

#### Print / IO

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |

## Canonical Form

irrelevant trailing section.
"""


def _write_fixture_tree(
    tmp: Path, *, grammar_effects="[io]", main_vow_ok=True, skill_rs_ok=True
) -> None:
    (tmp / "docs" / "spec").mkdir(parents=True)
    (tmp / "docs" / "spec" / "grammar.md").write_text(
        GRAMMAR_FIXTURE.replace(
            "| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |",
            f"| `print_str`      | `fn(s: String) -> ()`                      | `{grammar_effects}`     |",
        )
    )
    (tmp / "compiler").mkdir(parents=True)
    main_vow_lines = [
        '    r.push_str(String::from("      \\"print_str\\": \\"fn(s: String) -> () [io]\\",\\n"));\n'
        if main_vow_ok
        else "    // no print_str doc line here\n",
        '    r.push_str(String::from("      \\"print_i64\\": \\"fn(v: i64) -> () [io]\\",\\n"));\n',
        '    r.push_str(String::from("      \\"print_u64\\": \\"fn(v: u64) -> () [io]\\",\\n"));\n',
    ]
    (tmp / "compiler" / "main.vow").write_text("".join(main_vow_lines))
    (tmp / "vow" / "src").mkdir(parents=True)
    skill_rs_lines = [
        '"print_str": "fn(s: String) -> () [io]",\n'
        if skill_rs_ok
        else "no doc line\n",
        '"print_i64": "fn(v: i64) -> () [io]",\n',
        '"print_u64": "fn(v: u64) -> () [io]",\n',
    ]
    (tmp / "vow" / "src" / "skill.rs").write_text("".join(skill_rs_lines))


def _write_target_fixtures(tmp: Path) -> None:
    for rel, header in [
        ("vow-ir/src/lower/mod.rs", "fn vow_static_builtin_to_runtime() {}\n"),
        ("vow-codegen/src/cranelift_backend.rs", "fn make_extern_sig() {}\n"),
        ("vow-clif-shim/src/lib.rs", "fn make_extern_sig() {}\n"),
        ("compiler/lower.vow", "fn builtin_to_extern() {}\n"),
        ("vow-ir/src/region.rs", "fn infer_regions() {}\n"),
        ("vow-verify/src/c_emitter.rs", "fn is_known_builtin() {}\n"),
        ("compiler/c_emitter.vow", "fn is_known_builtin() {}\n"),
    ]:
        path = tmp / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            f"{header}// GENERATE:OPERATIONS:START\nplaceholder\n// GENERATE:OPERATIONS:END\n"
        )


ROUTES = ["__vow_map_get", "__vow_string_parse_i64_opt"]


def _eval_vow_symbol_set(source: str, sym: str) -> bool:
    """Interpret the decision-tree function emitted by `_vow_symbol_set_fn`."""
    lines = [ln.strip() for ln in source.splitlines()[1:-1]]
    env = {"n": len(sym)}

    def run(i: int, end: int):
        while i < end:
            line = lines[i]
            if line.startswith("let "):
                name, _, rhs = line[4:].partition(": ")
                if name != "n":
                    pos = int(rhs[len("i64 = sym.byte_at(") : -2])
                    env[name] = ord(sym[pos]) if pos < len(sym) else -1
                i += 1
            elif line.startswith("if "):
                var, _, rest = line[3:].partition(" == ")
                value = int(rest[:-2])
                depth, j = 1, i + 1
                while depth:
                    if lines[j].endswith("{"):
                        depth += 1
                    elif lines[j] == "}":
                        depth -= 1
                    j += 1
                if env[var] == value:
                    result = run(i + 1, j - 1)
                    if result is not None:
                        return result
                i = j
            elif line.startswith("return sym == String::from("):
                return sym == line[len('return sym == String::from("') : -3]
            elif line == "return false;":
                return False
            else:
                raise AssertionError(f"unexpected line {line!r}")
        return None

    return bool(run(0, len(lines)))


def _heap_op(name="heap_op", ret="String", **extra):
    if extra.get("arena_routing") == "none":
        extra.setdefault("arena_routing_reason", "returns a reused scratch buffer")
    return dict(
        name=name,
        runtime_symbol=f"__vow_{name}",
        params=[],
        **{"return": "ptr"},
        doc_signature=f"fn() -> {ret}",
        effects="[]",
        **extra,
    )


# The symbols the ESBMC C model recognised in vow-verify/src/c_emitter.rs's
# hand-written is_known_builtin before the migration, restricted to the
# catalogued query/utility builtins. Pinned independently of the catalogue so
# that moving the fact into operations.json provably did not change it.
PRE_MIGRATION_C_MODEL_SYMBOLS = {
    "__vow_string_substr",
    "__vow_string_substr_in_arena",
    "__vow_string_trim",
    "__vow_string_trim_in_arena",
    "__vow_string_to_upper",
    "__vow_string_to_upper_in_arena",
    "__vow_string_to_lower",
    "__vow_string_to_lower_in_arena",
    "__vow_string_from_i64",
    "__vow_string_from_i64_in_arena",
    "__vow_string_from_u64",
    "__vow_string_from_u64_in_arena",
    "__vow_string_parse_i64_opt",
    "__vow_string_parse_i64_opt_in_arena",
    "__vow_string_parse_i8_opt",
    "__vow_string_parse_u8_opt",
    "__vow_string_parse_i16_opt",
    "__vow_string_parse_u16_opt",
    "__vow_string_parse_u32_opt",
    "__vow_string_parse_i32_opt",
}


def _write_catalogue(tmp: Path, ops: list) -> None:
    spec = tmp / "docs" / "spec"
    spec.mkdir(parents=True, exist_ok=True)
    (spec / "operations.json").write_text(json.dumps({"operations": ops}))


def _write_excluded_catalogue(tmp: Path, excluded, ops=None) -> None:
    spec = tmp / "docs" / "spec"
    spec.mkdir(parents=True, exist_ok=True)
    body = {"operations": PRINT_OPS if ops is None else ops, "excluded": excluded}
    (spec / "operations.json").write_text(json.dumps(body))
