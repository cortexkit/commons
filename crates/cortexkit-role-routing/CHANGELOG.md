# Changelog

## 0.7.19

- Pin gateway `explicit_providers` matching to the provider whose catalog rows
  the credential serves, runner-wide and per harness. Naming the gateway's
  credential provider id admits no extra routes; other auth methods continue
  to compare the credential's own provider. No wire shape or capability changes.

## 0.7.18

- Add independently discovered consumer declarations, per-harness facts,
  harness-bound decisions, workload kinds, eligible route subsets, select
  value estimates and provider attempt outcomes. Existing members retain
  their types; omitted additions preserve legacy body and fingerprint bytes.
- Add constructors, builders and capability-aware validators, including a
  declaration-default resolver that preserves an explicit zero iq. Pin module
  roots, attested selectors, narrowing placement and dotted inheritance.
- Share the exclusion-spec validator/matcher and existing ladder pin parser.
  Prices use JSON numbers and verbatim cache-write lifetime maps; callers
  settle usage with list or already-divided subscription-equivalent rates.
  Unknown attempt statuses count nothing; rejected requests never penalize routes.
- Treat a throttled attempt's `retry_after_ms` as optional provider evidence:
  absent means unknown and routing chooses backoff, never an invented wait.
  Other statuses and malformed supplied durations remain invalid.
- Readers first: declarations are not sent until the router's live reader
  advertises the corresponding group. Add checksummed samples for every group.

## 0.7.17

- Add capability-gated `warm_credential` to `route.account_for` admissions,
  with a builder and public credential-id validation. A fresh conversation
  can keep its last-used eligible credential; affinity, pins, holds, reserves,
  hard filters and quota checks still take precedence. Ignored hints record
  their rejection rather than causing a refusal.
- Include present hints in admission idempotency while preserving omitted
  request bytes and fingerprints. Runners send the hint only to routers
  declaring `account_warm_credential`; older routers refuse the unknown member.

## 0.7.16

- Accept `subc-protocol` 0.34.1 through 0.36.x, so a module on the 0.36 line
  links one copy; no API or behaviour change.

## 0.7.15

- Add `gateway` as the fifth shared authentication spelling. One vault
  credential supplies the endpoint and gateway credential; the gateway keeps
  the upstream provider key. Gateway pairs normally do not require catalog
  `wire.api`. The vault and runner permit HTTPS, or HTTP only to loopback.
- Readers first for meaning: adopt routing 0.7.15 and gateway credential matching
  before a runner declares a `gateway` pair. Older parsers keep the declaration:
  `RunnerPair.auth_method` is a string and `validate_runner` checks token shape
  only (`src/declarations.rs`). Exact matching can already select a credential
  reported as `gateway`, but not one reported with another or no method;
  decoding alone does not derive `gateway` from an `llm-gateway` credential.
  Validation remains open-token and this release is non-breaking.

## 0.7.14

- Add optional per-pair `requires_wire_api`: a pair's value wins over the
  runner-level value, which still defaults to false. Apply the effective value
  to select, panels, head choices and the `runner_requires_wire_api` explanation.
  Harness limits only narrow pairs; a requirement written there has no effect.
- Readers first: a 0.7.13 or older reader refuses a pair carrying
  `requires_wire_api` and drops the whole runner declaration. No runner declares
  the member on a pair until routing runs 0.7.14 live.

## 0.7.13

- Add runner-level `requires_wire_api`, defaulting to false, with a builder,
  getter and `admits_row` helper. Routing must exclude rows without a served
  `wire.api` at pair matching for select, panels and head choices when true.
  Non-boolean, non-null values refuse the declaration by field path.
- Readers first: runners declare the member only after routers run 0.7.13;
  older parsers refuse unknown members and drop the runner.
- Clarify that account quota and credit walls go through
  `route.account_observe`, not `route.report`, which has no `limit_window`.

## 0.7.12

- `parse_engine_declarations` keys `duplicate_engine` by `(kind, provider, model)`,
  matching `cortexkit-role-media` 0.1.4: one media provider may declare the same
  kind and model served through two providers (a vendor and a gateway). A
  repeated full identity is still refused. Requires media 0.1.4.
