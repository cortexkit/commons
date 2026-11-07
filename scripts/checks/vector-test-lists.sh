#!/usr/bin/env bash
# Fails when a crate has integration tests that run only with the
# `test-vectors` feature, but the crate is missing from the explicit
# `--features test-vectors` test command in scripts/gate.sh or in CI.
#
# Those commands list packages by hand, because package selection changes
# feature unification (see the comment above them in gate.sh). A crate left
# out of either list still builds and passes everywhere: its feature-gated
# tests simply never run, which no other check notices.
#
# Usage: vector-test-lists.sh [repository root], default the current directory.
set -euo pipefail

root=${1:-.}
gate=$root/scripts/gate.sh
ci=$root/.github/workflows/ci.yml

# Package names named with `-p` on the line that enables test-vectors.
# A `@version` suffix only disambiguates; compare by name.
listed() {
    grep -E -- '--features test-vectors' "$1" \
        | grep -oE -- '-p [A-Za-z0-9_-]+' \
        | awk '{print $2}' \
        | sort -u
}

required=$(
    for manifest in "$root"/crates/*/Cargo.toml; do
        if grep -qE 'required-features *= *\[[^]]*"test-vectors"' "$manifest"; then
            awk -F'"' '/^name = /{print $2; exit}' "$manifest"
        fi
    done | sort -u
)

gate_list=$(listed "$gate")
ci_list=$(listed "$ci")
status=0
for crate in $required; do
    if ! printf '%s\n' "$gate_list" | grep -qxF "$crate"; then
        echo "FAIL: $crate has tests that need the test-vectors feature but is missing from the vector test command in scripts/gate.sh" >&2
        status=1
    fi
    if ! printf '%s\n' "$ci_list" | grep -qxF "$crate"; then
        echo "FAIL: $crate has tests that need the test-vectors feature but is missing from the vector test command in .github/workflows/ci.yml" >&2
        status=1
    fi
done
if [ "$status" -ne 0 ]; then
    exit 1
fi
printf 'All %s crates with test-vectors-only tests are in both test commands.\n' \
    "$(printf '%s\n' "$required" | grep -c .)"
