#!/usr/bin/env bash
# Share the fast/lint entry points with CI so their checks cannot drift.
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/gate.sh {fast|lint|test|all}

  fast  Formatting, locked/offline metadata, path-dependency checks (including
        their self-tests), repository layout, tracked ignored state, and every
        test-vector *SHA256SUMS manifest. Stops at the first failed check.
  lint  CI's workspace/all-targets Clippy with warnings denied, then the same
        check for x86_64-pc-windows-gnu and the mutation workflow's package-only
        Windows check. Install that Rust target first; cross compilation may
        also need the platform's MinGW C compiler.
  test  SLOW: run on the build server with the bash tool's runon: "linux,16c".
        Runs CI's full workspace/all-targets suite, its exact vector-feature
        package selection, and the mutation runner's nextest-required controls.
        Requires cargo-nextest, as does the mutation-runner workflow.
  all   Run fast, lint, and test on this host, reporting a verdict per stage.
        This includes the slow tests; it is not a substitute for other OSes.

Use stable Rust with rustfmt and clippy, a populated Cargo cache for offline
metadata, and the existing path check's python3 dependency. No tools are installed
by this script. Commands run from the repository root, regardless of your cwd.
CI sets GATE_CI_OS to its matrix OS: path-check self-tests and Windows cross-lint
run only on ubuntu-latest there, preserving the existing self-test OS scope.
Outside CI both checks run. Ordinary fmt/lint/tests still run on each CI OS.

CI-only coverage (not silently replaced by a local pass):
  - Native Linux/macOS/Windows execution: a host cannot run the other OS kernels.
  - Live PostgreSQL job: needs CI's healthy PostgreSQL 16 service and required DSN.
    See CONTRIBUTING.md to run its exact command against your own database.
  - PR design gate: an external reusable workflow needs GitHub events/permissions.
  - Release run lookup, tag validation, and publish: need a release ref, GitHub
    credentials, and registry credentials; they are not push/PR source checks.
  - Checkout, toolchain/nextest/MinGW installation and caches are CI provisioning.

No public-text private-name scanner exists in this checkout. No private-name
list is embedded here; the gate does not claim that coverage.
USAGE
}

if [ "$#" -ne 1 ]; then
    usage >&2
    exit 2
fi
stage=$1
case "$stage" in
    -h|--help|help) usage; exit 0 ;;
    fast|lint|test|all) ;;
    *) usage >&2; exit 2 ;;
esac

cd "$(dirname "${BASH_SOURCE[0]}")/.."
script="$PWD/scripts/gate.sh"
steps=0

run_step() {
    local name=$1
    local status
    shift
    printf '\n==> %s\n' "$name"
    if "$@"; then
        steps=$((steps + 1))
        printf 'PASS: %s\n' "$name"
    else
        status=$?
        printf 'FAIL: %s (exit %s)\n' "$name" "$status" >&2
        exit "$status"
    fi
}

stage_verdict() {
    local status=$?
    if [ "$status" -eq 0 ]; then
        printf 'gate %s: PASS (%s checks)\n' "$stage" "$steps"
    else
        printf 'gate %s: FAIL (exit %s; %s checks passed)\n' "$stage" "$status" "$steps" >&2
    fi
}

metadata() {
    # The dependency graph is large; only Cargo's diagnostics are useful here.
    cargo metadata --locked --offline --format-version 1 > /dev/null
}

tracked_ignored() {
    local tracked
    tracked=$(git ls-files -ci --exclude-standard -- .cortexkit) || return
    if [ -n "$tracked" ]; then
        printf 'Tracked ignored .cortexkit paths:\n%s\n' "$tracked" >&2
        return 1
    fi
    echo 'No tracked ignored .cortexkit paths.'
}

checksum() {
    local manifest=$1
    (
        cd "$(dirname "$manifest")" || exit
        if command -v sha256sum > /dev/null 2>&1; then
            sha256sum -c "$(basename "$manifest")"
        else
            shasum -a 256 -c "$(basename "$manifest")"
        fi
    )
}

fast() {
    local manifest
    local manifests=0
    run_step 'Format check' cargo fmt --all -- --check
    run_step 'Locked offline metadata' metadata
    run_step 'No external path dependencies' bash scripts/no-external-path-deps.sh
    if [ -z "${GATE_CI_OS:-}" ] || [ "$GATE_CI_OS" = ubuntu-latest ]; then
        run_step 'Path-dependency checker self-tests' bash scripts/no-external-path-deps.test.sh
    fi
    run_step 'Repository tooling layout' bash scripts/check-cortexkit-layout.sh
    run_step 'Tracked ignored .cortexkit files' tracked_ignored

    # Git includes new, unignored manifests too, without scanning build output.
    # Auxiliary checksum manifests pin additional vector sets and need checking.
    while IFS= read -r -d '' manifest; do
        case "$manifest" in
            test-vectors/*|*/test-vectors/*)
                run_step "SHA256SUMS: $manifest" checksum "$manifest"
                manifests=$((manifests + 1))
                ;;
        esac
    done < <(git ls-files --cached --others --exclude-standard -z -- '*SHA256SUMS')
    if [ "$manifests" -eq 0 ]; then
        echo 'FAIL: no test-vector SHA256SUMS manifests found' >&2
        exit 1
    fi
    printf 'Verified %s test-vector checksum manifests.\n' "$manifests"
}

lint() {
    run_step 'Clippy (default target)' cargo clippy --workspace --all-targets --locked -- -D warnings
    if [ -z "${GATE_CI_OS:-}" ] || [ "$GATE_CI_OS" = ubuntu-latest ]; then
        run_step 'Clippy (Windows GNU target)' cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-gnu -- -D warnings
        # A package-only build can enable different features from a workspace build.
        run_step 'Mutation runner Clippy (Windows GNU target)' cargo clippy -p cortexkit-mutate --all-targets --locked --target x86_64-pc-windows-gnu -- -D warnings
    fi
}

test_stage() {
    # Keep these invocations intact: package selection affects feature unification.
    run_step 'Workspace tests (all targets)' cargo test --workspace --all-targets --locked
    run_step 'Shipped vector feature tests' cargo test --locked -p cortexkit-role-agent-data -p cortexkit-role-compaction-provider -p cortexkit-role-step-transform-provider -p cortexkit-role-llm-runner -p cortexkit-role-tool-provider -p cortexkit-anthropic-policy -p cortexkit-chatgpt-policy -p cortexkit-role-agent-inbox -p cortexkit-role-classifier -p cortexkit-role-routing -p cortexkit-model-batch -p cortexkit-role-extractor -p cortexkit-rich-ui-syntax -p cortexkit-llm-tools --features test-vectors
    run_step 'Mutation runner positive controls (nextest required)' env CK_MUTATE_REQUIRE_NEXTEST=1 cargo test -p cortexkit-mutate --locked
}

if [ "$stage" = all ]; then
    status=0
    # Separate shells keep fail-fast behavior inside each stage while all still
    # reports every stage, even when an earlier stage fails.
    for next_stage in fast lint test; do
        if bash "$script" "$next_stage"; then
            :
        else
            status=1
        fi
    done
    exit "$status"
fi

trap stage_verdict EXIT
case "$stage" in
    fast) fast ;;
    lint) lint ;;
    test) test_stage ;;
esac
