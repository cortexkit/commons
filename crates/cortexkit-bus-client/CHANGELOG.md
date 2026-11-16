# Changelog

## 0.1.1

- Add a body-bearing EVENT reader that only attaches to an existing module-owned
  durable and rebinds it after credential re-issue, leaving the cursor on the
  server and acknowledgements to the caller after their own commit.
- Expose parsed subjects, message ids, bodies, delivery metadata and read-only
  headers (first value wins), with explicit ack, delayed nak and term operations.
- Return malformed records separately with a reason and a term-only handle.
- Refuse event bodies larger than 4 KiB; events carry ids, counters and enums,
  while large content is retrieved through its owner's own call.

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
