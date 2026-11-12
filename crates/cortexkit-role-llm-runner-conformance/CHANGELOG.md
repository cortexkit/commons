# Changelog

## 0.17.48

- Pin runner contract `=0.8.39` for the separate account-wait outcome.
  Extend refusal transport/predicate tests to cover both account-wait codes,
  plain retries, unknown codes and diagnostic details that must not change
  classification. Fork code checks also exclude account waits.

## 0.17.47

- Add opt-in `plan_view` cases for the closed validated reply, optional system
  item byte ranges, and `no_plan` after planless admission. Reuse ordinary
  scripted-session hooks; the no-plan case is not applicable when admission
  requires a plan. Test each case against a runner that intentionally violates
  its rule and require the kit to report the case's name.
- Pin runner `=0.8.38`. Add opt-in `session_class` cases for malformed labels,
  frozen-label changes, repeated or omitted labels, and later valid labels on
  unlabelled sessions. Use existing scripted-session hooks, with no new adapter
  hooks; runners without the group are not applicable.

## 0.17.46

- Pin runner contract 0.8.37 for the optional step-event `message_id` member.
  Step-event cases still use native-record and publication evidence; they do
  not read session message ids, so no message-id matching case is added.

## 0.17.45

- Add four opt-in keep-warm cases for idle `auto` warming, an exact one-rewrite
  budget, re-arming after a real request, and explicit `off`. Reject incorrect
  fake runners by each check's name, including early and late budget stops.
- Add defaulted price-fixture, scheduler-clock and raw provider-journal hooks.
  The kit drives real sends and computes the budget itself; adapters report
  requests and usage without classifying requests or deciding results. Missing
  fixtures remain not applicable. Existing public types are unchanged.

## 0.17.44

- Pin runner `=0.8.36`. Replace `signed_tool_deferred_addition_keeps` with
  `signed_tool_deferred_addition_strips`: even an unreferenced deferred addition
  must record a floor and strip earlier signed reasoning. Keep the adapter hook
  name and observation shape; update its contract and faulty-runner controls.
- Add `SignedToolChange::ToolNote` and `signed_tool_note_keeps` using the existing
  raw-ingress fixture. Appended system tool notes preserve signed reasoning only
  with byte-identical tools and earlier messages. Check raw JSON prefix bytes,
  reject a deliberately stripping fixture by name, and keep the default hook
  not applicable.

## 0.17.43

- Pin runner `=0.8.35`. Add four signed-tool-change cases: a tool edit strips
  earlier signed reasoning, reordering alone keeps it, an unreferenced
  deferred-loading-only addition keeps it, and resume renders identical bytes.
- Add the optional `signed_tool_change_observation` adapter hook, defaulting to
  `Ok(None)` (not applicable). Document live refresh, signed fixture and restart
  steps; the kit judges raw model-ingress bytes, canonical tool triples and
  markers itself, never adapter verdicts. Existing adapters keep passing.
- Reject deliberately incorrect fixture runners by each check's name, and test
  canonical schema equivalence and invalid fixture refusal.

## 0.17.42

- Pin runner `=0.8.34`. Add declared combined-mark and name round-trip checks,
  name retry-identity checks, and undeclared name refusal. Keep combined-mark
  refusal active only without `notice_with_speaks_for`, preserving its contract.
- Each new check rejects an intentionally incorrect fake by check name: rejecting
  the declared combination, dropping the name, ignoring changed-name retries,
  or accepting an undeclared name. Existing undeclared combination coverage stays.
- Patch bump: the new capability variant is on a non-exhaustive enum and no
  adapter signature changes. The operator approved the carried-agent attribution
  contract for release.
- Readers first: Magic Context deploys reading `Entry.speaks_for` and declares
  `compaction-entry-speaks-for/v1` before any runner sends the member. Core sends
  `notice` with `speaks_for` only to runners declaring `notice_with_speaks_for`.

## 0.17.41

- Pin the role contract to `=0.8.33`. Exclude `host_code` together with `host`
  from shared projection equality, reject code-without-host and non-user host
  marks. The independent `host_parts_agree_with_display_rows` case requires
  display marks/codes to agree with opted-in read `host_parts`, with no shared
  serializer profile needed. It is not applicable without both surfaces or
  when neither form serves host parts. Validate read-fact role, key membership,
  uniqueness and stored order, and keep host facts out of default reads. Add
  matching, mismatched, no-profile and read-only runner self-tests.

## 0.17.40

- Update the runner contract dependency to `=0.8.32`. The display projection
  check validates that host marks are emitted only as true on user parts, then
  excludes those marks from profile equality. All other differences still fail.
- Self-tests accept a host-marked user part and reject both host-marked assistant
  parts and unrelated projection differences by the projection check's name.

## 0.17.39

- Media cases never put an oversize argument on the image call; a runner that caps tool input before dispatch passes them.

## 0.17.38

- Add compaction-item plan-refresh cases for mandatory addition, removal refused
  with `invalid_params` at `plan.compaction_item` without changing the applied
  plan, and optional provider/preset/params switches with the same refusal field.
  Deliberately incorrect fakes are rejected by each case name.
- These checks use only the runner's own management route, so they cannot see
  provider Setup or whether an accepted switch keeps the applied view until the
  new provider applies one; that needs a compaction provider. The runner contract
  pin stays `=0.8.31`.
- Add optional `compaction_change_plan` and `compaction_switch_plan` adapter hooks
  for valid compaction plans used only by these cases, with defaults preserving
  the existing `send_fields` and `refresh_plan(ChangedTools)` fixture fallbacks.

## 0.17.37

- Correct `host_start_invalid_params_leaves_nothing` to require the contract's
  empty `session.head` success answer for a never-written session, not an
  `unknown_session` refusal. A different-plan restart with a new `send_id` also
  proves that the refused start left no plan record. Self-test fakes reject both
  the wrong head refusal and a leftover plan record.

## 0.17.36

- Pin runner `=0.8.31`. Add four `harness.host_start` cases, gated by the live
  describe's extension advertisement: invalid-parameter refusal leaves the
  session unknown and permits a corrected start, exact repeats replay, changed
  plans under the same id refuse `send_id_reuse`, and new ids with changed
  plans refuse `plan_drift`. Each rejects its own deliberately wrong fake.
- Extend the fake runner with faithful host-start creation and replay support.

## 0.17.35

- Pin runner `=0.8.30`, accepting unknown advertised delivery-order mode names
  without treating them as known declarations. Add a self-test that the kit
  accepts an unknown name alongside a declared known mode.

## 0.17.34

- Pin runner `=0.8.29`. Add seven optional cleared-ride cases covering confirmed
  recording, unconfirmed-route fallthrough, free-text pending and append-only
  fallthrough, plan-note exclusion, route-switch omission and no persistent
  upgrade. Each has a deliberately wrong fake rejected by case name.
- Default adapters skip these cases. Opt-in adapters must explicitly list
  `turn_system_cleared` in `delivery_order_modes`, as well as provide
  `turn_system_cleared_supported`, `configure_turn_system_cleared_route`, and
  model-ingress `cleared_system_text` observations alongside system text.
  General delivery-mode probes also omit the new mode unless explicitly listed,
  so older decoders never receive it.
- Let the call-yield queue case script and wait for the queued idle-start run.
  In the blocked store-before-yield case, poll for the yield request after
  durable staging before releasing the call; staging and yielding need not
  complete together. Keep the store-before-yield assertion unchanged.

- Record the media-only fixture's creating send before its follow-ups, so
  absent/empty-prompt media sends do not resend frozen tool configuration.

## 0.17.33

- Use bare module ids, not route-stamp principals, in plan sender fixtures;
  reject malformed fixture principals with a case-named error. The faithful
  fake now refuses reserved-principal spellings in plan sender module ids.
- Grant `idle_start` to the person sender in `call_yield_*` fixtures so legacy
  `steer` and `queue` delivery requests reach the behavior under test. The fake
  enforces the contract's delivery admission rules before accepting sends.
- Keep the exact runner pin `=0.8.28`; runners need no change.

## 0.17.32

- Accept missing tool-output media references only when that output is unchanged
  from inline; retain exact descriptor equality everywhere outside tool outputs.
  Flattening adapters must supply `media_refs_tool_output_pointer`; its default
  keeps the strict comparison everywhere.
- Pin runner 0.8.28. Check changed message marks under the same send identity
  and replay with identical marks.
- Cover the `admission_reports` describe token's round trip.

## 0.17.31

- Pin runner `=0.8.27` and rework `tag_read_before_undo`: a surviving call
  still resolves its original entry and part, while a removed call refuses
  `unknown_call` without changing session head or reads. Drop the replacement
  entry and reused-number fixture; the case remains inapplicable without undo.
- The faithful fake refuses removed calls, and a named faulty-runner self-test
  checks that the case rejects a runner that still answers them.
- Accept person-entry fixtures from a plan sender marked `person: true` as
  well as sender-less entries; keep checking the resolved sender exactly.

## 0.17.30

- Add seven `tag_read` cases and named faulty-runner checks for call-fixed
  person-message resolution, cut and undo retention, unknown calls/tags,
  owner-route authority and undeclared selectors. Fixture hooks default to
  inapplicable where the adapter cannot seed a person entry, cut or undo.

- Pin runner `=0.8.26` and add six `notice_keep_*` cases: owner acceptance and
  entry surfaces, required notice, owner-only admission before any write,
  undeclared refusal (including false), false-as-absent, and kept-ride refusal
  naming `delivery`. Each rejects a deliberately faulty runner by case name;
  a write-before-refusal fault also exercises the owner's no-write check.
- Cases are inapplicable without `notice_keep`, except the undeclared probe.
  Roll out runners and providers that decode `keep` before core sends it; core
  sends it only to runners declaring `notice_keep`.

## 0.17.29

- Pin runner contract 0.8.25 and add `notice_yield` cases for urgent hand-back,
  sender and delivery admission, owner-only plan grants, undeclared members,
  notice-marked sends and already-yielded call deduplication. Reuse the
  call-yield provider fixture and independent observations.
- Defend every new case with a named deliberately faulty runner; report
  behavioral cases inapplicable without the group and check undeclared-member
  refusals only on runners that do not declare it.

## 0.17.28

- Pin runner `=0.8.24` and add `live_attach_replays_step_controls` and
  `live_attach_idle_unchanged`, checking real current-step controls before
  the display snapshot and no replay when idle.
- Exercise missing replay, earlier-step replay and controls-after-snapshot
  faults, plus idle replay, with named faulty-runner self-tests. Display-only
  reset state retention remains a reader-side rule, outside the runner adapter.

## 0.17.27

- Pin runner contract `=0.8.23` and add named cache-key cases: valid keys on
  plans and planless creating sends, plus exact pre-acceptance refusals for
  empty, oversized, space-containing, control/non-ASCII and non-string keys
  (including null).
  Each case has a faulty-runner self-test that fails it by name. Undeclared
  cache-policy runners remain untouched and receive N/A outcomes.
- Cache-key checks cover admission, not provider-specific routing-hint mapping.

## 0.17.26

- Pin runner 0.8.22, whose display projection carries bad tool-result files
  and standalone parts without losing the rest of the row. No kit case
  depended on refusing these rows.

## 0.17.25

- Pin runner 0.8.21 so display rows preserve `media_ref` objects in OpenCode
  file `uri` and `url` fields and Anthropic base64 source `data`, instead of
  refusing rows or displaying empty payloads.

## 0.17.24

- Pin runner 0.8.20 and check the opt-in `send_ids` guarantee on keyed and
  unkeyed module sends. Non-declaring runners may omit the field; any present
  value must still match. Readers rely only on declaring runners or served ids.
- Check describe prerequisites and reject faulty read, history and unkeyed ids.

## 0.17.23

- Pin runner contract 0.8.19 and add seven `cache_policy` cases: valid plan
  and planless-send decoding, exact pre-acceptance refusal fields, frozen
  continuation policy, refusal of both policy locations, and actual cache
  lifetime plus warming state in model-step reports. Each case has a faulty
  runner self-test that fails it by name. Undeclared runners remain untouched
  and receive N/A outcomes. Real-time warming behaviour is not kit-tested.
- Add an overridable known-zone adapter fixture, defaulting to `Etc/UTC`,
  because the capability does not advertise zone knowledge. Keep an invented
  zone for the negative refusal check.
- Consume the contract's checksum-pinned cache vectors through its published
  `test-vectors` feature, rather than depending on a sibling source checkout.

## 0.17.22

- Pin runner 0.8.18 and add default-false `host_survives_restart` plus a
  crash/orderly restart experiment hook with independent host/provider evidence.
- Add eight named restart-survival checks, each rejected against a faulty fake:
  natural model/tool completion, keyed at-most-once dispatch, send handoff,
  cursor continuity, history identity, catch-up ordering/display gaps,
  unknown-host Stop and recovery operations. Opted-out adapters retain the
  existing crash checks; opted-in adapters use survival instead of the three
  checks requiring restart-based interruption.

