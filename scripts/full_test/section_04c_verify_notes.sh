# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Skip set independent of cache and --verify-jobs (#1561) ──────
#
# A VerificationSkipped warning is a static fact about the program, so the set
# of skipped functions and the verdict must not depend on how fast a failing
# function came back: a cold cache, a warm cache and any job count agree.
# Only the verdict and the skip set are compared; other warnings produced
# inside the verifier pool can still vary on a halted parallel run.

section_begin "Skip set independent of cache and --verify-jobs (#1561)"

skip_set_summary() {
    python3 -c "
import json, sys
d = json.loads(sys.argv[1])
skips = sorted(x.get('message', '') for x in d.get('diagnostics', [])
               if x.get('error_code') == 'VerificationSkipped' and x.get('severity') == 'warning')
print(d.get('status', ''), len(skips), *skips, sep='\\n')
" "$1" 2>/dev/null || echo "<unparseable>"
}

for fixture in tests/verify-fail/skip_after_failure.vow stdlib/heap/main.vow tests/verify-fail/clamp_wrong_op.vow; do
    name="verify_skip_set_scheduling_independent/$(basename "$(dirname "$fixture")")-$(basename "$fixture" .vow)"
    errors=()
    skip_cache_dir=$(mktemp -d "$TMPDIR/skip-cache.XXXXXX")

    base_json=$($RUST verify --no-cache --verify-jobs 1 "$fixture" 2>/dev/null) || true
    base=$(skip_set_summary "$base_json")
    if [ "$base" = "<unparseable>" ]; then
        errors+=("baseline produced no parseable verify JSON; the comparisons below would be vacuous")
    fi
    for pass in "no-cache-jobs4" "cold-jobs4" "warm-jobs4" "warm-jobs1"; do
        case "$pass" in
            no-cache-jobs4) pass_json=$($RUST verify --no-cache --verify-jobs 4 "$fixture" 2>/dev/null) || true ;;
            cold-jobs4|warm-jobs4) pass_json=$(VOW_CACHE_DIR="$skip_cache_dir" $RUST verify --verify-jobs 4 "$fixture" 2>/dev/null) || true ;;
            warm-jobs1) pass_json=$(VOW_CACHE_DIR="$skip_cache_dir" $RUST verify --verify-jobs 1 "$fixture" 2>/dev/null) || true ;;
        esac
        if [ "$(skip_set_summary "$pass_json")" != "$base" ]; then
            errors+=("rust $pass differs from --no-cache --verify-jobs 1")
        fi
    done

    self_base_json=$(run_self verify --no-cache --verify-jobs 1 "$fixture" 2>/dev/null) || true
    self_jobs4_json=$(run_self verify --no-cache --verify-jobs 4 "$fixture" 2>/dev/null) || true
    if [ "$(skip_set_summary "$self_base_json")" != "$base" ]; then
        errors+=("self --verify-jobs 1 differs from rust")
    fi
    if [ "$(skip_set_summary "$self_jobs4_json")" != "$base" ]; then
        errors+=("self --verify-jobs 4 differs from rust")
    fi

    case "$fixture" in
        tests/verify-fail/skip_after_failure.vow)
            if [ "$(printf '%s' "$base" | sed -n 2p)" != "2" ]; then
                errors+=("expected 2 VerificationSkipped warnings, got: $(printf '%s' "$base" | sed -n 2p)")
            fi
            ;;
        tests/verify-fail/clamp_wrong_op.vow)
            # Non-vacuity: the warm passes only exercise the cache if it stored a failure.
            if [ -z "$(find "$skip_cache_dir" -name '*.vr' 2>/dev/null | head -1)" ]; then
                errors+=("verify cache stored no entry; the cold/warm comparison was vacuous")
            fi
            ;;
    esac
    rm -rf "$skip_cache_dir"

    if [ ${#errors[@]} -eq 0 ]; then
        pass "$name"
    else
        fail "$name" "$(IFS='; '; echo "${errors[*]}")"
    fi
done
echo ""

# ─── Checked-arithmetic abort model (#585) ────────────────────────
#
# Sections 4b/4c already hold Rust/self parity and the Verified/VerifyFailed
# verdicts for the two committed fixtures. What they cannot express is the
# property the issue is actually about: that `+!` and `+` are *distinguishable*,
# and that the reachable-abort warning appears exactly where the abort is
# reachable. Both compilers are checked, so a self-hosted regression cannot hide
# behind the Rust verdict.

section_begin "Checked-arithmetic abort model (#585)"

arith_dir="$TMPDIR/arith585"
mkdir -p "$arith_dir"

# Same contract, same body, one operator apart. Under the old model these two
# produced byte-identical counterexamples.
cat > "$arith_dir/wrapping.vow" <<'VOW'
module ArithWrapping
fn last_index(n: u64) -> u64 vow { ensures: result <= n } { n - 1 }
fn main() -> i32 [io] { print_i64(0); 0 }
VOW
cat > "$arith_dir/checked.vow" <<'VOW'
module ArithChecked
fn last_index(n: u64) -> u64 vow { ensures: result <= n } { n -! 1 }
fn main() -> i32 [io] { print_i64(0); 0 }
VOW

arith_status() {
    python3 -c "import json,sys; print(json.loads(sys.argv[1]).get('status',''))" "$1" 2>/dev/null || echo ""
}
# Query the diagnostics of a verify/build JSON document. MODE is one of:
#   arith-warns FN  count of ArithOverflowReachable warnings naming FN
#   bound-note FN   the `(...)` bound list of FN's ModelCapacityAssumed note
#   bound-notes     every ModelCapacityAssumed message, in output order
diag_query() {
    local mode="$1" json="$2" fn="${3:-}" err=ERR
    [ "$mode" = arith-warns ] && err=-1
    python3 -c "
import json, sys
mode, d, fn = sys.argv[1], json.loads(sys.argv[2]), sys.argv[3]
diags = d.get('diagnostics', [])
named = lambda g: ('\`' + fn + '\`') in g.get('message', '')
if mode == 'arith-warns':
    print(sum(1 for g in diags
              if g.get('error_code') == 'ArithOverflowReachable' and named(g)))
else:
    for g in diags:
        if g.get('error_code') != 'ModelCapacityAssumed':
            continue
        if mode == 'bound-notes':
            print(g['message'])
        elif g.get('severity') == 'note' and named(g):
            print(g['message'].rsplit('(', 1)[1].rstrip(')'))
" "$mode" "$json" "$fn" 2>/dev/null || echo "$err"
}

for compiler in rust self; do
    errors=()

    # 1. Wrapping still wraps: the counterexample is real and must be reported.
    if [ "$compiler" = rust ]; then
        j=$($RUST verify "$arith_dir/wrapping.vow" 2>/dev/null) || true
    else
        j=$(run_self verify "$arith_dir/wrapping.vow" 2>/dev/null) || true
    fi
    [ "$(arith_status "$j")" = "VerifyFailed" ] || errors+=("wrapping '-' should fail, got $(arith_status "$j")")

    # 2. Checked is distinguishable: the abort prunes the wrapped value, so the
    #    postcondition holds on every returning execution and the reachable
    #    abort is reported as a warning instead.
    if [ "$compiler" = rust ]; then
        j=$($RUST verify "$arith_dir/checked.vow" 2>/dev/null) || true
    else
        j=$(run_self verify "$arith_dir/checked.vow" 2>/dev/null) || true
    fi
    [ "$(arith_status "$j")" = "Verified" ] || errors+=("checked '-!' should verify, got $(arith_status "$j")")
    [ "$(diag_query arith-warns "$j" last_index)" = "1" ] || errors+=("checked '-!' should warn once, got $(diag_query arith-warns "$j" last_index)")

    # 3. The warning is precise, not blanket: in the committed fixture only the
    #    two functions whose abort is genuinely reachable are named, and a
    #    `requires` that rules the abort out silences it.
    if [ "$compiler" = rust ]; then
        j=$($RUST verify tests/verify/checked_arith_abort_modelled.vow 2>/dev/null) || true
    else
        j=$(run_self verify tests/verify/checked_arith_abort_modelled.vow 2>/dev/null) || true
    fi
    [ "$(arith_status "$j")" = "Verified" ] || errors+=("fixture should verify, got $(arith_status "$j")")
    for fn in twice scale doomed; do
        [ "$(diag_query arith-warns "$j" "$fn")" = "1" ] || errors+=("$fn should warn, got $(diag_query arith-warns "$j" "$fn")")
    done
    for fn in last_index halve rem_pos; do
        [ "$(diag_query arith-warns "$j" "$fn")" = "0" ] || errors+=("$fn abort is ruled out by requires; must not warn")
    done

    # 4. A site inside a co-emitted callee is attributed to the callee, not to
    #    the verify target that pulled it in, and the two targets that both
    #    reach it report it once.
    if [ "$compiler" = rust ]; then
        j=$($RUST verify tests/verify/checked_arith_callee_attribution.vow 2>/dev/null) || true
    else
        j=$(run_self verify tests/verify/checked_arith_callee_attribution.vow 2>/dev/null) || true
    fi
    [ "$(arith_status "$j")" = "Verified" ] || errors+=("attribution fixture status $(arith_status "$j")")
    [ "$(diag_query arith-warns "$j" helper)" = "1" ] || errors+=("site must be attributed to helper exactly once, got $(diag_query arith-warns "$j" helper)")
    [ "$(diag_query arith-warns "$j" caller)" = "0" ] || errors+=("caller has no checked arithmetic and must not be named")

    if [ ${#errors[@]} -eq 0 ]; then
        pass "checked_arith_abort_model/$compiler"
    else
        fail "checked_arith_abort_model/$compiler" "$(IFS='; '; echo "${errors[*]}")"
    fi
done
echo ""

# ─── Model-capacity bound note (ModelCapacityAssumed) ─────────────
#
# A proof from a model that restricts a collection length to the verifier's model
# capacity is bounded; the verifier reports that as one `ModelCapacityAssumed`
# note per proved function (severity `note`, status and exit code unchanged),
# naming each bounded collection kind and capacity. Both compilers must emit the
# same note, and functions whose model restricts nothing (scalars, a constant
# in-range length) must carry none.
section_begin "Model-capacity bound note"

# The notes are asserted on both pipelines: `verify` (frontend + verification
# only) and `build` (the default user path, which verifies and then compiles the
# fixture's trivial `main`). Both compilers must agree on the full ordered list.
fixture="tests/verify/model_capacity_bound_note.vow"
for cmd in verify build; do
    notes_by_compiler=()
    for compiler in rust self; do
        extra=()
        [ "$cmd" = build ] && extra=(-o "$TMPDIR/model_capacity_bound_note_${compiler}")
        if [ "$compiler" = rust ]; then
            j=$($RUST "$cmd" --no-cache "$fixture" ${extra[@]+"${extra[@]}"} 2>/dev/null) || true
        else
            j=$(run_self "$cmd" --no-cache "$fixture" ${extra[@]+"${extra[@]}"} 2>/dev/null) || true
        fi
        errors=()
        if [ "$cmd" = build ] && [ "$(arith_status "$j")" != "Verified" ]; then
            errors+=("build should verify, got $(arith_status "$j")")
        fi
        [ "$(diag_query bound-note "$j" first_byte)" = "Vec<T>: 128" ] || errors+=("first_byte note: '$(diag_query bound-note "$j" first_byte)'")
        [ "$(diag_query bound-note "$j" string_len)" = "String: 256" ] || errors+=("string_len note: '$(diag_query bound-note "$j" string_len)'")
        [ "$(diag_query bound-note "$j" map_size)" = "HashMap<K, V>: 64" ] || errors+=("map_size note: '$(diag_query bound-note "$j" map_size)'")
        [ "$(diag_query bound-note "$j" vec_and_string)" = "Vec<T>: 128, String: 256" ] || errors+=("vec_and_string note: '$(diag_query bound-note "$j" vec_and_string)'")
        [ -z "$(diag_query bound-note "$j" scalar_only)" ] || errors+=("scalar_only must not carry a capacity note")
        [ -z "$(diag_query bound-note "$j" constant_in_range)" ] || errors+=("constant_in_range restricts nothing and must not carry a capacity note")
        notes_by_compiler+=("$(diag_query bound-notes "$j")")

        if [ ${#errors[@]} -eq 0 ]; then
            pass "model_capacity_bound_note/$cmd/$compiler"
        else
            fail "model_capacity_bound_note/$cmd/$compiler" "$(IFS='; '; echo "${errors[*]}")"
        fi
    done
    if [ "${notes_by_compiler[0]}" = "${notes_by_compiler[1]}" ] && [ -n "${notes_by_compiler[0]}" ]; then
        pass "model_capacity_bound_note/$cmd/parity"
    else
        fail "model_capacity_bound_note/$cmd/parity" "rust and self-hosted $cmd notes differ"
    fi
done
echo ""

# ─── span_pack verifier-bound regression (#1308) ──────────────────
#
# `span_pack` (compiler/parser.vow) packs a byte offset (`start`) and a
# length (`len`) into a single i64. `start < 65536` was never a real domain
# fact -- `start` is a cumulative file offset with no reason to stay under
# 64 KiB -- and was silently violated once real compiler sources grew past
# that size. The fix drops that bound and switches the body to checked
# `*!`/`+!` so an astronomical `start` aborts cleanly instead of the
# postcondition going unsound under wraparound. `vow verify
# compiler/parser.vow` has no `main` (verify-only, no codegen -- that's
# fine) and pulls in ~51 pre-existing, unrelated `RegionRootEscape` notes
# from its `use token`/`use ast`/`use diag` graph, so this checks the
# `ArithOverflowReachable` diagnostic naming `span_pack` specifically, not
# whether `diagnostics` is empty.
section_begin "span_pack verifier-bound regression (#1308)"

for compiler in rust self; do
    if [ "$compiler" = rust ]; then
        j=$($RUST verify --no-cache compiler/parser.vow 2>/dev/null) || true
    else
        j=$(run_self verify --no-cache compiler/parser.vow 2>/dev/null) || true
    fi
    errors=()
    [ "$(arith_status "$j")" = "Verified" ] || errors+=("compiler/parser.vow should verify, got $(arith_status "$j")")
    [ "$(diag_query arith-warns "$j" span_pack)" = "1" ] || errors+=("span_pack should warn once (start's magnitude bound was dropped, so the abort is reachable), got $(diag_query arith-warns "$j" span_pack)")

    if [ ${#errors[@]} -eq 0 ]; then
        pass "span_pack_verifier_bound/$compiler"
    else
        fail "span_pack_verifier_bound/$compiler" "$(IFS='; '; echo "${errors[*]}")"
    fi
done
echo ""
