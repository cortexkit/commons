# Changelog

## 0.3.3

- Add optional `Entry.speaks_for` with agent identity and an optional display-only
  registry name. Agent words stay attributed to that agent in summaries and
  context maps; attribution alone is not a notice or automatically swept.
- Gate projection on provider manifest `compaction-entry-speaks-for/v1`, read
  from the catalog at every Setup and retained until the next Setup. Providers
  deploy first and cannot withdraw the capability mid-session without restarting.
  Add the capability constant, declaration helper and gated entry builder.
- Exclude attribution from every checksum; stored marks can be backfilled with
  no digest or view rewrite. Wire vectors cover marked entries with and without
  notice, provider-declaration omission and digest preservation.
- Patch bump: `Entry` and the shared `SpeaksFor` are non-exhaustive with
  constructors. Require runner 0.8.34 for its optional name; no existing
  constructor or checksum shape changes. The operator approved the carried-agent
  attribution contract for release.
- Readers first: Magic Context deploys reading `Entry.speaks_for` and declares
  `compaction-entry-speaks-for/v1` before any runner sends the member. Core sends
  `notice` with `speaks_for` only to runners declaring `notice_with_speaks_for`.

## 0.3.2

- Add MV7: every accepted view of a moved-in session must cut through the
  entry the move's cut goes through or a later entry on the active path.
  Runners check handed-over views, `compaction.ride` answers and step
  boundaries, keep the view already in force on refusal, and report
  `view_refused` at a step boundary.
- Add `ViewRefusal::CutBeforeMove` (`cut_before_move`) with `moved_through`,
  the `mid` the move's cut goes through, for a missing or earlier cut.
  This is additive: `ViewRefusal` is non-exhaustive, and its `open_tagged!`
  decoder preserves unrecognized codes and their fields as `Unknown`, so
  older readers keep the new code and payload without interpreting them.

## 0.3.1

- Add optional `Conditions.last_prompt_view` to notices and rides to attribute
  `last_prompt_tokens` to the applied view used when the runner built the request.
  Positive versions name a view, `0` means no view, and absence means unknown.
  Send it only with `last_prompt_tokens`; providers ignore it without that reading.
  Explain how providers avoid compacting from stale readings after a newer view
  is applied; over-limit rides still act on their own estimate.
- No Setup opt-in is needed: `Conditions` has no `deny_unknown_fields`, so older
  providers ignore the new field, and the field enters no checksum defined in
  CONTRACT.md W7. Existing vectors and their checksums are unchanged.

## 0.3.0

- Add the `turn_system_cleared` ride form for confirmed clearing routes and
  define omission on other routes. `RideForm` is now `#[non_exhaustive]`, a
  breaking source change: downstream matches must add a wildcard arm. Unknown
  form strings still decode by skipping them, falling back to `ToolResult`
  when no known form remains. Ride token-count semantics are unchanged.
- Require llm-runner 0.8.29 for the coordinated delivery contract.

## 0.2.12

- Align the runner dependency with 0.8.28.
- Add runner-sent Setup opt-in `view_anchor`. Providers include `built_through`
  and `new_signed` in views and their checksums only for opted-in runners; without it,
  preserve the pre-anchor sealed shape byte for byte. Future checksummed view
  fields also require an individual Setup opt-in.

## 0.2.11

- Add `NoticeOrigin.keep`: default false, omitted when false, and included in
  the entry digest only when true. Existing vectors and entry digests stay
  unchanged. The owner marks notices carrying the person's own words; providers
  never drop them automatically, but an agent's explicit drop still may.
- Clarify RD3's one cache clock per session: a serving-model change invalidates
  earlier touches, even after a switch away and back, and expiry remains unknown
  until a touch on the model now serving.
- Roll out runners and providers that decode `keep` before core sends it; core
  sends it only to runners declaring `notice_keep`.

## 0.2.10

- Define compaction-item refreshes: accept adding one, refuse removing one
  with `plan.compaction_item`, and allow refusing or applying provider, preset
  and params changes. Applied changes trigger Setup; the last view stays in
  force until its replacement, never exposing the whole history at once.
- Add optional `AppliedViewAnswer.applied_by`, recorded from the attested
  applying route, so providers replace foreign views without adopting their
  cuts or edits. Add optional `SetupRequest.child_harness` for child sessions
  when the parent's host cannot create them.
- Add checksummed optional `View.built_through` and open-string `new_signed`.
  Define `strip` as permission to drop invalidated signed parts strictly after
  an active-path message anchor, preserving PR5 and PR6. Document captured
  Anthropic evidence and the absence of Gemini evidence; encrypted reasoning
  is unaffected. Include wire and checksum compatibility coverage.
