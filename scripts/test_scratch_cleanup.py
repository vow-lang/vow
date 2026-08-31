#!/usr/bin/env python3
"""Signal-cleanup tests for the shell scripts that allocate scratch trees.

`scripts/full_test.sh` and friends build multi-GB scratch trees under `mktemp
-d`. On hosts where `/tmp` is a tmpfs -- measured here at 63G of RAM -- a tree
abandoned by a killed run is abandoned memory, and killed runs are routine:
watchdogs, cancellations and daemon shutdowns all terminate the process group.

A `trap ... EXIT` does not fire on an untrapped SIGTERM, so the historical
EXIT-only traps leaked the whole tree on every kill. These tests pin the fix
from both directions: the real preamble is actually signalled and observed to
clean up, and a lint pass keeps the signal set from drifting back out of the
other scripts the way it once did.
"""

import os
import re
import signal
import subprocess
import time
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = REPO_ROOT / "scripts"

# Scripts that allocate a top-level scratch tree, and the variable holding it.
SCRATCH_SCRIPTS = {
    "full_test.sh": "TMPDIR",
    "cli_compat_test.sh": "TMPDIR",
    "measure_bootstrap_rss.sh": "scratch",
}

TRAP = re.compile(
    r"^\s*trap\s+(?P<body>'[^']*'|\"[^\"]*\")\s+(?P<signals>[A-Z0-9 ]+)$", re.M
)


def trap_signals(text):
    """Every signal trapped anywhere in a script, and those used for cleanup.

    Args:
        text: The full source of a shell script.

    Returns:
        tuple[set[str], set[str]]: All trapped signal names, and the subset
            trapped by a handler that removes something.
    """
    every, cleanup = set(), set()
    for m in TRAP.finditer(text):
        names = set(m.group("signals").split())
        every |= names
        if "rm " in m.group("body"):
            cleanup |= names
    return every, cleanup


def extract_preamble(text, var):
    """The scratch-allocation preamble, verbatim from a real script.

    Args:
        text: The full source of a shell script.
        var: Name of the variable the scratch directory is assigned to.

    Returns:
        str: Lines from the `mktemp -d` assignment through the last trap that
            immediately follows it.

    Raises:
        AssertionError: The script has no such assignment, or no trap block
            after it -- either of which is the regression under test.
    """
    lines = text.splitlines()
    start = next(
        (i for i, ln in enumerate(lines) if ln.startswith(f"{var}=$(mktemp -d")),
        None,
    )
    assert start is not None, f"no `{var}=$(mktemp -d` assignment"
    end = start
    for i in range(start + 1, len(lines)):
        if lines[i].lstrip().startswith("trap "):
            end = i
        elif lines[i].strip() and not lines[i].lstrip().startswith("#"):
            break
    assert end > start, f"no trap follows the {var} assignment"
    return "\n".join(lines[start : end + 1])


