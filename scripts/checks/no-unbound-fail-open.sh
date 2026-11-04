#!/usr/bin/env bash
# Refuse shell patterns that let a nounset script exit 0 without finishing.
# On bash 3.2 (macOS /bin/bash), an unbound variable under `set -euo pipefail`
# with an EXIT trap set exits with status 0, so a gate can pass having run nothing.
# This is a pattern check, not shell analysis: it recognises the forms below.
set -euo pipefail

if (($# > 1)); then
    echo "Usage: $0 [repo-root]" >&2
    exit 2
fi
repo_root=${1:-"$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"}

python3 - "$repo_root" <<'PY'
from pathlib import Path
import re
import shlex
import subprocess
import sys

root = Path(sys.argv[1])
paths = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z"]).split(b"\0")
array = re.compile(r"\$\{[A-Za-z_][A-Za-z_0-9]*\[[@*]\]\}")
safe_array = re.compile(
    r'\$\{(?P<name>[A-Za-z_][A-Za-z_0-9]*)\[(?P<kind>[@*])\]\+"'
    r'\$\{(?P=name)\[(?P=kind)\]\}"\}'
)
set_command = re.compile(r"(?:^|[;\n])[ \t]*set[ \t]+([^;\n]*)")
trap_command = re.compile(
    r'''(?:^|[;\n])[ \t]*trap[ \t]+(?:--[ \t]+)?'''
    r'''(?P<action>'[^']*'|"(?:\\.|[^"\\])*"|[^\s;]+)[ \t]+(?:EXIT|0)\b'''
)
finished_read = re.compile(r"\$(?:finished\b|\{finished(?:\}|:|[-+?=]))")
finished_assignment = re.compile(r"(?:^|[;\n])[ \t]*finished=1(?:[ \t;\n]|$)")


def sets_nounset(source):
    for command in set_command.finditer(source):
        words = shlex.split(command[1], comments=True)
        for index, word in enumerate(words):
            if re.fullmatch(r"-[A-Za-z]*u[A-Za-z]*", word):
                return True
            if word == "-o" and words[index + 1:index + 2] == ["nounset"]:
                return True
    return False


def trap_reads_finished(action, source):
    if finished_read.search(action):
        return True
    # Named handlers keep cleanup readable; inspect the function the trap calls.
    handler = re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", action)
    if handler:
        body = re.search(
            r"(?m)^" + re.escape(action) + r"\(\)[ \t]*\{\n(.*?)^\}",
            source,
            re.DOTALL,
        )
        return body is not None and finished_read.search(body[1]) is not None
    return False


def shell_source(source):
    # Blank the bodies of quoted heredocs whose delimiter matches the simple
    # pattern below: they are literal data, not shell commands or expansions.
    lines = []
    delimiter = None
    for line in source.splitlines(keepends=True):
        if delimiter is not None:
            if line.strip() == delimiter:
                delimiter = None
            lines.append(re.sub(r"[^\n]", " ", line))
            continue
        heredoc = re.search(r'''<<-?[ \t]*(['"])([A-Za-z_0-9]+)\1''', line)
        if heredoc:
            delimiter = heredoc[2]
        lines.append(re.sub(r"^[ \t]*#[^\n]*", lambda m: " " * len(m[0]), line))
    return "".join(lines)


checked = 0
violations = 0
for raw_path in paths:
    if not raw_path:
        continue
    path = raw_path.decode()
    file = root / path
    if not file.is_file():
        continue
    # Scan every tracked .sh and .test path, and extensionless paths under
    # scripts/ and .github/actions/ whose shebang names bash or sh.
    if not path.endswith((".sh", ".test")):
        if not path.startswith(("scripts/", ".github/actions/")):
            continue
        with file.open("rb") as stream:
            shebang = stream.readline()
        if not re.match(rb"#![^\n]*\b(?:bash|sh)\b", shebang):
            continue
    # Leave offsets intact so diagnostics still name the original source line.
    source = shell_source(file.read_text())
    if not sets_nounset(source):
        continue
    checked += 1

    def complain(offset, message):
        global violations
        violations += 1
        line = source.count("\n", 0, offset) + 1
        print(f"{path}:{line}: {message}", file=sys.stderr)

    safe_spans = [match.span() for match in safe_array.finditer(source)]
    for match in array.finditer(source):
        if any(start <= match.start() and match.end() <= end for start, end in safe_spans):
            continue
        line_end = source.find("\n", match.end())
        suffix = source[match.end():line_end if line_end != -1 else len(source)]
        if re.search(r"#\s*nonempty-array\b", suffix):
            continue
        complain(match.start(), f"unguarded whole-array expansion: {match[0]}")

    for trap in trap_command.finditer(source):
        offset = source.index("trap", trap.start(), trap.end())
        if not finished_assignment.search(source):
            complain(offset, "EXIT trap set, but no `finished=1` assignment found in this file")
        action = shlex.split(trap["action"])[0]
        if not trap_reads_finished(action, source):
            complain(offset, "EXIT trap set, but its action or handler never reads `finished`")

if violations:
    sys.exit(1)
print(f"No hazards this check recognises ({checked} nounset files checked).")
PY
