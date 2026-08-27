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
| `cortexkit-cache-core` | cache-policy primitives |
| `cortexkit-model-catalog` | model catalog wire types (transitioning to the fusiform-served schema) |

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
