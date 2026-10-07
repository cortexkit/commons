from pathlib import Path
import sys

sys.stdout.reconfigure(newline="\n")
mutated = Path("guard.py").read_bytes() == b"ENABLED = False\n"
assert Path(".git/command-log").exists(), "named baseline precedes JUnit baseline"
if mutated:
    assert "mutant " in Path(".git/command-log").read_text(), "named tests run first"
with Path(".git/broad-log").open("a", newline="\n") as log:
    log.write("mutant\n" if mutated else "baseline\n")
mode, report = sys.argv[1:]
if mutated:
    mode = mode.removesuffix("_mutant")
path = Path(report)
if mode == "stale":
    # Refuse to replace an old report: the runner must remove it before spawn.
    assert not path.exists(), "stale report was not deleted"
if mode == "missing":
    sys.exit(1)
if mode == "garbage":
    path.write_text("this is not XML", encoding="utf-8")
elif mode == "empty":
    path.write_text("<testsuites><testsuite/></testsuites>", encoding="utf-8")
else:
    classname = "Guard" if mode == "same" else "Other"
    failure = '<failure message="guard assertion failed">expected failure</failure>' if mutated else ""
    collateral = '<error message="collateral error &amp; message"><![CDATA[trace <details>]]></error>' if mutated and mode != "baseline_red" else ""
    preexisting = '<testcase classname="Preexisting" name="already red on clean tree"><failure message="preexisting failure"/></testcase>' if mode == "baseline_red" else ""
    path.write_text(f'''<?xml version="1.0" encoding="UTF-8"?>
<testsuites><testsuite name="package"><testsuite name="nested">
  <testcase classname="Guard" name="script.tests.flows_rig.RigChecks.test_guard">{failure}</testcase>
  <testcase classname="Guard" name="script.tests.flows_rig.RigChecks.test_other">{failure}</testcase>
  <testcase classname="{classname}" name="unlisted collateral &amp; failure">{collateral}</testcase>
  <testcase classname="Other" name="green companion"/>
  <testcase classname="Other" name="skipped companion"><skipped/></testcase>
  {preexisting}
</testsuite></testsuite></testsuites>
''', encoding="utf-8")
print("breadth command ran")
sys.exit(1 if mutated or mode == "baseline_red" else 0)
