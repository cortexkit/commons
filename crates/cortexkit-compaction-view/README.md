# cortexkit-compaction-view

The shared view library for the CortexKit compaction contract. A runner and a
compaction provider use the same implementation to compute render plans, apply
views, check provider rules, clean replies, and number tags.

```sh
cargo add cortexkit-compaction-view@0.1
```

The wire types come from
[`cortexkit-role-compaction`](https://crates.io/crates/cortexkit-role-compaction).
The crate ships JSON vectors in `vectors/cases.json`. `PLAN_VERSION` identifies
rendering semantics: supported rendering behavior changes require a version bump.
A regression test pins representative JSON outputs for selected rendering
operations and a sample tag prefix. It is not exhaustive and does not hash final
host requests; retaining historical pins and requiring version bumps are
release-review obligations. Runners requiring replay compatibility must persist
and check the version themselves.

The companion
[`cortexkit-compaction-view-wasm`](https://crates.io/crates/cortexkit-compaction-view-wasm)
crate exposes the same implementation to JavaScript runners.
