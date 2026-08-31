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
# not finish. Boolector is the only one that fits, hence --boolector below.
#
# ESBMC version matters too, and not monotonically -- peak RSS across the
# versions this harness has been run under has gone both down and back up:
#
#   8.1.0   ~508 MiB
#   8.3.0   ~290 MiB
#   8.5.0   ~585 MiB
#
# All fit inside the 2 GB ulimit that scripts/verify_arena.sh enforces, but
# the trend is not in one direction, so the headroom cannot be assumed. That
# is what the CI run guards (#546, #747) -- and why the ESBMC pin is one of
# the paths that forces the arena proof to re-run, per scripts/ci_docs_only.py.
# The pin itself lives in .github/actions/install-esbmc/action.yml.
UNWIND ?= 5

.PHONY: verify
verify:
	$(ESBMC) arena.c --unwind $(UNWIND) \
	    --no-bounds-check --no-pointer-check --64 --boolector
