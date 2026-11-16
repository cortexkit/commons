# Changelog

## 0.1.0

- Add a shared credential client for connecting Rust modules to the local bus
  through ck-bus without holding a signing seed.
- Renew same-key JWTs and single-flight revoked or superseded re-issues, with
  replacement connection snapshots and a change signal for reopening readers.
- Add stored-event and versioned live-hint helpers using the shared naming rules.
- Warn with the refusal code when terminal renewal failure stops maintenance,
  and document the closed change stream as a reconnect/degraded signal.

### Verification notes

- Linux tests, the opt-in real-bus durable/re-issue test, workspace Clippy and
  Windows GNU Clippy pass.
- ARM64 Windows MSVC is blocked by missing Windows CRT headers on the Linux cross
  host (`ring` cannot find `assert.h`). The native Windows job was refused for
  capacity; native Windows compilation is still owed.
