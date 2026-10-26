# cortexkit-usage-steps

The wire contract of the live usage step feed, contract version 1: the
`usage.steps.subscribe` notice stream and the paged `usage.steps.read`. Apps on
every platform decode the same shapes, so the Rust types here, the normative
text in [CONTRACT.md](CONTRACT.md) and the vectors in
`test-vectors/usage-steps-v1/` are one definition for all of them.

The crate is pure serde plus one helper, `LatestRows`, which applies rows in
the order reads return them and keeps the last row per `harness` plus
`step_ref`. It has no transport and no runtime.

## Use

```toml
[dependencies]
cortexkit-usage-steps = { path = "../commons/crates/cortexkit-usage-steps" }
```

The crate is not on crates.io yet; depend on it by path or git until it is.

```rust
use cortexkit_usage_steps::{LatestRows, ReadReply, ReadRequest};

let mut latest = LatestRows::new();
let request = ReadRequest::new(); // then .after(cursor) for later reads
// send `request`, decode the result as a ReadReply, then:
// latest.apply_reply(reply);
// the next read starts at latest.resume_after()
```

Readers ignore unknown members on rows, replies and notices, and keep an
unknown verdict, source or request size as `Other`. Requests are strict.

Both requests take an optional session filter: `for_session(harness, session)`
on `ReadRequest` and `SubscribeRequest` narrows the feed to one session's rows
or notices. The pair comes together or not at all, and `check_pairing` on
either request reports a request naming one alone; the provider refuses one
`invalid_request`. An unfiltered request encodes exactly as before the filter
existed.

A row's `input` excludes both `cache_read` and `cache_write`: the three counts
are disjoint. When all components are known, `prompt` equals their sum; summing
only present components can give a partial total, and absence never means zero.
The usage-feed provider subtracts cache counts still included in a source's
prompt or input count before reporting `input`; already-normalized runner
counts pass through unchanged. It reports duplicate names for the same cache
writes only once. Price the three fields separately; never subtract one from
another.

A row's `output` includes reasoning: `reasoning` is a breakdown of `output`,
never an addition to it, so `reasoning <= output` whenever both are present.
Charge `output` once and never add `reasoning` on top. The provider's tests
can check this with `StepRow::check_counts`; readers still decode a row that
breaks it.
