#!/usr/bin/env python3
"""Extract only stored session rows from checksum-pinned OpenCode captures.

Usage: python3 extract-opencode-v2.py [--reinjection] CAPTURE_ROOT [OUTPUT_DIRECTORY]
CAPTURE_ROOT contains versioned capture.json files, or runner-side
reinjection.json files with --reinjection. Source captures are not shipped.
"""

import argparse
import hashlib
import json
from pathlib import Path

SOURCES = {
    "2.0.26": ("9b4ec5714d481559990db0a816d5dec19541a814", "e093940553cf318c15d8b87015586e95e52bb133e4394bb2f5b34e050400a350"),
    "2.2.3": ("082ac6281fd395c2d0c17fd3c38214db2f89621f", "9a889986eb00906d6f0ef65dd761970486f23ac386d984515385ed5d1f2d68ec"),
}
REINJECTION_SHA256 = {
    "2.0.26": "97c0ba81844f568a506a7b92e9ad0570416c1455f2a1f48fce335525dc3c38db",
    "2.2.3": "7ede021d49bfeaad7320f76243c7e51ea5621173cdd3a1b608925e6437dec16f",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reinjection", action="store_true")
    parser.add_argument("capture_root", type=Path)
    parser.add_argument("output_directory", nargs="?", type=Path, default=Path(__file__).parent)
    args = parser.parse_args()
    stages = ("previous_marker", "before_cut", "current_marker", "after_injection",
              "between.before", "between.committed", "below.before", "below.committed") if args.reinjection else (
                  "before_cut", "after_cut", "marker_next")
    for version, (commit, checksum) in SOURCES.items():
        filename = "reinjection.json" if args.reinjection else "capture.json"
        if args.reinjection:
            checksum = REINJECTION_SHA256[version]
        raw = (args.capture_root / version / filename).read_bytes()
        if hashlib.sha256(raw).hexdigest() != checksum:
            raise ValueError(f"{version}: capture checksum mismatch")
        capture = json.loads(raw)
        if (capture["version"], capture["commit"]) != (version, commit):
            raise ValueError(f"{version}: capture source pin mismatch")
        result = {
            "version": version,
            "commit": commit,
            "snapshots": {
                stage: capture["stored_snapshots"][stage]["session_message"]
                for stage in stages
            },
        }
        prefix = "opencode-v2-reinjection-rows" if args.reinjection else "opencode-v2-rows"
        (args.output_directory / f"{prefix}-{version}.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n"
        )
        print(f"Extracted {version}: {len(stages)} snapshots, "
              f"{sum(len(rows) for rows in result['snapshots'].values())} stored rows")


if __name__ == "__main__":
    main()
