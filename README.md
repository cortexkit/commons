# cortexkit/commons

Shared building blocks for [CortexKit](https://github.com/cortexkit) —
small, dependency-light Rust crates used across the CortexKit daemons and
tools. Nothing here belongs to a single product; everything here is meant to
be boring, stable, and safe to depend on.

## Crates

| crate | what it does |
|---|---|
| `cortexkit-paths` | canonical project-root identity across platforms (case, symlinks, Windows verbatim paths) |
| `cortexkit-store` / `cortexkit-store-types` | managed SQLite store layout, path derivation, and single-writer lease |
| `cortexkit-lease` | advisory-lock + epoch fence primitive backing the store |
| `cortexkit-store-postgres` | Postgres flavor of the store contract |
| `cortexkit-provider-usage` | provider quota/usage wire types shared by producers and renderers |
| `cortexkit-push-seal` | HPKE sealing for push-notification payloads |
| `cortexkit-log` | fleet logging: canonical line format, `CK_LOG` levels and tags, module-owned files with rotation and retention, redaction |
| `cortexkit-cache-core` | cache-policy primitives |
| `cortexkit-model-catalog` | model catalog wire types (transitioning to the fusiform-served schema) |
| `cortexkit-role-harness` | the crash-cut harness trait every role implementation supplies to its conformance suite: spawn on a state root, kill at a named durable point, restart on the same root |
| `cortexkit-role-tool-provider` | the `tool-provider/v1` role: wire types and `CONTRACT.md`, which marks every item pinned or open |
| `cortexkit-role-tool-provider-conformance` | the `tool-provider/v1` conformance runner, driving a live provider through its harness |

### SQLite database ownership

`cortexkit-store` records a file-backed database's `(module_id, storage_namespace)`
owner in `cortexkit_owner` on its first writer open, before acquiring a lease.
Concurrent first opens are serialized with an immediate transaction. Opening the
same database under a different owner, including through a symlink or hard link,
returns `StoreError::OwnershipMismatch` with both identities. Existing databases
without an owner row are claimed automatically; matching owners can reopen.
Read-only operations (`with_read` and its pool) do not claim or check ownership,
and in-memory databases and SQLite URIs are exempt. `open_sqlite` and
`open_sqlite_with` are writer opens, even if the caller only plans to read.

The first ownership claim attempts a WAL checkpoint to publish the row in the
main file. This is best effort: a busy or incomplete checkpoint emits a warning
but does not prevent startup. Until a later checkpoint completes, a hard-link
alias may not see that new owner. An open for an already-recorded owner only
reads the row, with no checkpoint and no write lock. So a module restarting while
another process holds a read snapshot still opens its store, and a second process
with the same owner still gets the lease refusal, even mid-write.

An intentional ownership change requires clearing the `cortexkit_owner` row while
all writers are stopped. There is no API for transferring ownership. Version
0.3.0 adds an error variant to the exhaustive (not `#[non_exhaustive]`)
`StoreError` enum, so downstream exhaustive matches must be updated.

## Module roles

Each module role (`tool-provider`, `llm-runner`, `compaction-provider`, …)
lives here as one crate, `cortexkit-role-<name>`, holding its wire types and a
`CONTRACT.md` role document, with its test vectors under
`test-vectors/<name>-v<N>/` and its conformance runner in a sibling
`cortexkit-role-<name>-conformance` crate. The runner is a separate crate, not
a feature, so a module's production build never compiles the suite and a
plain `cargo test --workspace` here tests it. Every runner drives the real
module over a real route through the module's `cortexkit-role-harness`
implementation; a role crate's own tests may use an in-process fake only to
test its runner. The role crates are not published: implementations and
consumers use them as path dependencies.

Some crates are published to crates.io; most are consumed as path
dependencies by sibling CortexKit repositories. A crate is published only
when an external consumer genuinely cannot use a path dependency — publishing
creates a second distribution path, and one is usually enough.

## Versioning

Path-dependency consumers see no checksum for these crates, so the version
number is the entire change signal: any change to observable behavior or
emitted bytes bumps the version, comment-only changes do not.

## Build

```
cargo build --workspace
cargo test --workspace
```

## License

MIT — see [LICENSE](LICENSE).
