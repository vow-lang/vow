#!/usr/bin/env python3
"""Reject a JSON document that violates one of the `docs/spec/schemas` files.

The parity gate runs under bare `python3` with no dependency install, so
`jsonschema` is not available to it. This covers exactly the nine keywords
those schemas actually use — `$ref`, `additionalProperties`, `enum`, `items`,
`minimum`, `oneOf`, `properties`, `required`, `type` — which is enough to
reject a document whose types or enums have drifted from the contract, and
small enough to audit in one sitting. A keyword outside that set is ignored
rather than guessed at, so this narrows a document's accepted shape and never
widens it.
"""

import json
from pathlib import Path

_PYTHON_TYPES = {
    "object": dict,
    "array": list,
    "string": str,
    "null": type(None),
}
_JSON_TYPE_NAMES = {
    dict: "object",
    list: "array",
    str: "string",
    bool: "boolean",
    int: "integer",
    float: "number",
    type(None): "null",
}


def load(path):
    """A schema document paired with the directory its `$ref`s resolve against."""
    path = Path(path)
    return json.loads(path.read_text()), path.parent


def validate(document, schema, schema_dir):
    """Every way `document` violates `schema`, as human-readable strings.

    Args:
        document: The parsed JSON value to check.
        schema: The parsed schema document to check it against.
        schema_dir: Directory sibling `$ref` file names resolve against.

    Returns:
        list: One message per violation, empty when the document conforms.
    """
    return _check(document, schema, schema, schema_dir, "")


def _check(value, schema, root, schema_dir, path):
    if "$ref" in schema:
        schema, root, schema_dir = _resolve(schema["$ref"], root, schema_dir)

    declared = schema.get("type")
    if declared is not None and not _has_type(value, declared):
        return [f"{path or 'document'} is {_type_name(value)}, expected {declared}"]

    errors = []
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{path} is {value!r}, not one of {schema['enum']}")
    if "minimum" in schema and _has_type(value, "number") and value < schema["minimum"]:
        errors.append(f"{path} is {value}, below the minimum {schema['minimum']}")
    if "oneOf" in schema and not any(
        not _check(value, option, root, schema_dir, path) for option in schema["oneOf"]
    ):
        errors.append(f"{path} matches none of its {len(schema['oneOf'])} shapes")

    if isinstance(value, dict):
        errors += _check_object(value, schema, root, schema_dir, path)
    elif isinstance(value, list) and "items" in schema:
        for index, item in enumerate(value):
            errors += _check(
                item, schema["items"], root, schema_dir, f"{path}[{index}]"
            )
    return errors


def _check_object(value, schema, root, schema_dir, path):
    errors = [
        f"{_join(path, field)} is missing"
        for field in schema.get("required", [])
        if field not in value
    ]
    properties = schema.get("properties", {})
    extra = schema.get("additionalProperties")
    for field, member in value.items():
        if field in properties:
            errors += _check(
                member, properties[field], root, schema_dir, _join(path, field)
            )
        elif extra is False:
            errors.append(f"{_join(path, field)} is not in the schema")
        elif isinstance(extra, dict):
            errors += _check(member, extra, root, schema_dir, _join(path, field))
    return errors


def _resolve(ref, root, schema_dir):
    """A `$ref` target, plus the root and directory it resolves further refs against."""
    if ref.startswith("#/"):
        node = root
        for part in ref[2:].split("/"):
            node = node[part]
        return node, root, schema_dir
    referenced, referenced_dir = load(schema_dir / ref)
    return referenced, referenced, referenced_dir


def _has_type(value, declared):
    names = declared if isinstance(declared, list) else [declared]
    return any(_is_type(value, name) for name in names)


def _is_type(value, name):
    # `bool` subclasses `int` in Python, so the numeric types must exclude it
    # explicitly or `true` would satisfy a schema asking for an integer.
    if name == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if name == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if name == "boolean":
        return isinstance(value, bool)
    return isinstance(value, _PYTHON_TYPES[name])


def _type_name(value):
    return _JSON_TYPE_NAMES.get(type(value), type(value).__name__)


def _join(path, field):
    return f"{path}.{field}" if path else field
