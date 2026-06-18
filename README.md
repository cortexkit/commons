# cortexkit/commons

Neutral home for cross-product [CortexKit](https://github.com/cortexkit) primitives — small, dependency-light building blocks shared across **subc**, **AFT**, and **Magic Context** that belong to no single product.

Each crate is published independently to [crates.io](https://crates.io). Product repos depend on the published version, or on a sibling path-dependency for local development.

## Crates

| Crate | Description |
|-------|-------------|
| [`cortexkit-paths`](crates/cortexkit-paths) | Path canonicalization → canonical project-root identity (`ProjectRootId`). Dependency-free, `#![forbid(unsafe_code)]`, cross-platform (incl. Windows verbatim/UNC/drive-case normalization). |

## License

MIT
