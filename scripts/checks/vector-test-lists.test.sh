#!/usr/bin/env bash
# Self-tests for vector-test-lists.sh: a crate whose tests need test-vectors
# passes only when both the gate and CI commands list it.
set -euo pipefail
finished=

here=$(cd "$(dirname "$0")" && pwd)
checker=$here/vector-test-lists.sh
work=$(mktemp -d)
# macOS's bash 3.2 can exit 0 from an unbound variable while an EXIT trap is
# set, so a run that stops early must not read as a pass.
cleanup() {
    local status=$?
    rm -rf -- "$work"
    if [ "$status" -eq 0 ] && [ "${finished:-}" != 1 ]; then
        echo 'vector-test-lists.test.sh: exited before the checks completed' >&2
        exit 1
    fi
    exit "$status"
}
trap cleanup EXIT

make_tree() {
    local dir=$1 gate_pkgs=$2 ci_pkgs=$3
    mkdir -p "$dir/crates/demo" "$dir/scripts" "$dir/.github/workflows"
    cat > "$dir/crates/demo/Cargo.toml" <<'EOF'
[package]
name = "demo-crate"

[[test]]
name = "vectors"
required-features = ["test-vectors"]
EOF
    printf "run_step 'Shipped vector feature tests' cargo test --locked %s --features test-vectors\n" "$gate_pkgs" > "$dir/scripts/gate.sh"
    printf '        run: cargo test --locked %s --features test-vectors\n' "$ci_pkgs" > "$dir/.github/workflows/ci.yml"
}

expect() {
    local want=$1 name=$2 dir=$3
    if bash "$checker" "$dir" > /dev/null 2>&1; then got=pass; else got=fail; fi
    if [ "$got" != "$want" ]; then
        echo "FAIL: $name: expected $want, got $got" >&2
        exit 1
    fi
    echo "ok: $name"
}

make_tree "$work/listed" "-p other -p demo-crate" "-p demo-crate@1.0"
expect pass "listed in both, version suffix ignored" "$work/listed"

make_tree "$work/no-gate" "-p other" "-p demo-crate"
expect fail "missing from gate" "$work/no-gate"

make_tree "$work/no-ci" "-p demo-crate" "-p other"
expect fail "missing from CI" "$work/no-ci"

make_tree "$work/prefix" "-p demo-crate-extra" "-p demo-crate-extra"
expect fail "a longer name does not count" "$work/prefix"

finished=1
