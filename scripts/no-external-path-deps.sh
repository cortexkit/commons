#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "$script_dir/.." && pwd -P)"

if (($# > 1)); then
  echo "Usage: $0 [manifest-path]" >&2
  exit 2
fi

manifest_args=()
if (($# == 1)); then
  manifest_args=(--manifest-path "$1")
else
  manifest_args=(--manifest-path "$repo_root/Cargo.toml")
fi

metadata_file="$(mktemp)"
trap 'rm -f -- "$metadata_file"' EXIT

if cargo metadata --format-version 1 --locked "${manifest_args[@]}" >"$metadata_file"; then
  :
else
  metadata_status=$?
  echo "The check itself failed: cargo metadata exited with status $metadata_status." >&2
  exit 1
fi

if python3 - "$metadata_file" <<'PY'
import json
import os
import sys

with open(sys.argv[1], encoding="utf-8") as metadata_file:
    metadata = json.load(metadata_file)
packages = metadata["packages"]
path_packages = [package for package in packages if package.get("source") is None]
print(f"Examined {len(packages)} packages; {len(path_packages)} path packages.")
if not packages:
    print("The check examined zero packages.", file=sys.stderr)
    sys.exit(1)

workspace_root = os.path.realpath(metadata["workspace_root"])
offenders = []

for package in path_packages:
    manifest_path = os.path.realpath(package["manifest_path"])
    try:
        is_inside_workspace = os.path.commonpath((workspace_root, manifest_path)) == workspace_root
    except ValueError:
        is_inside_workspace = False

    if not is_inside_workspace:
        offenders.append((package["name"], manifest_path))

if offenders:
    for name, manifest_path in offenders:
        print(f"External path dependency: {name} ({manifest_path})", file=sys.stderr)
    sys.exit(3)

print("No external path dependencies found.")
PY
then
  :
else
  parser_status=$?
  if ((parser_status == 3)); then
    exit 3
  fi
  echo "The check itself failed: metadata parser exited with status $parser_status." >&2
  exit 1
fi