- Readers first: a router before 0.7.12 refuses such a declaration as a whole,
  so a media provider must not declare a shared `(kind, model)` until its router
  runs 0.7.12.

## 0.7.11

- Add narrow-only per-harness pairs and provider allow-lists. Keep the member
  raw with typed accessors; invalid restrictions warn and fall back to
  runner-level values for every harness without discarding the declaration.
- Add optional `harness` to head choices; selection resolves the session
  harness before applying restrictions. Omitted head-choice harness keeps
  the runner-level union.
- Deploy readers first: routing must run 0.7.11 or later before runners
  declare `harness_limits`. Parsers in 0.7.10 and earlier reject the unknown
  member and discard the whole runner declaration.

## 0.7.10

- Define observation `limit_window: "credit"` for a subscription account's
  spending or credit wall on a still-valid credential: a non-quota fence for at most
  one hour, then one probe, never lifted by quota readings. Deploy readers
  before runners send it.
- Keep observation reset evidence optional and continuation quota-only.
  Pin credit observations without reset fields with unit coverage and a vector.

## 0.7.9

- Read only `engines` from the `media-provider/v1` catalog slot and ignore
  every other member. A provider can carry describe's group facts in that
  slot without its engines being dropped. `engines` stays required, and each
  engine is still validated strictly.
- Contract: the media slot is an object whose required `engines` member lists
  the engines; readers use `engines` and ignore any other member.

## 0.7.8

- Type panel shortfall evidence, candidate rejection reasons, optional reset
  times, and package-fit calculations without changing the existing JSON detail
  slot. Keep unknown evidence fields and reason strings forward-compatible,
  including prose package detail from older providers.
- Add optional replacement detail only for unseated causes; reject seated
  replacements carrying detail. Publish typed, legacy-prose, and replacement
  evidence vectors.

## 0.7.7

- Add opt-in `engine_reports` and `engine_evals`, requiring `select_engine`,
  for attested engine owners to report job speed and reference-set accuracy.
  Pin score validation, comparison-set selection, exact model identity, and
  metric defaults and overrides; speed never ranks engines.
- Clarify that account admission state is an open string for logging, not an
  authorization or success discriminator.

## 0.7.6

- Add the pure `refusals::retry_class` helper and non-exhaustive `RetryClass`.
  Share the timed-code logic with `Refusal::wait_until_ms`: every timed
  refusal is `RetryAt`, including `no_route` and `account_pool_exhausted`.
  Untimed account selection unavailability is transient; other untimed
  refusals, including unknown codes and malformed timestamps, are permanent.

## 0.7.5

- Recognise `call.yield/v1`, the tool-provider tag for a tool that can hand
  back a waiting call (tool-provider 0.15.3), as a defined unprefixed
  capability name, so routing accepts it wherever it accepts the other
  defined tool capabilities. No other change.

## 0.7.4

- Add optional `route.select` requirements `questions`, `input_tokens` and
  `max_cost_usd`, advertised individually through `requirement_fields` and
  usable by any consumer. Question limits and estimated input-price caps
  constrain eligibility; a cap excludes unstated prices and requires a
  positive token estimate. Without a cap, zero means no estimate.
- Keep unknown requirements flattened for refusal by name, and keep legacy
  describe replies valid without the new requirement members.
- Clarify that `context_window` is the largest single request, image
  modalities cover images, non-iq-ranked consumers send required `iq: 0`,
  and a pin without fallback chooses only an account for that model.

## 0.7.3

- Add the opt-in `select_engine` group, media engine declarations, strict
  selection requests and answers, and media matching helpers. Pin selection's
  filters, ranking, decision ownership and attested media-provider admissions.
- Add `no_engine` and its failed-filter detail and refusal vector.

- Contract text: §3 says which scope a module selects under when it selects
  for someone else's work. It's the scope of the session or run that will
  spend the decision, created first when it doesn't exist yet, or a scope
  made for that one operation where no session or run will exist. The old
  wording, "the project's scope for work on a project", could be read as
  asking for a scope that names only a project, which isn't a work scope.
  No wire change.