- Roll out providers that decode the new members before runners send them.

## 0.2.9

- Pin PR6's protection through trailing system entries: the newest entry with
  parts still keeps its final parts; when its role is `system`, the newest
  non-system entry with parts keeps its final parts too. Role, not `notice`,
  selects the entries, so a trailing user notice changes nothing. This strictly
  extends the original protections; every accepted view passes the old rule.
  Roll out the provider first, then runners: a runner checking the new rule
  would refuse views that a provider still on the old rule builds. Violations
  keep the existing `final_turn_removed` refusal and its zero-based edit index;
  no wire change.

## 0.2.8

- Clarify RD3 and the matching restart and `cache_expired` text: proven expiry
  requires every cache touch since the runner's own start, warms included,
  and the newest touch plus its reported lifetime to have passed. Expiry is
  unknown after start until a touch arrives; implicit caches never count as
  expired. Stopping warming leaves expiry at the last touch plus its lifetime.
  This is a text-only clarification with no wire change.

## 0.2.7

- Add provider rule PR6: a view never removes the turn the request ends on,
  the newest entry that has parts. Its last tool part stays (its output may
  be replaced), or at least one of its parts, and no cut reaches it. Runners
  refuse a view that breaks it with the new `rule` refusal
  `RuleBreak::FinalTurnRemoved` (`{"rule": "final_turn_removed", "index"}`),
  keeping the view in force, and report `view_refused` at a step boundary.
  `RuleBreak` is non-exhaustive, so older readers decode it as an unknown
  rule. New vector `view-rule-final-turn.json`.

## 0.2.6

- Add optional `Delivered.send_id`, ordered rendering `forms` and ride-text
  `tokens`, with builders and compatibility vectors. Absent members preserve
  existing bytes; unknown forms fall back safely. Counts follow the existing
  tokenizer-family rules and never enter part or entry checksums.
- Pin rides following their output through views and the runner's 24-hour
  post-cut retention condition. Eligible discarded rides leave the read
  projection with refreshed digests, no history rewrite and no notice;
  readers recognize that narrow exception. Served items never gain send ids
  or forms. Re-export llm-runner's dependency-free `rides` rendering helpers
  for shared string and content-list placement.

## 0.2.5

- `cache_expired` now means proven expiry only: the runner's own clock shows
  its last request for the session, keep-warm requests included, is older
  than the cache lifetime it requested. A runner that cannot tell, including
  after its own restart without a durable record of its last request, sends
  no `cache_expired` (RD3, F3). Providers treat the cause as a rebuild that
  costs nothing extra, so an unproven report could prompt needless provider
  work, including a new view, depending on the provider's policy.
- RD3 names what proof needs: the lifetime the runner asked the model
  provider for, the last request's time on the same clock, kept durably to
  survive a restart. RD5 says how a provider reads a missing cause, and the
  ride deadline's note no longer suggests a cold cache.
- A runner lists `cache_expired` in `detects` only for routes where it keeps
  that clock (RD5). The wire is unchanged.

## 0.2.4

- Add optional runner token counts on text, tool and file parts, assistant
  reasoning, and notice system text and tool definitions. Pages and notices
  name their opaque `token_family` and may identify whether it uses the model's
  own tokenizer with `token_family_exact`; absent counts still mean estimate.
- Define full-content coverage, part-count stability within a family, family
  adoption, measured-reasoning operands and freezing within an entry and family,
  and next-request notice-only surface counts. Drop entry counts first to fit
  the serialized compact entries-array budget; condition counts stay.
  Count metadata updates alone do not change entries or trigger notices;
  updates that also include a notice trigger send the notice normally.
  Count and family metadata enter no checksum, including on unknown part kinds.
- Add counted page and notice vectors, checksum-invariance and numeric-boundary
  tests, and stable canonical pretty-printed fixture-byte checks, plus typed
  round-trips for existing wire samples.

## 0.2.3

- The contract now says who may call the runner (W2.5). A runner admits a
  `compaction-runner/v1` route only when the daemon-attested caller is the
  session's selected provider, `compaction_item.provider` of the plan applied
  to the session, for every op on that role, reads included. Otherwise it
  refuses `compaction_not_permitted` at bind, and it may also refuse single
  calls. `detail.reason` is an open string; `provider_mismatch`,
  `no_compaction_item` and `session_not_held` are defined. Before, the
  contract did not specify this check.
- When a change takes effect that leaves a caller without that authority (a
  different provider, no compaction item, or the runner no longer holding the
  session), the runner closes that caller's routes or refuses every later call
  on them. A runner may instead refuse a refresh that changes the provider.
