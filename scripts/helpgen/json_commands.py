"""Command, option and output sections of the help JSON, derived from cli.md."""

import re

from helpgen.config import (
    DEFAULT_MAX_K_STEP,
    SCHEMA_VERSION,
)
from helpgen.tables import extract_table, normalize_option


def _options(
    cli: str, heading: str, *, merge_mode: bool = False, skip_angle: bool = False
) -> tuple[dict[str, str], list[dict]]:
    """Read one command's option table as (legacy key->text map, structured entries).

    With merge_mode the `--mode debug` row stands for both modes and the
    `--mode release` row is dropped; skip_angle drops positional rows.
    """
    options: dict[str, str] = {}
    entries: list[dict] = []
    for row in extract_table(cli, heading, heading_level=3):
        if len(row) < 3:
            continue
        flag, default, desc = row[0], row[1], row[2]
        if skip_angle and flag.startswith("<"):
            continue
        if merge_mode and flag == "--mode release":
            continue
        key, value, option = normalize_option(
            flag, default, desc, merge_mode=merge_mode and flag == "--mode debug"
        )
        options[key] = value
        entries.append(option)
    return options, entries


_MUTANTS_USAGE_FLAG = re.compile(r"(--[a-z0-9-]+) ([^\s\]-][^\s\]]*)")


def _mutants_value_names(cli: str) -> dict[str, tuple[str, str]]:
    """Map each `vow mutants` flag to (value placeholder, kind) from its usage block."""
    section = cli.split("### `vow mutants`", 1)[1].split("\n### ", 1)[0]
    block = section.split("```")[1]
    names: dict[str, tuple[str, str]] = {}
    for flag, token in _MUTANTS_USAGE_FLAG.findall(block):
        name = token.strip("'")
        kind = {"N": "integer", "DIR": "path"}.get(name, "string")
        names[flag] = (name, kind)
    return names


def _mutants_options(cli: str) -> list[dict]:
    """Read the `vow mutants` flag table as structured entries.

    A flag whose usage line shows no placeholder takes no value.
    """
    value_names = _mutants_value_names(cli)
    entries: list[dict] = []
    for row in extract_table(cli, "vow mutants", heading_level=3):
        if len(row) < 3:
            continue
        flag, default, desc = row[0], row[1], row[2]
        long_flag = flag.split(" ")[0]
        value_name, value_kind = value_names.get(long_flag, (None, "flag"))
        form = long_flag if value_name is None else f"{long_flag} <{value_name}>"
        description = desc if default in ("", "off") else f"{desc} (default: {default})"
        entry: dict[str, object] = {
            "form": form,
            "description": description,
            "long": long_flag,
            "value_kind": value_kind,
        }
        if value_name is not None:
            entry["value_name"] = value_name
        entries.append(entry)
    return entries


def _flag_option(flag: str, description: str) -> dict:
    return {
        "form": flag,
        "description": description,
        "long": flag,
        "value_kind": "flag",
    }


def verification_defaults() -> dict:
    # Collection model capacities (Vec/String/HashMap/BTreeMap) are deliberately
    # absent: they are internal verifier-model bounds, not language properties or
    # tunable CLI flags. See docs/design/verifier-model-bounds.md.
    # `strategy`/`max_k_step` describe the default ESBMC backend and stay as
    # they are for agents that read them; `native` describes `--backend native`.
    return {
        "strategy": "incremental-bmc",
        "max_k_step": DEFAULT_MAX_K_STEP,
        "native": {
            "strategy": "bounded unwinding with invariant induction and k-induction",
            "tuning_flags": ["--timeout"],
        },
    }


