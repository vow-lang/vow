#!/usr/bin/env bash
set -euo pipefail

BOLD="\033[1m"
GREEN="\033[32m"
RED="\033[31m"
YELLOW="\033[33m"
RESET="\033[0m"

PASS=0
FAIL=0
SKIP=0
FAILURES=()

RUST="${VOW_FULL_TEST_RUST:-./target/release/vow}"
SELF=""
TMPDIR=$(mktemp -d)
# EXIT alone does not fire on an untrapped SIGTERM/SIGINT/SIGHUP: bash dies
# immediately and this whole scratch tree survives. It routinely reaches several
# GB, and on hosts where /tmp is a tmpfs that is abandoned RAM. Re-raise rather
# than cleaning up inline -- a bare `trap 'rm -rf "$TMPDIR"' TERM` would delete
# the scratch dir and then let the script keep running against it, because a
# bash signal handler resumes execution unless it exits.
trap 'rm -rf "$TMPDIR"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

# The suite is split by section into scripts/full_test/*.sh. Each file is sourced here, in the
# order the sections run, so every helper, counter and variable stays a global of this one shell.
FULL_TEST_DIR="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/full_test"

source "$FULL_TEST_DIR/lib_core.sh"
source "$FULL_TEST_DIR/lib_compare.sh"
source "$FULL_TEST_DIR/lib_run_tests.sh"
source "$FULL_TEST_DIR/lib_bootstrap.sh"

# The boundary test recursively invokes this script with fake VOW_FULL_TEST_RUST/VOW_FULL_TEST_CONCAT tools to isolate bootstrap failure handling.
if [ "${VOW_FULL_TEST_BOOTSTRAP_ONLY:-0}" = "1" ]; then
    echo -e "${BOLD}=== Phase 20.1: Full Test Suite ===${RESET}"
    echo ""
    section_begin "Section 9: Bootstrap Triple Test"
    run_bootstrap_triple "${VOW_FULL_TEST_RUST:?}" "${VOW_FULL_TEST_CONCAT:?}"
    echo ""
    summary_status=0
    print_summary || summary_status=$?
    exit "$summary_status"
fi

echo -e "${BOLD}=== Phase 20.1: Full Test Suite ===${RESET}"
echo ""

section_begin "Section 0: Setup"
setup_compilers

if [ "${VOW_FULL_TEST_SETUP_ONLY:-0}" = "1" ]; then
    exit 0
fi

if [ "${VOW_FULL_TEST_PROMOTED_ONLY:-0}" = "1" ]; then
    run_promoted_run_tests
    echo ""
    run_promoted_error_tests
    echo ""
    summary_status=0
    print_summary || summary_status=$?
    exit "$summary_status"
fi

source "$FULL_TEST_DIR/section_00b_block_region.sh"
source "$FULL_TEST_DIR/section_01_build.sh"
source "$FULL_TEST_DIR/section_02_verify.sh"
source "$FULL_TEST_DIR/section_03_runtime.sh"
source "$FULL_TEST_DIR/section_04c_verify_fail.sh"
source "$FULL_TEST_DIR/section_04c_verify_notes.sh"
source "$FULL_TEST_DIR/section_04d_verify_skip.sh"
source "$FULL_TEST_DIR/section_04g_native.sh"
source "$FULL_TEST_DIR/section_04h_cli_flags.sh"
source "$FULL_TEST_DIR/section_05_modes.sh"
source "$FULL_TEST_DIR/section_06_multi_error.sh"
source "$FULL_TEST_DIR/section_08_help_tooling.sh"
source "$FULL_TEST_DIR/section_08c_contract_quality.sh"
if [ "${VOW_FULL_TEST_TIER15_ONLY:-0}" = "1" ]; then
    summary_status=0
    print_summary || summary_status=$?
    exit "$summary_status"
fi

source "$FULL_TEST_DIR/section_09_bootstrap_default.sh"
source "$FULL_TEST_DIR/section_10b_test_subcommand.sh"
source "$FULL_TEST_DIR/section_11_arena_mutants.sh"
source "$FULL_TEST_DIR/section_13_complexity.sh"
source "$FULL_TEST_DIR/section_14_perfetto.sh"

# ─── Summary ────────────────────────────────────────────────────────

summary_status=0
print_summary || summary_status=$?
exit "$summary_status"