## 0.7.2

- Add `observe_credential_refused` (requires `observe_backoff`) and the
  `credential_refused` wall kind for credential-wide provider refusals, with
  admission or credential binding, no caller retry time, and both authorization
  arms. Undeclared kinds stay advisory. Describe checks name a missing prerequisite
  with `DescribeProblem::MissingCapabilityDependency { group, requires }`.
  Pin runner reporting to structured provider errors before admission release;
  document bounded re-probes without treating credit as a resettable limit.

- Contract §5.4: `hold_when_reset_within` no longer has a 900-second default.
  Unset, a blocked entry is held whenever its known reset fits within the
  hold limit, `min(max_hold, hold_wait_max_s)`; an operator may still set it
  to hold only nearer resets. `max_hold` is a per-provider safety ceiling
  whose default each router documents; the contract fixes no number.
  §5.1: a runner that knows when its prompt cache expires sends at most the
  time left until then as `hold_wait_max_s`, and `0` with no accepted
  response on that account. An omitted `hold_wait_max_s` on
  `route.account_for` still defaults to 900 seconds. Text only; no API
  change.
- Contract §9.1: `route.account_for`'s `provider` and `model` are the route
  as routing names it, which for a select answer is exactly its `route`, so a
  key-pool call names routing's pool owner, not the vendor in
  `runner.model`. The old wording, "the call about to be made", left that
  open. Text only; no API change.

## 0.7.1

- Add optional run-attributed report usage with `report_usage_run_id`, preserving
  the first report for each decision/run pair without changing legacy reports.
  Among a decision's reports that carry usage, either all carry a run id or
  none do; mixing them is `report_conflict`. Outcome-only reports without
  usage never conflict with run-attributed ones.
- Add optional pin-only and billing metadata to head choices.
- Add the `consumer_models` read for consumer-specific stored scores, exclusions
  and billing, with strict requests, answer checks and shipped vectors.
- Fix: a select without `service_tier` sets no cap, so a tier chosen by the
  router's tier settings passes `select::check_answer` and the panel checks.
  0.7.0 treated the absence as standard and refused a settings-chosen `fast`.
  A requested tier still caps the answer, unknown tiers stay refused, and
  `route.account_for` without a tier still means standard.

- Contract text only (§8): a decision owner's report `usage`, when sent, is
  the only per-decision token and timing total, so token profiles are built
  from it, and a decision without it is unrecorded, not zero. Token totals
  never establish exhaustion or resets. Runners' live step hints may feed a
  router's own statistics where a missed hint only lowers precision, but
  never replace report usage.

## 0.7.0

- Breaking: `route.account_observe`'s `upstream_origin` and
  `resets_at_source` are now closed enums, `observe::UpstreamOrigin` and
  `observe::ResetsAtSource`, and the parser refuses a value outside the enum as
  `invalid_request` naming the member. Before 0.7.0 this crate decoded any
  string there while routers acted on a fixed set of values the contract
  never listed, and the published samples sent a URL as the origin. The contract now gives each value's meaning and how a router
  uses it, and the samples use real values. New helpers:
  `UpstreamOrigin::is_genuine` and `ResetsAtSource::is_stated_reset`.
- Breaking: a report's `eligibility.scope` is now a closed enum. A scope the
  contract doesn't name is `invalid_request {field: "eligibility"}`, where
  0.6.0 kept it as a string the contract gave no meaning.
- With `report_outcomes`, a report may carry `refusal_class`
  (`report::RefusalClass`): `credential_refused` or `request_too_large`,
  mapped by the runner from its provider's wording, so routers act on a
  typed member instead of matching vendor text in `runner_code`.
- Breaking: `ContinueRequest.mode` is now `account_continue::ContinueMode`, a
  one-value closed enum. The wire is unchanged: any mode other than
  `continue` was already refused as `invalid_request {field: "mode"}`.
- The contract no longer mentions a "used-account report": an observation's
  `pool` names the pool it is about, and no observation kind of that name
  exists.
- Register the five optional operations and all eleven new groups. Reject
  operation/group mismatches with a named `DescribeProblem` while retaining the
  required-operation baseline and 0.6.0 describe compatibility.
