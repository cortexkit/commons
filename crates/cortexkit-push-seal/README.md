# cortexkit-push-seal

HPKE sealing of push-notification payloads so only the recipient device can read them.

This crate is not published on crates.io. With a checkout at `../commons`, add:

```toml
[dependencies]
cortexkit-push-seal = { path = "../commons/crates/cortexkit-push-seal" }
```

See the [crate source](src/lib.rs) for the `seal` API and payload format.
