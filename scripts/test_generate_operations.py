#!/usr/bin/env python3
"""Behavior tests for scripts/generate_operations.py.

The generator is the Operation Catalogue's single writer/checker for the
runtime-symbol/ABI/return-shape/doc projections it splices into both
compilers. These tests exercise its functions directly (the same seam
scripts/test_check_help_coverage.py uses for generate_help.py's siblings),
plus the real docs/spec/operations.json catalogue for the end-to-end cases.
"""

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

sys.path.insert(0, str(REPO_ROOT / "scripts"))
import generate_operations as go  # noqa: E402


PRINT_OPS = [
    {
        "name": "print_str",
        "runtime_symbol": "__vow_string_print",
        "params": ["ptr"],
        "return": "unit",
        "doc_signature": "fn(s: String) -> ()",
        "effects": "[io]",
    },
    {
        "name": "print_i64",
        "runtime_symbol": "__vow_print_i64",
        "params": ["i64"],
        "return": "unit",
        "doc_signature": "fn(v: i64) -> ()",
        "effects": "[io]",
    },
    {
        "name": "print_u64",
        "runtime_symbol": "__vow_print_u64",
        "params": ["u64"],
        "return": "unit",
        "doc_signature": "fn(v: u64) -> ()",
        "effects": "[io]",
    },
]

# Filesystem, stdin, args, and direct-stderr operations migrated in #1272 --
# see docs/spec/operations.json for the checked-in source of truth.
FS_STDIN_ARGS_STDERR_OPS = [
    {
        "name": "fs_read",
        "runtime_symbol": "__vow_fs_read",
        "params": ["ptr"],
        "return": "ptr",
        "doc_signature": "fn(path: String) -> String",
        "effects": "[read]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "fs_open",
        "runtime_symbol": "__vow_fs_open",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String) -> i64",
        "effects": "[read]",
    },
    {
        "name": "fs_read_line",
        "runtime_symbol": "__vow_fs_read_line",
        "params": ["i64"],
        "return": "ptr",
        "doc_signature": "fn(handle: i64) -> String",
        "effects": "[read]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "fs_status",
        "runtime_symbol": "__vow_fs_status",
        "params": ["i64"],
        "return": "i64",
        "doc_signature": "fn(handle: i64) -> i64",
        "effects": "[read]",
    },
    {
        "name": "fs_close",
        "runtime_symbol": "__vow_fs_close",
        "params": ["i64"],
        "return": "i64",
        "doc_signature": "fn(handle: i64) -> i64",
        "effects": "[read]",
    },
    {
        "name": "fs_write",
        "runtime_symbol": "__vow_fs_write",
        "params": ["ptr", "ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String, data: String) -> i64",
        "effects": "[write]",
    },
    {
        "name": "fs_exists",
        "runtime_symbol": "__vow_fs_exists",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String) -> i64",
        "effects": "[read]",
    },
    {
        "name": "fs_mkdir",
        "runtime_symbol": "__vow_fs_mkdir",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String) -> i64",
        "effects": "[io]",
    },
    {
        "name": "fs_listdir",
        "runtime_symbol": "__vow_fs_listdir",
        "params": ["ptr"],
        "return": "ptr",
        "doc_signature": "fn(path: String) -> Vec<String>",
        "effects": "[read]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "fs_remove",
        "runtime_symbol": "__vow_fs_remove",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String) -> i64",
        "effects": "[io]",
    },
    {
        "name": "fs_remove_dir",
        "runtime_symbol": "__vow_fs_remove_dir",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String) -> i64",
        "effects": "[io]",
    },
    {
        "name": "fs_remove_dir_all",
        "runtime_symbol": "__vow_fs_remove_dir_all",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String) -> i64",
        "effects": "[io]",
    },
    {
        "name": "fs_is_dir",
        "runtime_symbol": "__vow_fs_is_dir",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String) -> i64",
        "effects": "[read]",
    },
    {
        "name": "fs_is_symlink",
        "runtime_symbol": "__vow_fs_is_symlink",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(path: String) -> i64",
        "effects": "[read]",
    },
    {
        "name": "fs_rename",
        "runtime_symbol": "__vow_fs_rename",
        "params": ["ptr", "ptr"],
        "return": "i64",
        "doc_signature": "fn(old: String, new: String) -> i64",
        "effects": "[io]",
    },
    {
        "name": "args",
        "runtime_symbol": "__vow_args",
        "params": [],
        "return": "ptr",
        "doc_signature": "fn() -> Vec<String>",
        "effects": "[read]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "stdin_read",
        "runtime_symbol": "__vow_stdin_read",
        "params": [],
        "return": "ptr",
        "doc_signature": "fn() -> String",
        "effects": "[read]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "stdin_read_line",
        "runtime_symbol": "__vow_stdin_read_line",
        "params": [],
        "return": "ptr",
        "doc_signature": "fn() -> String",
        "effects": "[read]",
        "arena_routing": "none",
        "arena_routing_reason": "Returns a view of a per-thread scratch line buffer that the next call overwrites; it allocates in no arena, so there is nothing to route and memory stays bounded by the longest line.",
    },
    {
        "name": "stdin_ready",
        "runtime_symbol": "__vow_stdin_ready",
        "params": [],
        "return": "bool",
        "doc_signature": "fn() -> bool",
        "effects": "[read]",
    },
    {
        "name": "eprintln_str",
        "runtime_symbol": "__vow_eprintln_str",
        "params": ["ptr"],
        "return": "unit",
        "doc_signature": "fn(s: String) -> ()",
        "effects": "[io]",
    },
    {
        "name": "try_eprintln_str",
        "runtime_symbol": "__vow_try_eprintln_str",
        "params": ["ptr"],
        "return": "i64",
        "doc_signature": "fn(s: String) -> i64",
        "effects": "[io]",
    },
]

# Process operations migrated in #1273 -- see docs/spec/operations.json for
# the checked-in source of truth. process_get_stdout/get_stderr/stdout_for/
# stderr_for return a freshly heap-allocated String, hence arena_routing.
PROCESS_OPS = [
    {
        "name": "process_exit",
        "runtime_symbol": "__vow_process_exit",
        "params": ["i64"],
        "return": "unit",
        "doc_signature": "fn(code: i64) -> !",
        "effects": "[io]",
    },
    {
        "name": "process_run",
        "runtime_symbol": "__vow_process_run",
        "params": ["ptr", "ptr"],
        "return": "i64",
        "doc_signature": "fn(cmd: String, args: Vec<String>) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_get_stdout",
        "runtime_symbol": "__vow_process_get_stdout",
        "params": [],
        "return": "ptr",
        "doc_signature": "fn() -> String",
        "effects": "[io]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "process_get_stderr",
        "runtime_symbol": "__vow_process_get_stderr",
        "params": [],
        "return": "ptr",
        "doc_signature": "fn() -> String",
        "effects": "[io]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "process_start",
        "runtime_symbol": "__vow_process_start",
        "params": ["ptr", "ptr"],
        "return": "i64",
        "doc_signature": "fn(cmd: String, args: Vec<String>) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_start_capped",
        "runtime_symbol": "__vow_process_start_capped",
        "params": ["ptr", "ptr", "i64"],
        "return": "i64",
        "doc_signature": "fn(cmd: String, args: Vec<String>, mem_kb: i64) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_wait",
        "runtime_symbol": "__vow_process_wait",
        "params": ["i64"],
        "return": "i64",
        "doc_signature": "fn(pid: i64) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_wait_timeout",
        "runtime_symbol": "__vow_process_wait_timeout",
        "params": ["i64", "i64"],
        "return": "i64",
        "doc_signature": "fn(pid: i64, timeout_ms: i64) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_poll_wait",
        "runtime_symbol": "__vow_process_poll_wait",
        "params": ["i64", "i64"],
        "return": "i64",
        "doc_signature": "fn(pid: i64, timeout_ms: i64) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_kill",
        "runtime_symbol": "__vow_process_kill",
        "params": ["i64"],
        "return": "i64",
        "doc_signature": "fn(pid: i64) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_stdout_for",
        "runtime_symbol": "__vow_process_stdout_for",
        "params": ["i64"],
        "return": "ptr",
        "doc_signature": "fn(pid: i64) -> String",
        "effects": "[io]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "process_stderr_for",
        "runtime_symbol": "__vow_process_stderr_for",
        "params": ["i64"],
        "return": "ptr",
        "doc_signature": "fn(pid: i64) -> String",
        "effects": "[io]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "process_start_piped",
        "runtime_symbol": "__vow_process_start_piped",
        "params": ["ptr", "ptr"],
        "return": "i64",
        "doc_signature": "fn(cmd: String, args: Vec<String>) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_write_stdin",
        "runtime_symbol": "__vow_process_write_stdin",
        "params": ["i64", "ptr"],
        "return": "i64",
        "doc_signature": "fn(pid: i64, data: String) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_close_stdin",
        "runtime_symbol": "__vow_process_close_stdin",
        "params": ["i64"],
        "return": "i64",
        "doc_signature": "fn(pid: i64) -> i64",
        "effects": "[io]",
    },
    {
        "name": "process_read_line",
        "runtime_symbol": "__vow_process_read_line",
        "params": ["i64", "i64"],
        "return": "ptr",
        "doc_signature": "fn(pid: i64, timeout_ms: i64) -> String",
        "effects": "[io]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "process_read_status",
        "runtime_symbol": "__vow_process_read_status",
        "params": ["i64"],
        "return": "i64",
        "doc_signature": "fn(pid: i64) -> i64",
        "effects": "[io]",
    },
]

