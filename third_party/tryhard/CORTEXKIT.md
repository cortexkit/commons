# tryhard 0.5.2, CortexKit copy

This is tryhard 0.5.2 from crates.io (https://github.com/EmbarkStudios/tryhard),
MIT OR Apache-2.0, by Embark Studios. Both licence files are kept unchanged.

The only change: `Cargo.toml` drops tokio's `test-util` feature from the normal
dependencies. tryhard 0.5.x enables it there although its code never uses it,
and async-nats depends on tryhard for JetStream reconnects. So every CortexKit
binary that talks to the message bus would otherwise link a pausable tokio clock
into production.

A binary workspace uses this copy with:

    [patch.crates-io]
    tryhard = { git = "https://github.com/cortexkit/commons", rev = "<commit>" }

`[patch]` applies only in the top-level workspace, so each binary adds the line;
a published library cannot carry it. Remove the patch once an upstream tryhard
release drops the feature.