- On `compaction_not_permitted`, or when the runner closes such a route, the
  provider suspends work for that session, keeps its state and resumes only
  after a later Setup from the same runner for the same session answers
  `ready`. A runner calls Setup on the new provider when
  an applied plan changes provider (S1).
- Add `codes::COMPACTION_NOT_PERMITTED`, the `not_permitted_reasons`
  constants and `Refusal::compaction_not_permitted`, with the
  `refusal-compaction-not-permitted.json` vector. The crate supplies the wire
  form only; runners implement the check.

## 0.2.2

- The contract now requires a runner whose Setup is refused (any code) or
  unanswered, with no view applied, to end the current run `error` with
  `provider_code: compaction_unavailable` before any model call, and to call
  Setup again before the next send (W1.3a, S1). Before, a refused Setup left
  such a session running without compaction. A runner with an applied view
  keeps using it and calls Setup again at its next S1 trigger.
- Add optional `Entry::completed_at` and `with_completed_at` for the host's
  completion timestamp in epoch milliseconds, under the same integer rules as
  `written_at`. Missing fields decode as absent and are omitted on the wire.
- Specify which host stamp each runner returns, and that the field is absent
  while an entry is unfinished. Time markers use `completed_at`, or
  `written_at` when it is absent; moved or imported history keeps the clock
  skew caveat (U3).
- Specify that a stored entry may gain a previously absent `completed_at`
  without a transcript change: it enters none of the W7 checksums, and adding
  it alone needs no notice (P5, N1, W4.9, W5.2).
- Add optional-timestamp round-trip and integer-boundary tests, plus a vector
  and tests showing unchanged entry digests. Existing vectors are unchanged.

## 0.2.1

- Depend on llm-runner 0.8 (the one-based entry contract).
- Add inserted-image wire types: PNG images on user-role inserted messages,
  rendered after their texts without tag numbers. Images count toward the
  view's size and checksum; empty image arrays are omitted and hash as absent.
  Specify the runner's required `unsupported_edit` (with the insert's edit
  index) for images on other roles, non-PNG media types and invalid standard
  padded base64. Wire decoding validates neither; the view library adds the
  checks.
- Add optional `accepts_images` to Setup and conditions: absent means unknown,
  and a provider inserts images only when the runner reports `true`.
- Specify that a message id is never given to a different message within a
  session, even after a revert or branch switch, and that providers key durable
  part state on both the message id and the part id (P6).
- Note that `step-transform-provider/v2`'s `pre_user` hook carries an optional
  `at_ms` taken on the runner's machine, so gaps against write times from that
  machine have no clock skew; without it the provider uses its own clock (U3).
- Add image-insert, image-capability Setup and image-refusal vectors (assistant
  role, non-PNG); existing vector bytes are unchanged. Add tests for image
  checksums, empty-image normalization and absent/true/false capability.
- Vector regeneration skips writing a file whose bytes already match.

## 0.2.0

- **Breaking:** a notice's `newest` is now optional and absent only for an
  empty transcript, including after a revert. Construct notices with
  `NoticeRequest::new(seq, conditions)` and add a cursor with `with_newest`.
- Allow signed parts to be omitted as an oldest run, a newest run, or both,
  while still refusing a gap between kept signed parts (PR2).
- Specify that empty tool `attachments` and `delivered` and notice `events`
  and `entries` may be omitted, decode as empty, and are omitted for checksums
  (W1.6). Add an entry checksum vector with both attachment forms.
- Restrict the contract's own integer fields to 0 through 9007199254740991
  for exact JavaScript round trips, with named `invalid_params` refusals;
  opaque JSON payloads keep their numbers unchanged (W1.2).
- Refuse a read's `limit: 0`; a supplied limit must be at least 1 (W5.4).
- Qualify byte-identical reply rendering: cleaning must change nothing else,
  including surrounding whitespace (T7).
- Explain that a view may hide the newest number and the runner replaces a
  lower number the model then writes with the assigned one (W6.4).
- Adopt step-transform-provider/v2: answer `role.describe` and
  `transform.declare` on any route of that role, and refuse an unbound
  `transform.hook` per call with `route_unbound`, without changing compaction
  route requirements (W2.1, W2.4). Retire the transitional rule for
  step-transform v1 hook routes, along with the `session_mismatch` refusal
  code, its constructor and its vector.
- Add vectors for an empty-transcript notice, accepted and refused integer
  boundaries, and a refused zero read limit; refresh vector checksums.

## 0.1.0

- Introduce the `compaction-provider/v2` and `compaction-runner/v1` wire contract.
- Provide forward-compatible wire enums, extensible construction APIs, and
  JSON vectors for requests, answers and refusals, round-trip tested, with a
  SHA256SUMS manifest.
- Use the published runner contract's canonical content digest and existing
  `session.send` / `run.result` protocol.
