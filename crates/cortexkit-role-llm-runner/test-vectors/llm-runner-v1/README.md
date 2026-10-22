# `llm-runner/v1` test vectors

`read-descriptors.json` uses compact expansion recipes for large UTF-8 text
and arguments. The shared descriptor tests expand them before checking the
Anthropic top-level image, nested tool-result image, PDF/document, URL image,
text-sourced document, file/audio and paired described call/result layouts.
Recorded message/part hashes are over full inline content, not descriptors.
It also pins the twenty-part fallback: each 250-KiB payload is below
`PART_INLINE_MAX`, and the message budget selects descriptors for the first
eight parts. The resolve case verifies that the image-only tool-result part
gets one tag number while its image ref remains nested in place.
The mixed-size fallback records the exact described part set `[1, 3]`: part 3
is largest, and part 1 wins the tie with part 5 at the cut. Smallest-first and
plain part order produce different sets.

The shared bytes every `llm-runner/v1` implementation and consumer checks.
The role document is `crates/cortexkit-role-llm-runner/CONTRACT.md`. The
files live in that crate's `test-vectors/llm-runner-v1/` and ship in its
package; with the crate's `test-vectors` feature, its public `vectors` module
carries every file's bytes. `SHA256SUMS` pins every file, and a test checks
that every file here is listed in `vectors::FILES` with its exact bytes and
its SHA-256. These vectors change together with the versioned contract.

