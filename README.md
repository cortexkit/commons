# cortexkit/commons

Neutral home for cross-product [CortexKit](https://github.com/cortexkit) primitives — small, dependency-light building blocks shared across **subc**, **AFT**, and **Magic Context** that belong to no single product.

## Ownership

Maintained by the **subc** seat: direction, review, and releases.

Two crates carry a standing review obligation to the **claustrum** (vault) seat,
who must be routed any change to:

- `cortexkit-store` / `cortexkit-lease` — they run the only real-daemon test
  exercising the single-writer lease across two processes.
- **`cortexkit-paths` canonicalization** — see the warning below.

That is a duty carried, not a veto held.

## Publication is per-crate, and most of these are NOT published

Measured 2026-08-09 against crates.io rather than inferred from release tags:

| state | crates |
|---|---|
| published | `cortexkit-paths` (0.1.1), `cortexkit-provider-usage` (0.4.1) |
| unpublished | the other six — by omission, none sets `publish = false` |

Release tags are **not** the authority on what is published: `provider-usage` has
five versions on crates.io and four tags. Ask the registry.

**Publishing a crate here is close to irreversible** — a version can be yanked but
never removed — and it creates a SECOND DISTRIBUTION PATH. A crate consumed only
by sibling path-dependencies has exactly one; publishing it means a consumer can
resolve a registry version while another repo's sibling checkout floats
elsewhere, and both can end up in one binary. That is already live: `claustrum`
compiles two copies of `cortexkit-paths` at the same version, one path and one
registry, agreeing only because the published bytes currently match.

So: publish only when an external consumer genuinely cannot use a path
dependency, and set `publish = false` explicitly with the reason at the key when
the answer is no.

## Version bumps are the only signal a path-dependency consumer gets

`Cargo.lock` records a path dependency as a bare version string with **no source
and no checksum**, so changed code compiles into every consuming repo with no
lockfile diff and nothing for `--locked` to catch. The version number is the
entire channel.

Bump on any change to observable behaviour or emitted bytes. Not for comments or
tests — a version that moves for prose trains readers to bump reflexively, which
is how it stops meaning anything.

## Crates

| Crate | Description |
|-------|-------------|
| [`cortexkit-paths`](crates/cortexkit-paths) | Path canonicalization → canonical project-root identity (`ProjectRootId`). Dependency-free, `#![forbid(unsafe_code)]`, cross-platform (incl. Windows verbatim/UNC/drive-case normalization). **Its canonical form is a cryptographic identity input** — the vault hashes it to derive the keychain service name holding its master key and the vault id fencing admin MACs. A canonicalization change breaks those and presents as a locked vault over an intact store, never as a path mismatch. The name reads as a path helper; it is not only that. |

## License

MIT
