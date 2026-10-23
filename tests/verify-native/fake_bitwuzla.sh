#!/usr/bin/env bash
set -eu
# Waits $1 seconds without a child process: a `sleep` orphaned by a kill would
# keep the solver's output pipe open and delay the parent's reap.
nap() {
    local fifo="$FAKE_BW_DIR/.nap.$$"
    mkfifo "$fifo"
    exec 7<>"$fifo"
    rm -f "$fifo"
    read -r -t "$1" -u 7 _ || true
}
# Claims the lowest unclaimed number for `$1.<k>` and sets `claimed`. `mkdir` is
# not exclusive under every coreutils (uutils 0.10 lets several concurrent
# callers all succeed, #1885); a noclobber redirection is one O_EXCL `open`.
claim() {
    claimed=1
    until ( set -C; : > "$1.$claimed" ) 2>/dev/null; do claimed=$((claimed + 1)); done
}
# True for the one caller that creates flag file `$1`.
first_to() { ( set -C; : > "$1" ) 2>/dev/null; }
if [ "${1:-}" = "--version" ]; then
    [ -n "${FAKE_BW_VERSION:-}" ] || exit 2
    echo "$FAKE_BW_VERSION"
    exit 0
fi
file="${!#}"
if [ "${file%.smt2}" = "$file" ]; then
    # an interactive session: commands on stdin, one reply per `(check-sat)`.
    claim "$FAKE_BW_DIR/.sess"
    s=$claimed
    t="$FAKE_BW_DIR/s.$s.smt2"
    : > "$t"
    echo "$*" >> "$FAKE_BW_DIR/sargs"
    echo $$ >> "$FAKE_BW_DIR/spids"
    case "$FAKE_BW_MODE" in
        exit1) echo "[error] boom" >&2; exit 1 ;;
        oom_msg) echo "(error) Out of memory" >&2; exit 1 ;;
        hang) echo $$ > "$FAKE_BW_DIR/pid"; exec sleep 29 ;;
        kill_worker) kill -9 "$PPID"; sleep 5 ;;
        crash_worker) kill -6 "$PPID"; sleep 5 ;;
    esac
    die_after=-1
    case "$FAKE_BW_MODE" in session_die_after_*) die_after="${FAKE_BW_MODE#session_die_after_}" ;; esac
    checks=0
    while IFS= read -r line; do
        echo "$line" >> "$t"
        case "$line" in
            "(push 1)")
                if [ "$FAKE_BW_MODE" = session_noscope ]; then
                    echo "[error] push is not supported" >&2
                    exit 1
                fi
                ;;
            "(check-sat)")
                checks=$((checks + 1))
                if [ "$die_after" -ge 0 ] && [ "$checks" -gt "$die_after" ]; then
                    echo "[error] session died" >&2
                    exit 1
                fi
                case "$FAKE_BW_MODE" in
                    garbage) echo "what is this" ;;
                    session_error_line) echo '(error "boom")' ;;
                    hedge_slow_unsat) if [ "$checks" -eq 3 ]; then nap 8; fi; echo unsat ;;
                    hedge_slow_sat) if [ "$checks" -eq 3 ]; then nap 8; echo sat; else echo unsat; fi ;;
                    hedge_win) if [ "$checks" -eq 3 ]; then nap 0.3; fi; echo unsat ;;
                    sat) echo sat ;;
                    unknown|session_unknown) echo unknown ;;
                    *) echo unsat ;;
                esac
                ;;
            "(echo "*)
                msg="${line#\(echo }"
                echo "${msg%)}"
                ;;
        esac
    done
    exit 0
