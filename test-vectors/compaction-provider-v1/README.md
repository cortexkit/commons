# `compaction-provider/v1` test vectors

The shared bytes every `compaction-provider/v1` implementation and runner
checks. The role document is
`crates/cortexkit-role-compaction-provider/CONTRACT.md`. This role is a
draft: these vectors change with the contract until it is reviewed.

| File | What it pins | Checked by |
|---|---|---|
| `role-describe.json` | canonical answers, answers a runner accepts (a missing `stability` read as `alpha`, unknown fields and ops tolerated, the runner groups a provider needs met or unmet), and answers it refuses with the problem: not an object, no majors, no `compaction-provider/v1` major, a missing required op | the wire crate's describe test |
| `setup.json` | `compaction.setup` requests (fresh, and with a first message already written), `ready` answers (a two-message head before every message with stability ranks (how rarely each of the provider's own messages changes, which the runner uses to place cache breakpoints) and `call_when` (the share of the model's context window at which the runner calls the provider, with per-model overrides), an empty view, an unknown condition kind), `refuse` answers (plain, with the provider's own `provider_code`, and with a code the role does not name, each with the retryability fixed by its code), a `refuse` still carrying the removed `retryable` member (tolerated and ignored), answers that do not decode (unknown answer, no initial message, no request id, a `refuse` without a code, a negative rank), model matching for `call_when.models` (exact id, then the longest trailing-`*` pattern, then `default`), and `call_when` shares out of range | the wire crate's setup test |
| `setup-descent.json` | absent/present describe support declaration, optional setup descent wire bytes; unknown parent, unheld cut, prefix gap, fixed-link conflicts, idempotent repeats, later omission, step-only descent, setup/step agreement, first-hook descent, cross-operation contradictions and undeclared roots | the wire crate's describe, setup and lineage tests |
| `status.json` | `compaction.step` statuses (first call with no cursor, a tool step with usage, after an overflow, a prefix rebuild (the runner re-sending the conversation from the start), a refused last answer, a byte-capped message list) with the next version a provider may use; runner extras a provider ignores; statuses that do not decode; statuses whose messages break the cursor rules | the wire crate's status test |
| `answers.json` | each answer (`noop`, two CompactionMessages, `wait`, four `refuse`s, each with the retryability fixed by its code, one with a `provider_code` and one with a code the role does not name, opaque ids and the largest `u64` version), unknown fields tolerated (including a `refuse` still carrying the removed `retryable`, which never decides retry), answers that do not decode (unknown or uppercase `answer`, no request id, no version, a range named by message ids, a negative version, `wait` without a bound, `refuse` without a code), an inverted range, and the model-view `source` each CompactionMessage maps to: `llm-runner/v1`'s half-open replacement source, an insertion for an empty range (Setup's head is `[0, 0)`), none for an inverted range | the wire crate's answer and model-view tests |
| `fence.json` | the runner's decision for an answer given its newest request, last applied version and newest ordinal: act, `superseded_request` for every answer that names any request but the newest issued (each kind of delayed answer, one with a higher version, and one naming a request never issued), `late` for an answer to the newest request at or after its call deadline (with an in-time twin that acts), `stale_version`, `range_beyond_newest`, `range_inverted` | the wire crate's fence test |
| `ready.json` | the `compaction.ready` request, extras the runner ignores, requests that do not decode, and the runner's check: call again, ignored, or refused `not_session_compaction_provider` by the provider at `plan.compaction_item.provider` | the wire crate's ready test, through `llm-runner/v1`'s check |
| `errors.json` | every refusal code a provider answers with, plus one it does not, with whether it is retried; the five role `refuse` codes with their fixed retryability (`async_not_supported` ends a run after Setup only), and a code the role does not name, which decodes and is never retried; the `provider_code`s the runner writes itself (`compaction_unavailable`, `compaction_wait_exceeded`), taken from `llm-runner/v1`; and every named code's fixed retryability (`true` means the runner may retry without the user acting) and the calls after which it may end a run, the same table as CONTRACT.md §14.3 | the wire crate's error test |
| `host-runner.json` | optional inclusive ancestry, the `coverage` field on a `compaction_message` answer (the newest message a summary covers) and retryable history-gap detail, plus malformed new fields; existing vectors still pin absent-field bytes | the wire crate's host-runner tests |
| `async.json` | the `async` group (CONTRACT.md §20): describe with and without the group and with a missing group op; advance-notice header sets with their subject, message id and digest, headers that are ignored, and notices refused by name; view-ready events; views with keep and replace ranges (an insertion head, part edits inside keep ranges that drop a tool output, strip an image or compress text, a whole-message fold, native-marker cuts) and the history each assembles over the shared transcript (a message's parts are its `content` elements, keyed by `parts`); views refused by name or not decoding; part edits refused by name (outside their keep range, out of order, duplicated, bad key, both or neither of `bytes` and `drop`, bytes not JSON); the runner's disposition of fetched views (apply, stale, discard after a revert, a lineage change or a changed edited message, `refused:unknown_part`, rejected for digest, version, form or part order); notices with `view_deferred`, `view_companion` and `view_refused`; `view.get` requests and answers; `compaction.ride` requests (rewrite causes capped at 10,000 ms, `ceiling` at 300,000 ms) and its `view` and `nothing` answers; the Setup ceiling policy with its range checks and end-of-wait decisions | the wire crate's `async_group` vector tests |
| `host-runner-lane.json` | optional service/burn fields absent and present, part byte limits, named lower-watermark refusal, route/principal admission and runner-principal plan-value refusals; sparse pre-op status, held-history gaps/conflicts and final-page wait handling | proposed host-lane conformance cases; not exercised by the current wire suite |

`host-runner-lane.json` follows the named `requests` and stateful-check format.
Its `requests` are concrete wire params; `checks` and `admission` attach fixture
metadata outside the wire object: `route` is the authenticated principal and
bind, `frozen_plan_params` comes from the admitted plan, and `provider_state`
is pre-call durable state. `host_records` is host-local, never a wire field.
Expected answers/refusals and effects are literal expectations, not codec
round-trips. `plan_value_refusals` carries `{method, params}` envelopes.
Admission cases test the runner-group/ready decision separately from those
provider refusals. The proposed host cases require a host-aware implementation;
older types may decode the extra fields but cannot round-trip or execute their
semantics. Existing canonical vectors remain unchanged. Version/type/setter
and conformance updates belong to the contract owner's implementation revision.

Every request vector carries `harness`, the harness named in the session's
key, which identifies the caller (for example `broca`). The wire crate's
tests also remove it from each request in the `requests` lists of `setup.json` and
`status.json` and check that the
request is refused, naming the missing field.

The `joint/` fixture was produced by a working compaction provider's own
message encoder, not written by hand, so it shows the exact bytes a real
provider sends. It is shared by every runner implementation that tests against
this role. `transcript.json` holds the source conversation, `exchanges.json`
holds the ordered request and answer pairs with the view each should produce,
and `README.md` describes the cases. `joint/SHA256SUMS` pins the files' bytes;
regenerate the fixture from the provider rather than editing it.

Every canonical vector round-trips: decoding it and encoding the result gives
back the same JSON. Changing a vector changes the contract: bump the role
crate's version and say why in the commit.

`claude-code.json` adds native endpoints/frozen raw views and page replies,
geometry/fill and explicit notice-id receipts. `CLAUDE_CODE_SHA256SUMS` pins this
new fixture separately; all older vectors and `SHA256SUMS` remain unchanged.
The capability bundle is shared from step-transform's new fixture and contains
only synthetic epoch/version identities. Trusted caller and connection state
are test inputs outside the wire object; serde round-trips alone never prove
lane admission, native endpoint safety or actual provider dispatch.

`claude-code-native-ingress.json` covers noncanonical member order, scalar
escapes, signed/redacted thinking and opaque tool-input cache members.
`claude-code-bootstrap.json` pins typed completion, incomplete ingress and
missing-original refusal; its source digest frames each raw UTF-8 message with
an 8-byte big-endian byte length. The source includes literal multibyte text
and a Unicode escape. Both are included in `CLAUDE_CODE_SHA256SUMS`.

`setup-descent.json` follows the named `requests` and stateful `checks`
format, plus named `describe` answers with and without setup descent support.
Callers send setup descent only when describe's `setup_descends_from` is
true; without that declaration they use the first step or hook instead.
Each check's `provider_state` supplies durable parent ordinals and a
saved link; `calls` are ordered concrete setup, step or hook wire params.
Literal `refusal`/`outcome`, `saved_descends_from` and `history_checks`
expectations live outside those params. The wire tests exercise the common
descent reference check, not a live provider or a transport. A provider must
still validate its actual retained originals and durably persist the descent
link from the session's current lineage to its parent and inclusive cut.
`SETUP_DESCENT_SHA256SUMS` pins this fixture separately; existing vector
bytes and checksum manifests are unchanged.

`async.json` is generated by `generate_async.py`, which computes every digest
independently of the Rust crate: notice digests over the length-framed header
set, view digests over the length-framed view fields, and content digests over
RFC 8785 encodings. The crate's tests therefore compare two implementations.
`ASYNC_SHA256SUMS` pins this fixture separately; existing vector bytes and
checksum manifests are unchanged.
