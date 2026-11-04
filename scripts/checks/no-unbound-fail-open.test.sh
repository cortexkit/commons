#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
python3 - "$script_dir/no-unbound-fail-open.sh" "$BASH" <<'PY'
from pathlib import Path
import subprocess
import sys
import tempfile

checker = Path(sys.argv[1])
bash = sys.argv[2]
scratch = checker.parents[2] / "target" / "scratch"
scratch.mkdir(parents=True, exist_ok=True)
tests = 0


def run(*args, **kwargs):
    return subprocess.run(args, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kwargs)


with tempfile.TemporaryDirectory(dir=scratch, prefix="unbound-check-") as directory:
    root = Path(directory)
    run("git", "init", "-q", str(root)).check_returncode()

    def fixture(name, source, expected=0, diagnostic=None, path="fixture.sh"):
        global tests
        run("git", "-C", str(root), "rm", "-rf", "--ignore-unmatch", ".").check_returncode()
        file = root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        # Placeholders keep intentionally unsafe fixture text out of this script's scan.
        file.write_text(source.replace("DOLLAR", "$"))
        run("git", "-C", str(root), "add", path).check_returncode()
        result = run(bash, str(checker), str(root))
        assert result.returncode == expected, f"{name}: exit {result.returncode}\n{result.stdout}"
        if diagnostic:
            assert diagnostic in result.stdout, f"{name}: missing {diagnostic!r}\n{result.stdout}"
        tests += 1
        print(f"PASS: {name}")
        return file

    for expansion in ('"DOLLAR{items[@]}"', 'DOLLAR{items[@]}', '"DOLLAR{items[*]}"', 'DOLLAR{items[*]}'):
        fixture("bare expansion rejected: " + expansion, "set -euo pipefail\nitems=()\necho " + expansion + "\n",
                1, "fixture.sh:3: unguarded whole-array expansion")

    fixture("checker accepts guarded whole-array forms", '''set -euo pipefail
items=()
args=(DOLLAR{items[@]+"DOLLAR{items[@]}"})
echo DOLLAR{items[*]+"DOLLAR{items[*]}"}
echo DOLLAR{#items[@]}
''')
    fixture("nonempty annotation accepted", '''set -eu
items=(literal)
echo "DOLLAR{items[@]}" # nonempty-array assigned a literal element
''')
    fixture("without nounset accepted", 'set -eo pipefail\nitems=()\necho "DOLLAR{items[@]}"\n')
    fixture("quoted heredoc data ignored", '''set -u
cat <<'LITERAL'
trap true EXIT
echo "DOLLAR{items[@]}"
LITERAL
''')
    fixture("nounset long option detected", 'set -o nounset\necho "DOLLAR{items[@]}"\n',
            1, "fixture.sh:2: unguarded whole-array expansion")
    fixture("split nounset flags detected", 'set -e -u\necho "DOLLAR{items[@]}"\n',
            1, "fixture.sh:2: unguarded whole-array expansion")
    fixture("extensionless tooling scanned", '#!/usr/bin/env bash\nset -u\necho "DOLLAR{items[@]}"\n',
            1, "scripts/tool:3:", "scripts/tool")
    fixture("shell test suffix scanned", 'set -u\necho "DOLLAR{items[@]}"\n',
            1, "fixture.test:2:", "fixture.test")
    fixture("action shell script scanned", '#!/bin/bash\nset -u\necho "DOLLAR{items[@]}"\n',
            1, ".github/actions/check/run:3:", ".github/actions/check/run")
    fixture("unguarded EXIT trap rejected", "set -euo pipefail\ntrap 'true' EXIT\n",
            1, "fixture.sh:2: EXIT trap set, but no `finished=1` assignment")
    fixture("assignment without trap read rejected", "set -u\ntrap 'true' EXIT\nfinished=1\n",
            1, "never reads `finished`")
    fixture("trap read without assignment rejected", '''set -u
trap 'test "DOLLAR{finished:-}" = 1' EXIT
''', 1, "EXIT trap set, but no `finished=1` assignment")

    guarded = '''set -euo pipefail
finished=
cleanup() {
    rc=DOLLAR?
    echo cleaned
    if [ "DOLLARrc" -eq 0 ] && [ "DOLLAR{finished:-}" != 1 ]; then
        echo 'fixture: exited before the check completed' >&2
        exit 1
    fi
    exit "DOLLARrc"
}
trap cleanup EXIT
BODY
finished=1
'''
    file = fixture("guarded scalar abort accepted by checker", guarded.replace("BODY", "echo DOLLAR{undefined_var}"))
    result = run(bash, str(file))
    assert result.returncode != 0, f"scalar abort passed\n{result.stdout}"
    assert "cleaned" in result.stdout and "undefined_var" in result.stdout, result.stdout
    tests += 1
    print("PASS: guarded unbound scalar fails and runs cleanup")

    file = fixture("guarded success accepted", guarded.replace("BODY", "true"))
    result = run(bash, str(file))
    assert result.returncode == 0 and "cleaned" in result.stdout, result.stdout
    tests += 1
    print("PASS: completed guard preserves success and cleanup")

    file = fixture("guarded failure accepted", guarded.replace("BODY", "exit 7"))
    result = run(bash, str(file))
    assert result.returncode == 7 and "cleaned" in result.stdout, result.stdout
    tests += 1
    print("PASS: guard preserves nonzero status and cleanup")

    fixture("inline EXIT guard accepted", '''set -euo pipefail
finished=
trap 'rc=DOLLAR?; if [ "DOLLARrc" -eq 0 ] && [ "DOLLAR{finished:-}" != 1 ]; then exit 1; fi; exit "DOLLARrc"' EXIT
finished=1
''')

    # Without a guard, an unbound scalar under `set -euo pipefail` with an EXIT
    # trap exits 0 on bash 3.2 but nonzero on bash 5. Record which this bash
    # does, so the gate check below asserts what this bash can show.
    baseline = run(bash, "-c", "set -euo pipefail; trap true EXIT; echo DOLLAR{undefined_var}".replace("DOLLAR", "$"))
    unguarded_passes = baseline.returncode == 0
    print(f"unguarded unbound scalar exits {baseline.returncode} under {bash}")

    gate_source = (checker.parents[1] / "gate.sh").read_text()
    trap_line = "trap stage_verdict EXIT\n"
    assert trap_line in gate_source, "gate EXIT trap was not found"
    file = fixture("gate completion guard present", gate_source.replace(
        trap_line, trap_line + "echo DOLLAR{undefined_var}\n"), path="scripts/gate.sh")
    result = run(bash, str(file), "fast")
    assert result.returncode != 0, f"gate scalar abort passed\n{result.stdout}"
    if unguarded_passes:
        assert "exited before the gate stage completed" in result.stdout, result.stdout
    assert "gate fast: PASS" not in result.stdout, result.stdout
    tests += 1
    print("PASS: gate rejects an unbound scalar before finishing")

print(f"Passed {tests} unbound fail-open checks under {bash}.")
PY