HASH_OPS = [
    {
        "name": "hash_u64",
        "runtime_symbol": "__vow_hash_u64",
        "params": ["u64"],
        "return": "u64",
        "doc_signature": "fn(x: u64) -> u64",
        "effects": "[]",
    },
    {
        "name": "hash_str",
        "runtime_symbol": "__vow_hash_str",
        "params": ["ptr"],
        "return": "u64",
        "doc_signature": "fn(s: String) -> u64",
        "effects": "[]",
    },
]

ENV_OPS = [
    {
        "name": "fs_read_status",
        "runtime_symbol": "__vow_fs_read_status",
        "params": [],
        "return": "i64",
        "doc_signature": "fn() -> i64",
        "effects": "[read]",
    },
    {
        "name": "getenv",
        "runtime_symbol": "__vow_getenv",
        "params": ["ptr"],
        "return": "ptr",
        "doc_signature": "fn(name: String) -> String",
        "effects": "[read]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "path_lookup",
        "runtime_symbol": "__vow_path_lookup",
        "params": ["ptr"],
        "return": "ptr",
        "doc_signature": "fn(name: String) -> String",
        "effects": "[read]",
        "arena_routing": "heap_fresh",
    },
    {
        "name": "mktemp_dir",
        "runtime_symbol": "__vow_mktemp_dir",
        "params": ["ptr"],
        "return": "ptr",
        "doc_signature": "fn(prefix: String) -> String",
        "effects": "[io]",
        "arena_routing": "heap_fresh",
    },
]


def _query_op(name, symbol, params, ret, sig, effects, heap=False, c_model=None):
    op = {
        "name": name,
        "runtime_symbol": symbol,
        "params": params,
        "return": ret,
        "doc_signature": sig,
        "effects": effects,
    }
    if heap:
        op["arena_routing"] = "none" if heap == "none" else "heap_fresh"
    if heap == "none":
        op["arena_routing_reason"] = (
            "Allocates in the root arena and has no _in_arena variant, so each "
            "call retains its result until exit; tracked in issue 1639."
        )
    if c_model:
        op["c_model"] = c_model
    return op


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
        "none",
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


# The native verifier models none of these builtins yet. The two pure hash
# builtins still say whether the ESBMC C model knows them.
KNOWN_OPS = [
    dict(op, verifier_model="unmodeled", **_pure_c_model(op))
    for op in (
        PRINT_OPS
        + FS_STDIN_ARGS_STDERR_OPS
        + PROCESS_OPS
        + HASH_OPS
        + ENV_OPS
        + QUERY_UTILITY_OPS
    )
]


