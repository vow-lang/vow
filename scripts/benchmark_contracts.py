"""Structural fidelity checks shared by the benchmark runners.

The runners deliberately compare only the immutable parts of a benchmark
skeleton: its module name, each top-level (or extern-block) function's
signature and top-level vow clauses, and any nested loop-invariant vow
clauses inside its body. Nested clauses are compared as a per-function
multiset, so a preserved invariant may move to a different loop or pick up
siblings; only dropping or weakening one is rejected. Ordinary body code and
additional helper functions remain free for a model to implement.
"""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass


@dataclass(frozen=True)
class FidelityResult:
    matches: bool
    message: str


@dataclass(frozen=True)
class _FunctionShape:
    signature: tuple[str, ...]
    clauses: tuple[tuple[str, tuple[str, ...]], ...]
    nested_clauses: tuple[tuple[str, tuple[str, ...]], ...]


@dataclass(frozen=True)
class _ProgramShape:
    module: str
    functions: dict[str, _FunctionShape]


class _StructureError(ValueError):
    pass


def compare_skeleton(skeleton: str, candidate: str) -> FidelityResult:
    """Return whether ``candidate`` preserves the skeleton's specification."""
    try:
        expected = _parse_program(skeleton)
    except _StructureError as error:
        return FidelityResult(False, f"invalid benchmark skeleton: {error}")

    try:
        actual = _parse_program(candidate)
    except _StructureError as error:
        return FidelityResult(False, f"response structure could not be read: {error}")

    if actual.module != expected.module:
        return FidelityResult(
            False,
            f"module changed: expected `{expected.module}`, found `{actual.module}`",
        )

    for name, expected_function in expected.functions.items():
        actual_function = actual.functions.get(name)
        if actual_function is None:
            return FidelityResult(False, f"skeleton function `{name}` is missing")
        if actual_function.signature != expected_function.signature:
            return FidelityResult(False, f"signature of `{name}` changed")
        # requires/ensures/invariant clauses are combined with logical AND, so
        # a harmless reorder must not be flagged as a weakened contract.
        if sorted(actual_function.clauses) != sorted(expected_function.clauses):
            return FidelityResult(False, f"contracts of `{name}` changed")
        if not _is_multiset_subset(
            expected_function.nested_clauses, actual_function.nested_clauses
        ):
            return FidelityResult(False, f"nested contracts of `{name}` changed")

    return FidelityResult(True, "")


def _is_multiset_subset(
    expected: tuple[tuple[str, tuple[str, ...]], ...],
    actual: tuple[tuple[str, tuple[str, ...]], ...],
) -> bool:
    actual_counts = Counter(actual)
    return all(
        count <= actual_counts[clause] for clause, count in Counter(expected).items()
    )


def _parse_program(source: str) -> _ProgramShape:
    tokens = _tokenize(source)
    if len(tokens) < 2 or tokens[0] != "module" or not _is_identifier(tokens[1]):
        raise _StructureError("expected `module <Name>`")

    functions: dict[str, _FunctionShape] = {}
    index = 2
    while index < len(tokens):
        if tokens[index] == "extern":
            extern_functions, index = _parse_extern_block(tokens, index)
            for name, shape in extern_functions:
                if name in functions:
                    raise _StructureError(f"duplicate top-level function `{name}`")
                functions[name] = shape
            continue

        function_start = index
        function_token = index
        if tokens[index] == "pub" and _token_at(tokens, index + 1) == "fn":
            function_token = index + 1
        elif tokens[index] != "fn":
            if tokens[index] == "{":
                index = _matching_delimiter(tokens, index) + 1
            else:
                index += 1
            continue

        name, shape, index = _parse_function(tokens, function_start, function_token)
        if name in functions:
            raise _StructureError(f"duplicate top-level function `{name}`")
        functions[name] = shape

    return _ProgramShape(tokens[1], functions)