## 0.17.21

- Check stop-all durable ordering, idempotent replay, isolated provider failure,
  plan-union targeting and interrupt admission through optional provider hooks.
- Pin the runner contract to `=0.8.17`. Add optional call-yield adapter hooks
  and named cases for person-steer admission, yielded held replies, late notices,
  prompt durable steer storage and Stop without withdrawal, with faulty fakes.

## 0.17.20

- Add `call_identity_imported_call_carrier` through the optional defaulted
  `seed_imported_tool_call` hook, with named inapplicability when unavailable
  and faulty fakes for re-minted keys, local carriers and dropped carriers.
- Add opt-in fork-answer
  conformance cases for base snapshots, tool refusal, replay, expiry, caller
  admission, head isolation, routing credential pins and restart recovery.
- Add raw model-request observations attributed to a head or fork and a named
  fork-clock hook. Map every pinned fork-answer rule to executable coverage or
  the contract's explicit not-observable exception.
- Keep the bounded-key fixture and scripted routing ERROR replies opt-in for
  existing adapters, reporting their unsupported cases as inapplicable.

## 0.17.19

- Add defaulted history-intake control hooks and cases for the running head
  marker, append-only reads with a steady version, and accepted sends whose
  runs wait for the imported history. Faulty fakes cover early completion,
  version changes, refused sends and early runs.

- Pin `cortexkit-role-llm-runner =0.8.15`.
- Add opt-in media-reference and plan-note cases with defaulted adapter
  fixtures, named inapplicability when absent, and deliberately faulty
  implementations exercising every new case.

## 0.17.16

- Pin `cortexkit-role-llm-runner =0.8.14`.
- Add `account_route_names_the_route` and `account_route_fixed_per_run`, gated
  on `account_route`, with faulty runners that use vendor names for account
  admission or accept a joining send that changes the active run's route.

## 0.17.15

- Pin `cortexkit-role-llm-runner =0.8.13`.
- Check `adopt_open`: opening an unknown session, immediate empty reads,
  named senders' grants where the runner declares `head_sender` and
  `delivery_order`, repeat replies and plan reuse, unchanged existing-session
  drift, and refusal, before any write, of a caller that isn't the scope's
  owner. Only that last case needs the adapter's optional hook for opening a
  route under a plain scope as a caller other than its owner; without the
  hook it is not applicable.
- Exercise the new checks against faulty runners and report their failing
  cases by name, including a runner that hides an opened session until a message.

## 0.17.14

- Pin `cortexkit-role-llm-runner =0.8.12`.
- Check that message marks survive reads and both history views, that conflicting
  marks refuse before writing, and that only the scope owner can speak for an
  agent. The non-owner check uses the existing optional carrier route hook.
- Exercise each new check against a deliberately faulty runner and report the
  failing case by name.

## 0.17.13

- Pin `cortexkit-role-llm-runner =0.8.11`. Rename `result_rides_*` cases to
  `notices_*`, checking fact `call_key` and `delivered_as: "tool_result"`
  and delivery-event `text`. Add a faulty runner that omits the fact's
  `call_key` and is rejected by `notices_display_fact`.
- Rename `Capability::ResultRides` to `Notices` and the native inspection
  hook to `notices_stored_output`, retaining deprecated Rust aliases for
  this release. Adapters implement the new hook.
- Extend `message_senders_rows_name_their_sender` to check the opted-in raw
  read entry's `sender` when `read_display_facts` is declared, with an
  omission control and coverage without that declaration.
- Add `subscribe_resume_refusals` alongside the existing resume check:
  malformed `from` values must name `from` in `invalid_params`, and a
  cursor issued for another session must be refused `cursor_expired`.
  Faulty-runner controls cover both refusal rules. Store-restore and
  retention-expiry simulation are not provided by the existing harness.

## 0.17.12

- Pin `cortexkit-role-llm-runner =0.8.10` for provider-executed display tool flags.

## 0.17.11

- Pin `cortexkit-role-llm-runner =0.8.9`. Add six `result_rides`-gated cases
  for entry-free delivery, unchanged native stored output, reader/display
  facts, live delivery events, exact provider append bytes and no system
  authority for tool-result-only sends. Each has a faulty-runner control.
- Add native-store ride inspection and provider system-text observations.
  Existing delivery-order checks use ride receipts on runners declaring the
  new group, while retaining legacy entry checks for older declarations.

## 0.17.10

- Pin `cortexkit-role-llm-runner =0.8.8` and cover opt-in sender attribution:
  exact stored headers, verified facts, photo-only and template sends,
  unaffected senders, capability refusals, tampered prefixes and display rows.
  Each deliberately faulty runner is rejected with the violated case reported
  as failed under its stable case name.

## 0.17.9

- Pin `cortexkit-role-llm-runner =0.8.7`. Add `routed_accounts` capability
  coverage for `follows_account_choice: true` and an `account_for` admission
  for every listed sign-in method, with deliberately faulty runners rejected
  by each check's name.
- Add required-with-group hooks `routing_runner_declaration`,
  `routed_account_fixture` and `install_routing_peer`, defaulting to named
  errors. `RecordingRoutingPeer` records raw JSON bodies without a routing
  dependency, answers fixture-account admit/renew calls, and replays observation
  ids per calling runner with `duplicate: true` and empty effects.
- Document live-coverage limits for renewal, release, forwarding and secret use.

## 0.17.8

- Pin `cortexkit-role-llm-runner =0.8.6` and check opt-in head selection:
  inherited plan tiers, explicit send-tier overrides, immediate head visibility,
  unsupported model and tier refusal without changing the head, and unchanged
  refreshes after a host-side switch when the adapter supports that hook.
- Add deliberately faulty runners rejected by each new check's name.
- Require `plans` and `session_change` for the refresh-based cases; check
  that a present model with its tier omitted resets fast to standard.
- Head-selection coverage does not yet check which model a run used, that a
  creating send's `plan.model` drives its first run, or send-model precedence.

## 0.17.7

- Every scripted call to `SCRIPTED_TOOL` now carries a string `marker`, which
  its input schema (`call_identity::DISPATCHED_DEFINITION`) requires. The
  `tool_result_too_large_is_error`, `tool_result_too_large_not_repeated`,
  `output_refs_slots` and `output_refs_shared_samples` cases sent `nonce`
  without `marker`, so a host that validates tool input refused the call
  before running it, and those cases failed for a reason they don't test.
  `SCRIPTED_TOOL`'s docs now state the schema. The kit's own fake-runner
  tests reject an installed `SCRIPTED_TOOL` call without a top-level string
  `marker`, so a new case that does this fails those tests.

## 0.17.6

- Pin `cortexkit-role-llm-runner =0.8.5`. Add
  `read_display_facts_created_at`, checking exact seeded host creation
  timestamps on user and assistant entries, chosen at least 60 seconds from
  any `written_at` or `completed_at` the initial read returns, and absent
  without `include_display_facts`. The optional `seed_host_created_at` adapter
  hook defaults to false; when it returns false the case is inapplicable with
  a reason naming the hook, never a pass. Faulty fakes cover omission and
  substitution of write or completion time.
- Include creation time in default-absence checks and in comparisons
  requiring decoded entries to stay unchanged once display-fact fields are
  removed.

## 0.17.5

- Pin `cortexkit-role-llm-runner =0.8.4`. Add `media_only_send` conformance:
  a declaring runner accepts a committed upload without prompt text and
  stores a user message containing media alone, with no empty text part.
  Report the case inapplicable by name without the group or upload fixture.
  Faulty fakes cover refusal and an extra empty text part.
- The contract adds head image-size limits and `media_too_large`; no new kit
  case checks those limits because the faithful fake does not decode images
  and cannot enforce pixel dimensions.

## 0.17.4

- Pin `cortexkit-role-llm-runner =0.8.3`. Add flow-scope frozen-plan sender
  refusal cases for refresh and send, checking `plan.senders` and unchanged
  head/history, with optional real-flow adapter fixtures and faulty fakes.
- Extend display-fact checks with assistant completion stamps and a named
  inapplicable outcome when the adapter cannot report host finishes. Default
  reads omit all display facts. Faulty fakes exercise completion violations.
  The completion case does not check the time's value against any other
  timestamp: `written_at` can be first-ingest time, not creation time.
- Fix `retention_honoured` to compare a never-written read against its head's
  `history_version` and `incarnation`, not fixed values `1` and `log-1`.
  Cover matching arbitrary placeholders and disagreeing read/head identities.

## 0.17.3

- Pin `cortexkit-role-llm-runner =0.8.2` and add
  `scoped_session_start_owner_only`: an optional adapter hook opens a plain
  scope as a non-owner carrier. The case requires `scope_owner_mismatch` and
  no visible session (empty owner read and head, no ready baseline), or reports
  inapplicable when the hook is absent. It observes wire replies, not storage.
- Repair `display_without_limits_is_whole` to use a kept non-error result
  above the app's result bound and below the provider acceptance limit.
  Error-result truncation is not exercised. The provider limit measures
  pre-render output and does not bound rendered display text.
- The pinned runner vectors replace illustrative module ids in samples with
  role names: `owner-module`, `compaction-provider`, `app-server`,
  `runner-a-head` and `tool-provider`. The published serializer-profile
  selector stays unchanged.

## 0.17.2

- Grade numbered `output_refs_*` fixtures and
  `delivery_order_template_part_gone_refused` inapplicable when the kit's
  opening plan has no `compaction_item`. Applicability follows the kit and
  adapter's admission fields, not missing numbers in a runner reply. Plans
  with a compaction item retain the existing assertions and still fail when
  required shown numbers are missing. The unsupported-plan case is unchanged.
- Add `output_refs_without_compaction_unknown_tag`: remove `compaction_item`
  from the adapter's reference plan and check a reply reference records
  `unresolved` / `unknown_tag` with `tag: "1"`, remains unchanged after another
  tool output, and refuses content reads as `unknown_ref` before and after.
  Faulty fakes resolve the reference immediately or after the later output;
  both are rejected by the new case's name. Keep the contract pin at `=0.8.1`.
- Move the aggregate text in `media_descriptors_frame_budget` and
  `media_descriptors_stable_across_pages` from oversized multipart tool
  results to separate text parts in one assistant message. Each fixture
  still exceeds `DESCRIPTOR_MESSAGE_MAX` with individually inline-sized
  text, preserving largest-first/tie selection and per-message page
  stability checks, so the fallback text no longer comes from a tool result
  that §11.3.1 would replace with `tool_result_too_large`. The image still
  relies on a negotiated format that excludes image bytes from that measure.
  The frame case keeps its two oversize sibling call arguments and inline
  `reply_too_large` check. Neither case needs multipart tool-result storage.

## 0.17.1

- Pin `cortexkit-role-llm-runner =0.8.1`. Its `ReadRequest` and
  `UserCommandRequest` decoders accept the two legacy 0.7 input members while
  still refusing every other unknown member of those requests. The kit adds
  no route-level case for them.
- Fix `head_sender_is_callers_effective_grant` for runners whose default
  read admission refuses non-owners. §4.3 reports `sender` only on reads the
  runner admits, and §9.19 keeps a runner's own read admission when the plan
  carries no `readers`; the case had required every caller's `session.head`
  to be admitted. With `app_readers`, the plan lists A and B as readers, and a refusal of either fails the case. Without it, a `scope_owner_mismatch` refusal of A's or B's `session.head` ends the whole case as inapplicable, without checking the later callers. Any other refusal fails the case. The unnamed caller may be refused
  `scope_owner_mismatch` instead of getting no grant; this case checks grant
  disclosure only, and the `app_readers` cases check §9.19's refusal of an
  unlisted reader. Each `session.head` probe is now one request.

## 0.17.0

- Require ordinal on every history and display entry. Pre-adoption host
  messages share the one-based count; marker/control rows are not entries.

Breaking release; pin llm-runner =0.8.0.

- Remove the compaction-provider/v1 dependency, scripted provider API and cases,
  checkpoint/lineage-bound read cases, tags/resolve cases and model-view cases.
  Compaction-runner conformance is outside this kit.
- Check one-based entry ordinals and whole-active-path reads, including below
  native markers, with required history_version/incarnation cache identity.
- Add named faulty-fake checks for read-page history_version, history_changed
  on stale cursors, from_ordinal 0, incarnation, run-fixed route_decision_id,
  route-bound user commands, both send-status names and served pruning keys.
- Let routing-backed adapters seed a valid decision for the run-fixed case
  through `route_decision_id_fixture`; other runners use its default literal.
- Check whole-result tool_result_too_large errors and at-most-once provider
  dispatch; faulty implementations truncate output or dispatch again.
- Preserve native no-provider archive pruning. Provider plans must not prune
  until the compaction contract defines acceptance; remove old Setup/view gates.