| File | What it pins | Checked by |
|---|---|---|
| `run_status.held.json`, `run_status.held_after_late.json`, `run_status.held_absent.json` | One run waiting for a held call and after its late answer; older active status bytes with no held field | `held_status_vectors_pin_bytes_and_wait_lifecycle` |
| `attachments.json` | Attachment discovery, every upload/media-read request and reply, open/committed/gone status, metadata probes/EOF, each new refusal, ordered send references, unchanged prompt-only bytes, read/model marker reasons, text/image/text tool-result joining, and indistinguishable other-session upload ids | `attachment_vectors_pin_wire_bytes_and_markers` |
| `role-describe.json` | `role.describe` answers a consumer accepts (with the capabilities each declares, including a `plans` runner and a steer-only runner with no `compaction` group and runners declaring `compaction` and `session_change`, and one declaring `run_cause` beside `live_events`), canonical answers (one stating the `max_bytes` cap), a missing `stability` read as `alpha`, and answers it refuses with the problem: a missing major or required op, a declared group missing one of its ops (including `plans` without `session.send`, `interrupt`, `compaction` without `session.resolve` and `session_change` without `session.flush_prefix`), `compaction` without `tags` (`compaction_without_tags`), `run_cause` without `live_events` (`run_cause_without_live_events`), no session-capability source, `transcript_reads` without `max_bytes` | `describe_vectors_check_as_recorded` (including a canonical `plans` declaration round-trip) |
| `read-requests.json` | `session.read` requests for each mode (tail, range, after), with the mode each selects, and refused requests with the `invalid_params` field: `after_mid` without `lineage_id`, `after_mid` with `from_ordinal`, a misspelled cursor, an unknown view, `after_mid` on the model view (naming `view`) | the wire crate's request round-trip and mode tests |
| `read-pages.json` | pages: a tail page, a page stopped by its byte cap with `next_from_ordinal`, a single message over the cap alone and whole on its page, run and dispatch attribution, originals, a denied call with no dispatch target, an empty transcript, a never-written session's empty page with no `lineage_id`; runner extras a decoder ignores; pages a decoder refuses (no `mid`, a non-object `head`); pages without a lineage that still carry messages or a next ordinal; the `lineage_changed` and `unknown_mid` refusals with the requests that get them; model-view pages (both `source` kinds, the compaction state absent before any compaction and named after it), model-view pages a consumer rejects with the problem, model-view entries it cannot decode, and the replacement-once series: the page holding a replacement's `first_ordinal` returns it whole, the next page and a read from inside its range do not repeat it, and a page that does is rejected | the wire crate's page and model-view tests |
| `head.json` | the `session.head` request, a request it refuses, answers with and without a last run, with `first_ordinal` at 0 and after a cut with its `cut` (including a cut that retained nothing, one above `last_ordinal`, and a host cut before adoption with `first_ordinal` 0), a never-written session's answer with every member absent, and answers inconsistent with their lineage (no lineage but another member present; a lineage without `updated_at` or `first_ordinal`; a `first_ordinal` more than one above `last_ordinal`; a `first_ordinal` above 0 without `cut`; a `cut` without a lineage; a lineage without `history_version`; a `history_version` without a lineage) | the wire crate's head test |
| `history.json` | `session.history` requests for each position (newest, cursor, `from_mid`, `before_mid`, `from_ordinal`, `before_ordinal`), pages carrying `history_version` and refused requests with the `invalid_params` field (two positions at once, the model view, a read cursor and a lineage field, an unknown view); pages (the window with tags, numbered entries below the window without tags, host entries without ordinals directly before the lowest ordinal, a never-written session); pages refused by name (tags below `first_ordinal` or on an unnumbered entry, an unnumbered entry after a numbered one, ordinals that do not rise, a duplicate `mid`, entries without a lineage, a lineage without a history version, a history version without a lineage); undecodable entries (no `written_at`, no `mid`); and the `unknown_mid`, `unknown_ordinal`, `cursor_expired` and `invalid_params` refusals | the wire crate's history tests |
| `run-result.json` | the `run.result` request, requests it refuses, an answer per run state with whether it is terminal (including a completed run whose final message has no text, answered `text: ""`), answers inconsistent with their state (completed without `final_message`, running with one), text-part joins with nothing inserted, and the `unknown_run` refusal | the wire crate's run-result, final-message and text-join tests |
| `display-anthropic-messages.json` | `anthropic-messages` bodies and the display row `display::anthropic_messages` projects from each: a string content, text and reasoning in order, a tool call with its compact JSON arguments, a tool result, an error result joining its text blocks, files and media described by refs (a base64 image, a URL image, a PDF, an image nested in a tool result), blocks shown as `other`, multi-byte arguments, results and error results cut on a UTF-8 boundary with their flags, the same bodies untouched without `limits`, content that fits its bound unflagged; and bodies refused by name | `anthropic_messages_vectors_project` |
| `display-opencode-aisdk.json` | OpenCode stored messages `{info, parts}` and the display row `display::opencode_aisdk` projects from each: the model from `info.model` and from `providerID`/`modelID`, reasoning and text with bookkeeping parts left out, a completed tool part as its call and result under one part key, an errored and a pending tool part, file parts as refs and URLs, a tool result's attachments, an `other` part, multi-byte cuts on a UTF-8 boundary with their flags and the same message untouched without `limits`; and bodies refused by name | `opencode_aisdk_vectors_project` |
| `display-requests.json` | `session.history` requests in display form (with and without `limits`) and `raw` spelled out, with the form and position each selects; requests refused naming `limits`, `media`, `include_originals` or `form`; a display page with a host entry, a tagged window entry carrying `client_message_id` and an assistant row with `model`; a display page whose user rows name their `sender` (the owner's, a named sender's beside its `client_message_id`, and none for a prompt typed into the host); `session.head.input` answers (`no`, modalities without types, exact types, `unknown`, an unnamed value read as not known); sends with and without `client_message_id` and sends refused naming it | `display_request_page_head_and_send_vectors` (each row's `sender` checked where a page names them) |
| `live-events.json` | every typed live event (`message_started`, `message_finished`, `tool_started` whole and cut, `tool_finished` with a known and an unnamed outcome, `run_started` with and without `send_id`, `run_finished`, `run_paused`, `text_delta` with and without a tool call, `reasoning_delta`, `tool_input_delta`, `display_gap`, `display_reset`); frames with `seq` and `event` on both lanes and a frame of a runner without the group; events a reader reads as `display_reset` (an unknown type, no type, a known type missing a member); `session.subscribe` requests with and without `limits`, which a runner without the group refuses naming `limits`, and requests refused by name; `run_causes`: a `run_started` naming the queued send that started it, one naming the interrupting send, and one without `send_id` for a host turn on an adopted session | `event_vectors_decode_strictly_and_round_trip`, `run_started_names_its_cause_or_none`, `an_unknown_or_malformed_event_reads_as_display_reset`, `subscribe_requests_with_limits_are_strict` |
| `subscribe.json` | `session.subscribe` requests (absent, `start`, `live`, after a page's `head`) with their attach point, refused requests with the field, and stream events (durable, best-effort, unknown kind) | the wire crate's subscribe test |
| `send.json` | owner `session.send` requests per delivery mode, with the runner-specific `mark` member, and with a plan and runner parameters; refused requests (unknown mode, no `send_id`); delivery checks against the modes a runner declares, with the `delivery_unsupported` refusal for an undeclared one; admission checks of a plan's `compaction_item` against the `compaction` group, with the `invalid_params {field: "plan.compaction_item"}` refusal; replies, including the admission reply's baseline and a delivered `step` with its `ref` but no `run_id` when the runner does not declare `run_ops` (alongside the reply with `run_id`); refusals with their retryability and named field | `send_vectors_decode_and_round_trip`, `undeclared_delivery_modes_are_refused_by_name`, and `a_compaction_item_is_refused_at_admission_without_the_compaction_group` |
| `plans.json` | first send with a plan admitted by a `plans` runner (and its admission baseline), plan-less sends, and sends with a plan refused without `plans` as `invalid_params {field: "plan"}`; identical-repeat/no-op and different-plan/`plan_drift` envelopes with the unchanged baseline | `a_plan_is_refused_without_the_plans_group` executes capability admission and round-trips requests/reply; `frozen_plan_vectors_round_trip` round-trips requests, baseline and drift detail only (no runner fetch/freeze or comparison implementation in this wire crate) |
| `baseline.json` | the `session.baseline` request, a request it refuses, the `not_yet` answer, a fresh and a changed baseline, and the non-owner refusal | the wire crate's baseline test |
| `session-change.json` | the `session_change` ops' requests (`session.refresh`, `session.refresh_policy`, `session.flush_prefix`) and requests refused naming the field (an unknown field, a missing member, a policy naming no rung); every refresh answer shape (`run_in_progress` and `applies_from_next_run` each way) and the policy and flush answers; baselines read after a lost refresh reply, still pending and already applied; every refusal with its op, typed detail and retryability (`rung_unsupported`, `pending_superseded`, `already_applied`, `send_id_reuse`, `generation_stale` at and below the accepted generation, `tool_outside_host_set` for an unregistered tool and for a changed definition, and admission's `fetch_unavailable`, `plan_stale` and `tool_name_collision`); host-set membership by whole definition; how a generation classifies (new, identical repeat, stale); and which sends are drift while a change is pending | the `session_change` module's tests and `session_change_vectors_name_group_ops_and_refusals` |
| `reply-envelope.json` | reply bodies: a wrapped `{"result": ...}` answer accepted (an object and a bare string inside `result`), bare answers refused `bare_answer`, an extra top-level key refused `extra_keys` naming it, a wrong inner shape refused `bad_answer`; refusal bodies with only `code`, `message` and `detail` (`class` inside `detail`); a stream event carried bare | `reply_envelope_vectors_read_as_recorded` |
| `errors.json` | every refusal code the role defines (`before_lineage_start` with its `first_ordinal` detail and `cursor_expired` among them), plus one it does not, with whether a caller retries it | the wire crate's retryability test |
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
past `u64`); read entries of one lineage carrying `tags`, one row per
exposed part, with consecutive numbers on the counted parts only (the
model's text counts and its reasoning and tool call do not, an image in a
user message and a `turn_system` message carry none, an image-only tool
result counts), with `role` and `written_at` in the envelope; the model's
own tag notation cleaned from its exposed text; `session.resolve` requests,
requests refused by field, and cases against the parts of the current
lineage (the whole part, a character range, a range counted in Unicode
scalar values, an empty range at the end, the model's text as exposed,
`unknown_tag` for a number no part of the lineage carries, malformed tags, a
range past the content and one starting after its end); the plan's
`render_tags` member, off when absent or `null`, refused when not a boolean
or when `true` without the group; rendered parts for every part kind (the
number, one space and the text with its leading tag notation stripped, an
image-only tool result's label, nothing on a part that does not count); and
a dropped part's placeholder. `tags-describe.json` pins the group's
discovery, its missing-op refusal and an answer without the group. Checked
by the `tags::tests` and `describe::tags_and_tool_calls_tests` modules of
the wire crate.

`tag-cleaning.json` pins the cleaning of the model's own text, the
render-time strip of leading tag notation, and the rendered prefix. It was
generated by running functions from standalone Magic Context, the plugin that
manages context inside OpenCode and Pi
(`stripPersistedAssistantText`, `stripTagPrefix` and `prependTag` in its
plugin's `tag-content-primitives.ts`) with one fix: a `§` followed by
digits, then `.` and a digit, is never a dangling tag, at any digit count
(`see §12.3 now` cleans to `see 12.3 now`, `§5.1` to `5.1`). The source pattern
backtracks one digit in multi-digit section references (`§12.3` cleans to
`2.3`); those rows record its output as `standalone_before_fix`. Every other expected value is what
standalone produces. Rows whose `source` names one of standalone's test
files (`tag-content-primitives.test.ts`, `text-complete.test.ts`) are that
file's cases, and the generator checked standalone still returns what those
tests expect; rows from `commons` are this crate's own (a section reference
beside a tag, decimals after two and three digits, JavaScript's
whitespace set, tags inside JSON, leading shapes needing several rounds).
Standalone's tests of `byteSize`, `isThinkingPart` and the single-step
helpers (`stripTagSectionCharacters`, `stripWellFormedLeadingTagPrefix`,
`stripDanglingTagNotationGlobally` beyond the cases above) are not ported:
they test helpers, not the rules. `exposure` pins who wrote a text against
how it is exposed: cleaned for a tag-rendering run, byte for byte for an
untagged run, and raw for a structured-output run's final text. Checked by
the `tags::cleaning_tests` module, which also checks that cleaning,
stripping and rendering are idempotent on every row.

`tag-numbering.json` pins the counting predicate for every part kind;
numbering from a checkpoint in rendered order (storage order different from
rendered order, a marker checkpoint, a Setup checkpoint at its cut, gaps
behind the tail with a dropped part keeping its number, the history head and
a marker uncounted); and the anchor's fallback chain (a cut with no
`tail_start_id`, `tail_start_id` as the cut, a marker not in effect, a marker
that lost its key, a Setup checkpoint at its cut, a marker checkpoint
winning over it, the Setup cut gone after a revert, no anchor at all, and a
key naming another session marking a fork); and, under `lost_keys`, markers
that lost their checkpoint key (the newest key lost with an older keyed
marker; every key lost, falling back to the Setup checkpoint or to the
session start; a newer cut inside the older marker's retained tail), where
counting every part from the fallback's cut, marker pairs never counting,
gives exactly the numbers computed with every key present; and, under
`overflow`, a checkpoint of `u64::MAX - 1` with two counted parts, refused
naming the checkpoint and the count, beside numberings that end exactly at
`u64::MAX`. Checked by the `tags::numbering_tests` module.

`tag-checkpoint-describe.json` pins the `tag_checkpoint` group's discovery,
its missing-op refusal, an answer without the group, and the refusal of the
group without `compaction`. Checked by
`describe::tags_and_tool_calls_tests`.

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
works; a list whose modes have no individual declaration refused
`delivery_unsupported` with its exact array in `detail.delivery`; mode choices at boundaries (fall-through order, order winning over what else
is possible, a list without `idle_start` never starting a run, `queue`
waiting for the run to end); a plan sender's typed template with uses that
render or name their problem (a percentage over 100 or written as a string,
an integer outside its bounds, a part the carrying request does not show (`template_part_gone`), free text, a number or a malformed part reference in a `part_tag` slot,
an unlisted enum value, missing and undeclared slots, another digest, an
unknown template); what goes out (a rendered template as system text, or
as the same text at user authority after a fall-through, a plain prompt at
user authority) and the refusal of every template that cannot render (naming `template`, or
`template_part_gone` naming the slot whose part is not shown); template sends, which carry no `prompt`, and the refusals of
a template beside a prompt (empty or not) and of a send with neither; plans with senders, valid and refused
(including a string slot and an `interrupt` grant that is not a boolean);
admission of the owner, named senders with and without the `idle_start`
grant, and unnamed modules refused `scope_owner_mismatch` even when a
request field names a sender; `string_admission`: send requests with a
string `delivery` from the owner, from a sender granted `interrupt`, from
senders without that grant (refused naming `delivery`), from an unnamed
module, and a named sender's string `queue` and `steer` admitted on its
`idle_start` grant (checked by `string_deliveries_follow_the_sender_grants`); dedupe and
expiry; `superseded`, `expired` and `template_part_gone` send states (the last
naming its slot); and entries recording each
mode. Template digests are SHA-256 over JCS bytes and are checked by
recomputation. `delivery-order-describe.json` pins the group's discovery.
`delivery-order-modes-describe.json` pins a runner advertising only `steer`
and the six malformed describe cases: an unknown name, a duplicate, an empty
list, missing `steer`, missing `queue`, and missing `delivery_order`.
Its declared and undeclared expectations are checked by
`describe::delivery_order_modes_tests`.
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
`context_overflow`. Its `waiting` section is a run held at the ceiling,
active with `reason: context_ceiling_wait` and `deadline_ms` on
`run.status`, `session.head` and the stream, and its ends; its
`compaction_waiting` section is a step held while it waits for the
compaction provider's answer to a rewrite ride (compaction-provider §8),
active with `reason: compaction_wait` and the ride's `deadline_ms`, and the
ride's end, after which the run carries no deadline. Checked by the
`errors::context_ceiling_tests` module of the wire crate.

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

## Trailing directives and interrupted reasons

`directives/*.json` (except `wire.json`) are `{text, expected}` parser cases:
`expected` is a wake result object or null. They cover every form/error,
closed and unclosed fences, mid-text tags, trailing whitespace, empty notes,
nine-digit hours and multibyte notes at 200/201 bytes. `parts.json` also
carries two text parts separated by a tool part and their joined final text.
`directives/wire.json` pins a send with raw directive parameters and a result
answer; its send round-trips byte for byte. `directives::tests` consumes every
parser vector and checks frozen/invalid requests and open consumer decoding.

`interrupt-reasons.json` pins both known interrupted-run reasons, an absent
historical reason and a runner-specific reason, decoded on result/status/head.
`run-tool-calls.json` now carries `position` and finished-call `result_ordinal`:
message-array results at the next ordinal and embedded results at the same
ordinal, including repeated model ids across sibling positions.

`user-commands.json` covers the independent owner relay, frozen provider-qualified
commands, open reply/status states including the stored terminal `outcome_unknown`
status, the optional plan `interactive` boolean and safe host menu naming. The
absent-command baseline reuses `baseline.json` and pins its unchanged JCS bytes.
Command output is never transcript or model input.
