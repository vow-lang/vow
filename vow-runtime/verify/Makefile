ESBMC ?= esbmc
MAX_K_STEP ?= 10

# Matches the convention used by vow-verify/src/esbmc.rs: incremental BMC with
# --max-k-step as the iteration cap (no --unwind, which is unused in this mode).
.PHONY: verify
verify:
	$(ESBMC) arena.c --incremental-bmc --max-k-step $(MAX_K_STEP) \
	    --no-bounds-check --no-pointer-check --64