- Publish operation tables, settings and runner-owned pooled-account rules,
  and one vector file per new group with compiled-in bytes and checksums.

- Add panel selection and replacement wire types, strict request parsers,
  per-seat request derivation and caller-held replacement history. Pure panel
  checks enforce slot policy, temperatures, diversity, fresh routes and decision
  ids, window floors and package ceilings using `budget::quote`.
- Add budget-quote, explain, account-status and head-choices wire types and
  pure answer checks. Share saturating package/output-limit arithmetic in
  `budget::quote`, using retained selection capacities and deterministic
  limiting-seat ties. Preserve unknown budget statuses/reasons, explanation
  reason codes and account-limit states as strings.
- Add group-gated select members `pin_slot`, `available_routes`,
  `prefer_prompt_cache` and `settings`, plus a `project_settings` answer result
  that retains unknown strings. Enforce route-list membership, project-settings
  presence and pin outcomes for slots while preserving legacy bytes when
  additions are absent.
- Add `SelectSettings::check_project_stamp` for routers to reject a settings
  project id different from the route's trusted project stamp.
- Add `report_outcomes` decoding for `contract_error`, `session_error`,
  `interrupted` and `launch_failed`, with a shared availability/affinity effect
  matrix. Without the group, these outcomes keep their legacy failure effect.
- Add `observe_backoff` parsing for account-local `transient_backoff` evidence
  and optional `decision_id`, `retry_at_ms` and verbatim `provider_code`.
  Undeclared members are refused by name; absent members preserve old bytes.
- Add `ObserveArm` and `check_observe_authorization`: decision owners may send
  `account_limit_exhausted` or `transient_backoff` only on an admission carrying
  their decision. Authorized runners may still release the account pin when
  `account_continue` switches reporting to an offer's decision id.

## 0.6.0

- Rust API break: `AccountForAnswer` and `AccountPin` are now
  non-exhaustive. Construct both types with `new` instead of struct literals
  so future optional fields can be additive. `with_service_tier_chosen` is
  available on `AccountForAnswer` only.

- Extend `route.account_for` under `service_tier` with optional request
  `service_tier` and answer `service_tier_chosen`, using selection's existing
  plan eligibility and ordered fallback. Legacy requests and replies are
  unchanged when these members are absent.
- A decision-bound account answer records the decision's non-standard chosen
  tier even when the request does not repeat `service_tier`.

## 0.5.7

- Add the opt-in `account_continue` group for switching a run durably paused
  on an account limit to a compatible account. Separate request and tagged
  offer, hold and refuse reply types preserve the legacy account API.
- Pin continuation eligibility, reservation retries, expiry and refusal rules,
  with shared wire samples and validation of limit evidence.
- Refuse changed parsed questions under a reused selection id without replacing
  the first answer. Clarify later switches, reservation use through reports,
  scope equality and pause-local refusals; include a second-switch sample.

## 0.5.6

- Requires subc-protocol 0.34 and tool-provider 0.14 for development checks
  only; no API or wire change.

## 0.5.5

- Add the service-tier capability group, plan-eligible selection with ordered
  fallback, and the chosen level on tier-aware decisions.
- Accept applied and served tier metadata in strict report usage. Callers send
  these new members only after discovery declares the group, protecting older
  readers. All other unknown usage members remain refused.
  A reported wire is a runner discovery entry with `served: "reported"`, permitting
  metadata only when that response states it (runner CONTRACT §9.3.2). Prefer a
  provider-stated served tier, with price unconfirmed if its tier or rate is
  unrecognised. Without served, applied is only an unconfirmed estimate, defaulting
  to standard when absent. Keep unfamiliar strings; never invent a rate.
- Keep a local request-level enum with dev-only drift checks against runner
  0.7.36. Record tier strings preserve unknown levels; retain old absent-field bytes.
- The answer checker is stricter than record decoding: `AnswerProblem::ServiceTier`
  also rejects an unrecognised chosen tier, besides missing choices or upgrades.

## 0.5.4