class LoadCatalogueTest(unittest.TestCase):
    def _write(self, tmp_path: Path, ops: list) -> None:
        spec_dir = tmp_path / "docs" / "spec"
        spec_dir.mkdir(parents=True, exist_ok=True)
        (spec_dir / "operations.json").write_text(json.dumps({"operations": ops}))

    def test_real_catalogue_loads_and_matches_known_ops(self):
        ops = go.load_catalogue(REPO_ROOT)
        self.assertEqual(ops, KNOWN_OPS)

    def test_missing_required_field_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            del bad["runtime_symbol"]
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("runtime_symbol", str(ctx.exception))
            self.assertIn("print_str", str(ctx.exception))

    def test_duplicate_name_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            dup = dict(PRINT_OPS[0])
            self._write(tmp, [PRINT_OPS[0], dup])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("duplicate", str(ctx.exception).lower())

    def test_duplicate_runtime_symbol_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            other = dict(PRINT_OPS[1])
            other["runtime_symbol"] = PRINT_OPS[0]["runtime_symbol"]
            self._write(tmp, [PRINT_OPS[0], other])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn(PRINT_OPS[0]["runtime_symbol"], str(ctx.exception))
            self.assertIn("duplicate", str(ctx.exception).lower())

    def test_unknown_return_token_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["return"] = "nonsense"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("nonsense", str(ctx.exception))

    def test_unknown_param_token_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["params"] = ["nonsense"]
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("nonsense", str(ctx.exception))

    def test_operations_not_a_list_raises_value_error(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            (tmp / "docs" / "spec").mkdir(parents=True)
            (tmp / "docs" / "spec" / "operations.json").write_text(
                json.dumps({"not_operations": []})
            )
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("operations", str(ctx.exception))

    def test_params_not_a_list_raises_value_error(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["params"] = "ptr"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("params", str(ctx.exception))

    def test_effects_unknown_token_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[frobnicate]"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("frobnicate", str(ctx.exception))

    def test_effects_malformed_bracket_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "io"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("effects", str(ctx.exception))

    def test_effects_trailing_comma_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[io,]"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("effects", str(ctx.exception))

    def test_effects_unsorted_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[write, read]"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("effects", str(ctx.exception))

    def test_effects_all_observed_real_values_parse_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            ops = []
            for i, eff in enumerate(["[]", "[io]", "[read]", "[write]"]):
                op = dict(PRINT_OPS[0])
                op["name"] = f"op_{i}"
                op["runtime_symbol"] = f"__vow_op_{i}"
                op["effects"] = eff
                ops.append(op)
            self._write(tmp, ops)
            go.load_catalogue(tmp)  # must not raise

    def test_effects_multi_token_sorted_parses_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            op = dict(PRINT_OPS[0])
            op["effects"] = "[read, write]"
            self._write(tmp, [op])
            go.load_catalogue(tmp)  # must not raise

    def test_doc_signature_arity_mismatch_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            self.assertEqual(bad["params"], ["ptr"])
            bad["doc_signature"] = "fn() -> ()"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("doc_signature", str(ctx.exception))

    def test_doc_signature_type_mismatch_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            self.assertEqual(bad["params"], ["ptr"])
            bad["doc_signature"] = "fn(s: i64) -> ()"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("doc_signature", str(ctx.exception))

    def test_doc_signature_return_type_mismatch_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            self.assertEqual(bad["return"], "unit")
            bad["doc_signature"] = "fn(s: String) -> i64"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("doc_signature", str(ctx.exception))

    def test_doc_signature_missing_arrow_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["doc_signature"] = "fn(s: String)"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("print_str", str(ctx.exception))
            self.assertIn("doc_signature", str(ctx.exception))

    def test_doc_signature_vec_return_matches_ptr_token(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            op = dict(PRINT_OPS[0])
            op["params"] = []
            op["return"] = "ptr"
            op["doc_signature"] = "fn() -> Vec<String>"
            self._write(tmp, [op])
            go.load_catalogue(tmp)  # must not raise

    def test_doc_signature_bool_return_matches_bool_token(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            op = dict(PRINT_OPS[0])
            op["params"] = []
            op["return"] = "bool"
            op["doc_signature"] = "fn() -> bool"
            self._write(tmp, [op])
            go.load_catalogue(tmp)  # must not raise

    def test_unknown_verifier_model_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["verifier_model"] = "nonsense"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("nonsense", str(ctx.exception))
            self.assertIn("verifier_model", str(ctx.exception))

    def test_known_verifier_model_values_parse_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            ops = []
            for i, val in enumerate(["known", "unmodeled"]):
                op = dict(PRINT_OPS[0])
                op["name"] = f"op_{i}"
                op["runtime_symbol"] = f"__vow_op_{i}"
                op["verifier_model"] = val
                ops.append(op)
            self._write(tmp, ops)
            go.load_catalogue(tmp)  # must not raise

    def test_unknown_arena_routing_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["arena_routing"] = "nonsense"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("nonsense", str(ctx.exception))
            self.assertIn("arena_routing", str(ctx.exception))

    def test_known_arena_routing_values_parse_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            ops = []
            for i, val in enumerate(["none", "heap_fresh"]):
                op = dict(PRINT_OPS[0])
                op["name"] = f"op_{i}"
                op["runtime_symbol"] = f"__vow_op_{i}"
                op["arena_routing"] = val
                ops.append(op)
            self._write(tmp, ops)
            go.load_catalogue(tmp)  # must not raise

    def test_arena_routing_is_optional(self):
        # PRINT_OPS carries no arena_routing field at all -- it must not be
        # required, since only heap-returning ops need to set it.
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write(tmp, [PRINT_OPS[0]])
            go.load_catalogue(tmp)  # must not raise

    def test_unexpected_field_name_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["bogus_field"] = "whatever"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("bogus_field", str(ctx.exception))

    def test_misspelled_optional_field_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["verifer_model"] = "known"
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("verifer_model", str(ctx.exception))

    def test_multiple_violations_all_reported(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            dup_name = dict(PRINT_OPS[0])
            bad_effects = dict(PRINT_OPS[1])
            bad_effects["effects"] = "[frobnicate]"
            bad_arity = dict(PRINT_OPS[2])
            bad_arity["doc_signature"] = "fn() -> ()"
            self._write(tmp, [PRINT_OPS[0], dup_name, bad_effects, bad_arity])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            message = str(ctx.exception)
            self.assertIn("duplicate operation name 'print_str'", message)
            self.assertIn("frobnicate", message)
            self.assertIn("doc_signature", message)
            self.assertIn("print_u64", message)

    def test_missing_field_does_not_crash_dependent_checks(self):
        # A missing 'params' must not reach the doc_signature arity check
        # and raise a raw TypeError -- it must stop at the clean, collected
        # "missing required field" message for that entry.
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            del bad["params"]
            self._write(tmp, [bad])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(tmp)
            self.assertIn("missing required field 'params'", str(ctx.exception))


class RealCatalogueProcessOpsTest(unittest.TestCase):
    """Fixture coverage for the process_* entries appended to the real
    catalogue: a non-unit (i64) return, a ptr return, and an
    arena_routing: heap_fresh entry, per issue #1273."""

    def test_process_run_has_i64_return(self):
        ops = go.load_catalogue(REPO_ROOT)
        op = next(o for o in ops if o["name"] == "process_run")
        self.assertEqual(op["return"], "i64")
        self.assertEqual(op["runtime_symbol"], "__vow_process_run")
        self.assertIsNone(op.get("arena_routing"))

    def test_process_get_stdout_has_ptr_return_and_heap_fresh_routing(self):
        ops = go.load_catalogue(REPO_ROOT)
        op = next(o for o in ops if o["name"] == "process_get_stdout")
        self.assertEqual(op["return"], "ptr")
        self.assertEqual(op["runtime_symbol"], "__vow_process_get_stdout")
        self.assertEqual(op["arena_routing"], "heap_fresh")

    def test_all_process_ops_present(self):
        ops = go.load_catalogue(REPO_ROOT)
        names = {o["name"] for o in ops}
        for name in [
            "process_exit",
            "process_run",
            "process_get_stdout",
            "process_get_stderr",
            "process_start",
            "process_start_capped",
            "process_wait",
            "process_wait_timeout",
            "process_poll_wait",
            "process_kill",
            "process_stdout_for",
            "process_stderr_for",
            "process_start_piped",
            "process_write_stdin",
            "process_close_stdin",
            "process_read_line",
            "process_read_status",
        ]:
            self.assertIn(name, names)

    def test_env_ops_present_with_expected_shape(self):
        ops = {o["name"]: o for o in go.load_catalogue(REPO_ROOT)}
        self.assertEqual(ops["fs_read_status"]["return"], "i64")
        self.assertEqual(ops["fs_read_status"]["params"], [])
        for name in ["getenv", "path_lookup", "mktemp_dir"]:
            self.assertEqual(ops[name]["return"], "ptr")
            self.assertEqual(ops[name]["params"], ["ptr"])
            self.assertEqual(ops[name]["arena_routing"], "heap_fresh")
            self.assertEqual(ops[name]["runtime_symbol"], f"__vow_{name}")
        self.assertNotIn("arena_routing", ops["fs_read_status"])

    def test_real_catalogue_projections_are_up_to_date(self):
        ops = go.load_catalogue(REPO_ROOT)
        routes = go.load_arena_routes(REPO_ROOT, ops)
        mismatches = go.check_projections(ops, REPO_ROOT, routes)
        self.assertEqual(mismatches, [])

    def test_real_catalogue_doc_facts_are_consistent(self):
        ops = go.load_catalogue(REPO_ROOT)
        mismatches = go.check_doc_facts(ops, REPO_ROOT)
        self.assertEqual(mismatches, [])


class MainCliTest(unittest.TestCase):
    """Exercises main() itself (the --check CLI entry point) rather than
    load_catalogue/check_projections/check_doc_facts directly, so the CLI
    plumbing (argv parsing, --repo-root threading) is proven end to end."""

    def _write_minimal_catalogue(self, tmp: Path, ops: list) -> None:
        spec_dir = tmp / "docs" / "spec"
        spec_dir.mkdir(parents=True, exist_ok=True)
        (spec_dir / "operations.json").write_text(json.dumps({"operations": ops}))

    def _run_main(self, argv: list[str]):
        old_argv = sys.argv
        sys.argv = ["generate_operations.py"] + argv
        try:
            return go.main()
        finally:
            sys.argv = old_argv

    def test_repo_root_flag_threads_through_check(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[frobnicate]"
            self._write_minimal_catalogue(tmp, [bad])
            stderr = io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            self.assertIn("frobnicate", stderr.getvalue())

    def test_repo_root_flag_defaults_to_real_repo(self):
        # No --repo-root: main() must still validate the real, checked-in
        # catalogue exactly as the CI/local `--check` invocation does.
        self._run_main(["--check"])

    def test_bad_catalogue_exits_cleanly_without_traceback(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bad = dict(PRINT_OPS[0])
            bad["effects"] = "[frobnicate]"
            self._write_minimal_catalogue(tmp, [bad])
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            combined = stdout.getvalue() + stderr.getvalue()
            self.assertNotIn("Traceback (most recent call last)", combined)
            self.assertIn("frobnicate", combined)

    def test_undecided_builtin_exits_cleanly_naming_verifier_model(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write_minimal_catalogue(tmp, [dict(PRINT_OPS[0])])
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            combined = stdout.getvalue() + stderr.getvalue()
            self.assertNotIn("Traceback (most recent call last)", combined)
            self.assertIn("verifier_model", combined)

    def test_heap_result_without_arena_routing_exits_naming_the_op(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            op = _heap_op("undeclared", verifier_model="unmodeled", c_model="unmodeled")
            self._write_minimal_catalogue(tmp, [op])
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            combined = stdout.getvalue() + stderr.getvalue()
            self.assertNotIn("Traceback (most recent call last)", combined)
            self.assertIn("'undeclared' returns a heap value", combined)

    def test_malformed_json_exits_cleanly_without_traceback(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            spec_dir = tmp / "docs" / "spec"
            spec_dir.mkdir(parents=True, exist_ok=True)
            (spec_dir / "operations.json").write_text("{not valid json")
            stdout, stderr = io.StringIO(), io.StringIO()
            with (
                self.assertRaises(SystemExit) as ctx,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                self._run_main(["--check", "--repo-root", str(tmp)])
            self.assertEqual(ctx.exception.code, 1)
            combined = stdout.getvalue() + stderr.getvalue()
            self.assertNotIn("Traceback (most recent call last)", combined)


class GenRustIrBlockTest(unittest.TestCase):
    def test_matches_expected_rustfmt_canonical_text(self):
        expected = (
            "// GENERATE:OPERATIONS:START\n"
            "fn catalogue_builtin_to_runtime(name: &str) -> Option<(&'static str, Ty)> {\n"
            "    match name {\n"
            '        "print_str" => Some(("__vow_string_print", Ty::Unit)),\n'
            '        "print_i64" => Some(("__vow_print_i64", Ty::Unit)),\n'
            '        "print_u64" => Some(("__vow_print_u64", Ty::Unit)),\n'
            "        _ => None,\n"
            "    }\n"
            "}\n"
            "\n"
            "fn catalogue_builtin_result_tag(name: &str) -> Option<BuiltinResultTag> {\n"
            "    match name {\n"
            "        _ => None,\n"
            "    }\n"
            "}\n"
            "// GENERATE:OPERATIONS:END"
        )
        self.assertEqual(go.gen_rust_ir_block(PRINT_OPS, []), expected)

    def test_i64_and_ptr_return_tokens_map_to_rust_ty(self):
        i64_op = dict(PRINT_OPS[1])
        i64_op["return"] = "i64"
        ptr_op = dict(PRINT_OPS[1])
        ptr_op["return"] = "ptr"
        block = go.gen_rust_ir_block([i64_op, ptr_op], [])
        self.assertIn(
            f'"{i64_op["name"]}" => Some(("{i64_op["runtime_symbol"]}", Ty::I64)),',
            block,
        )
        self.assertIn(
            f'"{ptr_op["name"]}" => Some(("{ptr_op["runtime_symbol"]}", Ty::Ptr)),',
            block,
        )

    def _op_returning(self, doc_ret, name="op"):
        return {
            "name": name,
            "runtime_symbol": f"__vow_{name}",
            "params": [],
            "return": "ptr",
            "doc_signature": f"fn() -> {doc_ret}",
            "effects": "[]",
        }

    def test_string_return_is_tagged_string_heap(self):
        block = go.gen_rust_ir_block([self._op_returning("String")], [])
        self.assertIn('"op" => Some(BuiltinResultTag::StringHeap),', block)

    def test_vec_return_is_tagged_vec_heap(self):
        block = go.gen_rust_ir_block([self._op_returning("Vec<u8>")], [])
        self.assertIn('"op" => Some(BuiltinResultTag::VecHeap),', block)

    def test_option_return_is_tagged_with_its_element_type(self):
        for elem in go.OPTION_ELEMS:
            block = go.gen_rust_ir_block([self._op_returning(f"Option<{elem}>")], [])
            rust_ty = go.OPTION_ELEMS[elem]["rust_ty"]
            self.assertIn(
                f'"op" => Some(BuiltinResultTag::OptionOf({rust_ty})),', block, elem
            )

    def test_scalar_and_unit_returns_are_untagged(self):
        for doc_ret in ("i64", "u64", "bool", "()"):
            block = go.gen_rust_ir_block([self._op_returning(doc_ret)], [])
            self.assertNotIn(
                "BuiltinResultTag::", block.split("fn catalogue_builtin_result_tag")[1]
            )

    def test_tag_does_not_depend_on_arena_routing(self):
        plain = self._op_returning("String")
        routed = dict(plain, arena_routing="heap_fresh")
        none = dict(plain, arena_routing="none")
        self.assertEqual(
            go.gen_rust_ir_block([plain], []), go.gen_rust_ir_block([routed], [])
        )
        self.assertEqual(
            go.gen_rust_ir_block([plain], []), go.gen_rust_ir_block([none], [])
        )


class GenCraneliftBlockTest(unittest.TestCase):
    def test_matches_expected_rustfmt_canonical_text(self):
        expected = (
            "// GENERATE:OPERATIONS:START\n"
            "fn catalogue_extern_sig(sym: &str, sig: &mut Signature) -> bool {\n"
            "    match sym {\n"
            '        "__vow_string_print" => {\n'
            "            sig.params.push(AbiParam::new(types::I64));\n"
            "            true\n"
            "        }\n"
            '        "__vow_print_i64" => {\n'
            "            sig.params.push(AbiParam::new(types::I64));\n"
            "            true\n"
            "        }\n"
            '        "__vow_print_u64" => {\n'
            "            sig.params.push(AbiParam::new(types::I64));\n"
            "            true\n"
            "        }\n"
            "        _ => false,\n"
            "    }\n"
            "}\n"
            "// GENERATE:OPERATIONS:END"
        )
        self.assertEqual(go.gen_cranelift_block(PRINT_OPS, []), expected)

    def test_non_unit_return_token_pushes_return_slot(self):
        # RETURN_TOKENS only defines "unit" today (clif_ret=None, no push).
        # Exercise the clif_ret plumbing itself so a future non-unit token
        # can't silently regress back to dropping the Cranelift return slot.
        go.RETURN_TOKENS["fake_i64"] = {
            "rust_ty": "Ty::I64",
            "ity_const": "ITY_I64()",
            "clif_ret": "types::I64",
        }
        try:
            op = dict(PRINT_OPS[1])
            op["return"] = "fake_i64"
            block = go.gen_cranelift_block([op], [])
        finally:
            del go.RETURN_TOKENS["fake_i64"]
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", block
        )

    def test_i64_and_ptr_return_tokens_push_return_slot(self):
        i64_op = dict(PRINT_OPS[1])
        i64_op["return"] = "i64"
        ptr_op = dict(PRINT_OPS[1])
        ptr_op["return"] = "ptr"
        block = go.gen_cranelift_block([i64_op, ptr_op], [])
        self.assertEqual(
            block.count(
                "sig.returns.push(AbiParam::new(types::I64));\n            true"
            ),
            2,
        )


class ReturnTokensRenderTest(unittest.TestCase):
    def _op(self, return_token: str) -> dict:
        op = dict(PRINT_OPS[1])  # single i64 param, easiest to reuse
        op["name"] = f"op_{return_token}"
        op["runtime_symbol"] = f"__vow_op_{return_token}"
        op["return"] = return_token
        return op

    def test_i64_return_token_renders(self):
        op = self._op("i64")
        self.assertIn("Ty::I64", go.gen_rust_ir_block([op], []))
        cranelift = go.gen_cranelift_block([op], [])
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", cranelift
        )
        self.assertIn("ITY_I64()", go.gen_vow_lower_block([op], []))

    def test_ptr_return_token_renders(self):
        op = self._op("ptr")
        self.assertIn("Ty::Ptr", go.gen_rust_ir_block([op], []))
        cranelift = go.gen_cranelift_block([op], [])
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", cranelift
        )
        self.assertIn("ITY_PTR()", go.gen_vow_lower_block([op], []))

    def test_bool_return_token_renders_as_i64_in_cranelift(self):
        # stdin_ready is the one operation using this token: the Vow surface
        # type is bool, but Cranelift has no dedicated bool return type, so
        # the ABI must still push types::I64 -- never a narrower I8/bool slot.
        op = self._op("bool")
        self.assertIn("Ty::Bool", go.gen_rust_ir_block([op], []))
        cranelift = go.gen_cranelift_block([op], [])
        self.assertIn(
            "sig.returns.push(AbiParam::new(types::I64));\n            true", cranelift
        )
        self.assertIn("ITY_BOOL()", go.gen_vow_lower_block([op], []))


class GenVowLowerBlockTest(unittest.TestCase):
    def test_matches_expected_text(self):
        expected = (
            "// GENERATE:OPERATIONS:START\n"
            "fn catalogue_builtin_to_extern(name: String) -> String {\n"
            '    if name == String::from("print_str") { return String::from("__vow_string_print"); }\n'
            '    if name == String::from("print_i64") { return String::from("__vow_print_i64"); }\n'
            '    if name == String::from("print_u64") { return String::from("__vow_print_u64"); }\n'
            '    return String::from("");\n'
            "}\n"
            "\n"
            "fn catalogue_builtin_ret_ty(name: String) -> i64 {\n"
            '    if name == String::from("print_str") { return ITY_UNIT(); }\n'
            '    if name == String::from("print_i64") { return ITY_UNIT(); }\n'
            '    if name == String::from("print_u64") { return ITY_UNIT(); }\n'
            "    return -1;\n"
            "}\n"
            "\n"
            "fn catalogue_builtin_result_tag(name: String) -> i64 {\n"
            "    return BRT_NONE();\n"
            "}\n"
            "// GENERATE:OPERATIONS:END"
        )
        self.assertEqual(go.gen_vow_lower_block(PRINT_OPS, []), expected)

    def test_i64_and_ptr_return_tokens_map_to_ity_const(self):
        i64_op = dict(PRINT_OPS[1])
        i64_op["return"] = "i64"
        ptr_op = dict(PRINT_OPS[1])
        ptr_op["return"] = "ptr"
        block = go.gen_vow_lower_block([i64_op, ptr_op], [])
        self.assertIn(
            f'if name == String::from("{i64_op["name"]}") {{ return ITY_I64(); }}',
            block,
        )
        self.assertIn(
            f'if name == String::from("{ptr_op["name"]}") {{ return ITY_PTR(); }}',
            block,
        )

    def _op_returning(self, doc_ret, name="op"):
        return {
            "name": name,
            "runtime_symbol": f"__vow_{name}",
            "params": [],
            "return": "ptr",
            "doc_signature": f"fn() -> {doc_ret}",
            "effects": "[]",
        }

    def test_string_and_vec_returns_are_tagged(self):
        string_block = go.gen_vow_lower_block([self._op_returning("String")], [])
        vec_block = go.gen_vow_lower_block([self._op_returning("Vec<String>")], [])
        self.assertIn(
            'if name == String::from("op") { return BRT_STRING(); }', string_block
        )
        self.assertIn('if name == String::from("op") { return BRT_VEC(); }', vec_block)

    def test_option_return_is_tagged_with_its_element_type(self):
        for elem in go.OPTION_ELEMS:
            block = go.gen_vow_lower_block([self._op_returning(f"Option<{elem}>")], [])
            ity = go.OPTION_ELEMS[elem]["ity_const"]
            self.assertIn(
                f'if name == String::from("op") {{ return {ity}; }}',
                block.split("fn catalogue_builtin_result_tag")[1],
                elem,
            )

    def test_scalar_return_is_untagged(self):
        block = go.gen_vow_lower_block([self._op_returning("i64")], [])
        self.assertEqual(
            block.split("fn catalogue_builtin_result_tag")[1].count("return BRT_"), 1
        )
        self.assertIn("return BRT_NONE();", block)


class GeneratorDeterminismTest(unittest.TestCase):
    def test_repeated_calls_are_byte_identical(self):
        for rel_path, gen_fn in go.TARGET_FILES:
            self.assertEqual(
                gen_fn(PRINT_OPS, ROUTES), gen_fn(PRINT_OPS, ROUTES), str(rel_path)
            )


class ReplaceBetweenMarkersTest(unittest.TestCase):
    def test_round_trip_preserves_surrounding_text(self):
        content = (
            "before\n"
            "// GENERATE:OPERATIONS:START\n"
            "old stuff\n"
            "// GENERATE:OPERATIONS:END\n"
            "after\n"
        )
        replacement = (
            "// GENERATE:OPERATIONS:START\nnew stuff\n// GENERATE:OPERATIONS:END"
        )
        result = go._replace_between_markers(
            content, go.MARKER_START, go.MARKER_END, replacement
        )
        self.assertEqual(
            result,
            "before\n// GENERATE:OPERATIONS:START\nnew stuff\n// GENERATE:OPERATIONS:END\nafter\n",
        )

    def test_missing_start_marker_raises(self):
        with self.assertRaises(ValueError) as ctx:
            go._replace_between_markers(
                "no markers here", go.MARKER_START, go.MARKER_END, "x"
            )
        self.assertIn(go.MARKER_START, str(ctx.exception))

    def test_missing_end_marker_raises(self):
        with self.assertRaises(ValueError) as ctx:
            go._replace_between_markers(
                f"{go.MARKER_START}\nno end", go.MARKER_START, go.MARKER_END, "x"
            )
        self.assertIn(go.MARKER_END, str(ctx.exception))

    def test_second_orphaned_marker_block_raises(self):
        # A bad merge leaving two marker pairs in one file must not silently
        # splice only the first and leave the second invisible to --check.
        content = (
            "before\n"
            "// GENERATE:OPERATIONS:START\nold 1\n// GENERATE:OPERATIONS:END\n"
            "middle\n"
            "// GENERATE:OPERATIONS:START\nold 2 (orphaned)\n// GENERATE:OPERATIONS:END\n"
            "after\n"
        )
        with self.assertRaises(ValueError) as ctx:
            go._replace_between_markers(
                content, go.MARKER_START, go.MARKER_END, "new stuff"
            )
        self.assertIn(go.MARKER_START, str(ctx.exception))


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


class SplitTableRowTest(unittest.TestCase):
    def test_strips_only_outer_pipe_cells(self):
        self.assertEqual(go._split_table_row("| foo | bar |"), ["foo", "bar"])

    def test_preserves_genuinely_empty_interior_cell(self):
        # A blank (unformatted) interior cell must survive splitting --
        # only the two empty strings produced by the row's own leading and
        # trailing "|" are dropped, never an interior column.
        self.assertEqual(go._split_table_row("| foo |  | bar |"), ["foo", "", "bar"])

    def test_escaped_pipe_is_not_a_column_separator(self):
        self.assertEqual(go._split_table_row(r"| a\|b | c |"), ["a|b", "c"])


class ExtractBuiltinSignaturesTableTest(unittest.TestCase):
    def test_sweeps_every_subsection(self):
        table = go.extract_builtin_signatures_table(GRAMMAR_FIXTURE)
        self.assertEqual(table["pin_to_root"], ("fn(value: String) -> String", "[]"))
        self.assertEqual(table["print_str"], ("fn(s: String) -> ()", "[io]"))
        self.assertEqual(table["print_i64"], ("fn(v: i64) -> ()", "[io]"))
        self.assertEqual(table["print_u64"], ("fn(v: u64) -> ()", "[io]"))

    def test_stops_at_next_level_2_heading(self):
        table = go.extract_builtin_signatures_table(GRAMMAR_FIXTURE)
        self.assertNotIn("irrelevant", " ".join(table.keys()))

    def test_real_grammar_md_contains_known_ops(self):
        grammar_text = (REPO_ROOT / "docs" / "spec" / "grammar.md").read_text()
        table = go.extract_builtin_signatures_table(grammar_text)
        for op in KNOWN_OPS:
            self.assertEqual(table[op["name"]], (op["doc_signature"], op["effects"]))

    def test_duplicate_row_across_subsections_raises(self):
        # A stale second row for the same builtin (e.g. left in another
        # subsection by a bad merge) must not be silently overwritten.
        duplicated = GRAMMAR_FIXTURE.replace(
            "#### Print / IO",
            "#### Debug\n\n"
            "| Function    | Signature                 | Effects |\n"
            "|---|---|---|\n"
            "| `print_str` | `fn(s: String) -> BOGUS`  | `[STALE]` |\n\n"
            "#### Print / IO",
        )
        with self.assertRaises(ValueError) as ctx:
            go.extract_builtin_signatures_table(duplicated)
        self.assertIn("print_str", str(ctx.exception))


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


class CheckDocFactsTest(unittest.TestCase):
    def test_clean_fixture_reports_no_mismatches(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp)
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        self.assertEqual(mismatches, [])

    def test_grammar_effects_mismatch_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp, grammar_effects="[]")
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        matching = [m for m in mismatches if "print_str" in m and "grammar.md" in m]
        self.assertTrue(matching)
        # The mismatch message must carry the *observed* (wrong) value, not
        # just the fact that something disagreed -- otherwise this test would
        # pass even if check_doc_facts never actually read the table.
        self.assertIn("[]", matching[0])

    def test_grammar_missing_row_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp)
            grammar_path = tmp / "docs" / "spec" / "grammar.md"
            lines = grammar_path.read_text().splitlines(keepends=True)
            grammar_path.write_text(
                "".join(line for line in lines if "print_str" not in line)
            )
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        self.assertIn("grammar.md: no row found for 'print_str'", mismatches)

    def test_main_vow_missing_doc_line_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp, main_vow_ok=False)
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        self.assertTrue(any("print_str" in m and "main.vow" in m for m in mismatches))

    def test_skill_rs_missing_doc_line_is_caught(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_fixture_tree(tmp, skill_rs_ok=False)
            mismatches = go.check_doc_facts(PRINT_OPS, tmp)
        self.assertTrue(any("print_str" in m and "skill.rs" in m for m in mismatches))

    def test_real_repo_has_no_mismatches(self):
        mismatches = go.check_doc_facts(KNOWN_OPS, REPO_ROOT)
        self.assertEqual(mismatches, [])


def _write_target_fixtures(tmp: Path) -> None:
    for rel, header in [
        ("vow-ir/src/lower/mod.rs", "fn vow_static_builtin_to_runtime() {}\n"),
        ("vow-codegen/src/cranelift_backend.rs", "fn make_extern_sig() {}\n"),
        ("vow-clif-shim/src/lib.rs", "fn make_extern_sig() {}\n"),
        ("compiler/lower.vow", "fn builtin_to_extern() {}\n"),
        ("vow-ir/src/region.rs", "fn infer_regions() {}\n"),
        ("compiler/ir.vow", "fn lower_to_ir() {}\n"),
        ("compiler/vc_ops.vow", "fn vc_op_model() {}\n"),
        ("vow-verify/src/c_emitter.rs", "fn is_known_builtin() {}\n"),
        ("compiler/c_emitter.vow", "fn is_known_builtin() {}\n"),
    ]:
        path = tmp / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            f"{header}// GENERATE:OPERATIONS:START\nplaceholder\n// GENERATE:OPERATIONS:END\n"
        )


class WriteAndCheckProjectionsTest(unittest.TestCase):
    def test_write_then_check_reports_clean(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, [])
            mismatches = go.check_projections(PRINT_OPS, tmp, [])
        self.assertEqual(mismatches, [])

    def test_write_actually_splices_generator_output(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, [])
            content = (tmp / "vow-ir/src/lower/mod.rs").read_text()
        self.assertIn("catalogue_builtin_to_runtime", content)
        self.assertIn("fn vow_static_builtin_to_runtime() {}", content)

    def test_hand_mutated_target_is_flagged_as_drift(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, [])
            mod_rs = tmp / "vow-ir/src/lower/mod.rs"
            mod_rs.write_text(
                mod_rs.read_text().replace("print_str", "print_str_TAMPERED")
            )
            mismatches = go.check_projections(PRINT_OPS, tmp, [])
        self.assertTrue(any("mod.rs" in m for m in mismatches))

    def test_missing_marker_in_target_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            (tmp / "compiler/lower.vow").write_text("no markers at all\n")
            with self.assertRaises(ValueError):
                go.check_projections(PRINT_OPS, tmp, [])

    def test_check_projections_reports_every_stale_target(self):
        # A single catalogue edit (e.g. hand-editing operations.json and
        # forgetting to regenerate) must invalidate every language
        # projection's check in one --check run, not just one of them.
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, [])
            edited_ops = [dict(op) for op in PRINT_OPS]
            edited_ops[0] = dict(edited_ops[0])
            edited_ops[0]["runtime_symbol"] = "__vow_string_print_v2"
            mismatches = go.check_projections(edited_ops, tmp, [])
        stale_targets = " ".join(mismatches)
        self.assertIn("mod.rs", stale_targets)
        self.assertIn("lower.vow", stale_targets)


ROUTES = ["__vow_map_get", "__vow_string_parse_i64_opt"]


class LoadArenaRoutesTest(unittest.TestCase):
    def _write(self, tmp: Path, routes, ops=None) -> None:
        spec_dir = tmp / "docs" / "spec"
        spec_dir.mkdir(parents=True, exist_ok=True)
        body = {"operations": ops or [], "arena_routes": routes}
        (spec_dir / "operations.json").write_text(json.dumps(body))

    def _errors(self, routes, ops=None) -> str:
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write(tmp, routes, ops)
            with self.assertRaises(ValueError) as ctx:
                go.load_arena_routes(tmp, ops or [])
            return str(ctx.exception)

    def test_real_catalogue_routes_load(self):
        ops = go.load_catalogue(REPO_ROOT)
        routes = go.load_arena_routes(REPO_ROOT, ops)
        self.assertIn("__vow_vec_sort", routes)
        self.assertIn("__vow_btreemap_insert", routes)
        self.assertIn("__vow_btreemap_new", routes)
        self.assertIn("__vow_i64_to_u8_try", routes)

    def test_missing_section_means_no_routes(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            spec_dir = tmp / "docs" / "spec"
            spec_dir.mkdir(parents=True)
            (spec_dir / "operations.json").write_text(json.dumps({"operations": []}))
            self.assertEqual(go.load_arena_routes(tmp, []), [])

    def test_valid_routes_load_unchanged(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write(tmp, ROUTES)
            self.assertEqual(go.load_arena_routes(tmp, []), ROUTES)

    def test_duplicate_symbol_raises(self):
        self.assertIn(
            "'__vow_map_get' appears more than once",
            self._errors([ROUTES[0], ROUTES[0]]),
        )

    def test_variant_of_another_route_cannot_also_be_a_base(self):
        self.assertIn(
            "appears more than once",
            self._errors([ROUTES[0], "__vow_map_get_in_arena"]),
        )

    def test_malformed_symbol_raises(self):
        self.assertIn("is not a symbol", self._errors(["map_get"]))

    def test_non_string_route_raises(self):
        self.assertIn("is not a symbol", self._errors([{"runtime_symbol": "x"}]))

    def test_non_list_section_raises(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            self._write(tmp, {"not": "a list"})
            with self.assertRaises(ValueError):
                go.load_arena_routes(tmp, [])


class CheckRuntimeExportsTest(unittest.TestCase):
    def _check(self, runtime_text: str) -> list[str]:
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            (tmp / "vow-runtime" / "src").mkdir(parents=True)
            (tmp / "vow-runtime" / "src" / "lib.rs").write_text(runtime_text)
            return go.check_runtime_exports(ROUTES, tmp)

    def test_all_symbols_present_is_clean(self):
        text = " ".join(sym for r in ROUTES for sym in (r, go.arena_variant(r)))
        self.assertEqual(self._check(text), [])

    def test_base_is_not_satisfied_by_its_variant(self):
        text = (
            " ".join(go.arena_variant(r) for r in ROUTES)
            + " __vow_string_parse_i64_opt"
        )
        self.assertEqual(
            self._check(text),
            ["vow-runtime/src: does not define '__vow_map_get'"],
        )

    def test_missing_variant_is_reported(self):
        text = " ".join(ROUTES)
        self.assertEqual(len(self._check(text)), 2)


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


class VowSymbolSetFnTest(unittest.TestCase):
    def _real_routes(self):
        return go.load_arena_routes(REPO_ROOT, go.load_catalogue(REPO_ROOT))

    def test_real_routes_match_exactly_their_symbols(self):
        routes = self._real_routes()
        base_fn = go._vow_symbol_set_fn("f", routes)
        for sym in routes:
            self.assertTrue(_eval_vow_symbol_set(base_fn, sym), sym)
            self.assertFalse(_eval_vow_symbol_set(base_fn, go.arena_variant(sym)), sym)

    def test_near_misses_are_rejected(self):
        routes = self._real_routes()
        base_fn = go._vow_symbol_set_fn("f", routes)
        known = set(routes)
        for sym in routes:
            near = {sym[:-1], sym + "x", ""}
            near |= {sym[:i] + "#" + sym[i + 1 :] for i in range(len(sym))}
            for candidate in near - known:
                self.assertFalse(_eval_vow_symbol_set(base_fn, candidate), candidate)

    def test_unrelated_runtime_symbols_are_rejected(self):
        base_fn = go._vow_symbol_set_fn("f", ["__vow_map_get", "__vow_vec_sort"])
        for sym in ["__vow_map_new", "__vow_vec_push", "__vow_string_print", "main"]:
            self.assertFalse(_eval_vow_symbol_set(base_fn, sym), sym)

    def test_each_symbol_is_spelled_once_and_length_is_read_first(self):
        routes = self._real_routes()
        source = go._vow_symbol_set_fn("f", routes)
        self.assertEqual(source.count("String::from("), len(routes))
        self.assertIn("let n: u64 = sym.len();", source)

    def test_empty_set_matches_nothing(self):
        fn = go._vow_symbol_set_fn("f", [])
        self.assertFalse(_eval_vow_symbol_set(fn, "__vow_map_get"))


class GenFreshRustBlocksTest(unittest.TestCase):
    def test_region_block_is_the_public_table(self):
        block = go.gen_region_block(PRINT_OPS, ROUTES)
        self.assertIn("pub const FRESH_ARENA_VARIANTS", block)
        self.assertIn('("__vow_map_get", "__vow_map_get_in_arena")', block)
        self.assertNotIn("fn ", block)

    def test_shim_block_keeps_the_table_private(self):
        block = go.gen_shim_block(PRINT_OPS, ROUTES)
        self.assertIn("fn catalogue_extern_sig(", block)
        self.assertIn("\nconst FRESH_ARENA_VARIANTS", block)
        self.assertNotIn("cfg(test)", block)
        self.assertNotIn("pub ", block)


class FreshRouteProjectionsTest(unittest.TestCase):
    def test_route_edit_makes_exactly_the_route_targets_stale(self):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            _write_target_fixtures(tmp)
            go.write_projections(PRINT_OPS, tmp, ROUTES)
            self.assertEqual(go.check_projections(PRINT_OPS, tmp, ROUTES), [])
            mismatches = go.check_projections(PRINT_OPS, tmp, ROUTES[:1])
        stale = " ".join(mismatches)
        self.assertIn("region.rs", stale)
        self.assertIn("ir.vow", stale)
        self.assertIn("vow-clif-shim", stale)
        self.assertNotIn("lower.vow", stale)
        self.assertNotIn("cranelift_backend.rs", stale)
        self.assertNotIn("vc_ops.vow", stale)


class VerifierModelDriftTest(unittest.TestCase):
    def test_entry_without_verifier_model_is_reported_by_name(self):
        ops = [dict(PRINT_OPS[0]), dict(PRINT_OPS[1], verifier_model="unmodeled")]
        problems = go.check_verifier_models(ops)
        self.assertEqual(len(problems), 1)
        self.assertIn(PRINT_OPS[0]["name"], problems[0])

    def test_explicit_unmodeled_and_known_are_clean(self):
        ops = [
            dict(PRINT_OPS[0], verifier_model="unmodeled"),
            dict(PRINT_OPS[1], verifier_model="known"),
        ]
        self.assertEqual(go.check_verifier_models(ops), [])

    def test_real_catalogue_has_a_decision_for_every_builtin(self):
        self.assertEqual(go.check_verifier_models(go.load_catalogue(REPO_ROOT)), [])


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


class ArenaRoutingInvariantsTest(unittest.TestCase):
    def _problems(self, op, routes):
        return go.check_arena_routing([op], routes)

    def test_heap_result_without_arena_routing_is_reported_by_name(self):
        for ret in ("String", "Vec<i64>", "Option<u8>"):
            problems = self._problems(_heap_op(ret=ret), [])
            self.assertEqual(len(problems), 1, ret)
            self.assertIn("'heap_op' returns a heap value", problems[0])

    def test_scalar_result_may_omit_arena_routing(self):
        scalar = dict(_heap_op(), doc_signature="fn() -> i64", **{"return": "i64"})
        self.assertEqual(self._problems(scalar, []), [])

    def test_route_without_heap_fresh_is_reported(self):
        problems = self._problems(_heap_op(), ["__vow_heap_op"])
        self.assertTrue(
            any("has an arena route" in p and "'heap_op'" in p for p in problems),
            problems,
        )

    def test_heap_fresh_without_route_is_reported(self):
        problems = self._problems(_heap_op(arena_routing="heap_fresh"), [])
        self.assertEqual(len(problems), 1)
        self.assertIn("'heap_op' is heap_fresh", problems[0])
        self.assertIn("has no arena route", problems[0])

    def test_none_with_route_is_reported(self):
        problems = self._problems(_heap_op(arena_routing="none"), ["__vow_heap_op"])
        self.assertEqual(len(problems), 1)
        self.assertIn("'heap_op' is arena_routing 'none'", problems[0])

    def test_consistent_declarations_are_clean(self):
        self.assertEqual(
            self._problems(_heap_op(arena_routing="heap_fresh"), ["__vow_heap_op"]),
            [],
        )
        self.assertEqual(self._problems(_heap_op(arena_routing="none"), []), [])

    def test_none_on_a_heap_result_needs_a_reason(self):
        op = _heap_op(arena_routing="none")
        del op["arena_routing_reason"]
        problems = self._problems(op, [])
        self.assertEqual(len(problems), 1)
        self.assertIn("'heap_op' is arena_routing 'none'", problems[0])
        self.assertIn("arena_routing_reason", problems[0])

    def test_none_on_a_scalar_result_needs_no_reason(self):
        scalar = dict(_heap_op(), doc_signature="fn() -> i64", **{"return": "i64"})
        scalar["arena_routing"] = "none"
        self.assertEqual(self._problems(scalar, []), [])

    def test_reason_is_only_allowed_on_none(self):
        op = _heap_op(arena_routing="heap_fresh", arena_routing_reason="why")
        problems = self._problems(op, ["__vow_heap_op"])
        self.assertEqual(len(problems), 1)
        self.assertIn("is not arena_routing 'none'", problems[0])

    def test_blank_or_non_string_reason_is_reported(self):
        for reason in ("  ", 7):
            op = _heap_op(arena_routing="none", arena_routing_reason=reason)
            problems = self._problems(op, [])
            self.assertEqual(len(problems), 1, reason)
            self.assertIn("empty or non-string", problems[0])

    def test_real_catalogue_reasons_sit_only_on_heap_nones(self):
        for op in go.load_catalogue(REPO_ROOT):
            has_reason = "arena_routing_reason" in op
            self.assertEqual(has_reason, op.get("arena_routing") == "none", op["name"])

    def test_route_without_a_catalogued_op_stays_legal(self):
        self.assertEqual(go.check_arena_routing([], ["__vow_map_get"]), [])

    def test_every_violation_across_the_catalogue_is_reported(self):
        ops = [
            _heap_op("a"),
            _heap_op("b", arena_routing="heap_fresh"),
            _heap_op("c", arena_routing="none"),
        ]
        self.assertEqual(len(go.check_arena_routing(ops, ["__vow_c"])), 3)

    def test_real_catalogue_declares_routing_exactly_where_a_route_exists(self):
        catalogue = go.load_catalogue(REPO_ROOT)
        routes = go.load_arena_routes(REPO_ROOT, catalogue)
        self.assertEqual(go.check_arena_routing(catalogue, routes), [])
        for op in catalogue:
            routed = op["runtime_symbol"] in routes
            self.assertEqual(
                op.get("arena_routing") == "heap_fresh", routed, op["name"]
            )
            if go.result_shape(op) is not None:
                self.assertIn("arena_routing", op, op["name"])

    def test_unrouted_heap_returns_are_explicit_nones(self):
        ops = {o["name"]: o for o in go.load_catalogue(REPO_ROOT)}
        for name in ("stdin_read_line", "proc_sample"):
            self.assertEqual(ops[name]["arena_routing"], "none", name)
        for name in ("fs_read", "fs_read_line", "fs_listdir", "args", "stdin_read"):
            self.assertEqual(ops[name]["arena_routing"], "heap_fresh", name)


class GenVowVcOpsBlockTest(unittest.TestCase):
    def test_only_known_symbols_land_in_the_lookup(self):
        ops = [
            dict(PRINT_OPS[0], verifier_model="known"),
            dict(PRINT_OPS[1], verifier_model="unmodeled"),
        ]
        block = go.gen_vow_vc_ops_block(ops, [])
        self.assertIn(PRINT_OPS[0]["runtime_symbol"], block)
        self.assertNotIn(PRINT_OPS[1]["runtime_symbol"], block)
        self.assertNotIn("catalogue_verifier_unmodeled", block)

    def test_no_known_symbols_still_yields_a_total_lookup(self):
        block = go.gen_vow_vc_ops_block(
            [dict(PRINT_OPS[0], verifier_model="unmodeled")], []
        )
        self.assertIn("fn catalogue_verifier_known(sym: String) -> bool", block)
        self.assertIn("return false;", block)
        self.assertNotIn("let n", block)


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


class ResultShapeTest(unittest.TestCase):
    def _op(self, doc_ret):
        return {"doc_signature": f"fn() -> {doc_ret}"}

    def test_shapes_follow_the_doc_return_type(self):
        self.assertEqual(go.result_shape(self._op("String")), ("string", None))
        self.assertEqual(go.result_shape(self._op("Vec<String>")), ("vec", None))
        self.assertEqual(go.result_shape(self._op("Option<u8>")), ("option", "u8"))
        for scalar in ("i64", "u64", "bool", "()", "!"):
            self.assertIsNone(go.result_shape(self._op(scalar)), scalar)

    def test_unknown_option_element_is_rejected_by_name(self):
        op = {
            "name": "bad",
            "runtime_symbol": "__vow_bad",
            "params": ["ptr"],
            "return": "ptr",
            "doc_signature": "fn(s: String) -> Option<f64>",
            "effects": "[]",
        }
        with tempfile.TemporaryDirectory() as d:
            _write_catalogue(Path(d), [op])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(Path(d))
        self.assertIn("Option element type 'f64'", str(ctx.exception))

    def test_wide_params_and_option_returns_are_valid_doc_types(self):
        self.assertEqual(go._doc_type_to_token("i128"), "i128")
        self.assertEqual(go._doc_type_to_token("u128"), "u128")
        self.assertEqual(go._doc_type_to_token("Option<i64>"), "ptr")
        self.assertEqual(go.PARAM_TOKENS["i128"], "types::I128")
        self.assertEqual(go.PARAM_TOKENS["u128"], "types::I128")

    def test_wide_param_reaches_the_cranelift_signature(self):
        op = next(o for o in QUERY_UTILITY_OPS if o["name"] == "int128_to_string")
        block = go.gen_cranelift_block([op], [])
        self.assertIn("sig.params.push(AbiParam::new(types::I128));", block)
        self.assertIn("sig.returns.push(AbiParam::new(types::I64));", block)

    def test_real_catalogue_tags_match_each_ops_runtime_shape(self):
        shapes = {o["name"]: go.result_shape(o) for o in go.load_catalogue(REPO_ROOT)}
        for name in ("hex_decode", "string_split", "vec_sort", "args", "fs_listdir"):
            self.assertEqual(shapes[name], ("vec", None), name)
        for name in (
            "hex_encode",
            "format_f64_bits",
            "proc_sample",
            "int_to_string",
            "uint_to_string",
            "int128_to_string",
            "uint128_to_string",
            "string_substr",
            "string_trim",
            "string_to_upper",
            "string_to_lower",
            "string_replace",
            "string_join",
            "fs_read",
            "stdin_read_line",
        ):
            self.assertEqual(shapes[name], ("string", None), name)
        for name in (
            "parse_i64",
            "parse_i8",
            "parse_i16",
            "parse_i32",
            "parse_i128",
            "parse_u8",
            "parse_u16",
            "parse_u32",
            "parse_u128",
        ):
            self.assertEqual(
                shapes[name], ("option", name.removeprefix("parse_")), name
            )
        for name in (
            "parse_f64_bits",
            "string_starts_with",
            "string_ends_with",
            "time_micros",
            "num_cpus",
            "gzip_write_file",
            "memory_peak_bytes",
        ):
            self.assertIsNone(shapes[name], name)


class QueryUtilityCatalogueTest(unittest.TestCase):
    def setUp(self):
        self.catalogue = go.load_catalogue(REPO_ROOT)
        self.ops = {o["name"]: o for o in self.catalogue}

    def test_every_query_and_utility_builtin_is_catalogued(self):
        for row in _QUERY_UTILITY_ROWS:
            op = self.ops[row[0]]
            self.assertEqual(op["runtime_symbol"], row[1], row[0])
            self.assertEqual(op["return"], row[3], row[0])

    def test_hand_written_survivors_are_not_catalogued(self):
        # `i64_to_string` shares its runtime symbol with `int_to_string`, the
        # literal-bytes lowering of `string_matches_literal_at` is a special
        # operation body, and there is no free `parse_u64`.
        for name in ("i64_to_string", "string_matches_literal_at", "parse_u64"):
            self.assertNotIn(name, self.ops)

    def test_native_verifier_models_none_of_them(self):
        for row in _QUERY_UTILITY_ROWS:
            self.assertEqual(self.ops[row[0]]["verifier_model"], "unmodeled", row[0])
        block = go.gen_vow_vc_ops_block(self.catalogue, [])
        self.assertIn("return false;", block)

    def test_c_model_symbols_equal_the_pre_migration_hand_list(self):
        routes = go.load_arena_routes(REPO_ROOT, self.catalogue)
        query_symbols = {r[1] for r in _QUERY_UTILITY_ROWS}
        query_symbols |= {go.arena_variant(r[1]) for r in _QUERY_UTILITY_ROWS}
        catalogued = {
            sym for sym in go.c_model_symbols(self.catalogue) if sym in query_symbols
        }
        self.assertEqual(catalogued, PRE_MIGRATION_C_MODEL_SYMBOLS)
        self.assertEqual(go.check_c_models(self.catalogue, routes), [])

    def test_effectful_query_ops_have_no_c_model(self):
        for name in ("time_unix", "num_cpus", "proc_sample", "gzip_write_file"):
            self.assertNotIn("c_model", self.ops[name], name)


class CModelTest(unittest.TestCase):
    def _op(self, name, model, effects="[]"):
        op = {
            "name": name,
            "runtime_symbol": f"__vow_{name}",
            "params": [],
            "return": "i64",
            "doc_signature": "fn() -> i64",
            "effects": effects,
        }
        if model is not None:
            op["c_model"] = model
        return op

    def test_symbols_are_base_and_optionally_arena_variant(self):
        ops = [
            self._op("a", "base"),
            self._op("b", "base_and_arena"),
            self._op("c", "unmodeled"),
            self._op("d", None),
        ]
        self.assertEqual(
            go.c_model_symbols(ops), ["__vow_a", "__vow_b", "__vow_b_in_arena"]
        )

    def test_rust_block_is_a_rustfmt_shaped_matches(self):
        ops = [self._op("a", "base"), self._op("b", "base_and_arena")]
        self.assertEqual(
            go.gen_rust_c_model_block(ops, ["__vow_b"]),
            "// GENERATE:OPERATIONS:START\n"
            "fn catalogue_c_model_known(name: &str) -> bool {\n"
            "    matches!(\n"
            "        name,\n"
            '        "__vow_a"\n'
            '            | "__vow_b"\n'
            '            | "__vow_b_in_arena"\n'
            "    )\n"
            "}\n"
            "// GENERATE:OPERATIONS:END",
        )

    def test_rust_block_with_one_symbol_stays_on_one_line(self):
        block = go.gen_rust_c_model_block([self._op("a", "base")], [])
        self.assertIn('    matches!(name, "__vow_a")\n', block)

    def test_vow_block_answers_exactly_the_c_model_symbols(self):
        ops = [self._op("a", "base"), self._op("b", "base_and_arena")]
        block = go.gen_vow_c_model_block(ops, ["__vow_b"])
        fn = block.split("\n", 1)[1].rsplit("\n", 1)[0]
        for sym in ("__vow_a", "__vow_b", "__vow_b_in_arena"):
            self.assertTrue(_eval_vow_symbol_set(fn, sym), sym)
        for sym in ("__vow_a_in_arena", "__vow_c", ""):
            self.assertFalse(_eval_vow_symbol_set(fn, sym), sym)

    def test_pure_builtin_without_c_model_is_reported_by_name(self):
        problems = go.check_c_models([self._op("a", None)], [])
        self.assertEqual(len(problems), 1)
        self.assertIn("'a'", problems[0])

    def test_effectful_builtin_without_c_model_is_clean(self):
        self.assertEqual(go.check_c_models([self._op("a", None, "[io]")], []), [])

    def test_effectful_builtin_cannot_be_known_to_the_c_model(self):
        problems = go.check_c_models([self._op("a", "base", "[io]")], [])
        self.assertEqual(len(problems), 1)
        self.assertIn("only knows pure builtins", problems[0])

    def test_arena_variant_needs_a_route(self):
        problems = go.check_c_models([self._op("a", "base_and_arena")], [])
        self.assertEqual(len(problems), 1)
        self.assertIn("no arena route", problems[0])
        self.assertEqual(
            go.check_c_models([self._op("a", "base_and_arena")], ["__vow_a"]), []
        )

    def test_unknown_c_model_value_is_rejected_by_the_loader(self):
        with tempfile.TemporaryDirectory() as d:
            _write_catalogue(Path(d), [dict(PRINT_OPS[0], c_model="maybe")])
            with self.assertRaises(ValueError) as ctx:
                go.load_catalogue(Path(d))
        self.assertIn("c_model", str(ctx.exception))

    def test_c_model_projection_targets_are_in_the_target_list(self):
        targets = {str(path) for path, _ in go.TARGET_FILES}
        self.assertIn("vow-verify/src/c_emitter.rs", targets)
        self.assertIn("compiler/c_emitter.vow", targets)

    def test_real_catalogue_has_a_decision_for_every_pure_builtin(self):
        ops = go.load_catalogue(REPO_ROOT)
        routes = go.load_arena_routes(REPO_ROOT, ops)
        self.assertEqual(go.check_c_models(ops, routes), [])


if __name__ == "__main__":
    unittest.main()