- Keep snapshot budget thresholds by seeding historical/imported stored outputs
  through a fixture-only hook, never by bypassing the new provider-result limit.
- Retained template and output-reference fixtures obtain compaction A3 numbers
  from the adapter's captured request mapping instead of removed tag rows.

## 0.16.11

- Pin `cortexkit-role-llm-runner` to `=0.7.48` for its served-bytes
  `run.tool_calls` rebuild rule and helper corrections. The page refusal is
  documented with an example in the contract; no new kit case is added.
- Pin `cortexkit-role-compaction-provider` to `=0.7.14`, which removes an
  obsolete claim of a `compaction_unavailable` difference between contracts.

## 0.16.10

- Pin `cortexkit-role-llm-runner` to `=0.7.47` and add
  `message_senders_rows_name_their_sender`: a plan-named sender's start and
  join messages expose its module id and client message id as soon as
  `message_started` arrives, while model replies carry no sender.
- Reuse the named-sender adapter fixtures. Missing adapter prerequisites,
  an undeclared group, or unavailable named-principal fixtures make the
  case inapplicable; invalid discovery, such as `message_senders` declared
  without its dependencies, still fails.
  Faulty fake runners cover omitted, wrong and late sender bindings.
- Add defaulted `RunnerRoute::observe_message_started` for immediate row
  reads at live event receipt. Without the hook only the timing rule is
  inapplicable; buffered checks still run. Without a steerable held run,
  the case covers start messages and marks the join rule inapplicable.

## 0.16.9

- Pin `cortexkit-role-llm-runner` to `=0.7.46` and add three `live_ids` cases:
  rekey on storage, no provisional ids without subscriber opt-in, and reset
  when an unstored message aborts. Undeclared groups are inapplicable by name.
- Add the defaulted `live_ids_observation` native delayed-storage fixture
  hook. Cases are inapplicable by name when the adapter cannot provide it.
  Faulty fake runners defend opt-in, storage timing, rekey identity,
  post-storage delta ids, and abort reset checks.

## 0.16.8

- Requires `cortexkit-role-llm-runner` 0.7.45, and accepts `pi-ai` from `model_finish_reason_source`.
- `finish_reason_last_model_step` scripts the source's own tool-call reason on its tool-call turn (`tool_use`, `tool_calls`, `tool-calls`, `toolUse`), not a plain stop. A real host whose loop only continues after a tool-call reason can now reach the second step. Responses and Gemini have no distinct reason for a successful tool-call step, so their turn keeps the ordinary completion. The expected `finish` of the last step is still `stop`.
- New case `finish_reason_provider_limit`: a scripted response carrying the source's length reason, with no `generation.max_output_tokens` in the send, must report `length`. It requires `finish_reason`, `queue`, `run_ops` and the raw-reason hook, but not `max_output_tokens` or `run_usage`, so a runner that cannot promise a caller cap is still graded on truncation. `finish_reason_output_cap` is unchanged.

## 0.16.7

- Pin llm-runner `=0.7.44` and add three `finish_reason` cases: the last
  model step's normalized finish, output-cap truncation as `length`, and no
  finish when no model step ran. Ended status is checked where served.
- Add optional raw-reason scripting with an unsupported-default adapter
  hook. Reuse the output-cap fixture, reporting a real truncation reason.
  Undeclared runners grade not applicable, and faulty runners are rejected
  by each case's name.

## 0.16.6

- Pin llm-runner `=0.7.43`. No new cases. The contract's sample and
  round-trip test pin only the JSON shape of `call_key` and `schema_pin` on
  read display facts; the kit doesn't yet check that a runner gates them
  on `call_identity` or copies them from stored dispatch facts.

## 0.16.5

- Add four opt-in pending-first-view cases: notice before publication and first
  model rendering, hold expiry without a model call, conflicting announcements,
  and an undeclared pending member failing Setup by name.
- Observe Setup's actual `view_features`; report every case not applicable
  without `view_pending`. Add default-unsupported fixture and publication hooks,
  with deliberately faulty runners for each case.
- Pin compaction-provider `=0.7.13` and llm-runner `=0.7.42`.
- Document pending-view fixture limits: readable writes rather than crash
  durability, publication without `view_ready`, observed fetches and terminal
  errors rather than provider holds or refusal codes, and lower-bound timing
  only. Served-view disposal and later-send retries remain contract obligations.

## 0.16.4

- Pin llm-runner 0.7.42 and add sender-succession cases for matching and
  conflicting retries, third-sender isolation, predecessor status and
  undeclared-plan refusals. Existing identity and plan hooks drive them.
- Add a same-sender changed-deadline retry case checking both the original
  receipt and original expiry. The optional delivery clock observation pairs
  with the controlled retention-clock hook; unavailable clocks are inapplicable.
- Add display-fact cases for opt-in envelope facts with unchanged bodies,
  default absence and undeclared refusals. An optional stored-time fixture
  hook defaults to unsupported. Each new case has a faulty-runner control
  that is rejected under the case's stable name.

## 0.16.3

- Pin llm-runner 0.7.41. Check that adding or removing a sender takes effect
  as soon as the runner acknowledges the refresh, before another run starts, including an
  addition during a held run when the adapter supports holding one.
- Report both cases by name when their required groups are absent. Add a
  faulty runner that waits until the next run to change senders, and tests
  showing each case rejects that behavior by name.
- Check listed and unlisted `app_readers` admission to head and transcript,
  and that a read grant cannot send. Each case has its own faulty runner.
- Allow stored tool times and unanswered outcomes beside the raw-body
  projection. No existing adapter hook independently observes those stored
  values, so their semantics are pinned by contract vectors, not a kit case.

## 0.16.2

- Pins llm-runner `=0.7.40`. The `tool_upfront` and `output_refs` cases put
  their hint on the plan's `tool_items`, and the output-refs refusal field is
  `plan.tool_items[].output_refs`.

## 0.16.1

- Pin llm-runner 0.7.39 and check declared `tool_upfront` plan sessions:
  upfront hints leave the tools offered to the model unchanged, and a stale
  upfront name is refused with no observable durable-state change and no model
  dispatch. Report both cases by name when not applicable.
- Add faulty runners that demonstrate each case rejects its violation.

## 0.16.0

- Requires subc-protocol 0.34; breaking because `Reply::Error` and
  `SubscribeOutcome::Refused` contain its `ErrorBody`.
- Pins llm-runner 0.7.38 and updates the development-only tool-provider
  dependency to 0.14. No conformance-check changes.

## 0.15.42

- Pin `cortexkit-role-llm-runner` 0.7.37. No conformance-case changes. The
  runner crate's unit tests check its `served-bytes.json` vector.

## 0.15.41

- Check unsupported-tier refusal before model dispatch, fixed run tiers,
  tier-change rides before changed requests, and omission of served on wires
  whose served policy is unknown. Mapping provider-reported served values back
  to abstract tiers on wires with `served: "reported"` is not checked.
- Add defaulted `LlmRunnerSubject::configure_service_tier` and named
  not-applicable outcomes when a
  runner does not declare tiers or an adapter cannot script them. Faulty
  runners prove every new check rejects its named contract violation.
- Capture `ModelRequestObservation::service_tier_change_rides` at arrival at the
  scripted model provider, counted with arrivals at the scripted compaction provider.
  The held response uses `LlmRunnerSubject::hold_next_model_step`,
  `LlmRunnerSubject::await_model_step` and `LlmRunnerSubject::release_model_step`.
- Pin runner 0.7.36 and compaction-provider 0.7.12.


## 0.15.40

- Pins llm-runner `=0.7.35` (`role.describe.step_hooks`). No case changes.
## 0.15.39

- `display_row_is_the_profile_projection` reports not applicable, naming the
  profile, when llm-runner has no display projection for the session's
  serializer profile (`cortexkit_role_llm_runner::display::projects`).
  Previously, the check tried to project rows for such a profile and failed
  with an unsupported-profile error, regardless of the rows' contents.
  Runners on a projected profile are graded exactly as before. Pins
  llm-runner `=0.7.34`.

## 0.15.38

- Pin llm-runner `=0.7.33`, a wording-only release.

## 0.15.37

- Pin compaction-provider `=0.7.11` and llm-runner `=0.7.32`.
- Add named opt-in truncate and tagged-bytes cases with an additive,
  default-unsupported part-edit publisher hook. Missing tokens or hooks are
  reported as not applicable on each affected named case, never as passes.
- Check deliberately faulty runners that ignore truncation or send provider
  bytes without the part's own tag fail the corresponding case by name.

## 0.15.36

- Pin compaction-provider `=0.7.10`. Add three opt-in Setup due-view cases:
  render the due view rather than `initial`, stop before the model after a
  refused fetch with `view_due_unapplied`, and retry and apply at a later step.
- Add a scripted due-view fixture and fetch control. Runners without the
  request opt-in, adapters missing the required hooks, and fixture plans with
  model-only decorations such as rendered tags are not applicable by name;
  missing coverage does not change their verdict.
- Check that deliberately faulty runners rendering `initial`, calling the
  model after a refused fetch, or dropping a due view fail the named cases.

## 0.15.35

- Pin runner `=0.7.31`. No case changes.

## 0.15.34

- Pin runner `=0.7.30`. No case changes.

## 0.15.33

- Pin runner `=0.7.29` and check the opt-in `retract_own_send` rule: the
  recorded sender can withdraw its pending submission before it runs, while
  another non-owner cannot, even with a forged identity in the request.
  All three checks report not applicable by name when the group is undeclared.
- Exercise faulty runners that refuse the sender, admit every non-owner,
  or trust a request field for caller identity, and require rejection by name.
- Add `retract_own_send_shared_send_id_isolated`: two non-owner senders
  reuse a `send_id`, and retracting one submission twice leaves the other's
  submission pending for the owner's next turn. Correct the fake's lookup
  to authorize and withdraw the same sender's entry, and reject a faulty
  fake retaining the sender-unqualified withdrawal lookup by name.

## 0.15.32

- Pin runner `=0.7.28`. The display projection case supports `opencode-v2`
  through the shared projector, including native notice roles and tool outcomes.
- Check that the case rejects a runner that relabels a system notice as user text.

## 0.15.31

- Pin runner `=0.7.27`, which adds `pi_served_body`. No kit case changes.

## 0.15.30

- Pin runner `=0.7.26`, which adds the Pi served-body projection
  (`served_body`) a runner must serve. No kit case changes.

## 0.15.29

- Pin runner `=0.7.25`. The display profile-projection case now supports Pi
  through the shared projector, including its distinct runtime roles.
- Exercise the case with Pi-shaped bodies and verify that it rejects a
  runner that incorrectly labels a tool-result row as user text.

## 0.15.28

- Pin runner 0.7.24 and require compaction-provider 0.7.9 for the optional
  edit-pricing notice and chain-reset ride documentation. Providers must
  upgrade their readers before runners send `history_edit_cost`, because
  older readers leave the header out of notice digests. No checks change.

## 0.15.27

- Pins `cortexkit-role-llm-runner` `=0.7.23`, a text-only release. No case,
  report or adapter hook changes.

## 0.15.26

- Check `call_identity` on runners declaring `run_tool_calls`
  without `dispatch_attribution`, comparing display and live call keys with
  `run.tool_calls`. Runners declaring `dispatch_attribution` are graded exactly
  as before against `session.read` attribution; runners declaring neither
  reference group are reported not applicable by name.
- Add correct and faulty fallback runners to exercise the three cases and
  reject a display key that differs from `run.tool_calls`.
- Pin `cortexkit-role-llm-runner` to `=0.7.22`, a text-only clarification of
  the existing call-key lifetime.

## 0.15.25

