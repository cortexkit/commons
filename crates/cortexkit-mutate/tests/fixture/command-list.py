import json
import re
import sys
from pathlib import Path


NAMES = [
    "pi slot published import graph permits only node builtins, common auth files and external Windows private-files",
    "prefix pi slot published import graph contains only node builtins and common auth files",
    "Pi slot published import graph contains only node builtins and common auth files",
]


mode, pattern = sys.argv[1:]
selected = [name for name in NAMES if re.search(pattern, name)]
if mode == "list":
    print("listing selected tests", file=sys.stderr)
    print(json.dumps(selected))
else:
    print(f"Ran {len(selected)} tests")
    enabled = "ENABLED = True" in Path("guard.py").read_text()
    sys.exit(0 if enabled else 1)
