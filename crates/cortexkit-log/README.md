# cortexkit-log

> [!WARNING]
> **Alpha.** cortexkit-log is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

Logging format, filtering, redaction, rotation, and retention for CortexKit modules, with a synchronous `tracing` layer.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-log = "0.3.7"
```

With a [`Crates`](src/lib.rs) policy, own crates retain their existing logger
names and dependencies are labelled `<module_id>.dep.<crate>`. Rust crate-name
underscores become hyphens, so `hyper_util` is logged as
`<module_id>.dep.hyper-util`. Dependencies default to WARN or the module's own
resolved level, whichever is lower. A `CK_LOG` directive such as
`<module_id>.dep.hyper-util=debug` can raise one dependency's level.

Use `Crates::ceiling` to set a maximum for a crate or module path after
`CK_LOG`; an OFF ceiling drops matching events. Paths match the event's Rust
module path (or its target if none is recorded) on `::` boundaries. The most
specific ceiling wins, and a module-path ceiling can also cap a path in an
`own` crate without changing its logger name.

Pair this policy with `cortexkit-bus-client` 0.1.17's outage logs to suppress
async-nats reconnect and handshake logs while keeping its other warnings and
errors:

```rust
use cortexkit_log::{init_with_crates, Config, Crates, LevelFilter};

let crates = Crates::new()
    .own(["my_module_core"])
    .ceiling("async_nats", LevelFilter::WARN)
    .ceiling("async_nats::connector", LevelFilter::OFF);
let config = Config::in_dir("my-module", "run/logs");
let _ = init_with_crates(config, crates);
```

For a caller-composed subscriber, use `layer(config)?.0.with_crates(policy)`;
the same method works on `layer_with_stderr_copy(config)?.0`. Under the default
`store-paths` feature, `init_from_env_with_crates(policy)` applies the same
policy to environment-based initialization.

Do not use an outer `filter_fn` to enforce `CK_LOG`; the layer already does
that, and its per-event checks mean an outer filter cannot bypass the policy.
A filter's own predicate can still suppress events. See the [crate
documentation](src/lib.rs) for initialization and log-segment retention APIs.
