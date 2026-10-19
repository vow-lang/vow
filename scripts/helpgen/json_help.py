"""Assemble the versioned --help JSON from the grammar and CLI specs."""

from helpgen.json_commands import build_cli_help, verification_defaults
from helpgen.json_language import build_language_help


def build_help_json(grammar: str, cli: str, _contracts: str) -> dict:
    # Key order is part of the output: language sits between exit_codes and
    # verification_defaults.
    data = build_cli_help(cli)
    data["language"] = build_language_help(grammar)
    data["verification_defaults"] = verification_defaults()
    return data