def build_cli_help(cli: str) -> dict:
    build_options, build_option_entries = _options(cli, "vow build", merge_mode=True)
    verify_options, verify_option_entries = _options(cli, "vow verify")
    decl_options, decl_option_entries = _options(cli, "vow decl")
    test_options, test_option_entries = _options(
        cli, "vow test", merge_mode=True, skip_angle=True
    )
    contracts_options, contracts_option_entries = _options(cli, "vow contracts")
    complexity_options, complexity_option_entries = _options(cli, "vow complexity")
    mutants_option_entries = _mutants_options(cli)

    return {
        "schema_version": SCHEMA_VERSION,
        "kind": "tool_help",
        "tool": "vow",
        "audience": "agent",
        "default_format": "json",
        "description": "Vow compiler: compiles Vow source to native executables with contract verification",
        "usage": "vow <command> [OPTIONS] <source.vow>",
        "legacy_usage": "vow [OPTIONS] <source.vow> (equivalent to vow build)",
        "references": {
            "grammar": "reference/grammar.md",
            "cli": "reference/cli.md",
            "contracts": "reference/contracts.md",
            "errors": "reference/errors.md",
            "stdlib": "reference/stdlib.md",
            "examples": "examples/examples.md",
            "schemas": {
                "build_result": "schemas/build-result.schema.json",
                "contracts_result": "schemas/contracts-result.schema.json",
                "diagnostic": "schemas/diagnostic.schema.json",
                "counterexample": "schemas/counterexample.schema.json",
                "mutants_result": "schemas/mutants-result.schema.json",
                "test_result": "schemas/test-result.schema.json",
                "complexity_result": "schemas/complexity-result.schema.json",
                "vow_violation": "schemas/vow-violation.schema.json",
            },
        },
        "invocation": {
            "canonical": "vow <command> [OPTIONS] <source.vow>",
            "default_command": "build",
            "legacy_equivalent": "vow [OPTIONS] <source.vow>",
            "source_argument": {
                "name": "source",
                "kind": "path",
                "required": True,
                "suffix": ".vow",
            },
        },
        "commands": {
            "build": "Compile source to native executable (verifies by default; use --no-verify to skip)",
            "verify": "Verify contracts without producing an executable (use --no-cache to skip cache)",
            "test": "Run tests: discover, compile, execute test_*.vow files with JSON results",
            "decl": "Emit declaration file (.vow.d) with type signatures only",
            "contracts": "List all contracts with optional verification status",
            "skill": "Generate or install the Claude Code skill document for this compiler version",
            "complexity": "Report per-function complexity metrics as deterministic, byte-identical JSON",
            "mutants": "Run mutation testing on a Vow source tree (self-hosted compiler only)",
        },
        "command_details": {
            "build": {
                "status": "implemented",
                "usage": "vow build [OPTIONS] <source.vow>",
                "default_when_command_omitted": True,
                "arguments": [
                    {
                        "name": "source",
                        "kind": "path",
                        "required": True,
                        "suffix": ".vow",
                    }
                ],
                "options": build_option_entries,
                "stdout": {
                    "format": "json",
                    "schema_ref": "schemas/build-result.schema.json",
                    "suppressed_by": ["--dump-ir"],
                },
                "stderr": {
                    "channels": ["diagnostic stream", "debug trace"],
                    "debug_trace_flag": "--debug-trace <off|calls|full>",
                },
                "notes": [
                    "verification is enabled by default",
                    "debug mode inserts runtime vow checks",
                ],
            },
            "verify": {
                "status": "implemented",
                "usage": "vow verify [OPTIONS] <source.vow>",
                "arguments": [
                    {
                        "name": "source",
                        "kind": "path",
                        "required": True,
                        "suffix": ".vow",
                    }
                ],
                "options": verify_option_entries,
                "stdout": {
                    "format": "json",
                    "schema_ref": "schemas/build-result.schema.json",
                    "fixed_fields": {"executable": None},
                },
                "notes": [
                    "runs verification only and never emits a binary",
                ],
            },
            "test": {
                "status": "implemented",
                "usage": "vow test [OPTIONS] [<path>]",
                "arguments": [
                    {
                        "name": "path",
                        "kind": "path",
                        "required": False,
                        "default": ".",
                        "description": "Directory to scan or single .vow file",
                    }
                ],
                "options": test_option_entries,
                "stdout": {
                    "format": "json",
                },
                "notes": [
                    "discovers test_*.vow and *_test.vow files",
                    "each test must contain main() -> i32 returning 0 on success",
                    "default mode is debug (runtime vow checks enabled)",
                ],
            },
            "decl": {
                "status": "implemented",
                "usage": "vow decl [OPTIONS] <source.vow>",
                "arguments": [
                    {
                        "name": "source",
                        "kind": "path",
                        "required": True,
                        "suffix": ".vow",
                    }
                ],
                "options": decl_option_entries,
                "stdout": {"format": "none"},
                "side_effects": [
                    {
                        "kind": "write_file",
                        "default_path": "<source>.vow.d",
                    }
                ],
            },
            "contracts": {
                "status": "implemented",
                "usage": "vow contracts [OPTIONS] <source.vow>",
                "arguments": [
                    {
                        "name": "source",
                        "kind": "path",
                        "required": True,
                        "suffix": ".vow",
                    }
                ],
                "options": contracts_option_entries,
                "stdout": {
                    "format": "json",
                    "schema_ref": "schemas/contracts-result.schema.json",
                },
                "notes": [
                    "runs frontend only by default",
                    "use --verify for per-contract verification status",
                ],
            },
            "complexity": {
                "status": "implemented",
                "usage": "vow complexity [OPTIONS] <source.vow>",
                "arguments": [
                    {
                        "name": "source",
                        "kind": "path",
                        "required": True,
                        "suffix": ".vow",
                    }
                ],
                "options": complexity_option_entries,
                "stdout": {
                    "format": "json",
                    "schema_ref": "schemas/complexity-result.schema.json",
                },
                "notes": [
                    "reports only functions defined in the queried entry file",
                    "complexity_score is a readability / refactor-priority gate, not a defect predictor",
                    "--max-score / --max-cognitive / --max-cyclomatic gates are opt-in; exit nonzero only when set",
                ],
            },
            "skill": {
                "status": "implemented",
                "usage": "vow skill [print [--bundle] | install [--local | --global]]",
                "default_subcommand": "print",
                "subcommands": {
                    "print": {
                        "usage": "vow skill print [--bundle]",
                        "options": [
                            _flag_option(
                                "--bundle",
                                "Print one self-contained document for harnesses that cannot load supporting files",
                            )
                        ],
                        "stdout": {"format": "markdown"},
                    },
                    "install": {
                        "usage": "vow skill install [--local | --global]",
                        "options": [
                            _flag_option(
                                "--local",
                                "Install to ./.claude/skills/vow/ (requires .git and .claude/ in the current directory)",
                            ),
                            _flag_option(
                                "--global",
                                "Install to $HOME/.claude/skills/vow/",
                            ),
                        ],
                        "stdout": {"format": "none"},
                        "side_effects": [
                            {
                                "kind": "write_directory",
                                "default_path": "<scope>/.claude/skills/vow/",
                            }
                        ],
                    },
                },
                "notes": [
                    "the skill text is embedded in the compiler binary and always matches its version",
                    "install prompts on stderr for a scope when neither --local nor --global is given; scripts should pass one explicitly",
                ],
            },
            "mutants": {
                "status": "self-hosted-only",
                "usage": "vowc mutants <version | list | run> [OPTIONS]",
                "subcommands": {
                    "version": {
                        "usage": "vowc mutants version",
                        "stdout": {"format": "text"},
                    },
                    "list": {
                        "usage": "vowc mutants list [--root DIR] [--shard X/Y]",
                        "stdout": {"format": "jsonl"},
                    },
                    "run": {
                        "usage": "vowc mutants run [OPTIONS]",
                        "stdout": {"format": "summary line"},
                        "writes": {
                            "schema_ref": "schemas/mutants-result.schema.json",
                            "directory": "--output-dir",
                        },
                    },
                },
                "options": mutants_option_entries,
                "notes": [
                    "implemented in the self-hosted compiler (build/vowc) only; the Rust bootstrap compiler reports an error pointing to build/vowc",
                    "see docs/mutants.md for mutation kinds, the skip list and known limitations",
                ],
            },
        },
        "build_options": build_options,
        "verify_options": verify_options,
        "test_options": test_options,
        "decl_options": decl_options,
        "contracts_options": contracts_options,
        "complexity_options": complexity_options,
        "global_options": {
            "--help": "Emit versioned JSON tool-help data",
            "--help --human": "Emit legacy human-readable help (compatibility mode)",
        },
        "outputs": {
            "build_result": {
                "schema_ref": "schemas/build-result.schema.json",
                "emitted_by": ["build", "verify"],
                "status_values": [
                    "Verified",
                    "Unverified",
                    "Skipped",
                    "CompileFailed",
                    "VerifyFailed",
                ],
                "legacy_fields": ["counterexample"],
            },
            "contracts_result": {
                "schema_ref": "schemas/contracts-result.schema.json",
                "emitted_by": ["contracts"],
            },
            "diagnostic": {
                "schema_ref": "schemas/diagnostic.schema.json",
                "embedded_in": "build_result.diagnostics",
            },
            "runtime_vow_violation": {
                "schema_ref": "schemas/vow-violation.schema.json",
                "emitted_on": "stderr",
                "requires_mode": "debug",
            },
            "runtime_trace": {
                "emitted_on": "stderr",
                "enabled_by": "--debug-trace <off|calls|full>",
                "format": "jsonl",
            },
        },
        "output_json": {
            "status": "Verified | Unverified | Skipped | CompileFailed | VerifyFailed",
            "executable": "path to compiled binary, or null",
            "diagnostics": "[array of {error_code, message, severity, span: {file, offset, length}}]",
            "message": "error detail (CompileFailed)",
            "function": "function name (VerifyFailed)",
            "counterexample": "verifier counterexample description (VerifyFailed)",
        },
        "diagnostics": {
            "schema_ref": "schemas/diagnostic.schema.json",
            "fields": [
                "error_code",
                "message",
                "severity",
                "span.file",
                "span.offset",
                "span.length",
            ],
        },
        "exit_codes": {
            "0": "success (Verified or Unverified)",
            "1": "failure (CompileFailed, VerifyFailed, or Skipped)",
        },
    }
