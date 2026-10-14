# ADR-2026-10-13-2212: Native verify workers are sessions over a task queue

Status: accepted (issue #1683, epic #1398)

Addendum to
[ADR-2026-10-08-1430](2026-10-08-1430-native-verifier-cli-and-status-surface.md)
(D11, the per-function worker) and to
[ADR-2026-10-11-1500-native-contract-probes-and-pool-independence](2026-10-11-1500-native-contract-probes-and-pool-independence.md)
("a worker that runs several tasks is a follow-up if the performance gate flags
it"). Scoped exception of
[ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md): the
native verifier has no Rust twin.

## Context

Every `verify-worker` lowered the whole program (lex, parse, type check, lower,
region inference over every reachable module) to decide one function, and the
parent started one worker per verify target. Verification cost was therefore
`targets x frontend(program)`. On `compiler/main.vow` (about 51k lines, 31
functions proven by a worker) a worker spent 13.4 s of CPU on the frontend
before it asked a solver anything: 415 s of the 484 s the P5 gate run measured,
1.79x ESBMC's wall time, while the solver is irrelevant to the number.

## Decision

1. **A worker is a session over a queue of tasks.** The pool starts at most
   `--verify-jobs` workers, each of which lowers the program once and then
   decides tasks one after another (`compiler/vc_task.vow`,
   `vowc verify-worker <src> --worker-session <dir> --worker-id <w>`). A task is
   what a worker used to be asked: the function's own contract, or one of the
   two weak-contract probes of `contracts` (task kinds unchanged). A session is
   only started while there are more unclaimed tasks than sessions, so the pool
   never grows past the work that is left, and a session costs one frontend.
2. **The queue is a directory, and a rename is the only synchronisation**
   (`compiler/vc_queue.vow`). Unclaimed task `k` is the file `q/k`. A worker
   claims it by renaming it to `s<w>/claim.k`; the rename is atomic and exactly
   one racing worker wins, so no lock or shared memory is needed and the claim
   is also the ownership record: a worker that dies right after claiming still
   leaves its task attributed to it, with no window where the task is claimed by
   nobody. The answer is the `vc_worker` wire format written under a temporary
   name and renamed to `res/k`, so it is whole or absent. The claim file stays
   until the parent has taken the answer over (its removal is the
   acknowledgement), so a task that starts and finishes between two looks of the
   parent is still attributed. The task table (function index, name and kind per
   task) is written once as `tasks`; a worker checks every task against its own
   lowering before it claims anything and exits 3, as the one-function worker
   did, when the program is not the one the parent planned.
3. **D11 is kept per task, not per process.** A session runs under the same
   address-space cap in its own process group. The parent starts the clock of
   `--timeout` for a task when it first sees the claim, and on expiry kills the
   whole group of that session: that task is `timeout`, and every task the
   session had not reached is still on the queue for another session (a
   replacement is started when tasks remain). A session that dies on a task
   costs that task's verdict, classified as before (out of memory is `unknown`,
   anything else is `panicked`), using only the stderr printed after the task's
   marker line, so an earlier task's solver message cannot be read into a later
   death. The one-time frontend of a session is no longer part of any task's
   budget. A session that dies before it takes any task twice in a row fails the
   remaining tasks as `panicked` rather than starting sessions forever.
4. **An out-of-memory death after earlier tasks is retried once.** The address
   space cap now covers the frontend and every task a session has decided, so a
   task that ran out of memory in a session that had already decided others may
   only have been squeezed by what they left behind. It is put back on the
   queue once and decided by a fresh worker, which is exactly the process the
   one-process-per-function pool gave it. A death on a session's first task, or
   on the retry, is final. Verdicts are therefore those of the
   one-process-per-function pool: no verdict depends on which tasks shared a
   worker.
5. **`verify` still stops at the first function that is not proven.** The queue
   is in task order, a session may already have started the next task when the
   parent reads a failure, and that task is cancelled with its session: nothing
   above the first non-proven function is reported or waited for.
6. **The one-function `verify-worker --worker-index` form stays**, for the
   wiring tests and for ad-hoc use; it runs the same task body as a session.

## Isolation between tasks of one session

All tasks of a session start from the same lowered `IrModule`. The verifier
builds new functions from the target (inlining, loop closing, unrolling and the
two probe rewrites each return new values) and never writes into the module's
vectors. This is pinned by comparing the report of a program verified in one
session against the one-process-per-function pool over the whole
`tests/verify*` corpus (byte-identical stdout, stderr and exit code, 387
fixtures), and by the wiring tests of `tests/verify-native/tests.sh`.

## Measurements

`compiler/main.vow` (about 51k lines, 40 targets), `vowc verify --no-cache
--backend native`, one run per cell on a shared machine under the benchmark
lock; user CPU is the contention-free figure. RSS is the peak of the process
tree. The same sources, before (one process per function) and after (sessions):

| `--verify-jobs` | wall before | wall after | CPU before | CPU after | tree RSS before | tree RSS after |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 307 s | 22 to 28 s | 305 s | 22 to 28 s | 454 MB | 460 MB |
| 12 | 96 s | 48 s | 758 s | 339 s | 2.63 GB | 2.67 GB |

ESBMC verifies the same program in 271 s (4.05 GB) in the P5 gate run, so the
native path is now about a tenth of ESBMC's wall time at one job instead of
1.8x. The cost per worker is one frontend instead of one per function; peak
memory is unchanged because the frontend's footprint is per resident worker.
With 12 jobs the remaining cost is the 12 frontends that run at once.

The report is byte-identical (stdout, stderr and exit code) to the
one-process-per-function pool for `compiler/main.vow` and for all 387 fixtures
of `tests/verify*`, in both `verify` and `contracts` modes.

## Consequences

- Verification cost is `workers x frontend(program) + sum(solver time)`, with
  `workers <= --verify-jobs` plus one per retry or per timeout, instead of
  `targets x frontend(program)`.
- `--verify-jobs N` bounds both concurrency and the number of frontends (and
  so of resident copies of the lowered program) at once.
- A killed or timed-out task costs one more frontend in its replacement
  session. A run with many timeouts pays for them, as before.
- Orphans: a worker whose parent is killed from outside now runs until the queue
  is empty, not for one function. Each task is still bounded by `--timeout` in
  the worker's own budget, so the orphan is bounded by `tasks x --timeout`.
- `--timeout` no longer includes the worker's frontend. It was measured from the
  start of the worker, which made it a function of the program size under
  `--timeout 5` on a large program; it is now the budget of the function.
- Not changed: the wire format, the task kinds, the statuses and every report.