def _parse_extern_block(
    tokens: list[str], start: int
) -> tuple[list[tuple[str, _FunctionShape]], int]:
    """Parse ``extern "C" { [vow { ... }] fn ...; fn ...; }``.

    A single outer brace pair holds an optional shared vow clause block
    followed by the function declarations. All declarations in one extern
    block share that one vow clause block, so each declared function's
    effective clauses are that shared set — if the shared contract changes,
    every function in the block is treated as changed.
    """
    index = start + 1
    if _token_at(tokens, index).startswith('"'):
        index += 1
    if _token_at(tokens, index) != "{":
        raise _StructureError('expected `{` after `extern "C"`')
    block_end = _matching_delimiter(tokens, index)
    index += 1

    clauses: tuple[tuple[str, tuple[str, ...]], ...] = ()
    if _token_at(tokens, index) == "vow":
        vow_start = index + 1
        if _token_at(tokens, vow_start) != "{":
            raise _StructureError("expected vow block in extern block")
        vow_end = _matching_delimiter(tokens, vow_start)
        clauses = _parse_clauses(tokens[vow_start + 1 : vow_end], "extern block")
        index = vow_end + 1

    functions: list[tuple[str, _FunctionShape]] = []
    while index < block_end:
        if tokens[index] != "fn":
            raise _StructureError("expected `fn` in extern block")
        fn_start = index
        name = _token_at(tokens, index + 1)
        if not _is_identifier(name):
            raise _StructureError("expected a function name after `fn`")
        params_start = index + 2
        if _token_at(tokens, params_start) != "(":
            raise _StructureError(f"expected parameter list for `{name}`")
        decl_end = _matching_delimiter(tokens, params_start) + 1
        while decl_end < block_end and tokens[decl_end] != ";":
            decl_end += 1
        if decl_end >= block_end:
            raise _StructureError(f"expected `;` after extern declaration of `{name}`")
        functions.append(
            (name, _FunctionShape(tuple(tokens[fn_start:decl_end]), clauses, ()))
        )
        index = decl_end + 1

    return functions, block_end + 1


def _parse_function(
    tokens: list[str], start: int, function_token: int
) -> tuple[str, _FunctionShape, int]:
    name = _token_at(tokens, function_token + 1)
    if not _is_identifier(name):
        raise _StructureError("expected a function name after `fn`")

    params_start = function_token + 2
    if _token_at(tokens, params_start) != "(":
        raise _StructureError(f"expected parameter list for `{name}`")
    params_end = _matching_delimiter(tokens, params_start)

    index = params_end + 1
    signature_end = -1
    clauses: tuple[tuple[str, tuple[str, ...]], ...] = ()
    body_start: int | None = None
    while index < len(tokens):
        token = tokens[index]
        if token == "vow":
            signature_end = index
            vow_start = index + 1
            if _token_at(tokens, vow_start) != "{":
                raise _StructureError(f"expected vow block for `{name}`")
            vow_end = _matching_delimiter(tokens, vow_start)
            clauses = _parse_clauses(tokens[vow_start + 1 : vow_end], name)
            body_start = vow_end + 1
            break
        if token == "{":
            signature_end = index
            body_start = index
            break
        if token == ";":
            signature_end = index
            return (
                name,
                _FunctionShape(tuple(tokens[start:signature_end]), clauses, ()),
                index + 1,
            )
        index += 1

    if signature_end < 0 or body_start is None:
        raise _StructureError(f"function `{name}` has no body")
    if _token_at(tokens, body_start) != "{":
        raise _StructureError(f"expected body for `{name}`")

    body_end = _matching_delimiter(tokens, body_start)
    nested_clauses = _collect_nested_clauses(tokens, body_start, body_end)
    return (
        name,
        _FunctionShape(tuple(tokens[start:signature_end]), clauses, nested_clauses),
        body_end + 1,
    )


def _collect_nested_clauses(
    tokens: list[str], body_start: int, body_end: int
) -> tuple[tuple[str, tuple[str, ...]], ...]:
    """Flatten every ``<stmt> vow { ... }`` block inside a function body.

    Only loops carry a nested vow block (loop invariants), and ``vow`` is a
    reserved keyword, so any ``vow`` token in body range starts one.
    """
    clauses: list[tuple[str, tuple[str, ...]]] = []
    index = body_start
    while index < body_end:
        if tokens[index] == "vow" and _token_at(tokens, index + 1) == "{":
            vow_end = _matching_delimiter(tokens, index + 1)
            clauses.extend(
                _parse_clauses(tokens[index + 2 : vow_end], "nested vow block")
            )
            index = vow_end + 1
            continue
        index += 1
    return tuple(clauses)


