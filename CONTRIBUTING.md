# Contributing

Keep shared APIs small and changes covered by tests. Crates in this workspace are versioned independently.

## Build and test

Use stable Rust with the `rustfmt` and `clippy` components. From the repository root:

```sh
scripts/gate.sh fast
scripts/gate.sh lint
scripts/gate.sh test
```

`scripts/gate.sh --help` lists the prerequisites and what only CI covers. The `test` stage is slow, so run it on a Linux machine with many cores rather than settling for a filtered package run.
[CI](.github/workflows/ci.yml) calls the same `fast` and `lint` stages. Native tests on each OS, the live PostgreSQL job and the mutation runner's controls keep their own workflow steps.

PostgreSQL integration tests need a running PostgreSQL instance and `CORTEXKIT_TEST_PG_DSN`. To require a live database rather than skip those tests:

```sh
CORTEXKIT_TEST_PG_DSN=postgresql://localhost/ckstore \
CORTEXKIT_REQUIRE_PG=1 \
cargo test -p cortexkit-store-postgres --locked -- --nocapture
```

CI supplies PostgreSQL 16; create a local database and use a DSN with appropriate credentials for your instance.

## Mutation testing

The [mutation-testing tool](crates/cortexkit-mutate/README.md) is built from this workspace as `ckdev-mutate`. Its [positive-control workflow](.github/workflows/mutation-runner.yml) installs `cargo-nextest` and requires it for the runner tests:

```sh
CK_MUTATE_REQUIRE_NEXTEST=1 cargo test -p cortexkit-mutate --locked
cargo run -p cortexkit-mutate --bin ckdev-mutate -- --help
```

See the tool README for `mutations.d/` suite manifests, selectors, and mutation evidence. This checkout does not currently contain a `mutations.d/` suite.

## Versions and packaged test vectors

Bump a published crate's version when changing observable behavior, public API, or emitted bytes. Comment-only changes do not require a bump. The same observable-change rule applies to path-consumed crates: their version is the change signal for consumers. Keep the crate's changelog in sync with behavior changes.

When a published crate reads test vectors, those vectors must ship **inside that crate's package**, not only in a repository-level directory. Keep embedded paths relative to the crate, and include the files in any explicit package `include` list. Check the package contents with:

```sh
cargo package -p cortexkit-role-agent-data --list
```

For examples, see the [agent-data manifest](crates/cortexkit-role-agent-data/Cargo.toml) and [bundled vectors](crates/cortexkit-role-agent-data/test-vectors/agent-data-v1/README.md). A vector change that changes the contract requires a version bump. Each role's [CONTRACT.md](docs/role-contracts.md) remains the normative text.

## Releases

The [release workflow](.github/workflows/release.yml) publishes one crate per tag. A pushed tag named `<crate>-v<version>` (for example, `cortexkit-paths-v0.1.2`) matches its `*-v*` trigger. The workflow runs the shared CI first, checks that `crates/<crate>/Cargo.toml` exists and its version matches the tag, then runs `cargo publish --package "$CRATE"` using the release credential.

Only tag a crate intended for crates.io publication; crates marked `publish = false` cannot be published. See the [crate table](README.md#crates) for current publication status.