fi
# `sat` and a model: every scalar parameter symbol (`p<k>`) and aggregate
# field symbol (`p<k>f<slot>s<sort>`) the query declares gets `$1`, except a
# slot-0 field (an enum's discriminant), which gets `$2`. A Bool is true unless 0.
# A float is +0.0.
emit_model() {
    echo sat
    echo "("
    grep -oE '^\(declare-const p[0-9]+(f[0-9]+s[0-9]+)? (Bool|\(_ BitVec [0-9]+\))\)' "$file" \
        | sed -E 's/^\(declare-const (p[0-9]+(f[0-9]+s[0-9]+)?) (Bool|\(_ BitVec ([0-9]+)\))\)/\1 \4/' | while read -r name width; do
        v=$1
        case "$name" in p*f0s*) v=$2 ;; esac
        if [ -z "$width" ]; then
            if [ "$v" -ne 0 ]; then echo "  ($name true)"; else echo "  ($name false)"; fi
        elif [ "$width" -gt 64 ]; then
            printf '  (%s #x%0*x)\n' "$name" $((width / 4)) "$v"
        else
            echo "  ($name (_ bv$v $width))"
        fi
    done
    grep -oE '^\(declare-const p[0-9]+ \(_ FloatingPoint [0-9]+ [0-9]+\)\)' "$file" \
        | sed -E 's/^\(declare-const (p[0-9]+) \(_ FloatingPoint ([0-9]+) ([0-9]+)\)\)/\1 \2 \3/' | while read -r name eb sb; do
        printf '  (%s (fp #b0 #b%s #b%s))\n' "$name" "$(printf '%0*d' "$eb" 0)" "$(printf '%0*d' $((sb - 1)) 0)"
    done
    echo ")"
}
claim "$FAKE_BW_DIR/.slot"
n=$claimed
cp "$file" "$FAKE_BW_DIR/q.$n.smt2"
echo "$file" >> "$FAKE_BW_DIR/paths"
echo "$*" >> "$FAKE_BW_DIR/args"
case "$FAKE_BW_MODE" in
    unsat|session_unknown|session_die_after_*|session_error_line|session_noscope|hedge_slow_unsat) echo unsat ;;
    hedge_slow_sat) emit_model 7 7 ;;
    hedge_win) echo $$ >> "$FAKE_BW_DIR/pids"; nap 8; echo unsat ;;
    unknown) echo unknown ;;
    garbage) echo "what is this" ;;
    exit1) echo "[error] boom" >&2; exit 1 ;;
    hang) echo $$ > "$FAKE_BW_DIR/pid"; exec sleep 29 ;;
    sat_empty) printf 'sat\n(\n)\n' ;;
    oom_msg) echo "(error) Out of memory" >&2; exit 1 ;;
    kill_worker) echo $$ >> "$FAKE_BW_DIR/pids"; kill -9 "$PPID"; sleep 5 ;;
    crash_worker) kill -6 "$PPID"; sleep 5 ;;
    mixed)
        echo $$ >> "$FAKE_BW_DIR/pids"
        h=$(cksum < "$file" | cut -d' ' -f1)
        sleep "0.$((h % 4))"
        if [ $((h % 5)) -eq 0 ]; then echo unknown; else echo unsat; fi
        ;;
    hang_tree)
        echo "$PPID" >> "$FAKE_BW_DIR/worker_pids"
        echo $$ >> "$FAKE_BW_DIR/pids"
        sleep 29 &
        echo $! >> "$FAKE_BW_DIR/grandchild_pids"
        wait
        ;;
    slow_unsat) echo $$ >> "$FAKE_BW_DIR/pids"; sleep 0.3; echo unsat ;;
    record_unsat) echo "$PPID" >> "$FAKE_BW_DIR/worker_ppids"; echo unsat ;;
    copy_ir)
        # keeps the module file the worker (our parent) was told to decode.
        if [ ! -s "$FAKE_BW_DIR/ir.bin" ]; then
            ir=$(tr '\0' '\n' < "/proc/$PPID/cmdline" | grep -x -A1 -- '--worker-ir' | tail -1)
            [ -z "$ir" ] || cp "$ir" "$FAKE_BW_DIR/ir.bin"
        fi
        echo unsat
        ;;
    kill_once_3)
        # the third query of the run kills its worker, once: a retry succeeds.
        echo "$PPID" >> "$FAKE_BW_DIR/worker_ppids"
        if [ "$n" -eq 3 ] && first_to "$FAKE_BW_DIR/.killed"; then kill -9 "$PPID"; sleep 5; fi
        echo unsat
        ;;
    kill_from_3)
        # every query from the third on kills its worker.
        echo "$PPID" >> "$FAKE_BW_DIR/worker_ppids"
        if [ "$n" -ge 3 ]; then kill -9 "$PPID"; sleep 5; fi
        echo unsat
        ;;
    hang_once_3)
        # the third query of the run hangs, once, with a solver child of its own.
        echo "$PPID" >> "$FAKE_BW_DIR/worker_pids"
        if [ "$n" -eq 3 ] && first_to "$FAKE_BW_DIR/.hung"; then
            echo $$ >> "$FAKE_BW_DIR/pids"
            sleep 29 &
            echo $! >> "$FAKE_BW_DIR/grandchild_pids"
            wait
        fi
        echo unsat
        ;;
    unknown_unwinding)
        # `unsat` for a claim whose goal is a negation (an ensures or invariant),
        # `unknown` for the unwinding claim, whose query ends in the bare path
        # condition.
        last=$(grep '^(assert' "$file" | tail -1)
        case "$last" in *"(not "*) echo unsat ;; *) echo unknown ;; esac
        ;;
    sat_if_contains|sat_unless_contains)
        hit=0
        grep -qF -- "$FAKE_BW_NEEDLE" "$file" && hit=1
        want=1
        [ "$FAKE_BW_MODE" = sat_unless_contains ] && want=0
        if [ "$hit" -ne "$want" ]; then echo unsat; exit 0; fi
        emit_model 7 7
        ;;
    sat|sat_zero)
        val=7
        [ "$FAKE_BW_MODE" = sat_zero ] && val=0
        emit_model "$val" "$val"
        ;;
    sat_tag_some) emit_model 0 1 ;;
    sat_tag_9) emit_model 0 9 ;;
    *) echo "unknown fake mode" >&2; exit 2 ;;
esac
