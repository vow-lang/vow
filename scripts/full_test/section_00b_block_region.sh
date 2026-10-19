# shellcheck shell=bash
# Sourced by scripts/full_test.sh: shares its globals and helpers; not runnable on its own.

# ─── Section 0b: Concrete block-region parity ──────────────────────

section_begin "Section 0b: Concrete Block-Region Parity"
rust_ir="$TMPDIR/compiler_rust.ir"
self_ir="$TMPDIR/compiler_self.ir"
rust_ir_err="$TMPDIR/compiler_rust_ir.err"
self_ir_err="$TMPDIR/compiler_self_ir.err"
if "$RUST" build --no-verify --dump-ir compiler/main.vow >"$rust_ir" 2>"$rust_ir_err" \
    && run_self build --no-verify --dump-ir compiler/main.vow >"$self_ir" 2>"$self_ir_err"; then
    if python3 - "$rust_ir" "$self_ir" <<'PY'
import re
import sys

inst_re = re.compile(r'%([0-9]+) = RegionAlloc.*<region=block_([0-9]+)>')

def collect(path):
    out = {}
    func = None
    with open(path, encoding='utf-8') as fh:
        for line in fh:
            if line.startswith('fn '):
                func = line[3:].split('(', 1)[0].strip()
                continue
            match = inst_re.search(line)
            if match and func is not None:
                out[(func, int(match.group(1)))] = int(match.group(2))
    return out

rust = collect(sys.argv[1])
self_hosted = collect(sys.argv[2])
if rust == self_hosted:
    print(f'OK ({len(rust)} concrete block placements)')
    sys.exit(0)

missing = sorted(set(rust) - set(self_hosted))
extra = sorted(set(self_hosted) - set(rust))
mismatched = sorted(k for k in set(rust) & set(self_hosted) if rust[k] != self_hosted[k])
parts = []
if missing:
    parts.append('missing in self: ' + ', '.join(f'{f}%{i}->block_{rust[(f, i)]}' for f, i in missing[:8]))
if extra:
    parts.append('extra in self: ' + ', '.join(f'{f}%{i}->block_{self_hosted[(f, i)]}' for f, i in extra[:8]))
if mismatched:
    parts.append('mismatch: ' + ', '.join(
        f'{f}%{i}: rust block_{rust[(f, i)]} vs self block_{self_hosted[(f, i)]}'
        for f, i in mismatched[:8]
    ))
print('; '.join(parts))
sys.exit(1)
PY
    then
        pass "compiler/concrete-block-region-parity"
    else
        fail "compiler/concrete-block-region-parity" "concrete block placements differ"
    fi
else
    fail "compiler/concrete-block-region-parity" "failed to dump compiler IR"
fi