- Add `held_calls_status_event_matches_run_status`, which requires the runner
  to declare `held_calls`, `run_status`, `queue`, `transcript_reads`,
  `run_ops` and `streaming` on `role.describe`. It attaches before the send,
  compares control-event held calls with typed `run.status` values (reading
  the status again before failing, since the contract doesn't order the two),
  rejects `held_calls: []` and checks that applying a late result never
  relists the call. The runner-defined event schema does not let the kit
  require a specific clearing status event.
- Add faulty fakes for missing event lists, mismatched call keys, stale lists
  after a late result and empty lists instead of omission. The contract crate
  pin remains `=0.7.21`: the case checks an existing streaming obligation.

## 0.15.24

- Pin `cortexkit-role-llm-runner` to `=0.7.21`, which spells out provider
  rendering and legal placement for `turn_system` and ships rendering vectors
  for runners' own tests. The kit still observes `delivered_as`, not provider
  requests; its rendering coverage has not changed.

## 0.15.23

- Pin `cortexkit-role-llm-runner` to `=0.7.20`. Read the live declaration for
  opt-in `run_cancel_by_interrupt` and `refresh_relaunch` cases; report each
  by name as not applicable when absent, preserving existing verdicts.
- Check interrupt-granted `now` cancellation and named refusals for missing
  interrupt authority, non-`now` cancellation and a different session. Keep
  faulty runners demonstrating each refusal check.
- Check out-of-host-set additions at `immediately` and `on_prefix_rebuild`,
  no relaunch during a run, and byte/ordinal/lineage-stable reads. The old
  `session_refresh_tool_outside_host_set_refused` case is not applicable to
  declaring runners. Faulty runners cover mid-run apply, history rewriting
  and old tools reported as a new generation.
- Add optional model-request-arrival `surface_change_rides` observations to check
  successful relaunch ride ordering using a shared provider-arrival counter,
  with a faulty runner caught by name. Missing observation support is reported
  as an unchecked rule. The adapter has no host-relaunch failure hook; fallback,
  required-failure, failure notices and no-fallback-ride rules remain contract
  coverage only, as documented in the README.

## 0.15.22

- Pin `cortexkit-role-llm-runner` to `=0.7.19`, whose OpenCode 1
  finished-summary test also excludes a `summary` stored as `1.0`.

## 0.15.21

- Pin `cortexkit-role-llm-runner` to `=0.7.18`, whose OpenCode 1 counting
  rule excludes nonempty compaction-only messages from counts and reads
  without renumbering retained messages. Mixed compaction/text messages
  still count.
- Extend `message_checkpoint_uncounted_rows_never_served` with a defaulted
  OpenCode 1 compaction-only fixture hook on adoption and native lineages.
  Add a faulty fake that counts and serves that row and fails the case by
  name; other hosts retain their existing finished-summary checks.

## 0.15.20

- Add the defaulted `LlmRunnerSubject::first_setup_names_newest` hook: true
  when the runner writes a fresh session's first message before its first
  compaction Setup, so that Setup names `newest`. An adapter whose runner
  runs the first Setup at send admission, before writing, answers false.
- `tags_setup_checkpoint_anchors_at_its_cut` is inapplicable, by name, when
  the hook answers false: that Setup names no `newest` and has no cut to
  anchor, while a Setup checkpoint only reaches moved sessions, which the
  `message_checkpoint` cases cover. The contract crate stays at 0.7.17.

## 0.15.19

- Add eight named `message_checkpoint` cases for nonzero adoption numbering,
  certified head-cut equality, the anchor after a surviving folded message,
  native marker message and tag counters, missing-counter refusal, and
  exclusion of uncounted host summaries with contiguous counted ordinals
  on adoption and native lineages, and marker-derived head metadata while
  the first Setup is outstanding. Bound the head call so a deadlock fails
  by name; faulty fakes cover both session-start metadata and waiting on Setup.
  During adoption-cut Setup, a bounded transcript read starts at
  `first_ordinal` and refuses earlier rows. Faulty fakes cover both a read
  waiting on Setup and a read that serves rows below the lineage boundary.
  Without the describe group each is Inapplicable, preserving old verdicts.
- Add defaulted moved-session fixture and actual native-marker observation
  hooks; unavailable hooks make their cases Inapplicable. Add a deliberately
  faulty fake runner for each named check. Pin runner 0.7.17 and require
  compaction-provider 0.7.8.

## 0.15.18

- A subject that cannot configure enforced structured output may refuse
  `set_structured_output_support(session, Some(true))`. The enforced cases
  then report inapplicable with the hook's error, without reducing the
  verdict. Best-effort cases remain required with `Some(false)` and `None`.
  Non-enforcement checks probe `Some(true)` on their own session before
  setting the real support value. Only a refused probe permits, and then
  requires, `host_unsupported`; a successful probe requires the exact catalog
  or route reason. Faulty runners claiming or hiding host incapability fail
  the reason checks by name.
- Read `structured_output_per_session` from the describe answer as another
  optional rule variant. `structured_output_not_inherited` checks that such
  a runner refuses changes to the first admitted send's presence or value
  without writes; an omitted first member must stay omitted, and a present
  first member must keep the same whole-member JCS value.
  The default per-run check remains unchanged. A faulty fake accepting an
  omitted member fails the case by name.
- Require `cortexkit-role-llm-runner` 0.7.16. Read its optional
  `submit_streamed_siblings` describe group as a rule variant, not a capability
  whose absence reduces a runner's conformance verdict.
- Keep `structured_output_sibling_never_dispatched` strict without the group;
  with it, check `structured_output_streamed_siblings` instead. Earlier calls
  may keep dispatched results without being skipped; later calls and later
  `submit_result` calls must never dispatch and must have never-executed
  `skipped_after_submit` results listed in emission order. The run must end
  `completed` with the first submitted value validated.
- The case for the other dispatch variant is inapplicable by name, without
  reducing the verdict of a fully conforming runner.
- A faulty fake runner declaring the group but dispatching a later sibling
  fails `structured_output_streamed_siblings` by name.

## 0.15.17

- Requires `cortexkit-role-llm-runner` 0.7.15, a text-only release that says
  which message a host's pre-adoption cut names. No check changes.

## 0.15.16

- Track llm-runner 0.7.14, which this crate pins exactly.
- Add the `rich_ui_stamp` cases: `rich_ui_stamp_served_on_stamped_run`,
  `rich_ui_stamp_absent_on_unstamped_run`, `rich_ui_stamp_read_needs_opt_in`,
  `rich_ui_stamp_conflict_refused` and `rich_ui_stamp_head_follows_last_run`.
  They check that a run serves its frozen plan's rich UI stamp verbatim on every
  assistant display row, `run_started`, `head.last_run_rich_ui` and opt-in
  `session.read` entry, and nowhere else. They also check that a read without
  `include_rich_ui` is unchanged, that two different stamps are refused
  `plan_stale` before anything is written, and that the head follows the last
  run after a replan. They are not applicable without the describe group.
- Add the optional subject hooks `rich_ui_plan_fields` (a plan whose fetched
  system-text answers declare chosen stamps) and `replan_rich_ui`. Without
  them the cases are not applicable, by name. Add `rich_ui_stamp::fixture_stamp`
  and `conflicting_stamp`.
- Faulty runners prove the cases fail when a runner stamps an unstamped run,
  serves another value, leaves an assistant row unstamped, stamps a user row
  or user read entry, serves the stamp without the read opt-in, omits or
  leaves over the head's stamp, or accepts two conflicting stamps.

## 0.15.15

- Track llm-runner 0.7.13, which this crate pins exactly. Its
  `owned-broca` `applied_edits` vector now uses a shape a served page
  carries (an assistant `tool_call` with its `thought_signature`) instead
  of `text_with_state`. No rule or check changed.

## 0.15.14

- Track llm-runner 0.7.12, which this crate pins exactly.
- Add `archive_pruning_applied_edit_pruned` (`compaction`). Before a prune,
  the kit applies a view whose keep-range part edit re-serializes a
  `signature`-kind part of the subject's native fixture, signature
  included, with its text or tool output replaced. Right after the prune,
  before any run, the model view must serve that edit with the prune rule
  applied: by `prune_part_in` for the subject's declared profile, or from
  the fixture's served form when no profile is declared. Not applicable by
  name without `archive_prunes_reasoning`, without `model_view`, or when
  the fixture has no `signature`-kind part. Named faulty runners prove it
  fails a runner that leaves the edit's pre-prune signature and one that
  drops the edit.

## 0.15.13

- Track llm-runner 0.7.11, which this crate pins exactly. The
  `archive_pruning_*` cases are unchanged: they already build their
  restoring views and part edits from the subject's own native fixture and
  served pages, whatever its serializer profile.

## 0.15.12

- Track llm-runner 0.7.10, which this crate pins exactly, and
  compaction-provider 0.7.3.
- Add `archive_pruning_generation`, `archive_pruning_restored_view_refused`,
  `archive_pruning_restored_edit_refused`, `archive_pruning_setup_after_prune` and
  `archive_pruning_setup_initial_replaced`. Like every `archive_pruning_*`
  case they are not applicable without `archive_prunes_reasoning`. Named
  faulty runners prove each fails a runner that prunes without raising the
  generation, applies a view that restores a pruned part through a replace
  range or through a keep-range part edit, skips the Setup
  after a prune, or keeps rendering the pre-prune `initial`.
- Add the `compaction_provider_calls` adapter hook (`ProviderCall`), and
  the `ScriptedRange::ReplaceVerbatim` and
  `ScriptedRange::KeepWithVerbatimEdits` (`ScriptedPartEdit`) ranges, which
  carry messages or parts already in the runner's schema unchanged. Document that the provider
  `archive_pruning_fixture` enrolls is the adapter's scripted compaction
  provider and does not hold model answers.

## 0.15.11

- Track llm-runner 0.7.9, which this crate pins exactly. No suite change.

## 0.15.10

- Track llm-runner 0.7.8, which this crate pins exactly. No suite change.

## 0.15.9

- `compaction_setup_durable_once` no longer decides from one `session.head`
  read right after the restart. It polls, with the subject's pause and at
  most `MAX_POLLS` times, until the recovered Setup run reads `interrupted`
  (then it sends the one continuation) or reads `active` with its first
  model request held (resumed on its own). A runner whose crash recovery
  seals its log in the background, and so still reads `active` for a moment,
  now passes. A run that settles neither way fails the case by name, naming
  the state it was last seen in. Every Setup-count and model-request check
  is unchanged. The fake runner gains a background-seal mode and a
  never-settling mode that prove both outcomes.

## 0.15.8

- Include `account_change` in scripted rewrite rides and the fake runner's
  rewrite-wait handling, matching the compaction-provider account-switch
  contract. Track llm-runner 0.7.7.

## 0.15.7

- Track llm-runner 0.7.6. Documentation-only patch: correct the
  `head_last_run_error_only_on_error` and
  `head_last_run_error_requires_last_run_state` README requirements to include
  `run_ops`, `run_status` and the model-failure fixture for failed/recovered
  head checks. No new conformance case or fixture hook.

## 0.15.6

- Track llm-runner 0.7.5. Add `head_last_run_error_only_on_error`,
  `head_last_run_error_matches_run_status` and
  `head_last_run_error_requires_last_run_state`, with named faulty runners
  proving each case independently rejects its violation. Compare fractional
  retry timing exactly, including a 1.5-second fixture, plus class and reset.
- Add optional `ScriptedTurn::model_failure` and its builder, gated by
  `LlmRunnerSubject::supports_model_failure_fixture`. Adapters drive a real
  terminal provider failure; the fake records it durably. Runners omitting
  the optional head field are inapplicable, never failed or silently credited
  with a pass. Without the fixture, fresh and completed heads are still
  checked for invalid error summaries; only the run.status comparison is
  inapplicable. Named faulty runners prove both fixture-free checks.

## 0.15.5

- `tags_tool_result_blocks_share_one_number` finds each tool result through its
  listed call, so a result made only of media, or of empty text plus media, no
  longer needs the fixture's marker text. The new subject hook
  `tool_result_part(call, result)` names the stored part that holds a call's
  result; its default matches common call-id fields. When the hook returns
  `None`, the case falls back to finding the marker in the result. Only a result
  the hook cannot name and that carries no marker is reported as an unchecked
  shape, never as a failure. Regression tests cover separate and embedded result
  layouts and the marker fallback.
- The llm-runner contract dependency remains 0.7.4.

## 0.15.4

- Track llm-runner 0.7.4 and compaction-provider 0.7.1. Add capability-gated
  archive-pruning cases and native fixture/maintenance/schema hooks, with independent
  original/served oracles and named faulty runners for payloads, identities,
  every-page attestations, write-time digests, unlisted changes, busy sessions
  and Setup acceptance. Undeclared pruning is not applicable.
- Accept unknown models on imported assistant display rows; independently
  require `model` on messages driven through the runner's own run, with a
  named faulty runner. The general projection case no longer assumes a
  recorded model for every imported message.
- Give each shared output-reference sample call a fresh tool-call id within
  its assistant message; exercise a runner refusing duplicate ids.

## 0.15.3

- Check event `attempt_id` against independently read durable step records
  using the existing observation hook; reject missing identities and test
  valid-but-wrong event identities with named faulty runners.

- Track llm-runner 0.7.3. Add `tags_tool_result_blocks_share_one_number`,
  checking whole-result counting, first non-empty text placement, image/media
  labels, and the next part through reads and observed model input.
- Add the defaulted `numbering_tool_result` fixture hook, naming unsupported
  shapes unchecked. Exercise both result layouts and named faulty runners
  that number every text block, number empty text, mislabel PDFs, or count
  structured values.

## 0.15.2

Carries the 0.12.7 fixes from the 0.12 line:

- `output_refs_snapshot_size` checks the size and SHA-256 of the tool-result part read back from storage, not the original result sent by the kit. Runners that cap stored results can pass; when the stored part does not exceed the snapshot cap, the case reports not applicable with its stored size.
- `output_refs_snapshot_budget` also uses stored output metadata and reports not applicable when the stored part cannot exercise the aggregate snapshot budget.
- `output_refs_writing_request` and `output_refs_held_placeholder` report not applicable, with the adapter's reason, when the runner has no renumbering or held-result event. Runners that support these events but change committed records or snapshots still fail by name.

## 0.15.1

- Depend on llm-runner 0.7.2. Add three cases gated on the `call_identity`
  describe group, not applicable (never skipped) without it:
  `call_identity_key_matches_attribution` (each display tool call and live
  tool event carries the `call_key` `session.read` attributes to that call),
  `call_identity_pin_frozen_at_dispatch` (each carries the `schema_pin` the
  tool provider received, still after the catalog changes the tool's
  definition) and `call_identity_denied_call_has_no_pin`. Faulty runners that
  omit the key, serve a sibling's key, recompute the pin from the current
  catalog, or pin a denied call each fail by name, including runners that
  get only their live tool events wrong and their history rows right.
- New adapter hooks, all defaulted so existing adapters compile unchanged:
  `dispatched_schema_pin` (required with the group; the default fails the
  pin case by name), `change_tool_definition` (without it the pin case
  reports the catalog rule unchecked) and `deny_tool_call` (without it the
  denied case is not applicable). The public `call_identity` module carries
  the fixture definitions and their pins, checked in a test against the
  tool-provider crate's pin function.
- `display_row_is_the_profile_projection` takes `call_key` and `schema_pin`
  from the served row, as it takes the other runner-filled members.
- `output_refs_rewind` and `output_refs_compaction` report not applicable
  when the adapter's `revert_session` or `compact_in_host` returns an error,
  as the subject trait documents for both hooks and as the host-history and
  lineage cases already do. They used to fail such a runner. A faulty runner
  that can revert or compact but gets the result wrong still fails both
  cases by name. Carried from 0.12.6.

## 0.15.0

- Add `session_retention_release_after_idle`: delay release after durable activity
  and require a keep-none session's full floor from that later release, both in
  head's deadline and actual deletion. A faulty activity-anchored deadline fails
  the named case; one-day and seven-day floors are covered.
- Depend on llm-runner 0.7.1. Add named `session_retention` cases for invalid
  floors, survival to the declared floor and deletion after it, readable pending
  release with head/refusal deadlines, and the floor-zero drain window.
- Defend the cases with subprocess faults for early deletion, inaccessible
  reads, missing release metadata, incorrect deadlines and non-pending repeats.
- Existing retention cases account for the declared floor instead of requiring
  immediate deletion. Time-dependent cases use the existing retention clock;
  that hook also lets declared readers finish reading due content. No new
  subject methods are required.

## 0.14.0

- Retain the retention and step-event cases from 0.13.x. Step-event fake
  records use the shared named session identity on the declared bare runner
  id; native-history observations still read those stored references.
- Use the shared helper's named `SessionIdentity` fields; `runner_id` supplies
  the attested runner principal's bare module id, never a display-form prefix.
- Breaking: adapters must declare `LlmRunnerSubject::runner_id`, the module id
  recorded in `session_binding.runner`.
- Add `session_ref_uses_runner_identity`: bus-captured advance-notice headers
  must match the shared `session_ref(&SessionIdentity { runner, harness, session })` function. A
  folder-derived fake fails only that named case.
- Update fake runners and user-command fixtures to the shared derivation and
  depend on llm-runner 0.7.0 and compaction-provider 0.7.0.

## 0.13.1

- Pin the wire dependency to llm-runner 0.6.11. All new adapter hooks have
  defaults, so existing `LlmRunnerSubject` implementations compile unchanged;
  the step-event hook is only required when its new group is declared.

- Capture the actual publication subject and check its publisher against the
  body before using an event. Add faulty-content checks for `prompt` and
  `project_root`, and a faulty publisher check. Native session references are
  read from durable records; consumers must not derive them.

- Gate three new cases on `step_events`: one publication per native recorded
  step at the declared granularity, exact native durable position, and
  publish-after-durability. Add an independent adapter observation hook;
  missing evidence fails by case name. Faulty runners duplicate events,
  publish an off-by-one position, or publish before syncing their records.

## 0.13.0

- Pair with llm-runner 0.6.10. Add named, capability-gated session policy cases
  for enforcement, required/optional first sends, conflicts, cross-principal
  release ownership, idempotency, observable deletion including media, active
  runs/queued sends, backup/none refusal, dumps and runner-policy replacement.
- Add a durable-pending release crash case requiring a real process kill and
  restart. A subprocess fake defends the cases, including an in-memory-only
  release defect. New fixture hooks have defaults for existing adapters.
- Ordinary suite sends/adopts select a policy only when discovery declares the
  new group, preserving the separate legacy retention cases.

## 0.12.5

- Use `cortexkit-rich-ui-syntax` 0.3.0 (exact `pulldown-cmark` 0.13.5). Cases and vectors are unchanged.

## 0.12.4

- Add `media_descriptors_provider_rows`, checking exact key equality with the entry's complete tags rows and the named serializer profile's part count, never deriving opaque keys from bytes. This case requires the serializer-profile hook; when the adapter names no profile, the case is reported as inapplicable.
- Keep `media_descriptors_part_keys` as the runner-side strict check and exercise multiple parts so reordered rows fail by name.
- Add a faithful arbitrary-token key runner, a reordered-row faulty runner, and a faulty runner omitting an unnumbered tool call's tag row as controls. Pair with llm-runner 0.6.6; existing vector bytes are unchanged.

## 0.12.3

- Fix attachment read checks to accept independently omitted inline `digest` and `part_digests`, while requiring descriptors identities and validating any inline identities that are present.
- Add `system_text_default_forwarded_unchanged` and `system_text_default_preset_required`, discovered from live describe; undeclared runners are not applicable.
- Add scripted provider/catalog setup and external item-byte/durable-state observations. Faulty runners inventing a preset or forwarding without the provider fact fail their matching case by name.
- Pair with llm-runner 0.6.5.

## 0.12.2

- Require cortexkit-rich-ui-syntax 0.2.0. The functions this crate calls
  behave as before, so nothing else changes.

## 0.12.1

- Require llm-runner 0.6.3.
- Fix `attachments_default_read` and `attachments_tool_result`: they found
  the stored media message by searching the blocks page for the media's hex
  sha256, which a correct runner need not write anywhere in its own message
  schema, so both failed runners that store media inline. They now find that
  message through the default read's marker and its `mid`, and still compare
  its digest and part digests against independently computed ones, and its
  served bytes against `session.media_read`. The descriptors `media_ref`
  check is unchanged, since that shape is contract-defined.
- Add a fake that stores a sent upload and a tool-result image as a `data:`
  URL part with no digest in it: both read cases pass against it. Add a
  fault for each rule a blocks page must meet (markers served instead of the
  stored message, a wrong `digest`, wrong `part_digests`), each failing both
  read cases by name with its own reason.
- Add a check to `run_usage_counts_reported_steps` covering reasoning:
  a runner that bills `reasoning_tokens` on top of `output_tokens` reports
  more output than the scripted model did and fails by name. The case now
  runs twice, once with no step reporting reasoning and once with a step
  that does, so both the absent-total rule and the reasoning total are
  observable.
- Fix the test harness for faulty runners: `only_these_fail` checked an
  expected reason only on a failed case, so an expectation against a case
  that passed or ran inapplicable was never compared.

## 0.12.0

- Moves to subc-protocol 0.33; take subc-protocol 0.33, subc-transport 0.14, subc-control 0.33 and subc-client-rs 0.31 together.
- Breaking dependency update: `Reply::Error` and `SubscribeOutcome::Refused`
  contain subc-protocol 0.33's `ErrorBody`.
- No behaviour changes; existing test-vector bytes are unchanged.

## 0.11.2

- Require llm-runner 0.6.2. Gate `head_sender_is_callers_effective_grant`
  on the live `head_sender` describe group as well as the adapter-provided
  fixture for two caller identities. An absent group is inapplicable with a
  named reason.
- Make the reference fake declare `head_sender`; add a faulty runner that
  declares it but omits every `sender`, failing the case by name. Existing
  caller, owner and mode-grant faulty runners remain covered by name.

## 0.11.1

- Add an incarnation lifetime case with defaulted cut/re-create hooks and
  faulty runners that change identity at a cut or reuse it on re-creation.

- Require llm-runner 0.6.1. Add `Capability::SessionUsage` and cases for
  two-run sums and the first counted run, monotonic spend after a rewind,
  and unknown token fields remaining absent. Add a recorded model-switch
  case when the runner reports `head.model`.
- Add defaulted subject hooks for session-usage fixtures, host rewinds and
  recorded model switches; existing adapters compile unchanged and report
  unsupported observations as inapplicable. Faulty reference runners prove
  each case fails by name, including a wrong `since` and plan-derived model.
- Add `head_sender_is_callers_effective_grant`, with defaulted two-identity
  fixtures. Compare named A/B, unnamed and owner routes against the modes
  admitted from checked describe and the plan's flags. Faulty runners cover
  caller-independent grants, omitted owners and an ungranted `idle_start`.

## 0.11.0

A breaking release for `cortexkit-role-llm-runner` 0.6.0 and
`cortexkit-role-compaction-provider` 0.6.0, whose types this kit's public
API uses. Existing test subjects compile unchanged: the new hook has a
default.

- New subject hook `seed_untimed_tool_call`: put a finished call into a
  session's stored history the way history stored before the runner kept
  tool-call times holds it. Its default makes the new case inapplicable by
  name.
- New case `live_untimed_history_call_leaves_times_out`: after one run, the
  seeded call is replayed from start, and its `tool_started` and
  `tool_finished` must carry no `started_at` and no `finished_at`. A runner
  that invents a time fails it by name.
- `live_message_and_tool_events_durable` requires both times on the call the
  runner ran itself, the finish no earlier than the start, as the
  required fields did before.

## 0.10.6

- Exercise immutable output references, scanner samples, slot opt-in, snapshot
  limits, canonical digests, compaction/rewind and session isolation through
  runner routes, with a reference fake and named faulty runners.
- Use the runner-owned reference-slot type without a direct tool-provider
  dependency.
- Positive output-reference cases are N/A when the runner does not declare the group,
  rather than reporting a missing-capability skip for an undeclared group.
- The unsupported-plan refusal case binds non-advertisers on send, refresh
  and adopt, and is N/A for advertisers.
- Add the default-at-once `send_staged` subject hook and wait for it before
  releasing the first-possible-mode case's tool result. Check that a result
  sent before host staging cannot carry a subsequently staged send.

## 0.10.5

An additive release: per-case progress for runs that can be cut off before
they report. `run_suite` keeps its signature and report.

- `run_suite_with(subject, work_dir, SuiteOptions)`, beside `run_suite`.
  `SuiteOptions::new().with_progress(callback)` takes an
  `FnMut(&SuiteProgress) + Send`, called synchronously before each case
  starts and right after its outcome is known. `SuiteOptions` and
  `SuiteProgress` are `#[non_exhaustive]`.
- `SuiteProgress::CaseStarted { case, index, total }` and
  `SuiteProgress::CaseFinished { case, outcome, elapsed_ms }`. Every case
  gets both, in suite order, including a case skipped for a missing
  capability or inapplicable before it runs.
- `progress::stderr_lines()` writes `START <case>` and
  `END <case> <PASS|FAIL|SKIP|N/A> <ms>` lines to stderr, flushing after
  each. The outcome word is the one `SuiteReport::render` uses for the case.
  Nothing turns it on but passing it.

## 0.10.4

- Require `cortexkit-role-llm-runner` 0.5.4 and exercise `held_calls` when
  declared on the real `role.describe` reply. Existing adapters need no new
  enum match arms; undeclared `held_calls` is inapplicable by name and does
  not lower the runner's verdict.
- Add default fixture hooks for a held first reply and separately published
  late entries, plus `ScriptedLateEntry`. Cases check the waiting list,
  removal and continuation with a late result, `not_started` conversion,
  and conversion of an `expired` answer to a tool result before continuing.
- Add never-listed and stale-entry faulty runners, each rejected by exactly
  one named case.

## 0.10.3

Requires `cortexkit-role-llm-runner` 0.5.3. Runners can now verify the optional
`attachments` group over their real session-bound routes; callers gain checked
upload resumption, integrity, model admission, legacy-safe reads and digest
isolation guarantees before relying on that group.

- Capability-gated attachment cases cover a multi-frame lifecycle, lost
  append replies, restart loss, the inclusive 2 MiB cap, size/hash refusals,
  new-run and frozen-model input, unknown/open/gone/other-session ids,
  default markers versus blocks/descriptors and stored digests, nested tool
  media, read/upload admission, and committed-media lifetime.
- Defaulted `attachment_model_fields`, `attachment_tool_result`, and
  `restart_attachments` fixture hooks keep existing adapters source-compatible.
  An adapter without the group reports every case inapplicable/skipped by
  name. Plan-sender and controlled-clock cases also require their existing
  capabilities. Declaring attachments requires implementing its fixtures.
- The reference fake implements uploads and a persistent byte store separate
  from history references. Each attachment case has a named faulty fake and
  an independent passing control, including cross-session status and commit.

## 0.10.2

Requires `cortexkit-role-llm-runner` 0.5.2 and
`cortexkit-role-compaction-provider` 0.5.1. An additive release: it adds no
subject hook, so an existing adapter builds and runs unchanged. The new
cases reuse the plan-sender and held-model-step hooks, and a case whose
hook or group is missing is skipped or reported inapplicable by name.

- New capability `Capability::RunCause` for the role's new `run_cause`
  group.
- `run_cause` cases: the run a `queue` send started names that send's own
  `send_id` on `run_started`, never an id the runner made for its queue;
  every run a send started carries a non-empty `send_id`; and an
  interrupt's run names the interrupting send, never the interrupted one
  (inapplicable without `interrupt` or the held-step hook).
- Interrupt-grant cases (`delivery_order`, inapplicable without
  `interrupt`): a plan sender whose entry lacks `interrupt: true` is refused
  `invalid_params` naming `delivery` for the `interrupt` string, writing
  nothing; one holding the grant interrupts a held run, which ends
  `cancelled`, and its message starts the next run.
- A plan sender granted `idle_start` that sends the string `queue` (not a
  list) is admitted and starts a run. A runner that takes only lists from
  named senders fails this case by name.
- The display `sender` case: a plan sender's message has a display row
  naming the sender's module, and the default `session.read` page carries
  no `sender` member. The projection case takes `sender` from the served
  row, as it takes `model` and `client_message_id`.
- Provider-set rewrite-ride waits (compaction-provider 0.5.1 §4 and §8,
  llm-runner §6 `compaction_wait`), each case inapplicable unless the
  runner sends `ride_wait_max_ms` in its Setup request:
  `ride_wait_provider_wait_honoured` (Setup answers `ride_wait_ms: 4000`
  and the provider's view for a `ttl_expired` ride, answered after 3 s,
  applies at the ride's step), `ride_wait_slow_answer_reported` (a ride
  answered after 2.5 s reads `compaction_wait` while it waits),
  `ride_wait_fast_answer_not_reported` (one answered within 200 ms never
  does) and `ride_wait_above_cap_fails_setup` (a `ride_wait_ms` above the
  runner's cap fails Setup, naming `ride_wait_ms`). New
  `CompactionSetup::ReadyWithRideWait` (the enum is non-exhaustive; an
  adapter's `_` arm answers it as a failed Setup until it handles it), new
  `ScriptedRide`, and two subject hooks with defaults that make the cases
  inapplicable: `observed_ride_wait_max_ms` and `script_next_ride`.
- Fix: `media_descriptors_pairing` no longer reports "descriptors lost
  non-payload pairing field id" when a runner correctly describes a call's
  arguments as an `args_ref` whose inline arguments hold an `id` or `name`
  member, as tool parameters often do. The pairing check stops at a value
  described as a `*_ref`, which stands for the whole payload; pairing
  fields beside the ref, on the part that holds it, are still checked. The
  case's call now carries such arguments, over the inline limit.
- New narrowings: the owner's authority over a run a named sender started
  is not observable through the route (CONTRACT §9), and `run_cause`'s
  omission for a host turn on an adopted session is checked against the
  vectors only.

## 0.10.1

Requires `cortexkit-role-llm-runner` 0.5.1. An additive release: every new
subject hook has a default, and a case whose hook is missing is reported
inapplicable by name, so an existing adapter builds and runs unchanged.

- New capabilities `Capability::HistoryDisplay`, `Capability::LiveEvents`
  and `Capability::ClientMessageId` for the role's new groups.
- Display cases (`history_display`): a served row equals the serializer
  profile's projection of the same entry's raw body, every assistant row
  names its model, `limits` cut on a UTF-8 boundary and set the flags,
  absent `limits` serve whole content, and display pages carry finished
  messages only.
- `client_message_id` cases: the id appears on the user message's display
  row and never in the default `session.read`; a re-send under the same
  `send_id` with another id, or without it, is refused `send_id_reuse`
  naming `client_message_id`.
- Live-event cases (`live_events`): message and tool lifecycle events that
  replay identically and resume from a cursor, run events, the attach
  replay of the step in flight, and `seq` rising across both lanes.
- Live `cache_ttl` cases (compaction-provider §8.1): an entry for another
  model is ignored, a configured `ms: null` never declares `ttl_expired`
  even with an observed lifetime, and after a missed `model_change` ride
  the runner falls through to the observed lifetime, never the old model's
  entry.
- New subject hooks, each with a default: `serializer_profile`,
  `hold_next_model_stream`, `advance_prompt_cache_clock`, `switch_model` and
  `set_observed_cache_lifetime`. New `CompactionSetup::ReadyWithCacheTtl`
  answers Setup with a `cache_ttl` entry; the enum is non-exhaustive, and an
  adapter's `_` arm answers it as a failed Setup until it handles it.

## 0.10.0

Requires `cortexkit-role-llm-runner` 0.5.0 and
`cortexkit-role-compaction-provider` 0.5.0. A breaking release.

- Breaking: `session.head` answers must carry `first_ordinal` (the role's
  `HeadMeta::lineage_consistent` now requires it), and the shared full read
  starts at the lineage's `first_ordinal` instead of ordinal 0.
- Breaking: `CompactionCall` gains `lineage_id`, the lineage the Setup body
  named, and `CompactionObservation` gains `notices`, every advance notice
  the runner published as its bus headers. Adapters that build them with
  struct literals add the fields.
- New `Capability::HostHistory` for the role's `host_history` group.
- New subject hooks, each defaulting to an error that makes the cases
  needing it inapplicable by name: `publish_compaction_view` (with the new
  `ScriptedView` and `ScriptedRange`), `revert_session`,
  `seed_host_history` and `seed_session_history`.
- Cut cases, gated on `compaction`: `cut_starts_a_new_lineage`,
  `old_lineage_reads_refused_after_a_cut` (`after_mid` included),
  `read_below_first_ordinal_refused` (with `detail.first_ordinal`),
  `ordinals_never_renumber_across_a_cut`, `setup_once_per_lineage`,
  `view_ranges_start_at_first_ordinal` (a view tiling from 0 refused
  `ranges_start`) and `cut_lineage_notice_descends_from`.
- Cut cases on the head, the notices and reverts: `head_names_the_cut`,
  `notice_carries_first_ordinal` (`AdvanceNotice::first_ordinal`),
  `revert_below_the_cut_restarts_at_the_previous_cut` (with two cuts, and
  below every cut) and `revert_below_a_cut_before_adoption_has_no_descent`.
  `view_ranges_start_at_first_ordinal` also refuses a cut lineage's view
  that leads with a keep range. `seed_host_history` takes `cut`: the host's
  history ends in a cut made before adoption.
- `checkpoint_on_descending_setup_refused` (gated on `tag_checkpoint`),
  `cut_setup_leaves_reads_alone` (inapplicable unless
  `holds_reads_during_setup` says the runner ever holds reads, and needing
  the new `hold_next_compaction_setup`, `await_compaction_setup_held` and
  `release_compaction_setup` hooks) and
  `cut_numbering_continues_from_the_old_lineage` (gated on `tags`).
- `history_version_unchanged_by_a_cut`, `history_version_rises_at_a_revert`,
  `history_from_ordinal_and_before_ordinal`,
  `history_unknown_ordinal_refused` and
  `history_before_lowest_ordinal_reaches_unnumbered_entries`.
- History cases, gated on `host_history`:
  `history_pages_back_to_the_first_message`, `history_mid_matches_read`
  (bytes, ordinal and tags too), `history_no_tags_below_first_ordinal`,
  `history_ordinal_kept_below_the_cut`, `history_cursor_survives_a_cut`,
  `history_never_serves_a_reverted_branch`,
  `history_cursor_expired_after_a_revert_past_it`, `history_boundary_order`
  and `history_before_mid_continues_below_the_window`.
- `per_step_cost_scales_with_new_messages`, opt-in through
  `seed_session_history`: with 1,000 and 150,000 seeded messages behind a
  cut that leaves the same window, the median of nine timed exchanges on
  the large session is at most 2.0 times the small one's (medians under
  5 ms count as 5 ms). Without the hook it is inapplicable, never passed.
- The suite's own reference fake implements every new rule, including a
  variant whose every session holds host history without ordinals, and a
  faulty fake fails each new case by name.

## 0.9.2

Requires `cortexkit-role-llm-runner` 0.4.1.

- The media cases' PNG now has a valid IDAT checksum, and the frame-budget
  case pads the image with a zero-filled private `ckPd` chunk inside the PNG
  (before IEND, same 2,400,000-byte total) instead of zeros after IEND, so a
  host that decodes tool-result images (OpenCode) passes them. A unit test
  checks every scripted PNG's chunk CRCs and its IHDR to IEND structure.
- `session_change` cases, gated on the group: `session_refresh_applies_from_next_run`,
  `session_refresh_waits_for_its_rung`, `session_refresh_never_changes_a_running_run`,
  `session_refresh_repeat_same_answer`, `session_refresh_generation_stale_refused`,
  `session_refresh_tool_outside_host_set_refused` and
  `flush_prefix_applies_pending`. They need the new
  `LlmRunnerSubject::refresh_plan` hook (`RefreshVariant`, `RefreshPlan`),
  defaulted to `None`, and `model_requests`.
- `max_steps_final_step_completes` and `max_steps_submit_result_completes`:
  a step that reaches `max_steps` without a tool call leading to another
  step ends the run `completed`, not `max_steps`.
- `RunnerRoute` documents that adapters decode every reply body with
  `wire::read_reply` and return the answer inside; the suite sees no raw
  reply bytes, so it has no case for a bare reply.

## 0.9.1

`media_descriptors_frame_budget` and `media_descriptors_stable_across_pages`
now fit runners whose module tool traffic crosses the subconscious daemon.
The daemon never delivers a module-to-client frame over 4,194,304 bytes
(header included), so a scripted tool provider could not return either case's
result and the run never ended; some client SDKs also refuse to send a
request of that size. The rules both cases check are unchanged.

- `media_descriptors_frame_budget`: the image result is 2,400,000 decoded
  bytes (3,200,000 base64) instead of 3,200,000. The single 4,194,304-byte
  `oversize` argument is now two sibling calls in the same step, each with a
  2,200,000-byte `oversize` argument: no one request exceeds the frame, and
  the assistant message holding both is still over 4 MiB. The case checks
  that both calls are described as `args_ref`s in that one message, that an
  inline read of it is still refused `reply_too_large`, and that the
  mixed-size fallback still describes parts `[1, 3]`.
- `media_descriptors_stable_across_pages`: the multipart result is 14 parts
  of 256,000 bytes (3,584,000) instead of 20 (5,120,000). Still over the
  descriptor message budget (3,145,728), it now expects exactly 2 described
  parts instead of 8, and still the same selection on every page.
- A unit test checks that every payload a scripted tool provider receives or
  returns in the media cases, as JSON plus an envelope allowance, stays under
  the daemon's 4,194,304-byte frame.
- `tags_on_every_read_entry` and `tags_count_only_parts_rendered_with_a_number`
  now accept embedded layouts such as `opencode-aisdk`, where a call's result
  is stored in the requesting entry's tool part, so that entry carries two
  numbers: the model's text's and its tool result's (a tool result counts, a
  tool call never does). Both cases used to require exactly one number per
  entry. The number of a marker is now the number of the part holding it,
  and the counted-parts case finds the part holding the call's result from
  what the runner serves (the call's `result_ordinal` where `run_tool_calls`
  is declared, otherwise the entry holding the result's marker), requires
  that part to carry a number wherever it is, and still requires reasoning
  and a part holding only a tool call to carry none.

## 0.9.0

Breaking. Requires llm-runner 0.4.0 and compaction-provider 0.4.0, whose tag
numbers are computed from the transcript and are no longer a never-reused
ledger. The tags cases are rewritten for that, and three are renamed because
their old names state the rule that no longer holds.

- Renamed and rewritten: `tags_never_reused_across_runs` is now
  `tags_numbers_stable_within_a_lineage` (numbers consecutive and kept within
  a lineage, changed only with a lineage change);
  `tags_model_written_tags_stored_verbatim` is now
  `tags_model_written_tags_cleaned_on_exposure` (cleaned on every exposure for
  tag-rendering runs, byte for byte for untagged runs, raw for a
  structured-output run's final text where `structured_output` is declared);
  `tags_resolve_left_lineage_refused` is now
  `tags_resolve_answers_the_current_lineage` (every number the new lineage
  shows resolves there; numbers may repeat across lineages);
  `tags_exist_whether_or_not_rendered` is now
  `tags_only_on_tag_rendering_sessions`, and
  `tags_resolve_returns_stored_content` is now
  `tags_resolve_returns_exposed_content`.
- Every tags case opens its sessions with `render_tags: true` through
  `send_fields_with_plan`, since numbering applies only to such sessions. The
  crash scenario's session B does so on a tags runner.
- New cases: `tags_numbered_in_rendered_order`,
  `tags_renumbering_waits_for_the_step_boundary`,
  `tags_count_only_parts_rendered_with_a_number`,
  `tags_setup_checkpoint_anchors_at_its_cut`,
  `tags_fork_of_a_managed_session_refused` (inapplicable unless the adapter
  can create a fork the runner can detect),
  `delivery_order_template_part_gone_refused` and `adopt_host_turn_is_a_run`
  (inapplicable unless the adapter can start a host turn).
- `compaction_setup_route_session_matches_setup` also checks that a Setup
  refused `setup_identity_mismatch` ends the run `error` with that code (now a
  typed compaction-provider code), with no model request.
- `CompactionSetup` is now `#[non_exhaustive]`: an adapter's `match` on it
  needs a `_` arm.
- Subject API: `Capability::TagCheckpoint` and
  `Capability::RenderedModelInput`; `ModelRequestObservation::rendered` with
  `with_rendered`; defaulted hooks `image_tool_result(marker)` and
  `fork_session(handle, parent, fork)`; `CompactionSetup::ReadyWithCheckpoint`.
  `structured_output_frozen_across_restart` ignores `rendered` when comparing
  requests.
- The reference fake numbers, cleans and renders through the role crate's
  shared functions, renders a steer stored during a held call after the
  call's result, and moves to a new lineage at the next step boundary when
  that renumbers an exposed part. Each new or rewritten case has a defect
  that fails it by name in the suite's own tests.
- Add descriptor-read cases for decoded media hashes, nested
  media, frame-sized arguments, UTF-8 heads, inline compatibility, full-inline
  message/part identities, pairing preservation, strict digest keys, and stable
  per-message fallback across page caps. Adapter media fixtures gate these
  cases; each supported rule has a named defect-fake check.

- Check lost marker-key fallback, in-process save seams,
  standalone refusal handoff only before first service, and command cutoffs
  on each side of forwarding. Adapter-only situations are named inapplicable
  when unsupported; defect fakes exercise the corresponding failures by name.
- The frame-budget case also checks mixed-size fallback selection: largest
  payload first, with the earlier of two tied parts selected at the cut.
  A smallest-first defect fake fails this case by name.
- Preserve the 0.8.11 route-before-prepare fix for the crash-cut command case;
  adapters may derive session references from the runner bound by the route.

## 0.8.11

- The crash-cut user-command case (`user_commands_outcome_unknown`) now calls
  `prepare_user_commands` after spawning the runner and opening the session's
  route, as every other user-command case does. Before, it called the hook
  before the runner existed, so an adapter that computes the session reference
  from the runner instance the route bound failed the case before it reached
  the runner. `prepare_user_commands` documents this order, and the reference
  fake now refuses a call made before the session's route is open.

## 0.8.10

- Require llm-runner 0.3.10. Add a crash-cut user-command case proving that a
  forwarded invocation whose reply was lost is stored as `outcome_unknown`,
  replayed by invocation and status reads, and never forwarded again. The
  reference runner persists its reservation before forwarding; faulty runners
  that forward again or report `failed` fail this case by name.

## 0.8.9

- Require `cortexkit-role-compaction-provider` 0.3.3. Add
  `compaction_setup_route_session_matches_setup`, which rejects a runner's
  single-session provider route when its bind session differs from Setup's
  session, while accepting shared project routes with an empty bind session
  and generic `runner` harness. The scripted provider observation now carries
  the Setup body identity and the actual incoming route bind identity
  separately. A faulty reference runner binding another session fails the
  case by name.

## 0.8.8

- Require llm-runner 0.3.9. Add live-route user-command checks for history and
  model-input isolation, once-only forwarding, no-person sessions, collisions,
  ordered progress, owner-only relay and unsafe host menus. Adapters install
  scripted providers and observe provider/model/host boundaries independently.
- Add faithful and deliberately faulty runner self-tests, failing by case name.

## 0.8.7

- Accept an unreadable requesting-message ordinal while a tool call is open in `run_tool_calls_result_ordinal_tracks_storage`: embedded call/result messages are not finished until the tool returns. If the message is already readable, it must still carry the call's arguments; open calls still require position 0 and no `result_ordinal`.
- After release, require the requesting message to exist and carry the arguments, as well as checking that `result_ordinal` locates the stored result. Self-tests cover an embedded layout with equal call and result ordinals, a faulty runner that loses the finished requesting message's arguments, and an early-readable placeholder that gains the arguments only after release.

## 0.8.6

- Require llm-runner 0.3.8. Add `delivery_order_declared_modes_respected`, gated on the live describe answer advertising `delivery_order_modes`. It tries every non-empty ordered list of distinct modes: lists containing only undeclared modes must be refused `delivery_unsupported` with the original `detail.delivery` and no transcript writes; lists containing a listed mode must be accepted.
- Mode-specific delivery scenarios are inapplicable when the describe answer excludes the modes they need. The fake can advertise only `steer`; a fake that ignores the member fails the new case by name in `ignoring_delivery_order_modes_fails_by_name`.

## 0.8.5

- Require llm-runner 0.3.6.
- Add `steer_without_run_ops_reports_state_and_step`, which runs only when the runner does not declare `run_ops`. A scripted steer into an active turn must answer with state `active` or `finished`, `delivered.as: "step"`, a non-empty `ref` and no `run_id`. `step` is never a state value.
- Add `delivery_order_retract_pending_tool_result`, gated on `retract` and `delivery_order`: while a tool call holds the run before its result, a `[tool_result]` send answers pending with a `submission_id`, retract answers `retracted` and then `not_pending`, and releasing the call does not deliver the send. The fake now keeps this withdrawn submission out of later delivery boundaries.
- Add `delivery_order_undeclared_mode_refused`, gated on `delivery_order`, `queue`, a held tool call and the absence of `steer`. A `[steer]`-only list must be refused `delivery_unsupported` with its exact array in `detail.delivery`; `[steer, idle_start]` is accepted because `queue` declares `idle_start`, but waits for the run to end and then delivers only as `idle_start`. Cases that require steer delivery also require the `steer` group; template fallback is inapplicable if neither `steer` nor `turn_system` is available. A fake defect that queues the all-undeclared list fails this case by name in `queuing_a_list_with_no_declared_mode_fails_by_name`.
- Keep the durable re-send and `send.status` cases gated on `run_ops`; both still require `run_id` for their delivered `step` answers. The fake defect that returns `state: "step"` fails the no-`run_ops` case by name in `a_steer_reply_with_state_step_fails_only_the_no_run_ops_case`; the fake defect that omits `run_id` while declaring `run_ops` fails the re-send and status cases by name in `a_delivered_steer_without_its_run_while_declaring_run_ops_fails_both_follow_cases`. The fake does not simulate provider switches, so the stored `turn_system` rendering rule has no separate case.

## 0.8.4

- Require llm-runner 0.3.5 and add `Capability::TurnDirectives` and `Capability::InterruptReason` to the non-exhaustive enum.
- Add seven named turn-directive cases, gated on the new group with named skips: each completed wake form appears on result and, where served, status without text edits; max_steps, cancelled and unrequested runs report none; unknown/non-array/duplicate names are refused. Each case has its own deliberately broken fake runner, which fails that case and no other.
- Add requesting-message sibling-position and open/finished result-ordinal cases under `run_tool_calls`; swapped positions and a premature result_ordinal each fail by name.
- Add a graceful handle-stop/restart case for `interrupt_reason`, checking runner_shutdown on result/head and on status where served. The fake distinguishes an orderly handle drop from a crash-cut handle and records the reason durably; a fake that omits the shutdown reason fails the `interrupt_reason` case, named in the report.
- Additive patch: no new adapter hooks, and callers not using new groups retain their wire shapes. `run.tool_calls`' new `position` and `result_ordinal` are optional on the wire, so older runners still decode, while the contract now requires a runner declaring `run_tool_calls` to send them.

## 0.8.3

- Require llm-runner 0.3.4. Add the independent `Capability::{MaxSteps,MaxOutputTokens,NoTools}` variants to the already non-exhaustive enum.
- Add four named, discovery-gated cases: `max_steps_seals_pending_calls` (two responses, pending siblings sealed at the terminal without dispatch), `max_output_tokens_caps_response` (positive dispatched maximum and single-response usage at or below the requested cap, also admitting a request above the model maximum), `no_tools_offered_to_model` (an unsuppressed control offers tools, the suppressed request offers none), and `no_tools_later_plan_send_refused` (empty and non-empty later lists both refused naming `tools` without writes). Missing groups are named skips; the output case also requires `run_usage`, and the late-tools case is inapplicable by name when the adapter cannot create a fetch-plan session.
- `ModelRequestObservation` gains optional `max_output_tokens` and a builder, recorded from the dispatched request. `model_requests` is now required when declaring `no_tools` or `max_output_tokens` as well as `structured_output`. In output-cap cases the scripted provider honours the maximum it actually receives and reports the truncated response's actual usage.
- Add five fake defects, each failing only its named case: ignored step limit and unsealed pending calls fail `max_steps_seals_pending_calls`; ignored output cap fails `max_output_tokens_caps_response`; offered tools despite an empty list fails `no_tools_offered_to_model`; ignored later-plan tools fails `no_tools_later_plan_send_refused`. The fake stores limits per run, counts model responses, caps its whitespace-tokenized response at its model maximum, and seals pending calls in the terminal record.
- Additive patch: no role wire bytes change and existing adapters need no new hooks unless they declare the new groups.

## 0.8.2

- Require cortexkit-role-llm-runner 0.3.3. `Capability` gains `SendStatus` (the new `send_status` group), listed last in `Capability::GROUPS`; `Capability` is already `#[non_exhaustive]`, so this is additive.
- New `tags_resolve_left_lineage_refused`, gated on `tags`, `queue`, `transcript_reads`, `dispatch_attribution` and the `StepRecorded` kill point. It reads the `crash_at_StepRecorded` scenario: after a seal that drops the never-sent call or its step onto a new lineage (§14 shape (a)), every tag number below the new lineage's highest that no part read from it carries (at most 64, the dropped part's tag among them, since the kill comes before any read of the cut step) must be refused `unknown_tag`, and the cut run's prompt must resolve with its ordinal and `mid` in the new lineage. It is reported inapplicable, by name, when the runner resumes the run or keeps the call, or when the seal dropped no tagged part.
- New `delivery_order_steer_receipt_durable_across_restart`, gated on `delivery_order`, `queue`, `transcript_reads` and held tool calls: on its own state root, a `[steer]` list sent while a run waits on a held call must answer `pending` with a `submission_id`, be delivered once as a steer when the call is released, and its re-send must answer `delivered: {as: "step", ref}` with `run_id` the run it joined, before a harness restart (no kill) and unchanged after it.
- New `send_status_answers_as_a_resend_after_restart` (the same, through `send.status`, which must answer exactly what a re-send answers before the restart and `step` with the joined run's `run_id`, never `pending`, after it) and `send_status_unknown_send_refused` (`send.status` of a settled send equals its re-send's answer, an unrecorded `send_id` is refused `unknown_send`, an unknown field is refused `invalid_params` naming it). Both are reported as named skips without `send_status`, and send `send.status` only after the live `role.describe` declares the group.
- The restart cases run their own runner and restart it with `Harness::restart`, without a kill, so they add nothing to the run's kill ledger.
- The reference fake answers a delivered queued send with the run that carried it and how (`step` with the row, or `turn` with the run), serves `send.status` when it declares the group, and keeps a dropped step's messages only for a new defect that resolves their tags. Three new defects fail their cases by name in the suite's own tests: `send.status` answering `pending` after a restart for a steer delivered before it fails `send_status_answers_as_a_resend_after_restart`; a delivered steer answered without `run_id`, on a re-send and on `send.status`, fails both `delivery_order_steer_receipt_durable_across_restart` and `send_status_answers_as_a_resend_after_restart`, each naming the run the steer joined; and resolving a left lineage's tag with its old content fails `tags_resolve_left_lineage_refused`.

## 0.8.1

- `crash_at_StepRecorded` accepts both seal shapes that llm-runner 0.3.2 (CONTRACT §14, property 4) names for the never-sent call, and fails anything else, naming the shape it was closest to (`closest to shape (a)` when the cut-run transcript has no attribution entry for the call, `closest to shape (b)` when it has one) and why:
  - (a) the call is absent from the cut-run and follow-up transcripts, the session is on a new lineage, and a read naming the old lineage is refused `lineage_changed` (the refusal is new: 0.8.0 checked only that the lineage moved);
  - (b) the lineage is unchanged, every message of session B read before the kill reads back identical, the call is kept and not indeterminate, the message right after the one carrying it holds its never-executed result once the run has ended `interrupted`, and the owner's follow-up leaves that transcript unchanged and writes no message carrying the call.
- Narrowed: 0.8.0 also passed a kept call whose result was written only when the owner's next send arrived. That result is in a later commit than the run's end, and the transcript read between the two holds a dangling call, so the case now fails it.
- The reference fake's keep-the-call seal writes the never-executed result in the record that ends the run. Two new defects each fail the case by name in the suite's own tests: writing the result only at the owner's next send, and changing the bytes of a message read before the kill while keeping the lineage. The existing defect that never writes the result fails it with the shape (b) reason.
- Still requires cortexkit-role-llm-runner 0.3 (0.3.1 or later); nothing in this release needs a newer type.

## 0.8.0

- Require cortexkit-role-llm-runner 0.3.1 and add `run_usage_counts_reported_steps`, run only when the subject declares the new `run_usage` group (otherwise reported as skipped, by name). After a two-step run whose scripted model reports input, cache-read, cache-write and output counts but no reasoning, `run.result` must carry `usage` with `steps: 2`, no unreported step, exactly the scripted totals mapped to `input_tokens`, `cached_input_tokens`, `cache_write_tokens` and `output_tokens`, a `by_model` that sums to them (`RunUsage::consistent`), and `reasoning_tokens` absent rather than `0`. Where the subject declares `run_status`, the ended run's `run.status` must carry the same `usage`. The case never reads `run.status` before the first step, where `usage` may be present or absent.
- Breaking: `Capability` gains `RunUsage` and is now `#[non_exhaustive]`, so a `match` on it outside this crate needs a wildcard (`_`) arm; later groups then arrive without a breaking release. `Capability::GROUPS` lists `RunUsage` after `RunToolCalls`, in the order of the role's `capabilities::GROUPS`.
- The reference fake reports usage when it declares `run_usage`; its new defect, reporting an absent `reasoning_tokens` as `0`, fails the case by name in the suite's own tests.

## 0.7.1

- `run_tool_calls_listed_in_dispatch_order` no longer gives two sibling tool calls in one step the same model id. A real provider never repeats an id within one response, and a host that keys tool parts by id (OpenCode does) stored the two siblings as one part, so the case failed a correct runner. Siblings now have distinct ids that sort against dispatch order, and the next step's call reuses the first id, so ordering or identifying calls by model id still fails.

## 0.7.0

- Requires cortexkit-role-llm-runner 0.3.0 and cortexkit-role-compaction-provider 0.3.0, which is async only. Removed with the synchronous compaction interface, with what callers lose:
  - the cases `compaction_wait_cap`, `compaction_late_or_stale_answers_discarded`, `compaction_step_timeout_uses_last_view`, `compaction_one_view_per_step` and `compaction_fence_crash_replays_model_view` (the last cut a step answer at `CompactionApplied`);
  - the types `CompactionScript`, `CompactionStep`, `CompactionAnswer` and `CompactionCallKind`, and `CompactionCall`'s `kind`, `issued_at_ms` and `deadline_ms`;
  - the `LlmRunnerSubject` hooks `answer_compaction` and `advance_compaction_clock`; `install_compaction_script(session, CompactionScript)` is replaced by `install_compaction_setup(session, CompactionSetup)`.
  - `SendRequest.prompt` is `Option<String>` in llm-runner 0.3.0: template sends now carry no `prompt`, and `delivery_order_template_renders_as_system` also checks that a template send carrying `"prompt": ""` is refused naming `template`.
  Kept, Setup-only: `compaction_setup_durable_once`, `compaction_unavailable_distinct_from_refuse` (its step-`REFUSE` half removed) and `model_view_half_open_ranges_and_travel` (now Setup's head insertion and the held request's canonical view; the summary half needed a step answer). A NARROWINGS entry says the runner half of compaction-provider 0.3.0 §3 has no live case yet.
- `RunState` is now non-exhaustive in llm-runner 0.3.0 with `BudgetExhausted` and `ContextCeiling` variants; every match in this crate keeps a catch-all arm.
- Add cases for llm-runner's new groups, each run only when the subject declares the group (otherwise reported as skipped, by name, with no op of the group sent):
  - `tags` (seven cases): part rows and tags on every entry, never reused across runs, present whether or not rendered, `session.resolve` content, ranges and refusals, model-written `§N§` stored and dispatched as written;
  - `run_tool_calls` (three cases): dispatch order through `next` paging, `unknown_run` and field refusals, `indeterminate` until a held call's result;
  - `delivery_order` (eight cases): a list without `idle_start` never starts a run, the first possible mode wins, dedupe supersedes, expiry, `template`/`dedupe_key`/`expires_at_ms` refused by name without a list in `delivery`, plan-named senders and the `idle_start` grant, a template rendered once for whichever mode carries it, and an out-of-range slot refused.
- Add `delivery_order_list_refused_without_group`, run only when the subject does not declare `delivery_order`: a list in `delivery` is refused naming `delivery` and writes nothing, the refusal that makes the overload of `delivery` safe.
- Add `run_context_ceiling_wait_is_not_a_stall` (harness capability `HoldContextCeiling`) and `read_stable_across_host_metadata_change` (harness capability `HostMetadataChange`).
- `crash_at_StepRecorded` now gives session B one finished run before the kill and fails a recovery that drops the never-sent call or its step without moving B to a new lineage.
- Breaking: `Capability` gains `Tags`, `RunToolCalls`, `DeliveryOrder`, `HoldContextCeiling` and `HostMetadataChange`, so an exhaustive `match` needs the new arms.
- `LlmRunnerSubject` gains defaulted `send_fields_with_plan`, `plan_sender`, `turn_system_supported`, `hold_context_ceiling`, `end_context_ceiling_wait` and `change_host_metadata` hooks; the new `PlanSenderIdentity` is non-exhaustive with a constructor.
- The reference fake's defects (tag reuse, rewritten model tags, reversed tool calls, runs started without `idle_start`, unchecked template slots, unnamed senders admitted, dedupe and expiry ignored, last possible mode, a list accepted without the group, list members accepted without a list, drop recovery keeping its lineage, reads serving live host metadata) each fail their named cases in the suite's own tests.

## 0.6.0

- Require cortexkit-role-llm-runner 0.2.6 and add `session.adopt` cases, run only when the subject declares `adopt` (otherwise each is reported as skipped, by name, and no `session.adopt` is sent); each case also refuses to send one unless the live `role.describe` declares `adopt`. They check that:
  - a first adoption, with no message yet and after the person's first turn, answers `owner_bound` with the transcript's last ordinal, writes nothing and starts no run, and `session.baseline` then answers only the adopting owner;
  - an exact repeat replays the original reply; the same `send_id` with another plan is refused `send_id_reuse` naming `plan`;
  - a plan other than the host's is refused `plan_drift` with distinct `frozen` and `sent` identities and no content, binding no owner and recording no `send_id`; a session the host never started answers `plan_drift` without `frozen`;
  - an adoption during a person's held run leaves the run in progress, and it completes once released;
  - another caller, even repeating the owner's request, is refused `scope_owner_mismatch`; the same owner re-adopting succeeds;
  - after adoption the owner's send runs, and another caller's is refused.
- Breaking: `Capability` gains `Adopt`, so an exhaustive `match` on it needs the new arm.
- `LlmRunnerSubject` gains defaulted `host_plan` and `start_host_session` hooks, required only when declaring `adopt`.

## 0.5.0

- Require cortexkit-role-llm-runner 0.2.5 and add structured-output cases, run only when the subject declares `structured_output` (otherwise each is reported as skipped, by name). They check that:
  - `final_text` on a tool-free run whose route reports support completes `enforced` and `passed`;
  - `final_text` on a run with tools, an admitted tool named `submit_result`, and a schema with an external `$ref` or `$dynamicRef` are refused at admission naming their fields, writing nothing;
  - `submit_tool` offers `submit_result` from the first request, and calling it ends the run `completed`;
  - a sibling of `submit_result` in the same step is never invoked at the scripted tool provider, even when emitted first, gets a never-executed `skipped_after_submit` result and is listed in `skipped_siblings`; a second `submit_result` is a skipped sibling;
  - a run finishing without `submit_result` reports `absent`, and a value failing the schema reports `failed` on a `completed` run;
  - the next model request after a kill and restart carries the same tools and schema bytes;
  - a later send without `structured_output` carries no schema.
- Breaking: `Capability` gains `StructuredOutput`, so an exhaustive `match` on it needs the new arm. That public type change makes this a minor (0.5.0) rather than a patch.
- `LlmRunnerSubject` gains defaulted `send_fields_with_tools`, `set_structured_output_support` and `model_requests` hooks, required only when declaring `structured_output`; the new `ModelRequestObservation` is non-exhaustive with a constructor. A `ScriptedToolCall` may name `submit_result`.

## 0.4.0

- Require runner 0.2.4 and add input-token cap cases, run only when the subject advertises `input_token_cap` (otherwise each is reported as skipped, by name). They check that:
  - a step does not start once consumed input equals the cap;
  - when a step crosses the cap, none of its tool calls run, and each gets a never-executed result;
  - the cap and the running total survive a runner restart between episodes;
  - a later send can lower the cap but not raise it;
  - unknown members of `budget` and of a nudge are refused;
  - a step with no reported usage ends the run as `budget_exhausted` with cause `usage_unreported`.
- Breaking: `ScriptedTurn` gains optional per-step `ScriptedUsage` and becomes non-exhaustive. Replace struct literals with `ScriptedTurn::new(parts)` and `.with_usage(...)`; absence deliberately omits usage. Report input/cache-read/cache-write/output exactly as scripted, without estimates.
- Input-cap adapters must implement external `model_invocations` observations and decode native not-executed results through `not_executed_tool_result`. The suite does not impose a shared tool-result body.
- The fake runner reads typed budget copies through `SendRequest::budget()` while preserving raw `runner_params["budget"]`; malformed budget refusals retain their field paths.

## 0.3.1

- Require cortexkit-role-llm-runner 0.2.3, which adds the terminal `reply_too_large` refusal and its frame-size detail.

## 0.3.0

- Requires subc-protocol 0.32 and cortexkit-role-compaction-provider 0.2.3.
  Consumers must use the new conformance major with protocol 0.32 because its
  public route errors expose `ErrorBody`; update the paired compaction provider
  dependency as well. No conformance behavior changes.

## 0.2.2

- Add discovery-gated boundary cancellation, unlisted-point refusal and immediate escalation cases. Missing `step_boundary` support is reported as named skips, never passes. The boundary case also checks owner-only admission and idempotent repeats.
- Add defaulted held-model control/observation hooks to the subject adapter and broken fake-runner variants for each new rule, with tests proving each case rejects its corresponding defect.
- Require `cortexkit-role-llm-runner` 0.2.2 for `RunCancelRequest.at`, cancellation-point constants and `RoleDescribe.run_cancel` discovery metadata.

## 0.2.1

- Update the compaction-provider dependency to 0.2.0, which adds optional native endpoints, immutable view retrieval, context geometry and notice receipts for Claude Code. Publish this dependency change under a new patch version; existing conformance cases and behavior are unchanged.

## 0.1.4

- The compaction cases that assert how a run ended now wait for it to end
  instead of reading `run.result` once. A runner may accept a send and end the
  run a moment later, so a single read could see `active` and fail
  `compaction_unavailable_distinct_from_refuse` with "Setup failure ended
  "active", not error" on a correct runner under load. `compaction_wait_cap`
  still reads once before the cap, where it checks the run has not ended.

## 0.1.3

- `compaction_setup_durable_once` checks that compaction Setup survives a crash
  without being run again. When the runner resumes by a new send, the case
  recognised that send's user message partly by the run named on the message.
  It now recognises it by its prompt text and the send's new run only. Naming
  the run on each message is a separate contract rule (section 6, run
  attribution), still required of runners that declare `run_ops` and checked
  by its own case, `run_attribution_per_message`.

## 0.1.2

- Accept both automatic and sender-driven recovery in `compaction_setup_durable_once`: an interrupted run resumes through a new send on the same session without repeating Setup.
- Check that the held model request preserves the recovered summary and kept messages, including message ids, content and ordering, with only the resume user's message appended.
- Exercise both recovery shapes, repeated Setup on a resume send, changed kept content, and unexpected recovery states in the suite's fake-runner tests.

## 0.1.1

- Add the optional controlled retention clock for conformance subjects, so long retention cases run without wall-clock delays; cases without the controlled retention clock skip waits longer than 30 seconds and report the skipped case by name.
