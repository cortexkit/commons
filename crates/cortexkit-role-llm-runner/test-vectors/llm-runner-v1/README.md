# `llm-runner/v1` test vectors

The shared bytes every `llm-runner/v1` implementation and consumer checks.
The role document is `crates/cortexkit-role-llm-runner/CONTRACT.md`. The
files live in that crate's `test-vectors/llm-runner-v1/` and ship in its
package; with the crate's `test-vectors` feature, its public `vectors` module
carries every file's bytes. `SHA256SUMS` pins every file, and a test checks
that every file here is listed in `vectors::FILES` with its exact bytes and
its SHA-256. This
role is a draft: these vectors change with the contract until it is
reviewed.

| File | What it pins | Checked by |
|---|---|---|
| `role-describe.json` | `role.describe` answers a consumer accepts (with the capabilities each declares, including a `plans` runner and a steer-only runner with no `compaction` group and runners declaring `compaction` and `session_change`), canonical answers (one stating the `max_bytes` cap), a missing `stability` read as `alpha`, and answers it refuses with the problem: a missing major or required op, a declared group missing one of its ops (including `plans` without `session.send`, `interrupt`, `compaction` without `session.resolve` and `session_change` without `session.flush_prefix`), `compaction` without `tags` (`compaction_without_tags`), no session-capability source, `transcript_reads` without `max_bytes` | `describe_vectors_check_as_recorded` (including a canonical `plans` declaration round-trip) |
| `read-requests.json` | `session.read` requests for each mode (tail, range, after), with the mode each selects, and refused requests with the `invalid_params` field: `after_mid` without `lineage_id`, `after_mid` with `from_ordinal`, a misspelled cursor, an unknown view, `after_mid` on the model view (naming `view`) | the wire crate's request round-trip and mode tests |
| `read-pages.json` | pages: a tail page, a page stopped by its byte cap with `next_from_ordinal`, a single message over the cap alone and whole on its page, run and dispatch attribution, originals, a denied call with no dispatch target, an empty transcript, a never-written session's empty page with no `lineage_id`; runner extras a decoder ignores; pages a decoder refuses (no `mid`, a non-object `head`); pages without a lineage that still carry messages or a next ordinal; the `lineage_changed` and `unknown_mid` refusals with the requests that get them; model-view pages (both `source` kinds, the compaction state absent before any compaction and named after it), model-view pages a consumer rejects with the problem, model-view entries it cannot decode, and the replacement-once series: the page holding a replacement's `first_ordinal` returns it whole, the next page and a read from inside its range do not repeat it, and a page that does is rejected | the wire crate's page and model-view tests |
| `head.json` | the `session.head` request, a request it refuses, answers with and without a last run, a never-written session's answer with every member absent, and answers inconsistent with their lineage (no lineage but another member present; a lineage without `updated_at`) | the wire crate's head test |
| `run-result.json` | the `run.result` request, requests it refuses, an answer per run state with whether it is terminal (including a completed run whose final message has no text, answered `text: ""`), answers inconsistent with their state (completed without `final_message`, running with one), text-part joins with nothing inserted, and the `unknown_run` refusal | the wire crate's run-result, final-message and text-join tests |
| `subscribe.json` | `session.subscribe` requests (absent, `start`, `live`, after a page's `head`) with their attach point, refused requests with the field, and stream events (durable, best-effort, unknown kind) | the wire crate's subscribe test |
| `send.json` | owner `session.send` requests per delivery mode, with a mark, and with a plan and runner parameters; refused requests (unknown mode, no `send_id`); delivery checks against the modes a runner declares, with the `delivery_unsupported` refusal for an undeclared one; admission checks of a plan's `compaction_item` against the `compaction` group, with the `invalid_params {field: "plan.compaction_item"}` refusal; replies, including the admission reply's baseline; refusals with their retryability and named field | `send_vectors_decode_and_round_trip`, `undeclared_delivery_modes_are_refused_by_name`, and `a_compaction_item_is_refused_at_admission_without_the_compaction_group` |
| `plans.json` | first send with a plan admitted by a `plans` runner (and its admission baseline), plan-less sends, and sends with a plan refused without `plans` as `invalid_params {field: "plan"}`; identical-repeat/no-op and different-plan/`plan_drift` envelopes with the unchanged baseline | `a_plan_is_refused_without_the_plans_group` executes capability admission and round-trips requests/reply; `frozen_plan_vectors_round_trip` round-trips requests, baseline and drift detail only (no runner fetch/freeze or comparison implementation in this wire crate) |
| `baseline.json` | the `session.baseline` request, a request it refuses, the `not_yet` answer, a fresh and a changed baseline, and the non-owner refusal | the wire crate's baseline test |
| `session-change.json` | the `session_change` group's refusals (`rung_unsupported`, `pending_superseded`, `already_applied`), each with the op that gets it and its retryability | the wire crate's session-change test |
| `errors.json` | every refusal code the role defines, plus one it does not, with whether a caller retries it | the wire crate's retryability test |
| `reply-too-large.json` | terminal `reply_too_large` refusals for an oversized raw message and model-view atomic group, identifying each without content | `reply_too_large_is_named_terminal_and_carries_only_safe_details` |

The following admission fixtures are copied byte-for-byte from the prefrontal
repository's `test-vectors/fetch-plan-v1/admission/` at commit `7079a4025`. The wire tests
re-encode the typed detail or absent item inside the unchanged fixture envelope
and compare the complete bytes, including field order and whitespace.

| File | Source commit and file | Checked by |
|---|---|---|
| `refuse-later-send-with-different-plan.json` | `7079a4025`, `admission/refuse-later-send-with-different-plan.json` | `prefrontal_plan_drift_bytes_round_trip` |
| `refuse-later-plan-after-planless-first-episode.json` | `7079a4025`, `admission/refuse-later-plan-after-planless-first-episode.json` | `prefrontal_plan_drift_bytes_round_trip` |
| `refuse-tool-name-collision.json` | `7079a4025`, `admission/refuse-tool-name-collision.json` | `prefrontal_tool_name_collision_bytes_round_trip` |
| `admit-optional-text-refused.json` | `7079a4025`, `admission/admit-optional-text-refused.json` | `prefrontal_absent_item_bytes_round_trip` |
| `admit-optional-text-timeout.json` | `7079a4025`, `admission/admit-optional-text-timeout.json` | `prefrontal_absent_item_bytes_round_trip` |
| `admit-optional-text-unknown-provider.json` | `7079a4025`, `admission/admit-optional-text-unknown-provider.json` | `prefrontal_absent_item_bytes_round_trip` |

The `plans.json` plan bodies are illustrative opaque maps, not canonical
fetch-plan bytes. Their `plan-a`/`plan-b` identities are symbolic; the copied
plan-drift fixtures above retain the upstream canonical identities. Frozen
cases describe runner obligations, not executable freeze checks in this wire
crate. The first-send admission test checks only the capability gate, not fetch.

Every pinned vector round-trips: decoding it and encoding the result gives
back the same JSON. Changing a vector changes the contract: say why in the
commit. These admission shapes align the unpublished `cortexkit-role-llm-runner`
0.1.0 draft with prefrontal's fetch-plan consumer; the crate remains
`publish = false` until review.
# Session input-token caps

`input-token-cap.json` pins accepted sends and every cap/member refusal,
including scoped closed nudge elements and malformed accessor values. Its
legacy budget fixture pins the bytes retained in `SendRequest.runner_params`
and read through the typed accessor. `input-token-cap-results.json` pins
`run.result` and `run.status` for both exhaustion causes and overshoot.
`input-token-cap-describe.json` pins independent optional-group discovery and
the missing-op refusal. Tool-result bodies remain runner-specific; the live
conformance adapter decodes the runner's existing hard-stop result shape.

# Structured output

`structured-output.json` pins `session.send` `structured_output` requests: the
raw member kept byte for byte in `SendRequest.runner_params`, accepted
requests for each mode (including in-document references, data keywords and a
schema exactly at the 65,536-byte JCS cap), and every refusal with its field:
`final_text` with tools, a reserved-name collision, external `$ref` and
`$dynamicRef`, a schema one byte over the cap, another dialect, malformed
names and modes, an unknown member, and an unadvertised group. Cap cases are
written as templates (`pad_to_jcs_bytes`) that the test fills, not as 64 KiB
of padding. `structured-output-results.json` pins `run.result` and
`run.status` for `enforced` and `passed`, `best_effort` with each reason,
`failed` with errors, errors cut to 32 with `errors_truncated`, and `absent`;
malformed results with the rule each breaks; and the `reply_too_large`
refusal naming a run. `structured-output-describe.json` pins the optional
group's discovery and its missing-op refusal. Checked by the
`structured_output::tests` module of the wire crate.

# Adoption

`adopt.json` pins `session.adopt`: a canonical request carrying the full
plan; requests refused by field (naming the session through `session_ref`,
no plan, no `send_id`, a digest in place of the plan); replies with and
without `last_ordinal`; and cases, each with the session's state (the
identity of the plan applied at host start, the recorded owner, the last
ordinal and any run in progress), earlier answered adoptions, the caller,
the request with its plan's identity, and the reply or refusal: a first
adoption, one before any message, an exact repeat answered with the original
reply after the transcript grew, `send_id_reuse` naming `plan`, `plan_drift`
with and without `frozen`, an adoption during an active run, another owner
(including one repeating the owner's request exactly) and a `direct` caller
refused `scope_owner_mismatch`, and the same owner re-adopting. No case
writes a message, starts a run or interrupts one. The plan identities are
symbolic, as in `plans.json`. `adopt-describe.json` pins the group's
discovery, its missing-op refusal and an answer without the group. Checked by
the `adopt::tests` and `describe::adopt_tests` modules of the wire crate,
which apply the pinned order of checks to every case.

# Tags

`tags.json` pins the `tags` group: the one spelling of a tag (valid and
invalid spellings, including a leading zero, a non-ASCII digit and a number
past `u64`); a fork continuing after its parent's highest number; read
entries carrying `tags` (one row per exposed part, every tag used once, an image part keyed but untagged) with `role`
and `written_at` in the envelope; a model-written `§N§` stored as written in
a text part and in tool-call arguments; `session.resolve` requests, requests
refused by field, and cases against a ledger of stored parts (the whole
part, a character range, a range counted in Unicode scalar values, an empty
range at the end, a tool-call part in the runner's stored encoding,
`unknown_tag`, malformed tags, a range past the content and one starting
after its end); and the plan's `render_tags` member, off when absent or
`null`, refused when not a boolean or when `true` without the group; and rendered parts, the tag, one space and the text, on the kinds that carry it, and nothing on parts without text.
`tags-describe.json` pins the group's discovery, its missing-op refusal and
an answer without the group. Checked by the `tags::tests` and
`describe::tags_and_tool_calls_tests` modules of the wire crate.

# Delivery order

`delivery-order.json` pins the `delivery_order` group: the string
`delivery` equivalences (`queue` and absent are `[idle_start]`, `steer` is
`[steer, idle_start]`, `interrupt` has none); sends whose `delivery` is a
list, with `dedupe_key`, `expires_at_ms` and `template`; `delivery` values
that do not decode, each naming `delivery` (an empty list, a mode twice, a
legacy mode inside a list, a number, an unknown string); sends refused by
member (a member beside a string or absent `delivery`, malformed members, a
template beside a prompt, an empty prompt without a template); a runner
without the group refusing a list naming `delivery` while the string still
works; mode choices at boundaries (fall-through order, order winning over what else
is possible, a list without `idle_start` never starting a run, `queue`
waiting for the run to end); a plan sender's typed template with uses that
render or name their problem (a percentage over 100 or written as a string,
an integer outside its bounds, an unassigned tag, free text in a tag slot,
an unlisted enum value, missing and undeclared slots, another digest, an
unknown template); what goes out (a rendered template as system text, or
as the same text at user authority after a fall-through, a plain prompt at
user authority) and the refusal naming `template` of every template that
cannot render; template sends, which carry no `prompt`, and the refusals of
a template beside a prompt (empty or not) and of a send with neither; plans with senders, valid and refused
(including a string slot); admission of the owner, named senders with and
without the `idle_start` grant, and unnamed modules refused
`scope_owner_mismatch` even when a request field names a sender; dedupe and
expiry; `superseded` and `expired` send states; and entries recording each
mode. Template digests are SHA-256 over JCS bytes and are checked by
recomputation. `delivery-order-describe.json` pins the group's discovery.
Checked by the `delivery_order::tests` and
`describe::tags_and_tool_calls_tests` modules of the wire crate.

# Run tool calls

`run-tool-calls.json` pins `run.tool_calls`: requests, requests refused by
field (a misspelled cursor, a named session, no `run_id`); pages following
`next` under a `limit`, in dispatch order, with one call for each named
outcome (a reused model id is a different call), and a run with no calls;
and refusals: `unknown_run`, a cursor never given, `limit: 0`, and
`reply_too_large` naming the call. `run-tool-calls-describe.json` pins the
group's discovery. Checked by the `tool_calls::tests` and
`describe::tags_and_tool_calls_tests` modules of the wire crate.

# Context ceiling

`context-ceiling.json` pins the `context_ceiling` refusal of a send, one per
documented cause, writing nothing; `run.result` and `run.status` of a run
that ended in the `context_ceiling` state with its detail and no error; a
cause this role does not name and an unknown member, both tolerated; and,
for contrast, a provider's own overflow, which ends `error` with
`context_overflow`. Checked by the `errors::context_ceiling_tests` module of
the wire crate.

# Serializer profile `anthropic-messages`

`serializer-anthropic-messages.json` pins the profile's entry bodies:
`cache_control` removed at every allowed place (top-level content blocks of
a user and an assistant message, and `text`, `image` and `document` blocks
nested in a `tool_result`) and kept inside `tool_use` input, in a nested
block of another type and inside an unknown block; string content and a
`tool_result` with string content; and resends that move the cache marker
(a string turning into a one-block array, and back), where the entry keeps
the first-ingested form. Checked by the `serializer_profile::tests` module
of the wire crate.

# Run usage

`run_status.usage_plain.json`, `run_status.usage_unreported_step.json`,
`run_status.usage_model_switch.json` and `run_status.usage_reasoning.json`
are `run.status` bodies a runner (Broca) produced from its real run loop,
each carrying a `usage` (contract §6.4) beside runner members the role does
not type (`final_step_finish_reason`, `retries_used`): three reported steps
on one model; three steps of which one reported no usage (and one reported
only `output_tokens: 0`, so the totals are the first step's); one step on one
model and two on another with a variant; and three reported steps on one
model, as in `usage_plain`, with `reasoning_tokens` reported on two of them. Each `usage` round-trips byte for byte, its
`by_model` sums to its totals, and an absent total decodes as `None`.
Checked by the `usage::tests` module of the wire crate.

# Send status

`send-status.json` pins `send.status` (contract §9.8): a request, requests
refused by field (a misspelled `send_id`, a named session, the send's body
beside its `send_id`, no `send_id`); replies, each the `SendReply` a re-send
of the same `send_id` would get: `pending` with a `submission_id` and no
run, a queued send delivered as a steer (`step` with its row and the run it
joined, while that run runs and after it ended and the runner restarted),
one that started a run (`turn`, its `ref` and `run_id` the same run), and
`unknown` with and without the run it was handed to; and the terminal
`unknown_send` refusal of a `send_id` the session has no record of.
`send-status-describe.json` pins the group's discovery, its missing-op
refusal and an answer without the group. Checked by the
`send_status::tests` and `describe::tags_and_tool_calls_tests` modules of
the wire crate.

# Declared run parameters

`run-parameters.json` carries all three role-meaning members in their existing
wire positions, pins the complete `runner_params` bytes (including unrelated
siblings), and names the malformed-value refusals for each member plus the
later-plan-tools refusal. `run-parameters-describe.json` declares all three
independent groups; `run_parameter_groups_are_independent_and_require_send`
also checks each group alone and without its required `session.send` op.
