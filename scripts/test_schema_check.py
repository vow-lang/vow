#!/usr/bin/env python3
"""Behavior tests for the stdlib-only schema checker."""

import json
import tempfile
import unittest
from pathlib import Path

import schema_check

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA_DIR = REPO_ROOT / "docs/spec/schemas"


def check(document, schema, schema_dir=SCHEMA_DIR):
    return schema_check.validate(document, schema, schema_dir)


class TypeCheckingTest(unittest.TestCase):
    def test_a_conforming_document_yields_no_errors(self):
        schema = {"type": "object", "properties": {"n": {"type": "integer"}}}

        self.assertEqual([], check({"n": 1}, schema))

    def test_a_wrong_type_names_the_path_and_both_types(self):
        schema = {"type": "object", "properties": {"n": {"type": "integer"}}}

        self.assertEqual(["n is string, expected integer"], check({"n": "1"}, schema))

    def test_a_union_type_accepts_either_member(self):
        schema = {"properties": {"code": {"type": ["integer", "null"]}}}

        self.assertEqual([], check({"code": None}, schema))
        self.assertEqual([], check({"code": 0}, schema))
        self.assertEqual(1, len(check({"code": "0"}, schema)))

    def test_a_boolean_does_not_satisfy_integer(self):
        # `bool` subclasses `int` in Python, so a naive isinstance check would
        # accept `true` wherever the schema asks for a count.
        self.assertEqual(
            ["n is boolean, expected integer"],
            check({"n": True}, {"properties": {"n": {"type": "integer"}}}),
        )

    def test_an_integer_satisfies_number(self):
        self.assertEqual([], check({"n": 3}, {"properties": {"n": {"type": "number"}}}))

    def test_a_type_error_suppresses_the_checks_that_assume_it(self):
        # Reporting "not one of [...]" as well would be noise; the value cannot
        # be an enum member if it is not even the right type.
        schema = {"properties": {"s": {"type": "string", "enum": ["a"]}}}

        self.assertEqual(["s is integer, expected string"], check({"s": 1}, schema))


class ObjectCheckingTest(unittest.TestCase):
    def test_a_missing_required_member_is_reported(self):
        schema = {"type": "object", "required": ["a", "b"], "properties": {}}

        self.assertEqual(["b is missing"], check({"a": 1}, schema))

    def test_an_undeclared_member_is_reported_when_additional_are_closed(self):
        schema = {"properties": {"a": {}}, "additionalProperties": False}

        self.assertEqual(["b is not in the schema"], check({"a": 1, "b": 2}, schema))

    def test_an_undeclared_member_is_checked_against_the_additional_schema(self):
        schema = {"additionalProperties": {"type": "string"}}

        self.assertEqual(["x is integer, expected string"], check({"x": 1}, schema))

    def test_nested_paths_are_dotted(self):
        schema = {
            "properties": {"outer": {"properties": {"inner": {"type": "string"}}}}
        }

        self.assertEqual(
            ["outer.inner is integer, expected string"],
            check({"outer": {"inner": 1}}, schema),
        )


class ArrayCheckingTest(unittest.TestCase):
    def test_every_item_is_checked_and_indexed(self):
        schema = {"properties": {"xs": {"type": "array", "items": {"type": "string"}}}}

        self.assertEqual(
            ["xs[1] is integer, expected string"], check({"xs": ["a", 2]}, schema)
        )


class KeywordTest(unittest.TestCase):
    def test_an_out_of_enum_value_is_reported(self):
        schema = {"properties": {"s": {"enum": ["a", "b"]}}}

        self.assertEqual(["s is 'c', not one of ['a', 'b']"], check({"s": "c"}, schema))

    def test_a_value_below_the_minimum_is_reported(self):
        schema = {"properties": {"n": {"type": "integer", "minimum": 0}}}

        self.assertEqual(["n is -1, below the minimum 0"], check({"n": -1}, schema))

    def test_one_of_accepts_any_matching_shape(self):
        schema = {
            "properties": {
                "source": {"oneOf": [{"type": "string"}, {"type": "object"}]}
            }
        }

        self.assertEqual([], check({"source": "a.vow"}, schema))
        self.assertEqual([], check({"source": {}}, schema))
        self.assertEqual(
            ["source matches none of its 2 shapes"], check({"source": 1}, schema)
        )


class ReferenceTest(unittest.TestCase):
    def test_a_local_ref_is_resolved(self):
        schema = {
            "properties": {"a": {"$ref": "#/$defs/Name"}},
            "$defs": {"Name": {"type": "string"}},
        }

        self.assertEqual(["a is integer, expected string"], check({"a": 1}, schema))

    def test_a_sibling_file_ref_is_resolved_against_the_schema_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "name.schema.json").write_text(json.dumps({"type": "string"}))
            schema = {"properties": {"a": {"$ref": "name.schema.json"}}}

            self.assertEqual(
                ["a is integer, expected string"], check({"a": 1}, schema, root)
            )

    def test_load_returns_the_directory_refs_resolve_against(self):
        schema, directory = schema_check.load(SCHEMA_DIR / "test-result.schema.json")

        self.assertEqual(SCHEMA_DIR, directory)
        self.assertEqual("TestResult", schema["title"])


class UnsupportedKeywordTest(unittest.TestCase):
    def test_an_unsupported_keyword_never_rejects_a_document(self):
        # Guessing at a keyword this checker does not implement would make the
        # blocking gate fail for a reason the schema never expressed.
        schema = {"properties": {"s": {"type": "string", "pattern": "^z"}}}

        self.assertEqual([], check({"s": "a"}, schema))


class PublishedSchemaTest(unittest.TestCase):
    def test_the_vow_test_schema_accepts_its_own_documented_example(self):
        # The example in docs/spec/cli.md is the contract both compilers are
        # written against, so the checker must not reject it.
        schema, directory = schema_check.load(SCHEMA_DIR / "test-result.schema.json")
        example = {
            "status": "TestsPassed",
            "total": 1,
            "passed": 1,
            "failed": 0,
            "skipped": 0,
            "tests": [
                {
                    "file": "compiler/test_arith.vow",
                    "name": "test_arith",
                    "status": "passed",
                    "exit_code": 0,
                    "stdout": "7",
                    "stderr": "",
                    "duration_ms": 72,
                    "diagnostics": [],
                    "counterexamples": [],
                }
            ],
            "contract_density": {
                "functions_total": 1,
                "functions_with_vows": 0,
                "density_pct": 0.0,
            },
        }

        self.assertEqual([], schema_check.validate(example, schema, directory))


if __name__ == "__main__":
    unittest.main()
