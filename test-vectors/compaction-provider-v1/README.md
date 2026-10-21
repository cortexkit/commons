# `compaction-provider/v1` test vectors

The shared bytes every `compaction-provider/v1` implementation and runner
checks. The role document is
`crates/cortexkit-role-compaction-provider/CONTRACT.md`. This role is a
draft: these vectors change with the contract until it is reviewed.

| File | What it pins | Checked by |
|---|---|---|
| `role-describe.json` | canonical answers, answers a runner accepts (a missing `stability` read as `alpha`, unknown fields and ops tolerated, the runner groups a provider needs met or unmet), and answers it refuses with the problem: not an object, no majors, no `compaction-provider/v1` major, a missing required op (`compaction.setup`, `view.get` or `compaction.ride`) | the wire crate's describe test |
| `setup.json` | `compaction.setup` requests (fresh, and with a first message already written), `ready` answers (a two-message head before every message, an empty initial view), `refuse` answers (plain, with the provider's own `provider_code`, with a code the role does not name, and the named `setup_identity_mismatch` code), tolerated answers (a `refuse` still carrying the removed `retryable` member, a `ready` still carrying the removed `stability` and `call_when`), answers that do not decode (unknown answer, no initial message, no request id, a `refuse` without a code), the model-view `source` an initial CompactionMessage maps to (`llm-runner/v1`'s half-open replacement source, an insertion for an empty range, none for an inverted range), and a malformed initial message | the wire crate's setup and message tests |
| `setup-descent.json` | absent/present describe support declaration, optional Setup descent wire bytes; unknown parent, unheld cut, prefix gap, fixed-link conflicts and an idempotent repeat | the wire crate's describe, setup and lineage tests |
| `errors.json` | every provider ERROR code plus one unknown; the four typed Setup refusal codes; the named `setup_identity_mismatch` wire code, decoded as `Unknown` and never retried in 0.3.x; an unknown refusal code; runner provider codes; and fixed retryability/terminal behavior for `KnownCode` plus `setup_identity_mismatch` decoded as `Unknown` | the wire crate's error test |
| `async.json` | advance-notice header sets with their subject, message id and digest, headers that are ignored, and notices refused by name (including `lineage_id` and `descends_from` encodings); fork descent (state kept through a matching cut, an old lineage id containing `:`, a digest mismatch, a missing header, another old lineage and an unrecorded ordinal, each leading to a resync); message ids that differ for a notice and a view-ready event with the same number; view-ready events; views with keep and replace ranges (an insertion head, part edits inside keep ranges that drop a tool output, strip an image or compress text, a whole-message fold, native-marker cuts) and the history each assembles over the shared transcript (a message's parts are its `content` elements, keyed by `parts`); views refused by name or not decoding; part edits refused by name; the runner's disposition of fetched views; notices with `view_deferred`, `view_companion` and `view_refused`; `view.get` requests and answers; `compaction.ride` requests (rewrite causes capped at 10,000 ms, `ceiling` at 300,000 ms; `overflow` with and without a stated limit, absent, and ignored on a rewrite cause) and its `view` and `nothing` answers; a lost-runner-state notice sequence (same session, new lineage, no descent, no revert) that resyncs; the Setup ceiling policy with its range checks and end-of-wait decisions | the wire crate's vector tests |
| `route-identity.json` | shared project routes with generic harnesses and empty sessions, single-session routes, a named mismatch refusal, conversation reverse-call identity, child-session identity, daemon-attested runner authentication, and named provider faults that reuse a shared route's empty session or a constant harness | the wire crate checks fixture identity relationships; no provider conformance kit executes reverse calls |

Every Setup request vector carries `harness`, the caller-chosen value the runner reports for the conversation. It is not an authentication credential; the daemon-attested principal identifies the runner. The wire crate's tests also remove `harness` from each request in the `requests` list of `setup.json` and check that the request is refused, naming the missing field. `project_root` is route metadata, not a Setup field.

Every canonical vector round-trips: decoding it and encoding the result gives
back the same JSON. Changing a vector changes the contract: bump the role
crate's version and say why in the commit.

`setup-descent.json` follows the named `requests` and stateful `checks`
format, plus named `describe` answers with and without Setup descent support.
Callers send Setup descent only when describe's `setup_descends_from` is
true. Each check's `provider_state` supplies durable parent ordinals and a
saved link; `calls` are ordered concrete Setup wire params. Literal
`refusal`/`outcome`, `saved_descends_from` and `history_checks` expectations
live outside those params. The wire tests exercise the common descent
reference check, not a live provider or a transport. A provider must still
validate its actual retained originals and durably persist the descent link.
`SETUP_DESCENT_SHA256SUMS` pins this fixture.

`async.json` is generated by `generate_async.py`, which computes every digest
independently of the Rust crate: notice digests over the length-framed header
set, view digests over the length-framed view fields, and content digests over
RFC 8785 encodings. The crate's tests therefore compare two implementations.
`ASYNC_SHA256SUMS` pins this fixture.