class SignalCleanupTest(unittest.TestCase):
    """Sends the real signal and looks at the filesystem afterwards."""

    def run_until_killed(self, script, sig):
        """Start a script that prints its scratch dir, signal it, and reap it.

        The signal goes to the whole process group, which is what a watchdog,
        a cancellation or a daemon shutdown actually does. It also matters for
        the outcome: bash defers a trap until the running foreground command
        returns, so signalling the shell alone would leave `sleep` running and
        the cleanup pending until it finished.

        Args:
            script: Shell source that echoes the scratch path then sleeps.
            sig: The signal to deliver.

        Returns:
            tuple[Path, int]: The scratch directory it created, and its exit
                status as reported by the shell.
        """
        proc = subprocess.Popen(
            ["bash", "-c", script],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            start_new_session=True,
        )
        try:
            path = Path(proc.stdout.readline().strip())
            # The dir must exist before the signal, or the test proves nothing.
            self.assertTrue(path.is_dir(), f"{path} was never created")
            # And the foreground child must already be running. Signalling the
            # group in the window before bash forks it delivers to bash only,
            # leaving the trap pending until the child it forks next finishes --
            # a race that makes this test hang rather than fail.
            self.await_foreground_child(proc.pid)
            os.killpg(proc.pid, sig)
            proc.wait(timeout=10)
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait(timeout=10)
            proc.stdout.close()
        return path, proc.returncode

    def await_foreground_child(self, pid, timeout=5):
        """Block until the shell has forked its foreground command.

        Args:
            pid: The shell's process id.
            timeout: Seconds to wait before giving up.
        """
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            kids = subprocess.run(
                ["ps", "-o", "pid", "--ppid", str(pid), "--no-headers"],
                capture_output=True,
                text=True,
            ).stdout.split()
            if kids:
                return
            time.sleep(0.02)
        self.fail(f"pid {pid} never forked a foreground child")

    def check(self, name, var, sig, expected_status):
        text = (SCRIPTS / name).read_text()
        script = extract_preamble(text, var) + f'\necho "${var}"\nsleep 30\n'

        path, status = self.run_until_killed(script, sig)

        self.assertFalse(path.exists(), f"{name}: {path} survived {sig.name}")
        self.assertEqual(expected_status, status, f"{name}: wrong exit status")

    def test_sigterm_removes_the_scratch_tree(self):
        for name, var in SCRATCH_SCRIPTS.items():
            with self.subTest(script=name):
                self.check(name, var, signal.SIGTERM, 143)

    def test_sigint_removes_the_scratch_tree(self):
        for name, var in SCRATCH_SCRIPTS.items():
            with self.subTest(script=name):
                self.check(name, var, signal.SIGINT, 130)

    def test_sighup_removes_the_scratch_tree(self):
        for name, var in SCRATCH_SCRIPTS.items():
            with self.subTest(script=name):
                self.check(name, var, signal.SIGHUP, 129)

    def test_the_handler_exits_rather_than_resuming(self):
        # A bash signal handler resumes the interrupted script unless it exits,
        # so a handler that only removed the tree would leave the run going
        # against a deleted scratch dir -- worse than the leak it fixed.
        text = (SCRIPTS / "full_test.sh").read_text()
        script = extract_preamble(text, "TMPDIR") + (
            '\necho "$TMPDIR"\nsleep 30\necho RESUMED\n'
        )
        proc = subprocess.Popen(
            ["bash", "-c", script],
            stdout=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        try:
            proc.stdout.readline()
            self.await_foreground_child(proc.pid)
            os.killpg(proc.pid, signal.SIGTERM)
            rest = proc.stdout.read()
            proc.wait(timeout=10)
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait(timeout=10)
            proc.stdout.close()

        self.assertNotIn("RESUMED", rest)


class TrapLintTest(unittest.TestCase):
    """Keeps the signal set from drifting back out of any script."""

    def shell_scripts(self):
        return sorted(SCRIPTS.glob("*.sh"))

    def test_every_cleanup_trap_covers_the_kill_signals(self):
        offenders = []
        for path in self.shell_scripts():
            every, cleanup = trap_signals(path.read_text())
            if "EXIT" not in cleanup:
                continue
            missing = {"INT", "TERM", "HUP"} - every
            if missing:
                offenders.append(f"{path.name}: missing {sorted(missing)}")

        self.assertEqual([], offenders)

    def test_the_lint_has_something_to_check(self):
        # Guards against the rule passing because it matched no trap at all.
        with_cleanup = [
            p.name
            for p in self.shell_scripts()
            if "EXIT" in trap_signals(p.read_text())[1]
        ]

        self.assertGreaterEqual(len(with_cleanup), 4, with_cleanup)

    def test_no_script_writes_scratch_straight_into_slash_tmp(self):
        # /tmp is a tmpfs on the development host, so an untrapped file there is
        # abandoned RAM. Scratch paths belong under a trapped scratch dir.
        offenders = []
        for path in self.shell_scripts():
            for m in re.finditer(r'mktemp[^\n]*"(/tmp/[^"]*)"', path.read_text()):
                offenders.append(f"{path.name}: {m.group(1)}")

        self.assertEqual([], offenders)


if __name__ == "__main__":
    unittest.main()
