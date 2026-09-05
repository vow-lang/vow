#!/usr/bin/env python3
"""Isolation primitives for running model-authored candidate programs (#1188).

Tier 3 (`scripts/pair_review.py`) compiles and executes `.vow` candidates a
model wrote, under a process that carries `ANTHROPIC_API_KEY`/
`OPENAI_API_KEY`. Vow effects are a declaration, not a capability gate:
nothing stops a candidate that declares `[io]` from calling
`process_run`/`fs_write`/`fs_remove`. These two helpers close the cheapest
half of that gap — env scrubbing and a disposable `cwd` — for the four
execution sites in `equivalence.py` and `verifier_runtime.py`. Full sandboxing
(filesystem allowlist, network denial, process-tree/CPU/fd limits) is
out of scope; see the issue for why it needs its own review.
"""

import os
import re
import tempfile

# Matches `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GITHUB_TOKEN`, `MY_APP_TOKEN`,
# etc. Deliberately narrow: only the two shapes the issue names. `PATH` and
# `VOW_CACHE_DIR` never match and need no special-casing.
_CREDENTIAL_NAME = re.compile(r"(?:^|_)(API_KEY|TOKEN)$", re.IGNORECASE)


def scrubbed_env(source=None):
    """A copy of `source` (default `os.environ`) minus credential-shaped vars."""
    source = os.environ if source is None else source
    return {k: v for k, v in source.items() if not _CREDENTIAL_NAME.search(k)}


def disposable_workdir():
    """A fresh, empty directory for a candidate-executed process's `cwd`."""
    return tempfile.TemporaryDirectory()
