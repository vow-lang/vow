"""The language section of the help JSON, derived from docs/spec/grammar.md."""

from helpgen.tables import extract_table, extract_table_col


def build_language_help(grammar: str) -> dict:
    prim_types = extract_table_col(grammar, "Primitive Types", 0)
    param_types = extract_table_col(grammar, "Built-in Parameterized Types", 0)
    all_types = prim_types + param_types

    # --- Effects ---
    effects = extract_table_col(grammar, "Effect Types", 0)

    # --- Builtins (with full signatures) ---
    builtin_rows = extract_table(grammar, "Builtin Function Signatures")
    builtins = {}
    for row in builtin_rows:
        if len(row) >= 3:
            name, sig, eff = row[0], row[1], row[2]
            builtins[name] = f"{sig} {eff}"

    # --- Operators ---
    arith = extract_table_col(grammar, "Wrapping Arithmetic (default)", 0)
    checked = extract_table_col(grammar, "Checked Arithmetic", 0)
    comparison = extract_table_col(grammar, "Comparison Operators", 0)
    logical = extract_table_col(grammar, "Logical Operators", 0)
    unary = extract_table_col(grammar, "Unary Operators", 0)
    bitwise = extract_table_col(grammar, "Bitwise Operators", 0)

    # --- Methods ---
    def method_names(heading: str) -> list[str]:
        return extract_table_col(grammar, heading, 0)

    vec_methods = method_names("Vec<T> Methods")
    string_methods = method_names("String Methods")
    hashmap_methods = method_names("HashMap<K, V> Methods")
    btreemap_methods = method_names("BTreeMap<K, V> Methods")
    option_methods = method_names("Option<T> Methods")
    option_methods.append("? operator")

    # --- Pattern kinds ---
    pattern_rows = extract_table(grammar, "Pattern Kinds")
    patterns = []
    for row in pattern_rows:
        if len(row) >= 2:
            kind = row[0]
            example = row[1]
            patterns.append(f"{kind} ({example})")

    return {
        "module": "module <Name>",
        "use_declaration": "use foo.bar",
        "const_declaration": "const NAME: i64 = 1024",
        "comments": "// line comments only; block comments unsupported",
        "let_binding": "let name: Type = expr; or let mut name: Type = expr;",
        "function": "fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
        "public_function": "pub fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
        "vow_function": "fn <name>(<params>) -> <RetTy> vow { requires: <expr>, ensures: <expr> } { <body> }",
        "while_with_invariant": "while <cond> vow { invariant: <expr> } { <body> }",
        "literals": {
            "integer": "42 | -1 | 42u64 (unsuffixed integers default to i64)",
            "float": "3.14 | -0.5",
            "bool": "true | false",
            "string": '"text" with escapes \\n \\t \\r \\\\ \\" \\0',
        },
        "casts": "x as u64 or y as i64",
        "types": all_types,
        "effects": effects,
        "builtins": builtins,
        "operators": {
            "arithmetic": arith,
            "checked_arithmetic": checked,
            "comparison": comparison,
            "logical": logical,
            "bitwise": bitwise,
            "unary": unary,
        },
        "vow_clauses": {
            "requires": "precondition \u2014 blame=Caller on violation",
            "ensures": "postcondition \u2014 blame=Callee on violation; use `result` for return value",
            "invariant": "loop invariant \u2014 checked at top of each iteration",
        },
        "where_clauses": "fn f(x: i64 where x >= 0) -> i64 \u2014 refinement types on parameters",
        "structs": {
            "definition": "struct Name { field: Type, ... }",
            "linear": "linear struct Name { field: Type, ... } \u2014 linear obligation must be consumed or returned before region close",
            "literal": "Name { field: value, ... }",
            "field_access": "value.field",
        },
        "enums": {
            "definition": "enum Name { Variant1(T), Variant2, Variant3 { field: T } }",
            "construction": "Name::Variant(value)",
            "builtin_option": "Option<T> \u2014 variants: Some(T), None",
            "builtin_result": "Result<T, E> \u2014 variants: Ok(T), Err(E)",
        },
        "match_expression": {
            "syntax": "match value { Pattern => expr, ... }",
            "patterns": patterns,
        },
        "control_flow": {
            "if_else": "if cond { expr } else { expr } \u2014 expression, both branches same type",
            "while": "while cond { body }",
            "for_each": "for item in vec { body } \u2014 iterate Vec elements",
            "for_enumerate": "for i, item in vec { body } \u2014 iterate with index",
            "loop": "loop { ... break value; } \u2014 infinite loop, break to exit",
            "break": "break; or break value;",
            "return": "return; or return value;",
        },
        "modules": {
            "declaration": "module Name",
            "import": (
                "use foo.bar \u2014 prefers <rootdir>/foo/bar.vow.d when present; "
                "otherwise <rootdir>/foo/bar.vow"
            ),
            "visibility": "pub fn \u2014 public functions visible to importers",
        },
        "type_aliases": "type Name = Type",
        "extern_blocks": 'extern "C" { vow { requires: ... } fn name(x: i64) -> i64 [unsafe]; }',
        "methods": {
            "Vec<T>": vec_methods,
            "String": string_methods,
            "HashMap<K,V>": hashmap_methods,
            "BTreeMap<K,V>": btreemap_methods,
            "Option<T>": option_methods,
        },
        "error_propagation": "? on Option<T> or Result<T, E> propagates None/Err to the caller",
        "indexing": {
            "read": "v[i] \u2014 Vec index access",
            "write": "v[i] = val \u2014 Vec index assignment",
        },
        "feature_status": {
            "implemented": {
                "function_vow_blocks": "requires / ensures / invariant",
                "where_clauses": "parameter-level refinement sugar",
                "loop_invariants": "simple invariant predicates",
            },
            "partial": {
                "refinement_type_predicates": "rejected with a type error (fail-closed, never silently unverified); use a where clause on the parameter or a requires/ensures contract",
                "effect_tracking": "user-defined effect propagation is enforced; some builtin panic/unsafe effects are not yet modeled",
            },
            "target": {
                "module_level_vow_blocks": "specified in docs but not parsed or represented in the AST",
                "quantifiers": "forall / exists are not yet in the lexer or parser",
            },
            "unsupported": [
                "user-defined generics",
                "traits",
                "closures",
                "operator overloading",
                "macros",
                "assert / assume statements",
            ],
        },
    }
