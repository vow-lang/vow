#!/usr/bin/env bash
# Equivalence tier 1, compiler half: run `test compiler/` under the Rust
# compiler and under the self-hosted compiler, then compare the two JSON
# documents with scripts/parity.py. Expects target/release/vow and build/vowc
# to exist. Shared by bootstrap.yml (post-merge) and compiler-tests.yml (PRs).
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

out=$(mktemp -d) || exit 1
trap 'rm -rf "$out"' EXIT

target/release/vow test compiler/ > "$out/rust_test.json"
rust_exit=$?
build/vowc test compiler/ > "$out/self_test.json"
self_exit=$?

python3 scripts/parity.py test "$out/rust_test.json" "$out/self_test.json" \
  "$rust_exit" "$self_exit"