- Give runner pair values one shared spelling each (CONTRACT §1.2), so a
  runner's declared pairs can be matched against the catalog's routes and
  the vault's credentials. Seven `wire_api` tokens
  name wire formats (`anthropic.messages`, `openai.responses`,
  `openai.chat_completions`, `openai.chatgpt_responses`,
  `google.generate_content`, `google.code_assist`, `aws.bedrock_converse`),
  and four `auth_method` tokens are the vault's own credential kinds
  (`apikey`, `oauth`, `chatgpt`, `antigravity`). The model catalog serves a
  route's `wire_api` as a per-route `wire.family` fact; a route without it
  matches no runner pair. The provider is never part of an `auth_method`. Validation is
  unchanged: any token of the right shape still decodes. Add
  `spellings::WIRE_APIS` and `spellings::AUTH_METHODS`, with a test that keeps
  them equal to the contract's tables.

## 0.5.3

- Add optional `SelectRequest::task_ref` and `with_task_ref` for decision
  attribution and per-task cost profiles. The opaque reference is 1–128 bytes,
  each printable ASCII `0x21`–`0x7E` (no spaces, controls or non-ASCII).
  Callers send it only to a provider declaring the new `select_task_ref`
  group (`capabilities::SELECT_TASK_REF`); invalid values and use without
  the group are `invalid_request` naming `task_ref`. It never affects selection:
  the same select without it answers exactly the same route. It is stored and
  retained with the decision, never returned in answers or to other callers;
  reports already name their decision. Omitted requests keep their wire bytes,
  and `correlation_id` remains log-only. Add request/refusal vectors and checksums.

## 0.5.2

- Declare `account_selection_disabled` for `route.account_for`: account
  selection is off on the router, module-wide rather than per provider or
  per pool, so the caller uses its own credential. The refusal is untimed
  (no `reset_at_ms`), not exhaustion or a hold, and must not cause a retry
  against another provider. Add the `RefusalCode::AccountSelectionDisabled`
  variant, a refusal vector and checksum, and coverage that it is a known
  code and never timed, even if a peer sends timing fields.

## 0.5.1

- Add the optional `select_exclude_providers` group: `exclude_providers`
  lists provider ids, each the provider part of a route key, and no answer
  names a route from a listed provider. `check_answer` refuses one that does
  (`AnswerProblem::ExcludedProvider`). When every adequate route is from a
  listed provider the select is the same `no_route` an `exclude` listing
  those routes gets, and a pin on a listed provider is `pin_unavailable`
  reason `excluded`. Entries use the route-key provider validator (new
  `select::is_provider_id`, which `is_route_key` now uses) and the list rules
  of `exclude`: no length limit, repeats not refused. A bad entry, and the
  member sent to a provider without the group, are `invalid_request` naming
  `exclude_providers`. `exclude` keeps its meaning. Build with
  `SelectRequest::with_exclude_providers`; read with `excludes_provider`.
  New vector file `route-select-exclude-providers.json`, pinned in
  `SHA256SUMS`. Additive: `SelectRequest` and `AnswerProblem` were already
  `#[non_exhaustive]`.

## 0.5.0

- Add `pin_fallback: "route"` beside a pin: when the pin cannot be served
  the provider routes normally instead of refusing `pin_unavailable`. The
  answer carries `pin_outcome: "pinned" | "fell_back"` whenever the
  request pinned, and on `fell_back` its `detail` carries `pin_reason` naming
  the pin refusal's reason. Any other `pin_fallback` value, or a fallback
  without a pin, is `invalid_request` naming the field. Without the field
  pins still refuse and a second select for the same work stays forbidden.
- Add a fenced reset on `route.report`: a report with `outcome: failed` and
  a future `reset_at_ms` fences only that decision's route on that decision's
  account until the reset. A reset further than 24 hours away is capped at
  24 hours; a reset already in the past is recorded as a failure with no
  fence. Only the decision's own consumer may report, as before.
- Say that `account_observe {kind: run_finished}` releases the runner's
  account pin, with `lease_valid_until_ms` only the backstop.
- Add the optional `select_prefer` group: `prefer` lists up to 16 route keys
  with no duplicates, each `provider/model`. Among the routes the provider
  would already answer it answers a listed adequate one in list order, never
  admitting a route the request would not otherwise get. A provider without
  the group refuses `prefer` by name. `role.describe` lists it in
  `capabilities` like the other groups.
