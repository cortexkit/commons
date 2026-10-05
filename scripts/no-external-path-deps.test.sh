#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "$script_dir/.." && pwd -P)"
checker="$script_dir/no-external-path-deps.sh"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT

write_package() {
  local directory="$1"
  local name="$2"
  mkdir -p "$directory/src"
  cat >"$directory/Cargo.toml" <<EOF
[package]
name = "$name"
version = "0.1.0"
edition = "2021"
EOF
  printf 'pub fn present() {}\n' >"$directory/src/lib.rs"
}

inside_root="$scratch/inside/root"
write_package "$inside_root" inside-root-app
write_package "$inside_root/dep" inside-root-dep
cat >>"$inside_root/Cargo.toml" <<'EOF'

[dependencies]
inside-root-dep = { path = "dep" }
EOF

outside_root="$scratch/outside/root"
outside_dependency="$scratch/outside/sibling-dep"
write_package "$outside_root" outside-root-app
write_package "$outside_dependency" outside-root-dep
cat >>"$outside_root/Cargo.toml" <<'EOF'

[dependencies]
outside-root-dep = { path = "../sibling-dep" }
EOF

replace_root="$scratch/replace/root"
replace_dependency="$scratch/replace/sibling-dep"
replace_vendor="$scratch/replace/vendor/replace-target-0.1.0"
write_package "$replace_root" replace-root-app
write_package "$replace_dependency" replace-target
write_package "$replace_vendor" replace-target
cat >>"$replace_root/Cargo.toml" <<'EOF'

[dependencies]
replace-target = "0.1.0"

[replace]
"replace-target:0.1.0" = { path = "../sibling-dep" }
EOF
mkdir -p "$replace_root/.cargo"
cat >"$replace_root/.cargo/config.toml" <<EOF
[source.crates-io]
replace-with = "test-vendor"

[source.test-vendor]
directory = "$scratch/replace/vendor"
EOF
python3 - "$replace_vendor" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

package = Path(sys.argv[1])
files = {
    str(path.relative_to(package)): hashlib.sha256(path.read_bytes()).hexdigest()
    for path in package.rglob("*")
    if path.is_file() and path.name != ".cargo-checksum.json"
}
(package / ".cargo-checksum.json").write_text(json.dumps({"files": files, "package": None}))
PY

for manifest in "$inside_root/Cargo.toml" "$outside_root/Cargo.toml"; do
  CARGO_NET_OFFLINE=true cargo generate-lockfile --offline --manifest-path "$manifest" >/dev/null
done
(
  cd "$replace_root"
  CARGO_NET_OFFLINE=true cargo generate-lockfile --offline --manifest-path "$replace_root/Cargo.toml" >/dev/null
)

if CARGO_NET_OFFLINE=true bash "$checker" "$inside_root/Cargo.toml"; then
  echo "PASS: in-root path dependency is accepted"
else
  status=$?
  echo "FAIL: the check itself failed for the in-root case (exit $status)" >&2
  exit 1
fi

outside_output="$scratch/outside-check.log"
if CARGO_NET_OFFLINE=true bash "$checker" "$outside_root/Cargo.toml" >"$outside_output" 2>&1; then
  echo "FAIL: outside-root path dependency is rejected (checker accepted it)" >&2
  exit 1
else
  status=$?
  if ((status != 3)); then
    echo "FAIL: the check itself failed for the outside-root case (expected exit 3, got $status)" >&2
    cat "$outside_output" >&2
    exit 1
  fi
fi

if ! python3 - "$outside_output" <<'PY'
from pathlib import Path
import sys

output = Path(sys.argv[1]).read_text()
if "External path dependency: outside-root-dep" not in output:
    print(output, end="", file=sys.stderr)
    sys.exit("checker failed without naming outside-root-dep")
PY
then
  echo "FAIL: outside-root path dependency was not named" >&2
  exit 1
fi
echo "PASS: outside-root path dependency is rejected and named"

replace_output="$scratch/replace-check.log"
if (cd "$replace_root" && CARGO_NET_OFFLINE=true bash "$checker" "$replace_root/Cargo.toml") >"$replace_output" 2>&1; then
  echo "FAIL: outside-root [replace] is rejected (checker accepted it)" >&2
  exit 1
else
  status=$?
  if ((status != 3)); then
    echo "FAIL: the check itself failed for the outside-root [replace] case (expected exit 3, got $status)" >&2
    cat "$replace_output" >&2
    exit 1
  fi
fi

if ! python3 - "$replace_output" <<'PY'
from pathlib import Path
import sys

output = Path(sys.argv[1]).read_text()
if "External path dependency: replace-target" not in output:
    print(output, end="", file=sys.stderr)
    sys.exit("checker failed without naming replace-target")
PY
then
  echo "FAIL: outside-root [replace] target was not named" >&2
  exit 1
fi
echo "PASS: outside-root [replace] target is rejected and named"

missing_manifest="$scratch/unreadable/Cargo.toml"
error_output="$scratch/check-error.log"
if CARGO_NET_OFFLINE=true bash "$checker" "$missing_manifest" >"$error_output" 2>&1; then
  echo "FAIL: unreadable manifest was accepted" >&2
  exit 1
else
  status=$?
  if ((status == 3)); then
    echo "FAIL: unreadable manifest was misreported as a dependency violation" >&2
    cat "$error_output" >&2
    exit 1
  fi
  if ! python3 - "$error_output" <<'PY'
from pathlib import Path
import sys

output = Path(sys.argv[1]).read_text()
if "The check itself failed" not in output:
    print(output, end="", file=sys.stderr)
    sys.exit("check error did not identify itself as a checker failure")
PY
  then
    echo "FAIL: unreadable manifest was not reported as a checker failure" >&2
    exit 1
  fi
  echo "PASS: unreadable manifest is a check error (exit $status, not violation exit 3)"
fi

# The real workspace has registry dependencies, and a fresh CI runner has no
# cached index for them, so this case may use the network. The scratch
# workspaces above have none and stay offline.
if bash "$checker" "$repo_root/Cargo.toml"; then
  echo "PASS: commons workspace has no external path dependencies"
else
  status=$?
  if ((status == 3)); then
    echo "FAIL: commons workspace has an external path dependency" >&2
  else
    echo "FAIL: the check itself failed for the commons workspace (exit $status)" >&2
  fi
  exit 1
fi
