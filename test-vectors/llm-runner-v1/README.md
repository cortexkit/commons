# `llm-runner/v1` test vectors

The shared bytes every `llm-runner/v1` implementation and consumer checks.
The role document is `crates/cortexkit-role-llm-runner/CONTRACT.md`. This
role is a draft: these vectors change with the contract until it is
reviewed.

| File | What it pins | Checked by |
|---|---|---|
| `role-describe.json` | `role.describe` answers a consumer accepts (with the capabilities each declares, including a `plans` runner and a steer-only runner with no `compaction` group and runners declaring `compaction` and `session_change`), canonical answers (one stating the `max_bytes` cap), a missing `stability` read as `alpha`, and answers it refuses with the problem: a missing major or required op, a declared group missing one of its ops (including `plans` without `session.send`, `interrupt`, `compaction` without `compaction.ready` and `session_change` without `session.flush_prefix`), no session-capability source, `transcript_reads` without `max_bytes` | `describe_vectors_check_as_recorded` (including a canonical `plans` declaration round-trip) |
| `read-requests.json` | `session.read` requests for each mode (tail, range, after), with the mode each selects, and refused requests with the `invalid_params` field: `after_mid` without `lineage_id`, `after_mid` with `from_ordinal`, a misspelled cursor, an unknown view, `after_mid` on the model view (naming `view`) | the wire crate's request round-trip and mode tests |
| `read-pages.json` | pages: a tail page, a page stopped by its byte cap with `next_from_ordinal`, a single message over the cap alone and whole on its page, run and dispatch attribution, originals, a denied call with no dispatch target, an empty transcript, a never-written session's empty page with no `lineage_id`; runner extras a decoder ignores; pages a decoder refuses (no `mid`, a non-object `head`); pages without a lineage that still carry messages or a next ordinal; the `lineage_changed` and `unknown_mid` refusals with the requests that get them; model-view pages (both `source` kinds, the compaction state absent before any compaction and named after it), model-view pages a consumer rejects with the problem, model-view entries it cannot decode, and the replacement-once series: the page holding a replacement's `first_ordinal` returns it whole, the next page and a read from inside its range do not repeat it, and a page that does is rejected | the wire crate's page and model-view tests |
| `head.json` | the `session.head` request, a request it refuses, answers with and without a last run, a never-written session's answer with every member absent, and answers inconsistent with their lineage (no lineage but another member present; a lineage without `updated_at`) | the wire crate's head test |
| `run-result.json` | the `run.result` request, requests it refuses, an answer per run state with whether it is terminal (including a completed run whose final message has no text, answered `text: ""`), answers inconsistent with their state (completed without `final_message`, running with one), text-part joins with nothing inserted, and the `unknown_run` refusal | the wire crate's run-result, final-message and text-join tests |
| `subscribe.json` | `session.subscribe` requests (absent, `start`, `live`, after a page's `head`) with their attach point, refused requests with the field, and stream events (durable, best-effort, unknown kind) | the wire crate's subscribe test |
| `send.json` | owner `session.send` requests per delivery mode, with a mark, and with a plan and runner parameters; refused requests (unknown mode, no `send_id`); delivery checks against the modes a runner declares, with the `delivery_unsupported` refusal for an undeclared one; admission checks of a plan's `compaction_item` against the `compaction` group, with the `invalid_params {field: "plan.compaction_item"}` refusal; replies, including the admission reply's baseline; refusals with their retryability and named field | `send_vectors_decode_and_round_trip`, `undeclared_delivery_modes_are_refused_by_name`, and `a_compaction_item_is_refused_at_admission_without_the_compaction_group` |
| `plans.json` | first send with a plan admitted by a `plans` runner (and its admission baseline), plan-less sends, and sends with a plan refused without `plans` as `invalid_params {field: "plan"}`; identical-repeat/no-op and different-plan/`plan_drift` envelopes with the unchanged baseline | `a_plan_is_refused_without_the_plans_group` executes capability admission and round-trips requests/reply; `frozen_plan_vectors_round_trip` round-trips requests, baseline and drift detail only (no runner fetch/freeze or comparison implementation in this wire crate) |
| `baseline.json` | the `session.baseline` request, a request it refuses, the `not_yet` answer, a fresh and a changed baseline, and the non-owner refusal | the wire crate's baseline test |
| `compaction-ready.json` | the `compaction.ready {session, request_id}` request, extras a runner ignores, requests missing a member, and checks: a ready for the newest request from the session's compaction provider calls again, one for an older request or before any is ignored, and one from another module or into a session without a compaction provider is refused `not_session_compaction_provider` | the wire crate's compaction tests |
| `fence.json` | provider-answer fence cases matching `compaction-provider/v1`: a stale request is superseded, an answer to the newest request exactly at its deadline is late, and one just before applies | `compaction_answer_fence_vectors_round_trip` checks metadata round-tripping only; these document §11.1 cases, not executable application checks in this crate. Answer shapes belong to the provider's role; a runner implements the fence |
| `session-change.json` | the `session_change` group's refusals (`rung_unsupported`, `pending_superseded`, `already_applied`), each with the op that gets it and its retryability | the wire crate's session-change test |
| `errors.json` | every refusal code the role defines, plus one it does not, with whether a caller retries it | the wire crate's retryability test |

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
