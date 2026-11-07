# cortexkit-role-llm-runner-conformance

The `cache_policy` cases are N/A without the declared group and send no cache
requests to such a runner. Declaring runners are checked for plan/creating-send
decoding, exact pre-acceptance refusals, continuation and both-location refusal,
and actual lifetime (`ttl_ms` or `implicit`) plus warming state on step hints
where `step_events` is available. Plan cases additionally need a plan fixture.
The kit does **not** test real-time warming: schedules, local-time windows,
cost/count caps, session-end stopping and cache expiry need runner-specific
tests. These cases make no claim to have exercised that behaviour.

`LlmRunnerSubject::cache_policy_zone()` supplies an IANA zone the runner knows,
defaulting to `Etc/UTC`; an adapter can override it. Zone knowledge is not
discoverable through the flag-only capability, so the kit uses this fixture
for hours requests instead of requiring every runner to know the same zone.
The unknown-zone refusal still uses the invented name `Example/Nowhere`.

The independent `history_intake` group needs only `session.head`. Its two cases
use the defaulted `begin_history_intake`, `advance_history_intake` and
`finish_history_intake` hooks to hold a growing native-history import. They
require read/send/run fixture operations, check raw history where served, and
grade absent fixtures not applicable. Provider observations establish that an
accepted send starts only after all imported entries have been stored.

Version **0.17.19** pins `cortexkit-role-llm-runner =0.8.15`. The independent
`media_refs_*` cases use the defaulted `media_tool_result` fixture and check
media references, unchanged large inline payloads, descriptor identities, and
raw history where served. The `plan_notes_*` cases use the defaulted
`plan_notes_fixture` hook: configure a stable prefix, initial plan offering the
removed tool, and a pending plan removing that tool and changing system text.
Observe note text at the model endpoint through `model_requests`, and calls at
the tool provider, never reconstructed from the fixture or management replies.
Absent optional fixtures are graded not applicable, not passed. Faulty fake
runners exercise every new check, including replacing the short system note
with the entire new system text.

Version **0.17.14** pins `cortexkit-role-llm-runner =0.8.12` and adds
`message_marks_served`, `message_marks_both_refused`, and
`message_marks_speaks_for_owner_only`. All three require `message_marks`; only
the owner-only case needs the optional non-owner carrier route hook.

The served-marks case also needs `read_display_facts`, `history_display`,
`host_history`, `queue` and `transcript_reads`; both refusal cases need `queue`
and `transcript_reads`. A subject declaring `message_marks` supplies an
`owner_stamp()` on a real scoped route owned by that principal. The owner-only
case uses `send_fields_with_plan` to grant the carrier an `idle_start` send when
`delivery_order` is declared; that hook is already required by `delivery_order`.

Version **0.17.13** pins `cortexkit-role-llm-runner =0.8.11`. The six
`notices_*` cases are inapplicable without `notices`; they check
entry-free delivery, unchanged host output, exact reader/display facts and
live events, provider append bytes and the tool-result-only authority rule.
Facts include the carrying `call_key` and `delivered_as: "tool_result"`;
delivery events include the notice text. The sender-binding case also checks
raw read entries when `read_display_facts` is declared.
Adapters implement `notices_stored_output` by reading native storage
independently of runner reads, returning `(mid, part, call_key, output)`.
They record actual provider texts in `ModelRequestObservation.rendered`
and all system/developer texts in `system_text: Some(...)`. An unobserved
authority list (`None`) cannot pass the authority check. These checks use
held scripted tool calls, never inferred delivery or provider configuration.

The `head_selection_*` cases check inherited plan tiers, explicit send-tier
overrides, immediate head visibility, that the head choice stays unchanged
when an unsupported model or tier is refused, and unchanged refreshes after
host-side switches. These refresh-based cases require both `plans` and
`session_change`. Implement
`head_selection_models(session)` with a selectable model and an unavailable
one; the selectable route supports fast but not ultrafast when `service_tier`
is declared. With a missing group or model-fixture hook, the report marks
each affected case `Inapplicable` with its name and reason, never passed.
The ordering case also uses `switch_head_model`, whose default is unsupported.

