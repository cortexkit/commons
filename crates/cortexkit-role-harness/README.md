# cortexkit-role-harness

Harness interface shared by role conformance suites: spawn a module on a state root, kill it at a durable point, restart it on the same root, and route requests to it.

Add as a test dependency:

```toml
[dev-dependencies]
cortexkit-role-harness = "0.1.0"
```

Implement the `Harness` trait for the module under test. See the [crate source](src/lib.rs) and [role/conformance guide](../../docs/role-contracts.md).
