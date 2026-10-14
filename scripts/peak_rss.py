"""Peak RSS of a measured process tree, including runs shorter than one sample.

Three readings bound a run's peak resident set:

  * `tree_kb`: the largest sampled sum of VmRSS over the process and its
    descendants. Needs the run to live across a sample.
  * `single_kb`: the largest kernel high-water mark (VmHWM) any sampled member
    reported. VmHWM is per address space and reset by exec, so one late read
    covers an allocation peak that happened between samples.
  * `ru_maxrss` of the reaped child. Linux seeds it with the parent's own
    high-water mark when the child is forked, so a value at or below the
    harness's own peak says nothing about the child.

The sampler runs a fast phase first (1 ms for the first FAST_WINDOW_S) because
the verifier runs the gate measures start at a few milliseconds.
"""

import functools
import os
import resource
import threading

import measure_build_tree_rss

FAST_INTERVAL_S = 0.001
FAST_WINDOW_S = 0.25
SLOW_INTERVAL_S = 0.02

SAMPLED = "sampled"
RU_MAXRSS = "ru_maxrss"
UPPER_BOUND = "ru_maxrss_upper_bound"


def read_rss_hwm(pid):
    """(VmRSS, VmHWM) in kB of a live pid, or None once it has gone."""
    rss = hwm = 0
    try:
        with open(f"/proc/{pid}/status", encoding="ascii", errors="replace") as fh:
            for line in fh:
                if line.startswith("VmRSS:"):
                    rss = int(line.split()[1])
                elif line.startswith("VmHWM:"):
                    hwm = int(line.split()[1])
    except OSError:
        return None
    return rss, hwm


@functools.cache
def children_listed():
    """True when the kernel exposes /proc/<pid>/task/<tid>/children."""
    return os.path.exists(f"/proc/self/task/{os.getpid()}/children")


def children_of(pid):
    """Direct children of `pid`; empty once it has gone."""
    found = []
    try:
        tids = os.listdir(f"/proc/{pid}/task")
    except OSError:
        return found
    for tid in tids:
        try:
            with open(f"/proc/{pid}/task/{tid}/children", encoding="ascii") as fh:
                found += [int(p) for p in fh.read().split()]
        except OSError:
            continue
    return found


def tree_members(root):
    """`root` and every descendant."""
    members, pending = [root], [root]
    while pending:
        kids = children_of(pending.pop())
        members += kids
        pending += kids
    return members


def sample_tree(root):
    """(sum of VmRSS, largest VmHWM) over the tree at this instant."""
    if not children_listed():
        tree, _ = measure_build_tree_rss.snapshot(root)
        own = read_rss_hwm(root)
        return tree, own[1] if own else 0
    total = single = 0
    for pid in tree_members(root):
        reading = read_rss_hwm(pid)
        if reading is not None:
            total += reading[0]
            single = max(single, reading[1])
    return total, single


def sample_loop(root, stop, peaks, clock):
    started = clock()
    while not stop.is_set():
        tree, single = sample_tree(root)
        peaks["tree"] = max(peaks["tree"], tree)
        peaks["single"] = max(peaks["single"], single)
        fast = clock() - started < FAST_WINDOW_S
        stop.wait(FAST_INTERVAL_S if fast else SLOW_INTERVAL_S)


class TreeSampler:
    """Samples the tree of `root` on a thread until `stop()`."""

    def __init__(self, root, clock):
        self.peaks = {"tree": 0, "single": 0}
        self._stop = threading.Event()
        self._thread = threading.Thread(
            target=sample_loop, args=(root, self._stop, self.peaks, clock)
        )
        self._thread.start()

    def stop(self):
        self._stop.set()
        self._thread.join()
        return self.peaks["tree"], self.peaks["single"]


def own_peak_kb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss


def trusted_maxrss(maxrss_kb, floor_kb):
    """`ru_maxrss` of a reaped child, or 0 when it cannot be told from the floor."""
    return maxrss_kb if maxrss_kb > floor_kb else 0


def resolve_peak(tree_kb, single_kb, maxrss_kb, floor_kb):
    """(peak kB, source) of one run; never 0.

    The sampled readings and a `ru_maxrss` above the floor are measurements.
    When none of them saw the run, `ru_maxrss` itself is still an upper bound
    (it is at least the child's true peak), so it is returned labelled as such:
    safe as a numerator of a "native is not worse" ratio, not as a denominator.
    """
    sampled = max(tree_kb, single_kb)
    trusted = trusted_maxrss(maxrss_kb, floor_kb)
    if sampled > 0 or trusted > 0:
        return max(sampled, trusted), SAMPLED if sampled >= trusted else RU_MAXRSS
    if maxrss_kb > 0:
        return maxrss_kb, UPPER_BOUND
    raise ValueError("no peak RSS reading: not sampled and ru_maxrss is 0")