The head-selection cases do not yet check:

- which model a run actually used;
- that a creating send's `plan.model` drives its first run;
- that a send's own model selection overrides the recorded selection.

Version **0.17.10** pins `cortexkit-role-llm-runner =0.8.8`, adding opt-in sender
attribution checks and deliberately faulty relay implementations.
The role remains
`llm-runner/v1` after the breaking 0.8 release. The runner's
[CONTRACT.md](../cortexkit-role-llm-runner/CONTRACT.md) is normative; the public
`CASES` registry names every executable case, its requirements and what it
checks.

The suite drives a **real runner** over its actual management route, through
its `cortexkit-role-harness` adapter, with a scripted model and tool provider.
The in-process fake under `tests/fake` tests the suite itself, never a runner's
production compliance. A production runner uses this crate as a dev-dependency.

## Changes from 0.16

- Remove compaction-provider/v1 fixtures, dependency and cases: Setup, descent,
  notices, views, due/pending promises, rides, ceiling-provider handling,
  checkpoint anchors and copied-checkpoint fork detection.
- Remove the tags/resolve and model-view cases and their adapter hooks.
  Compaction-runner conformance belongs to the compaction protocol's kit,
  not this one.
- Count active-path host records (excluding markers) from 1 under
  [compaction W4.1/W4.2](../cortexkit-role-compaction/CONTRACT.md#w4-the-neutral-part-model) and read the whole active path,
  including entries below native host markers. Zero is an alias for the
  first entry, not a distinct ordinal.
- Replace lineage-bound reads with required `history_version` and
  `incarnation` cache identity on heads and pages. Stale after-mid cursors
  fail `history_changed`.
- Keep no-provider archive pruning, with served part keys. Provider-enrolled
  sessions cannot prune until the compaction contract defines acceptance;
  the former Setup ready.accepts permission and post-prune Setup/view tests
  are gone.
- Add run-fixed routing-decision, route-bound user-command, both send-status
  names, and the 64 KiB tool-result replacement checks.

## Running the suite

Implement `LlmRunnerSubject` and its harness/route associated types. Then call
`run_suite(&subject, empty_work_directory)` or `run_suite_with` with progress
options. The directory must be empty. The suite owns a crash driver and
records which advertised kill points were actually reached.

The report has a named outcome for every case. Missing capability requirements
are skipped; an unsupported optional fixture is not applicable by name, never
passed. A simulated kill does not satisfy the suite's real-process-kill
requirement. Do not infer success only from the absence of an individual case
failure: inspect the report's verdict and kill ledger too.

The runner adapter supplies:

- attested owner and stranger route identities and one route per session;
- scripted model turns and exact provider tool answers, with held-call
  release/count observations kept outside the runner's process and stores;
- runner-owned send/plan fields, serializer-profile decoding, and native part
  replacement where a case requires it;
- the real persistence and restart path, never a test-only acknowledgement
  standing in for a durable write;
- optional native-history, marker, revert, retention-clock and pruning hooks
  only when the host can really create those situations.

Observe model requests at the model endpoint and provider calls at the
provider. Never rebuild expected observations from the runner's configuration
or its management replies. Native body decoding belongs to the adapter;
opaque part keys come from the served envelope, not a reader's invented index.

## New named cases

| Case | What an incorrect runner must not do |
|---|---|
| `history_version_on_read_pages` | Omit the rewrite version from a read page. |
| `stale_read_cursor_history_changed` | Answer a stale after-mid cursor instead of refusing `history_changed`. |
| `read_from_ordinal_zero` | Skip the first entry when the request starts at zero. |
| `incarnation_on_every_page` | Omit the required session-record identity or disagree with the head's cache pair. |
| `read_below_host_marker` | Hide or renumber entries from reads when a native marker is written. |
| `route_decision_id_fixed_per_run` | Accept a joining send with another decision id. |
| `user_commands_without_session_ref` | Require a session selector in either route-bound command operation. |
| `send_status_both_names` | Serve only one of the canonical and compatibility operation names. |
| `archive_pruning_part_keys` | Name a pruned part absent from its entry's served keys. |
| `archive_pruning_provider_blocks_pruning` | Prune a plan with a compaction provider before acceptance is specified. |
| `tool_result_too_large_is_error` | Cut provider output short or lose the synthetic reason, measured bytes, limit or discard text. |
| `tool_result_too_large_not_repeated` | Dispatch an oversized-result call again. |
| `output_refs_without_compaction_unknown_tag` | Resolve a reply reference without `plan.compaction_item`, change its `unresolved` / `unknown_tag` record after another tool output, or serve content for it instead of `unknown_ref`. |
| `scoped_session_start_owner_only` | Admit a non-owner carrier's session-creating send under a plain scope, or show a visible session after refusing `scope_owner_mismatch`: the owner must see an empty read and head, and no ready baseline. |
| `display_without_limits_is_whole` | Cut arguments or a kept non-error tool result without requested limits; the fixture exceeds the app's 16,384-byte result bound but stays below the pre-render provider acceptance limit. Error-result truncation is not exercised. |
| `read_display_facts_created_at` | Omit or change a recorded host creation time, substitute `written_at` or `completed_at`, or expose it without the opt-in. |

Each new case is exercised against a deliberately faulty fake and verifies
rejection under the case's name. The unnumbered-reference case has
immediate- and late-resolution fakes; no fake reaches its `unknown_ref`
read check. Existing unrelated admission, send receipts,
run results, usage, display/live events, attachments, retention, cancellation,
structured output, tool identity and crash guarantees remain in the registry.

`scoped_session_start_stamps()` optionally supplies carrier and owner stamps
under the same daemon-admitted plain scope, with no flow or run id. The adapter
must open real routes with those stamps. Without the hook, the scoped-start
case and `message_marks_speaks_for_owner_only` report `Inapplicable` by name;
the other message-mark cases still run. Flow and run scope creation is optional
under the contract, so no case requires admission there without a fixture.
`flow_session_start_stamps()` optionally supplies a real flow-scope opener and
owner. The `frozen_plan_senders_send_refused` and
`frozen_plan_senders_refresh_refused` cases require `invalid_params` naming
`plan.senders` for a plan naming anyone but that opener, with unchanged head,
history and baseline. Without real flow-scope routes they are inapplicable by
name. `reports_read_completed_at()` enables `read_display_facts_completed_at`:
a finished assistant entry has a host finish stamp, and user entries have none.
The case does not compare it to other timestamps: `written_at` can be first
ingest rather than creation time. Without host finish reporting that
case is inapplicable, without skipping the other display-fact checks.
`seed_host_created_at(session, mid, created_at_ms)` sets the host's recorded
creation time for an existing entry, independently of the read route. The
`read_display_facts_created_at` case seeds user and assistant times at least
60 seconds from any `written_at` or `completed_at` the initial read returns,
then requires exact values with the opt-in and absence without it. When the
hook returns false (the default), that case is `Inapplicable` with a reason
naming the hook, never passed.
These cases observe public wire replies, not storage. The scoped-start case
checks that refusal leaves no visible session. The frozen-plan cases check
that refusal leaves the existing session's head, history and baseline
unchanged.

The whole-display case uses a kept non-error result above the app's result
bound and below the provider acceptance limit. It does not exercise error-result
truncation. CONTRACT §11.3.1 measures pre-render provider output, so that limit
does not bound rendered display text.

The run-fixed routing-decision case uses `route_decision_id_fixture(session)`.
Its default literal suits runners without routing accounts. A routing-backed
adapter overrides it with a valid decision seeded in routing for the session's
provider and model; the test does not synthesize routing replies.

### Runner-owned routed accounts

Declare `Capability::RoutedAccounts` when the runner advertises the
`routed_accounts` group. Live discovery validates its 1–64 distinct auth-method
tokens with the runner contract's helper. Two cases require three new hooks;
each hook's default is an error naming the missing hook, not a pass or skip:

- `routing_runner_declaration()` returns the raw routing `runner` slot.
  `routed_accounts_follows_account_choice` requires its
  `follows_account_choice` member to be the JSON boolean `true`.
- `routed_account_fixture(method)` returns public ids
  `{provider, account_id, credential_id}`. Provision a credential of that
  method in the runner's own store that the scripted model accepts. Never
  return credential secrets.
- `install_routing_peer(session, peer)` directs that session's actual
  `route.account_for` and `route.account_observe` requests to a cloneable
  `RecordingRoutingPeer`. Configure the session's plan/model for
  `peer.method()` and `peer.fixture()` before its first send. Call
  `peer.request(authenticated_runner, operation, body)` with actual incoming
  bodies and the calling runner's identity from the route; return the peer's
  JSON RESPONSE body (`{result: ...}`) to the runner. Do not synthesize calls
  from management requests or model observations.

`routed_accounts_account_for` starts a fresh scripted run for **each** listed
method and requires an `account_for` admission naming the fixture provider and
a model invocation. The recording peer answers admit/renew with the fixture's
account ids and records all received bodies. Observation replay is per calling
runner: the first `event_id` receives a fresh observation id, `duplicate: false`,
`bound: true` and empty `effects`; repeats return that first id with
`duplicate: true` and empty `effects`, even if the retried body differs.

Without the capability the two cases are skipped by name. `NARROWINGS` records
that the live cases do not exercise automatic renewal, release timing,
`auth.pool`/credential forwarding, or independently inspect which secret reached
the provider. The peer's unit tests cover admit/renew replies and observation
replay; deliberately faulty runners missing `account_for` or the declaration
flag are rejected by the corresponding case's name.

Declare `Capability::AccountRoute` only with `RoutedAccounts` when the runner
advertises `account_route`. The two `account_route_*` cases are inapplicable
without it. `RecordingRoutingPeer::account_route()` exposes the kit's optional
synthetic routing names; `fixture()` continues to describe the vendor model's
provisioned account and credential. When route names are set, the peer accepts
`route.account_for` only under those names and returns the route provider with
the fixture's credential id. No new adapter hook is required.

`account_route_names_the_route` proves that the runner forwards `account_route`
verbatim rather than its vendor names; it does not prove that the runner can
load a pooled key. The kit mints unique `kit-pool-` route names, derived from
the vendor fixture's provider and a per-case model suffix. When `head.model`
is present it also checks both names differ; otherwise distinctness rests on
the minted prefix, without requiring new model metadata.
`account_route_fixed_per_run` holds a run and checks that a
different joining value refuses `invalid_params` naming `account_route`
without writing a message. Both cases have deliberately faulty runners that
fail under the violated case's name. Automatic renewal and reassignment are
not driven by these cases.

## Part numbers without the removed tags group

The retained `delivery_order_template_part_gone_refused` and numbered
`output_refs_*` fixtures need the number actually shown for a message/part.
They are **not applicable** when the kit's opening plan has no
`compaction_item`. The kit and adapter decide applicability from the opening
plan's `compaction_item`, never from the runner's answer. If the plan has a
compaction item, missing shown numbers still fail the existing assertions. Implement
`shown_part_number(session, mid, part)` using the runner's captured request
mapping, whose numbers follow
[compaction A3, Tag numbers](../cortexkit-role-compaction/CONTRACT.md#a3-tag-numbers). The kit does not compute numbers
from read ordinals or part keys and does not call `session.resolve`.
`part_digests` itself supplies descriptor keys in stored order; no duplicate
tag-row identity surface is required.

`output_refs_unsupported_plan` does not require numbers. The separate
`output_refs_without_compaction_unknown_tag` case removes `compaction_item`
from the adapter's output-reference plan, writes a reply reference after a
tool output, and checks `state: "unresolved"`, `reason: "unknown_tag"` and
`tag: "1"` (§9.13). It appends another tool output and checks the original
records are unchanged. `session.output_ref_read` must refuse `unknown_ref`
both before and after that append. The numbered `output_refs_*` fixtures and
`output_refs_without_compaction_unknown_tag` require the runner to declare
`output_refs`; the unsupported-plan case runs on non-advertisers.

## Oversized results and historical snapshot fixtures

`media_descriptors_frame_budget` and `media_descriptors_stable_across_pages`
put their aggregate fallback text in one **assistant message**, not a tool
result. The 19 mixed-size or 14 equal-size parts each fit `PART_INLINE_MAX`
but together exceed `DESCRIPTOR_MESSAGE_MAX`. The former checks largest-first
selection (including the earlier-part tie break); the latter requires exactly
two `text_ref`s for its 14 assistant text parts, not counting the image's
`media_ref`, and identical read messages on single-message pages with
`max_bytes: 1024`.
The frame case also keeps two sibling calls with over-threshold arguments
and requires an inline read of their combined message to refuse
`reply_too_large`. The aggregate text is assistant output, so it no longer
consumes the provider-result budget, and no host has to keep separate text
parts inside one tool result. The image must use a negotiated result format
that leaves image bytes out of the model-visible measure (`tool-result/v1`
image parts). The kit's guard checks text bytes in its reference fixture
schema, not every adapter's wire result.

`oversized_tool_result` decodes the actual native stored size-error result,
including its reason, `bytes`, `limit`, error state and model-visible text.
The deliberately faulty fixtures either truncate a successful output or
actually dispatch the provider call twice. A completed result over 65,536
model-visible bytes must instead be discarded whole, without a retry.

The retained `output_refs_snapshot_size` and `output_refs_snapshot_budget`
cases test their original snapshot thresholds, which can exceed 64 KiB.
They use `fixture_seed_historical_tool_output`, a **fixture-only** import of
stored output that predates the provider-result limit or was imported. The
hook never dispatches a provider or exempts a newly returned result from size
enforcement. An adapter without it grades the affected case not applicable
by name. Do not lower the snapshot thresholds to make these cases reachable.

## Archive-pruning fixtures

`archive_pruning_fixture` supplies independently scripted original/served
native parts, with every declared kind in two messages and an unaffected
part. Its boolean chooses whether the plan names a compaction provider; it
installs no provider protocol. `prune_archived_session` exercises the real
maintenance path, including the active-run and queued-send exclusions.

Every affected raw/read and raw/display history page repeats
`pruned_reasoning`. `part_digests`, when present, publishes the stored-order
keys; otherwise `part_keys` does. Readers match the listed part to those
keys and never synthesize one. `reasoning_pruned` remains on head/read pages,
with a durable generation, highest touched ordinal and commit time.

## Fork-answer fixtures

`ForkAnswer` adapters capture each raw model request through
`raw_model_requests`, attributing it to the head (`None`) or the issued
`fork_id`. Record unmodified JSON bytes, not a parsed and reserialized value.
Keep the observations outside the runner's state root across
restarts; a reconstructed digest is not an independent observation.
`advance_fork_clock` must complete due idle expiries before returning and
must not complete held model calls. The default hooks fail with their names.
The base-removal case uses `revert_session`; an adapter without that fixture
reports the rule inapplicable by name. Compaction isolation is deliberately
not observable in this kit. `rule-map.json` records that exception and the
executable cases for every pinned `fork_answer` rule.

The frozen request's conversation prefix, tools and settings are compared
lexically, including whitespace and member order. Only explicit Anthropic
content-block `cache_control` markers may relocate; the fork cannot add more
breakpoints than its head used or exceed four. A failure names the first
differing byte offset in the compared prefix or settings with both excerpts.

`configure_fork_head_key` optionally sets a 256-character routing key before
the first head send. Its default returns false, making only the bounded-key
case inapplicable with the hook's name. `routing_peer_error_replies` also
defaults to false: the wrong-account and wrong-tier checks still run, but
scripted routing-refusal mapping is inapplicable. Adapters opting in must send
a recording peer's `{"error": ErrorBody}` answer as an ERROR frame, not a
RESPONSE containing an error-shaped object. `set_account_refusal` scripts
those errors; `set_account_override` scripts a successful but wrong credential
or service-tier choice without changing admission-id replays.
The stepped-down-tier scenario starts the head at `fast` through the existing
`service_tier` group and `configure_service_tier` fixture. Without that fixture,
credential and tier mismatch checks still run, while the stepped-down scenario
is reported as an inapplicable rule rather than a plain pass.

## Scope and verdicts

The kit tests behavior through public routes and genuinely independent
observations. Native marker creation and imports are optional fixture hooks;
the kit does not claim provider-protocol conformance through them. Resident
memory and per-host integration still need the runner owner's measurements.
The public `NARROWINGS` inventory records boundaries of executable coverage.

## Sender attribution (0.17.10)

The kit pins runner `=0.8.8`. `sender_attribution` requires `delivery_order`,
`read_display_facts` and `history_display`. The `sender_attribution_*` cases
check exact stored header bytes, verified read/history facts, photo-only sends,
plan-authored templates, unaffected senders, tampered prefixes and header-free
rows. They are not applicable without the group; the complementary
`sender_attribution_undeclared_refused` case instead runs on an undeclaring
runner with delivery-order support. Every case rejects a deliberately faulty
fake and reports the violated case's stable name as failed.

Adapters supply their existing `plan_sender`, `send_fields_with_plan` and
stored-order `read_parts` hooks. For `sender_attribution`, also implement
`tamper_attribution_prefix`: change the stored first prefix byte to `!` without
changing its recorded metadata, independently of read replies. The photo-only
case additionally needs `media_only_send` and the upload fixture; otherwise it
is explicitly not applicable. Default reads omit the fact, opted-in reads keep
the full body, raw history carries the verified fact without a new request
flag, and display rows remove only a verified prefix. The kit observes stored
bytes through reads; it does not inspect the model's request or exercise a plan
refresh between intake and storage.

## Host-preserving runner restarts

Runner 0.8.18's additive `restart_survival` group is checked by kit 0.17.22.
The adapter's `host_survives_restart()` defaults to false: the eight
`restart_survival_*` cases are then not applicable, and existing crash checks
stay unchanged. An adapter returning true must expose the group (including
its `interrupt_reason` and `live_events` dependencies), and implement
`restart_survival_observation`. A missing hook fails, rather than earning a pass.

The kit selects a model-step, held-tool, send-handoff, unknown-host or recovering
experiment, and runs it with both crash and orderly restarts on isolated roots.
The hook drives real management routes and restarts only the runner process.
It returns unmodified replies/frames and host/provider records kept outside the
runner's process and state root; configured scripts are not evidence of execution.
Use a reply-custody provider for held keyed calls. Hold send confirmation after
handoff, then replay the same send_id and count all handoffs. For event experiments,
lose live text in the gap, finish with the supplied marker as stored assistant
text, and emit no newer live text deltas after reconnect; include the first run's
end and a later run's start/end in the captured control stream.

The checks cover natural completion with the same run_id, keyed execution once,
final send receipt replay without double handoff, stored-cursor continuity,
unchanged history identity, ordered catch-up with a display gap, immediate
unknown-host Stop, and stored reads/no-write mutation refusal during reattach.
For unknown-host Stop, keep another session active on the same host: resolving
liveness must interrupt only the stopped session's turn, never kill the shared
host. Faulty in-process runners exercise every check by name.

For surviving-host adapters only, `crash_at_DispatchIntent`,
`run_result_interrupted_not_cancelled` and `interrupt_reason_shutdown_durable`
are not applicable: those three assert restart-based sealing, which survival
replaces. Other crash properties, including durable identity and replay, remain
applicable.
