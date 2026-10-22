# cortexkit-log

Logging format, filtering, redaction, rotation, and retention for CortexKit modules, with a synchronous `tracing` layer.

Add to `Cargo.toml`:

```toml
[dependencies]
cortexkit-log = "0.3.6"
```

With a [`Crates`](src/lib.rs) policy, own crates retain their existing logger
names and dependencies are labelled `<module_id>.dep.<crate>`. Rust crate-name
underscores become hyphens, so `hyper_util` is logged as
`<module_id>.dep.hyper-util`. Dependencies default to WARN or the module's own
resolved level, whichever is lower. A `CK_LOG` directive such as
`<module_id>.dep.hyper-util=debug` can raise one dependency's level.

Use `Crates::ceiling` to set a maximum for a dependency after `CK_LOG`; an OFF
ceiling drops that crate entirely. Ceilings are useful for transports whose
verbose logs may contain credentials or connection secrets.

```rust
use cortexkit_log::{init_with_crates, Config, Crates, LevelFilter};

let crates = Crates::new()
    .own(["my_module_core"])
    .ceiling("sensitive_transport", LevelFilter::INFO);
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