- Replace the report `usage` with token counts and timing that match the
  runner's usage names: `input_tokens`, `cached_input_tokens`,
  `cache_write_tokens`, `output_tokens`, `reasoning_tokens`, `duration_ms`
  and `first_token_ms`, plus the existing `cost_usd` claim. Input never
  includes cache reads or writes. Every count is a non-negative integer up
  to 2^53-1; `cost_usd` stays an optional non-negative finite number, the
  caller's own claim the provider may ignore. Unknown members are refused,
  and `usage` is part of the report body for idempotency.
- Breaking Rust minor: `SelectAnswer` and `Usage` were exhaustive and are now
  `#[non_exhaustive]`. Build answers with `SelectAnswer::new` and its
  `with_*` setters, and usage with `Usage::new` and its `with_*` setters
  (including `with_cost_usd`). New vectors and checksums cover the new members
  and the new prefer file.

## 0.4.3

- Contract text only, with no code change: §9.1 defines when the parent
  conversation is on an account, for the parent-account preference. It is
  on the account its conversation key's latest `admit`, `renew` or
  `reassign` answered, whether or not the call has started, until the pin
  lapses (`admission_valid_until_ms` if never started,
  `lease_valid_until_ms` once started).

## 0.4.2

- Contract text only, with no code change: operator configuration may name
  an owner for a consumer name found only in configuration. With an owner,
  the two ownership arms of §1.1 apply to it, and every other caller is
  refused `consumer_not_owned`. With no owner, the name stays usable by any
  caller, as before. A verified declaration still decides ownership.

## 0.4.1

- Add optional runner-wide `harnesses` (1–16 unique pair-grammar tokens),
  `default_harness` (must be listed), and `starts_sessions` (defaults true,
  omitted when true). Add builders and getters without making the declaration
  exhaustive. Existing slots keep their exact 0.4.0 bytes; field-path refusals
  and unknown-member rejection remain all-or-nothing.
- Add `select_harness`: a caller may request a program, not a wire/auth pair.
  Match only a runner listing it and echo it; no hosting runner is `no_route`
  reason `harness_unavailable`, naming the requested program in the message.
  Without a request use the declared default, else singleton, else omit the
  answer's harness without filtering candidates. Non-starting runners never
  answer select; all other operations retain their rules.
- Add declaration and select vectors with refreshed checksums. Rollout is
  readers first: 0.4.0 providers refuse these declaration members, so runners
  wait until their provider serves 0.4.1. Callers send `harness` only when the
  group is declared; other providers refuse it by name.

## 0.4.0

- Add optional catalog `runner` declarations: unique open-set wire API/auth
  pairs, the promise to call `route.account_for` and use its answer,
  validated tool capability names, and optional standing provider limits.
  Pair, capability and provider lists are bounded at 64 entries; pairs and
  explicit provider lists must be non-empty. Unknown members and malformed
  values refuse the whole slot with a field path. Tool capability validation
  stays local so role crates remain independent at runtime. Development-only
  tests check that accepted names and grammar stay aligned with the
  tool-provider role.
- Make `consumers` optional (empty when omitted), allowing pure runner slots.
  Keep the `ConsumerDeclarations` name and its `consumers` vector, but make
  it non-exhaustive: use `new`, `with_consumers` and `with_runner` instead
  of struct literals. This is a breaking Rust minor. Runner declaration
  types are also non-exhaustive. Legacy consumers-only slots, including an
  explicit empty list, keep their bytes; absent optional members are omitted,
  and `standing_limits: {}` retains its empty object.
- Pin readers-first rollout: runner modules must not declare `runner` until
  the routing provider adopts 0.4.0. A 0.3.x provider rejects the whole slot.

## 0.3.1

- Contract text only: in the hold rules (5.4), a per-token account counts among an entry's accounts only where the provider's account policy allows falling back to one. No code or wire change.

## 0.3.0

