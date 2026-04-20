ESBMC ?= esbmc
UNWIND ?= 4

.PHONY: verify
verify:
	$(ESBMC) arena.c --unwind $(UNWIND) --no-bounds-check --no-pointer-check --64
