import os
from pathlib import Path
import signal
import sys
import time

# Read bytes rather than importing the guard, so every invocation observes the
# current source and creates no cached bytecode in the fixture tree.
mutated = Path("guard.py").read_bytes() == b"ENABLED = False\n"
test_id = sys.argv[1]
assert len(sys.argv) == 2
with Path(".git/command-log").open("a") as log:
    log.write(f"{'mutant' if mutated else 'baseline'} {test_id}\n")

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
):
    sys.exit(3 if mutated else 0)
if test_id in ("exit126", "exit127", "signal", "sleep_mutant"):
    sys.exit(0)
raise AssertionError(f"unknown test id: {test_id}")
