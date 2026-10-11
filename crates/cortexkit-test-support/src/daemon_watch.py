#!/usr/bin/env python3
"""Run a test daemon only while its harness owns the stdin pipe.

The launcher must give this watcher a new process group and a piped stdin.
The daemon never inherits the pipe. EOF (including harness SIGKILL), a signal,
or daemon exit tears down descendants. That includes modules the daemon placed in
their own process groups, which a signal to the watcher's group would not reach.
"""
import os
import select
import signal
import subprocess
import sys


def descendants(root):
    rows = subprocess.check_output(["ps", "-axo", "pid=,ppid=,pgid="], text=True)
    processes = [tuple(map(int, row.split())) for row in rows.splitlines()]
    owned = {root}
    while True:
        found = {pid for pid, parent, _ in processes if parent in owned}
        if found <= owned:
            break
        owned |= found
    return [(pid, group) for pid, _, group in processes if pid in owned]


def main():
    program = sys.argv[1]
    if not os.path.basename(program).startswith("ckdev-"):
        raise SystemExit(f"test executable must be named ckdev-*: {program}")
    if os.getpgrp() != os.getpid():
        raise SystemExit("test daemon watcher requires its own process group")
    stopping = False

    def stop(_signal, _frame):
        nonlocal stopping
        stopping = True

    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, stop)
    daemon = subprocess.Popen(sys.argv[1:], stdin=subprocess.DEVNULL)
    # Record the daemon's PID separately from the watcher returned by spawn,
    # so lifecycle tests can assert that the supervised process is gone.
    # Written to a temporary name and renamed, so a reader that sees the file
    # always sees the whole PID, never an empty file mid-write.
    if os.environ.get("CORTEXKIT_TEST_DAEMON_PID_FILE"):
        target = os.environ["CORTEXKIT_TEST_DAEMON_PID_FILE"]
        with open(target + ".tmp", "w") as pid_file:
            pid_file.write(str(daemon.pid))
        os.replace(target + ".tmp", target)
    # Retain descendants while the daemon is alive: a crashing daemon can
    # reparent its separately grouped modules before the next scan.
    owned = {}
    try:
        while not stopping:
            owned.update(descendants(daemon.pid))
            if daemon.poll() is not None:
                break
            ready, _, _ = select.select([sys.stdin.buffer], [], [], 0.1)
            if ready and not os.read(0, 4096):
                break
    finally:
        # Stop the daemon first so no module can be respawned during teardown.
        try:
            os.kill(daemon.pid, signal.SIGSTOP)
        except ProcessLookupError:
            pass
        current = dict(descendants(daemon.pid))
        # Only retain recorded PIDs still in their recorded group; do not kill
        # an unrelated process if the kernel has already reused a PID.
        live = {int(pid): int(group) for row in subprocess.check_output(
            ["ps", "-axo", "pid=,pgid="], text=True).splitlines()
            for pid, group in [row.split()]}
        current.update({pid: group for pid, group in owned.items() if live.get(pid) == group})
        for group in set(current.values()) - {os.getpgrp()}:
            try:
                os.killpg(group, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for pid in current:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        daemon.wait()
    return daemon.returncode if daemon.returncode >= 0 else 128 - daemon.returncode


if __name__ == "__main__":
    sys.exit(main())
