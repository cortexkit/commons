import os
from pathlib import Path
import signal
import sys
import time

# UTF-8 and LF on every platform: redirected to a file on Windows, Python
# would otherwise write the console code page and CRLF.
sys.stdout.reconfigure(newline="\n", encoding="utf-8")
# Read bytes rather than importing the guard, so every invocation observes the
# current source and creates no cached bytecode in the fixture tree.
mutated = Path("guard.py").read_bytes() == b"ENABLED = False\n"
test_id = sys.argv[1]
zero_tests = test_id == "zero_baseline" or (mutated and test_id == "zero_mutant")
print(f"Ran {0 if zero_tests else 1} tests", flush=True)
if test_id in ("zero_baseline", "zero_mutant"):
    sys.exit(3 if mutated else 0)
assert len(sys.argv) == 2
# Write \n explicitly: text mode on Windows would translate it to \r\n, and the
# tests compare this log byte for byte.
with Path(".git/command-log").open("a", newline="\n") as log:
    log.write(f"{'mutant' if mutated else 'baseline'} {test_id}\n")

if test_id == "invalid_utf8":
    # A tool whose output is not UTF-8: the grade must still come from the exit.
    if mutated:
        sys.stdout.flush()
        sys.stdout.buffer.write(b"failure \xff\xfe not utf-8\n")
        sys.stdout.buffer.flush()
    sys.exit(3 if mutated else 0)
if test_id == "always_red":
    sys.exit(3)
if test_id == "sleep_baseline" or (mutated and test_id == "sleep_mutant"):
    time.sleep(30)
if mutated and test_id in ("exit126", "exit127"):
    sys.exit(int(test_id[4:]))
if mutated and test_id == "signal":
    os.kill(os.getpid(), signal.SIGTERM)
if test_id == "script.tests.flows_rig.RigChecks.test_vacuous":
    sys.exit(0)
if test_id in (
    "script.tests.flows_rig.RigChecks.test_guard",
    "script.tests.flows_rig.RigChecks.test_other",
    "scoped route opens > cached routes isolate A & B — café",
):
    if mutated:
        print(f"guard assertion failed: {test_id}", flush=True)
    sys.exit(3 if mutated else 0)
if test_id in ("exit126", "exit127", "signal", "sleep_mutant"):
    sys.exit(0)
raise AssertionError(f"unknown test id: {test_id}")
