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
