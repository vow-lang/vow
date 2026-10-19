# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 1: Build --no-verify ─────────────────────────────────

section_begin "Section 1: Build --no-verify"
for vow_file in examples/*.vow; do
    name=$(basename "$vow_file" .vow)

    rust_json="" self_json="" rust_exit=0 self_exit=0
    rust_json=$($RUST build --no-verify "$vow_file" -o "$TMPDIR/rust_${name}" 2>/dev/null) || rust_exit=$?
    self_json=$(run_self build --no-verify "$vow_file" -o "$TMPDIR/self_${name}" 2>/dev/null) || self_exit=$?

    if check_empty_output "${name}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit"; then
        continue
    fi

    compare_json "${name}/build-no-verify" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$vow_file"

    # Save JSON for Section 3 (runtime execution)
    echo "$rust_json" > "$TMPDIR/rust_${name}.json"
    echo "$self_json" > "$TMPDIR/self_${name}.json"
done

missing_parent_fixture="examples/hello.vow"
rust_missing_parent_output="$TMPDIR/rust_missing_output_parent/out"
self_missing_parent_output="$TMPDIR/self_missing_output_parent/out"
rust_json="" self_json="" rust_exit=0 self_exit=0
rust_json=$($RUST build --no-verify "$missing_parent_fixture" -o "$rust_missing_parent_output" 2>/dev/null) || rust_exit=$?
self_json=$(run_self build --no-verify "$missing_parent_fixture" -o "$self_missing_parent_output" 2>/dev/null) || self_exit=$?

if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
    fail "build-no-verify/missing-output-parent" "empty output (rust=$rust_exit, self=$self_exit)"
else
    compare_json "build-no-verify/missing-output-parent" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$missing_parent_fixture"
fi

for name_output in "rust:$rust_missing_parent_output" "self:$self_missing_parent_output"; do
    compiler="${name_output%%:*}"
    output="${name_output#*:}"
    if [ -x "$output" ]; then
        pass "build-no-verify/${compiler}-creates-output-parent"
    else
        fail "build-no-verify/${compiler}-creates-output-parent" "missing executable: $output"
    fi
done

# default_output's basename scan on a bare filename with no '/' in it — #1119
# PR1. Never exercised above since every call there passes -o explicitly.
repo_root=$(pwd -P)
rust_abs="$repo_root/target/release/vow"
bare_src='module Bare

fn main() -> i32 [io] {
    print_i64(42);
    0
}'
rust_bare_dir="$TMPDIR/rust_default_output_no_slash"
self_bare_dir="$TMPDIR/self_default_output_no_slash"
mkdir -p "$rust_bare_dir" "$self_bare_dir"
printf '%s\n' "$bare_src" > "$rust_bare_dir/bare.vow"
printf '%s\n' "$bare_src" > "$self_bare_dir/bare.vow"

rust_exit=0 self_exit=0
rust_json=$(cd "$rust_bare_dir" && "$rust_abs" build --no-verify bare.vow 2>/dev/null) || rust_exit=$?
self_json=$(cd "$self_bare_dir" && run_self build --no-verify bare.vow 2>/dev/null) || self_exit=$?

if [ -z "$rust_json" ] || [ -z "$self_json" ]; then
    fail "build-no-verify/default-output-no-slash" "empty output (rust=$rust_exit, self=$self_exit)"
else
    compare_json "build-no-verify/default-output-no-slash" "$rust_json" "$self_json" "$rust_exit" "$self_exit" "$rust_bare_dir/bare.vow"
fi

for name_output in "rust:$rust_bare_dir/build/bare" "self:$self_bare_dir/build/bare"; do
    compiler="${name_output%%:*}"
    output="${name_output#*:}"
    if [ -x "$output" ]; then
        pass "build-no-verify/${compiler}-default-output-no-slash-names-build-bare"
    else
        fail "build-no-verify/${compiler}-default-output-no-slash-names-build-bare" "missing executable: $output"
    fi
done
echo ""
