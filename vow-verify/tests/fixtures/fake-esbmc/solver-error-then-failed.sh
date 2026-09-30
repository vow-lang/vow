#!/bin/sh
echo "ERROR: SMT solver failed" >&2
echo "[Counterexample]"
echo ""
echo "Violated property:"
echo "  file /tmp/test.c line 1 column 1 function main"
echo "  vow:1"
echo ""
echo "VERIFICATION FAILED"
exit 1