- Add the optional `select_hold_control` group. `role.describe` gains
  `capabilities`, the optional groups a provider serves (left out when
  empty). A provider declaring the group reads two new optional `route.select`
  members: `hold_ok` (default `true`) and `hold_wait_max_s`, the caller's own
  cap on a hold (`0` means no hold). Both are valid together: the fallback a
  `hold_ok: false` request can take may still hold, and the cap applies to
  it. A provider without the group refuses either member as
  `invalid_request {field}`, `hold_ok` first. A caller sends them only to a
  provider declaring the group. `parse_select_request_serving` takes the
  declared groups; `parse_select_request` declares none.
- Pin how a provider declaring the group decides a ladder entry whose
  eligible accounts are all blocked (contract section 5.4). Select never
  waits: it answers the entry, so the runner holds at `route.account_for`, or
  it moves to the next entry. The checks run in order: `hold_ok: false`, an
  unknown reset or one beyond the operator's `hold_when_reset_within`, then a
  wait beyond the hold limit (the smaller of `max_hold` and the request's
  cap). With `hold_ok: false` and no entry usable now, the provider answers
  the entry whose reset is soonest within the limit. When none is within the
  limit, it refuses `no_route` with the new reason `accounts_blocked` and the
  earliest known reset. A demand-routed consumer skips a blocked model. The
  decision records the caller's cap, and `route.account_for` naming it holds
  only within the smallest of `max_hold`, that cap and its own.
- Add optional `runner.harness` to the select answer: the harness the
  runner hosts the route on, filled only from the runner's own declaration.
  A caller never binds a runner session on an answer without it.
- Breaking Rust minor: `SelectRequest`, `RoleDescribe` and `RunnerRef` are
  now `#[non_exhaustive]`. Build them with `SelectRequest::new`,
  `RoleDescribe::new` and `RunnerRef::new` and their `with_*` setters;
  every optional select member now has a setter. Absent members are left
  out, so earlier requests, answers and describes serialize byte for byte as
  before. The original vectors are unchanged. `route-select-hold-control.json`
  is new, with its `SHA256SUMS` line.
## 0.2.0

- Add optional `parent_conversation_key` and quota `pool` to account
  allocation, `pool` to flat readings and used-account observations, and
  `pool`, `eligibility` and `request_input_tokens` to outcome reports.
- Pin parent preference/fallback, opaque pool grammar and unseen-pool
  acceptance, disabled-window omission, eligibility without exhaustion or
  holds, size-class fencing, and provider-first rollout. Eligibility with
  an exhaustion code or reset time is `invalid_request {field: "eligibility"}`.
  Eligibility scope follows the existing open-string enum convention.
- Breaking Rust minor: the three extended request structs were exhaustive in
  0.1.1. `AccountForRequest`, `AccountObserveRequest` and `ReportRequest` are
  now `#[non_exhaustive]` so future optional fields can be additive. Build
  requests with constructors and `with_*` setters; every optional member has
  a setter. Absent members preserve the original bytes;
  original vectors are unchanged, with new vectors and checksum pins alongside.

## 0.1.1

- Add catalog `ConsumerDeclarations`, optional defaults using selection
  demand types except `iq`, a typed all-or-nothing validator, `ROLE_DECLARATIONS_KEY`,
  and pinned declaration vectors including a byte-exact round trip.
- Define verified catalog ownership, conflict health reporting, admission
  by either attested caller or daemon-stamped scope owner, operator/request/
  declaration precedence, and rollout for configuration-only names.
- Add terminal `consumer_not_owned` and its typed detail. This is a patch:
  the refusal enum was already non-exhaustive and preserves unknown codes;
  existing request types and required `iq` validation are unchanged.
- Declarations cannot set `iq`; it is refused as an unknown member. The
  request supplies its required `iq`, with operator configuration the only
  override. Include an invalid-`iq` declaration vector.

## 0.1.0

- First release of the `routing/v1` wire contract: `role.describe`,
  `route.select`, `route.report`, `route.account_for` and
  `route.account_observe`, with request validators, refusal codes and their
  details, the `{"result": <answer>}` reply envelope, `CONTRACT.md`, and the
  `routing-v1` vectors behind the `test-vectors` feature.
