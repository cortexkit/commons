# `compaction-provider/v1` test vectors

The shared bytes every `compaction-provider/v1` implementation and runner
checks. The role document is
`crates/cortexkit-role-compaction-provider/CONTRACT.md`. This role is a
draft: these vectors change with the contract until it is reviewed.

| File | What it pins | Checked by |
|---|---|---|
| `setup-accepts.json` | ready Setup opt-in to archived reasoning pruning, with empty omission and open names | `setup_accepts_pruned_reasoning_is_optional_and_open` |
| `reasoning-pruned.json` | the optional prune state: advance notices with and without the `reasoning_pruned` header (one also reporting `view_refused:<version>:pruned_reasoning_restored`), with digests computed independently of the crate; notices refused `malformed:reasoning_pruned` (generation 0, too few or too many numbers, a leading zero, a sign, an empty value); Setup requests with and without `reasoning_pruned` | `reasoning_pruned_notices_and_setup_requests` |
| `role-describe.json` | canonical answers, answers a runner accepts (a missing `stability` read as `alpha`, unknown fields and ops tolerated, the runner groups a provider needs met or unmet), and answers it refuses with the problem: not an object, no majors, no `compaction-provider/v1` major, a missing required op (`compaction.setup`, `view.get` or `compaction.ride`) | the wire crate's describe test |
| `setup.json` | `compaction.setup` requests (fresh, with a first message already written, and from a runner declaring `ride_wait_max_ms`), `ready` answers (a two-message head before every message, one carrying a provider-set `ride_wait_ms`, an empty initial view, a session moved from standalone Magic Context carrying its `checkpoint`), `refuse` answers (plain, with the provider's own `provider_code`, with a code the role does not name, the typed `setup_identity_mismatch` code, and the terminal `standalone_session_not_migrated` and `tag_checkpoint_unsupported`), tolerated answers (a `refuse` still carrying the removed `retryable` member, a `ready` still carrying the removed `stability` and `call_when`), answers that do not decode (unknown answer, no initial message, no request id, a `refuse` without a code), the model-view `source` an initial CompactionMessage maps to (`llm-runner/v1`'s half-open replacement source, an insertion for an empty range, none for an inverted range), a malformed initial message, and `ride_wait`: request and answer pairs a runner accepts (a wait at its cap, no wait) or treats as a failed Setup naming `ride_wait_ms` (above the cap, 0, or a wait the request never asked for) | the wire crate's setup and message tests |
| `setup-descent.json` | absent/present describe support declaration, optional Setup descent wire bytes; unknown parent, unheld cut, prefix gap, fixed-link conflicts and an idempotent repeat | the wire crate's describe, setup and lineage tests |
| `errors.json` | every provider ERROR code plus one unknown; the seven typed Setup refusal codes (`standalone_session_not_migrated`, `tag_checkpoint_unsupported` and `setup_identity_mismatch` among them, all terminal and ending the run after Setup, but for the one hand-off of `standalone_session_not_migrated` to an in-process standalone); an unknown refusal code; runner provider codes; and fixed retryability/terminal behavior for every `KnownCode` | the wire crate's error test |
| `async.json` | advance-notice header sets with their subject, message id and digest, headers that are ignored, and notices refused by name (including `lineage_id` and `descends_from` encodings); fork descent (state kept through a matching cut, a lineage a cut started descending from the old lineage at the cut, an old lineage id containing `:`, a digest mismatch, a missing header, another old lineage and an unrecorded ordinal, each leading to a resync); message ids that differ for a notice and a view-ready event with the same number; view-ready events; views with keep and replace ranges (an insertion head, part edits inside keep ranges that drop a tool output, strip an image or compress text, a whole-message fold, native-marker cuts, and a view on a lineage a cut started that tiles from that lineage's first ordinal) and the history each assembles over the shared transcript (a message's parts are its `content` elements, keyed by `parts`); views refused by name or not decoding; part edits refused by name; the runner's disposition of fetched views, each against its lineage's `first_ordinal` (a view tiling from anywhere else is refused `ranges_start`); notices with `view_deferred`, `view_companion` and `view_refused` (`ranges_start` among the reasons), and the first notice of a lineage a cut started, declaring its descent at the cut; `view.get` requests and answers; `compaction.ride` requests (every cause capped at 300,000 ms, rewrite rides past the runner's own 10,000 ms default among them, as a Setup `ride_wait_ms` allows; `overflow` with and without a stated limit, absent, and ignored on a rewrite cause) and its `view` and `nothing` answers; a lost-runner-state notice sequence (same session, new lineage, no descent, no revert) that resyncs; the Setup ceiling policy with its range checks and end-of-wait decisions; notices carrying the optional `first_ordinal` header (refused when it is more than one past `through_ordinal` or not canonical), and the first notices of lineages a revert started, with and without descent; the prompt cache's lifetime (`cache_ttl` entries, a Setup answer, ride answers and a view carrying one, and the runner's precedence decisions, by name); the optional view `apply` mark (a view marked `next_step` whose digest frames the mark, the same view unmarked with a different digest, a mark this crate does not define that still decodes) and a Setup request carrying `view_apply` | the wire crate's vector tests |
| `reply-shape.json` | a reply body that is the bare answer object (a `compaction.ride` `nothing` answer), never wrapped in `result`, and a refusal body whose other fields ride inside `detail` | `replies_are_bare_answers_and_refusals_keep_detail_inside` |
| `route-identity.json` | shared project routes with generic harnesses and empty sessions, single-session routes, a named mismatch refusal, conversation reverse-call identity, child-session identity, daemon-attested runner authentication, and named provider faults that reuse a shared route's empty session or a constant harness | the wire crate checks fixture identity relationships; no provider conformance kit executes reverse calls |

Every Setup request vector carries `harness`, the caller-chosen value the runner reports for the conversation. It is not an authentication credential; the daemon-attested principal identifies the runner. The wire crate's tests also remove `harness` from each request in the `requests` list of `setup.json` and check that the request is refused, naming the missing field. `project_root` is route metadata, not a Setup field or a provider state-key component. Reverse calls target the incoming route's daemon-attested runner principal with Setup's harness and session; the fixture retains `project_root` in reverse binds only as routing metadata. The non-default `conformance` feature provides separate focused probes, for conversation state identity and for held copies after an archive prune, not a full provider kit.

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

The account-change cases in `async.json` pin the required account label on
rides and notice events, rejection of missing, empty, overlong or malformed
labels, the 1- and 64-byte label boundaries, ignored ride labels on other
causes, and the same deadline bounds as other rewrite rides.

`async.json` is generated by `generate_async.py`, which computes every digest
independently of the Rust crate: notice digests over the length-framed header
set, view digests over the length-framed view fields, and content digests over
RFC 8785 encodings. The crate's tests therefore compare two implementations.
`ASYNC_SHA256SUMS` pins this fixture.
