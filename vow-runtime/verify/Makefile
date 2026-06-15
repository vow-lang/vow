ESBMC ?= esbmc

# The reachable chunk chain is shallow (1 initial + up to 2 from the symbolic
# loop + the directed #391 big+tail), so a tight single-shot --unwind is both
# sufficient and far cheaper than incremental BMC. --incremental-bmc rebuilds the
# whole BMC/symex/SMT context per k step (20 independent runs for k=1..10) and
# overwrites --unwind with k_step, over-unwinding the chunk-walk loops to 10. On
# this cast-heavy --64 harness that peaked at ~25 GB RSS / ~1h before being killed
# (issue #516). --unwind 5 is the smallest bound that covers the chain
# (arena_try_free_oversized_chunk can walk all 5 chunks); 4 fails its unwinding
# assertion.
#
# Solver choice matters a lot here: Bitwuzla (the ESBMC default) balloons to
# ~4.5 GB on this bitvector/points-to-heavy formula at --unwind 5, and Z3 does
# not finish; Boolector proves it in ~90s at ~290 MB RSS — comfortably inside the
# 2 GB ulimit full_test.sh runs verification under (#546).
UNWIND ?= 5

.PHONY: verify
verify:
	$(ESBMC) arena.c --unwind $(UNWIND) \
	    --no-bounds-check --no-pointer-check --64 --boolector