def _parse_clauses(
    tokens: list[str], function_name: str
) -> tuple[tuple[str, tuple[str, ...]], ...]:
    clauses: list[tuple[str, tuple[str, ...]]] = []
    clause_kinds = {"requires", "ensures", "invariant"}
    index = 0

    while index < len(tokens):
        kind = tokens[index]
        if kind not in clause_kinds:
            raise _StructureError(
                f"expected a contract clause in `{function_name}`, found `{kind}`"
            )
        if _token_at(tokens, index + 1) != ":":
            raise _StructureError(f"expected `:` after `{kind}` in `{function_name}`")
        index += 2

        expression: list[str] = []
        delimiters: list[str] = []
        while index < len(tokens):
            token = tokens[index]
            if not delimiters and token == ",":
                break
            if not delimiters and expression and token in clause_kinds:
                break
            if token in _OPEN_TO_CLOSE:
                delimiters.append(token)
            elif token in _CLOSE_TO_OPEN:
                if not delimiters or delimiters[-1] != _CLOSE_TO_OPEN[token]:
                    raise _StructureError(
                        f"unbalanced `{token}` in contracts of `{function_name}`"
                    )
                delimiters.pop()
            expression.append(token)
            index += 1

        if delimiters:
            raise _StructureError(
                f"unclosed delimiter in contracts of `{function_name}`"
            )
        if not expression:
            raise _StructureError(f"empty `{kind}` clause in `{function_name}`")
        clauses.append((kind, tuple(expression)))

        if index < len(tokens) and tokens[index] == ",":
            index += 1

    return tuple(clauses)


def _tokenize(source: str) -> list[str]:
    tokens: list[str] = []
    index = 0
    while index < len(source):
        char = source[index]
        if char.isspace():
            index += 1
            continue
        if char == "/" and index + 1 < len(source) and source[index + 1] == "/":
            newline = source.find("\n", index + 2)
            index = len(source) if newline < 0 else newline + 1
            continue
        if char == '"':
            end = index + 1
            while end < len(source):
                if source[end] == "\\":
                    end += 2
                    continue
                if source[end] == '"':
                    end += 1
                    break
                end += 1
            else:
                raise _StructureError("unterminated string literal")
            tokens.append(source[index:end])
            index = end
            continue
        if char.isalnum() or char == "_":
            end = index + 1
            while end < len(source) and (source[end].isalnum() or source[end] == "_"):
                end += 1
            tokens.append(source[index:end])
            index = end
            continue

        tokens.append(char)
        index += 1

    return tokens


_OPEN_TO_CLOSE = {"(": ")", "[": "]", "{": "}"}
_CLOSE_TO_OPEN = {close: open_ for open_, close in _OPEN_TO_CLOSE.items()}


def _matching_delimiter(tokens: list[str], start: int) -> int:
    opening = _token_at(tokens, start)
    closing = _OPEN_TO_CLOSE.get(opening)
    if closing is None:
        raise _StructureError(f"`{opening}` is not an opening delimiter")

    stack = [opening]
    for index in range(start + 1, len(tokens)):
        token = tokens[index]
        if token in _OPEN_TO_CLOSE:
            stack.append(token)
        elif token in _CLOSE_TO_OPEN:
            if stack[-1] != _CLOSE_TO_OPEN[token]:
                raise _StructureError(f"unbalanced `{token}`")
            stack.pop()
            if not stack:
                return index

    raise _StructureError(f"unclosed `{opening}`")


def _token_at(tokens: list[str], index: int) -> str:
    return tokens[index] if 0 <= index < len(tokens) else ""


def _is_identifier(token: str) -> bool:
    return (
        bool(token)
        and (token[0].isalpha() or token[0] == "_")
        and all(char.isalnum() or char == "_" for char in token[1:])
    )
