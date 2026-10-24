# `routing/v1`: role contract

Stability: **alpha**. This document is the role definition; the Rust types
in this crate are its wire shapes, `test-vectors/routing-v1/` holds its
vectors (they ship in the package, behind the `test-vectors` feature, with
a `SHA256SUMS` pinning every file), and the sibling
`cortexkit-role-routing-conformance` crate holds its kit (§12).

Every rule is marked **[pinned]**: the role states it. A provider or a
consumer must do it, and the other side may rely on it. Text marked
**[informative]** is guidance and adds no obligation.

A routing provider decides which model a piece of model work runs on, and
on which runner. A consumer states requirements; **what `route.select`
answers is exactly what runs**. Nothing here names a particular
implementation: "a routing provider", "a runner" and "a consumer" are roles
a module plays.

## 1. Role identity and discovery

- [pinned] A routing provider lists `routing/v1` (`PROVIDES`) in its
  manifest's `capabilities.provides`. It is a valid capability identifier
  under the capability grammar of the subc daemon (the CortexKit module
  router, in the public `subconscious` repository).
- [pinned] The provider serves `role.describe`, `route.select`,
  `route.report`, `route.account_for` and `route.account_observe`
  (`REQUIRED_OPS`), each a request `{method, params}`. A provider may serve
  older ops beside them while its consumers move.
- [pinned] A consumer finds the provider from the catalog. When the catalog
  lists none, the consumer refuses its own work as `routing_unavailable`
  and never picks a model itself. When it lists two or more, the consumer
  refuses its work as `routing_ambiguous {modules}`, naming them, and never
  picks one. These two codes are the consumer's own; a provider never
  answers them.
- [pinned] The consumer sends its work to the runner the answer names, with
  the model the answer names. It never selects a second time for the same
  work, and never substitutes another runner or model.
- [pinned] Routing grants nothing. An answer names where to run; the
  consumer opens its own route to that runner under its own principal and
  scope, and the runner's own admission applies. A routing provider never
  relays model calls.

### 1.1 Consumer declarations

- [pinned] A module may declare its consumers in its manifest's daemon slot
  `role_declarations["routing/v1"]` (`ROLE_DECLARATIONS_KEY`). The daemon
  relays this JSON object unread; this role owns its schema
  (`ConsumerDeclarations`, validated by `parse_consumer_declarations`).
  Declarations come only from the daemon catalog, never from a call.
- [pinned] The object is `{consumers: [{name, eq?, context_window?,
  max_output_tokens?, tools?, tool_capabilities?, modalities?,
  structured_output?}]}`. `name` is required and uses the consumer id
  grammar of §5.1. Every other member is an optional default for the
  same-named requirement in §5.1, with that requirement's type and limits.
  Unknown members, invalid names or demands, and duplicate names within
  one declaration invalidate the whole declaration; the provider applies
  none of it and reports the problem on its health.
- [pinned] A declared name belongs to the module whose manifest declares
  it, keyed by that module's attested identity in the catalog, not by a
  declaration body member. A provider trusts ownership only for catalog
  entries the daemon nonce-verified. It ignores unverified entries for
  both ownership and defaults and reports them on its health; an
  unverified entry never reserves or steals a configured name.
- [pinned] If two verified modules declare the same name, the provider
  accepts neither owner's requests for it, refuses all selects for that
  name as `consumer_not_owned {consumer}`, and reports the conflict on its
  health. Operator configuration cannot bypass ownership or a conflict.
- [pinned] A select naming a declared consumer is admitted when either the
  caller's attested principal is the declaring module, or the request's
  daemon-stamped scope is owned by the declaring module. Scope ownership
  comes from the bind stamp's `owner` in `(owner, ref, epoch)`, never from
  request params; `ref` and `epoch` identify the scope, not its owner. A
  third-party caller carrying the declaring module's scope is admitted;
  a module carrying only its own scope cannot borrow another's consumer.
  Every other caller is refused `consumer_not_owned {consumer}`. This refusal
  is terminal, never timed. A name found only in operator configuration
  remains usable by any caller. Until a module declares, the provider
  keeps its configuration names.
- [pinned] For each demand field, the first value present wins: operator
  configuration, then the request's own requirement, then the declared
  default. Omission is not a zero or false value. The request's required
  `iq` remains required; a declaration cannot set `iq` and does not relax
  request validation. An operator may still override `iq` in configuration.

## 2. Wire

- [pinned] Every reply is a `RESPONSE` frame whose body is exactly
  `{"result": <answer>}` (`wire::reply_body`). A reader refuses a bare
  answer, and a body with any other top-level key, by name
  (`wire::read_reply`, `ReplyShapeError`).
- [pinned] A refusal is an `ERROR` frame whose body is
  `{code, message, detail}` (`errors::Refusal`). `detail` is always an
  object; every refusal field other than `code` and `message` lives in it.
  A reader decodes a `detail` sent as `null`, or left out, as an empty
  object.
- [pinned] Refusal codes are an open set (`RefusalCode`, non-exhaustive). A
  reader keeps an unknown code and treats it as a refusal that is not
  timed, whatever its detail carries.
- [pinned] Every other enumeration a reader decodes is an open set too: a
  report's `outcome`, `pin_unavailable`'s and `no_route`'s `reason`, an
  observation's `kind`, and an account answer's `reason.code`. An unknown
  value decodes and is kept as the string it was sent as.
- [pinned] An optional member is left out when it has no value. A member
  sent as `null` reads exactly as one left out.

## 3. Scope comes from the route

- [pinned] `route.select` applies the scope stamp of the route it arrives
  on (project id, workdir, session, run), never a body member. A body
  member naming a project, workdir, session or run is never read as scope,
  whether the provider ignores it or refuses it.
- [pinned] A module that selects for work it runs on someone else's behalf
  selects on a route bound to that work's scope: the run's scope for work
  inside a run, the project's scope for work on a project.
- [pinned] A select on an unscoped route is attributed to the calling module
  itself.
- [pinned] Per-project pins and cost attribution follow the scope of the
  route, so they follow the work, not the module that asked.
- [pinned] The caller of every op is the route's principal, never a body
  member.

## 4. `role.describe`

- [pinned] Params are `{}`. The answer (`describe::RoleDescribe`) is
  `{role: "routing", version: 1, ops, consumers: [{id, priced_only}],
  requirement_fields, detail_supported}`.
- [pinned] `ops` lists every op the provider serves, at least
  `REQUIRED_OPS`.
- [pinned] `consumers` lists every consumer id in the provider's
  configuration or accepted declarations, each once; conflicted names are
  not listed.
- [pinned] `priced_only: true` states that the consumer is answered only
  with routes priced per token, never with a route paid from subscription
  quota. A consumer reads the policy here instead of knowing it.
- [pinned] `requirement_fields` lists every requirement member the provider
  reads. A `routing/v1` provider reads every member §5 defines
  (`select::REQUIREMENT_FIELDS`), so it lists all of them.
- [pinned] `detail_supported` states whether a `detail: true` select is
  answered with the provider's explanation object.
- [pinned] A consumer checks the answer with `describe::check_describe`
  before routing anything to the provider.

## 5. `route.select`

### 5.1 Request

`select::SelectRequest`, validated by `select::parse_select_request`.

- [pinned] `consumer` (required): an id the provider knows from its
  configuration or accepted declarations (§1.1). Unknown:
  `unknown_consumer {consumer}`. Operator include and exclude pins attach
  to consumer ids. A declared name used without either ownership arm is
  `consumer_not_owned {consumer}`.
- [pinned] A consumer id is a non-empty string, with no character-set or
  length restriction in this version.
- [pinned] `requirements` (required, an object):
  - `iq` (required, 0 to 100) and `eq` (optional, 0 to 100): task demand,
    which the provider maps onto its own model facts.
  - `context_window` and `max_output_tokens`: floors, in tokens.
  - `tools`: `"none"` (only runner and model pairs that run with no tools
    offered) or `"required"` (only routes that accept tool calls). Absent
    means no constraint.
  - `tool_capabilities`: tool capability tags, such as `"code.read/v1"`,
    the work's plan lists. Only runners that can reach a tool provider
    serving every one of those tags for the scoped project are answered.
    These are tags, never module names: any tool provider serving the same
    tags can stand in, so routing never keys on a module name. The
    runner's own admission still refuses a send it cannot serve
    (`tool_unavailable`), as a backstop.
  - `modalities`: input modalities the work sends (`image`, `pdf`,
    `audio`).
  - `structured_output`: bool.
- [pinned] A requirement member the provider does not read is refused
  `invalid_requirements {field}` naming it as sent; it is never ignored. A
  malformed requirement (a missing `iq`, a demand above 100, a `tools` or
  modality value this version does not define) is refused the same way,
  naming it.
- [pinned] `pin`: `"provider/model[:variant]"`. The provider is everything
  before the first `/`; a variant is what follows the last `:` after it. A
  pin that cannot be served is `pin_unavailable`; a malformed one is
  `pin_unavailable` with reason `invalid`.
- [pinned] `exclude`: route keys the caller will not take. A route key is
  `provider/model`; the variant and the runner are separate from it.
  Excluding a route key excludes every variant of it. An entry that is not
  a route key is `invalid_request {field: "exclude"}`.
- [pinned] `runners`: runner module ids the caller can send to. Absent: any
  runner the provider knows.
- [pinned] `affinity`: an opaque key of at most 128 characters (§7).
- [pinned] `correlation_id`: opaque, at most 128 characters, echoed in logs
  only.
- [pinned] `concurrency_slots`: an optional hint,
  `[{key, running, limit, queued?}]`.
- [pinned] `detail`: bool; asks for the provider's explanation object.
- [pinned] Any other malformed member is `invalid_request {field}` naming
  it.
- [informative] Providers and consumers may share a tier guide that places
  demand at 30, 50, 70, 85 and 100. It is guidance only; the provider's
  own mapping decides.

### 5.2 Answer

`select::SelectAnswer`:

```
{
  decision_id,
  route: {provider, model, variant?},
  runner: {module, model: {provider, model, variant?}, auth?: {pool?, credential_id?}},
  context_window, max_output_tokens,   // tokens, from the provider's own catalog read
  expires_at_ms,
  explanation,                         // one sentence, at most 200 characters
  detail?                              // the explanation object, when asked for and supported
}
```

- [pinned] `runner.module` is one of the request's `runners` when the
  request sent them, and the route is never one the request excluded
  (`select::check_answer`).
- [pinned] `runner.model` names the model as the runner names it.
- [pinned] `auth` is present only when the provider's policy names a key
  pool. It names the key by public id only, never secret material. The
  runner sends with that key, still under its own admission check, and a
  runner refuses a key-pool send without it.
- [pinned] `context_window` and `max_output_tokens` are the answered model's
  figures from the provider's own catalog read, so a consumer sizes its
  work without a second lookup.
- [pinned] `expires_at_ms` is in the future when the answer is given (§6).
- [pinned] `explanation` is one sentence of at most 200 characters.
- [pinned] `detail` is non-normative; a consumer never acts on it.

### 5.3 Refusals

- [pinned] `invalid_request {field}`, `invalid_requirements {field}`,
  `unknown_consumer {consumer}`, `consumer_not_owned {consumer}` (terminal,
  never timed).
- [pinned] `pin_unavailable {pin, reason, reset_at_ms?}`. `reason` is an
  open set: `invalid`, `unknown_provider`, `unknown_model`,
  `invalid_variant`, `excluded`, `not_available`, `credential_unentitled`,
  `credential_billing_exhausted`, `cooldown`, `quota_exhausted`, `backoff`.
- [pinned] `no_route {reason, reset_at_ms?}`. `reason` is an open set:
  `no_available_model`, `missing_capability`, `context_too_small`,
  `quota_or_cooldown`, `all_inadequate`.
- [pinned] **A timed cause carries `reset_at_ms`**: a cooldown, a backoff, or
  exhausted quota whose reset is known. A consumer waits until then and
  asks again, rather than failing its work. A refusal without `reset_at_ms`
  is not timed. `reset_at_ms` alone decides whether a consumer waits.

## 6. Decisions

- [pinned] A decision is bound to the selecting `(principal, consumer)`.
  Only that caller may report it; any other caller gets
  `unknown_decision`, answered exactly as for a decision that never
  existed, so it never reveals that the decision exists.
- [pinned] After `expires_at_ms` a report on the decision is still
  accepted, but running on the decision is the caller's risk and it is not
  current routing.

## 7. Affinity

- [pinned] `affinity` is a preference keyed by `(principal, consumer,
  affinity)`. The previous decision for a key is the last decision answered
  for it, pinned or not.
- [pinned] While the previous decision's route still meets the request and
  is not fenced (the provider has stopped answering it for now: a cooldown,
  a backoff, exhausted quota, or a failure it recorded), the provider
  answers it again, so successive calls keep a warm prompt cache.
- [pinned] A failure report on that decision (§8) ends the preference.
- [pinned] Affinity is never a hard pin: a route that no longer meets the
  request is not answered because of it.

## 8. `route.report`

`report::ReportRequest`, validated by `report::parse_report_request`:
`{decision_id, outcome, runner_code?, credential_id?, reset_at_ms?, usage?}`.

- [pinned] `outcome` is an open set: `completed`, `refused`, `failed`,
  `timed_out`, `cancelled`. `refused`, `failed` and `timed_out` are
  failures; `completed` and `cancelled` are not. An unknown outcome is kept
  and counts as a failure, so a provider never keeps preferring a route on
  a report it cannot read (`Outcome::is_failure`).
- [pinned] `runner_code`: the runner's typed refusal or error code,
  verbatim, so the provider can tell "this route cannot serve this" from
  "this attempt failed".
- [pinned] `credential_id` and `reset_at_ms`: when the runner reported them.
- [pinned] `usage`: `{input_tokens, output_tokens, cost_usd?}`. Reasoning
  tokens are counted in `output_tokens`. `cost_usd` is the caller's own
  claim: the provider prices usage from its own catalog and may ignore it.
- [pinned] The answer (`report::ReportAnswer`) is
  `{decision_id, report_id, outcome}`: the provider's id for the recorded
  report and the outcome recorded.
- [pinned] A report is idempotent on `decision_id`: the same body again
  returns exactly the first answer; a different body after the first is
  `report_conflict`. Bodies are compared as decoded, so key order, and a
  member sent as `null` instead of left out, never make bodies differ.
- [pinned] Refusals: `invalid_request {field}`, `unknown_decision`,
  `report_conflict`.

## 9. Runner ops

- [pinned] `route.account_for` and `route.account_observe` are called only
  by attested runner modules. A caller is an attested runner module when its
  route carries a principal the daemon verified itself (`reserved:<module>`,
  proven by the launch secret the daemon handed that module), and the
  provider knows that module as a runner. Any other caller, a `direct` client
  included, is refused `account_not_authorized`. They are separate from select, with
  their own refusals, so a consumer that only selects never depends on
  them.
- [pinned] The calling runner comes from the route; neither op carries a
  `caller` member.
- [pinned] Accounts and credentials are named by public id only, never
  secret material.

### 9.1 `route.account_for`

The account a runner uses for a model call on a provider whose accounts
the routing provider allocates (subscription logins).
`account::AccountForRequest`, validated by
`account::parse_account_for_request`:

- [pinned] `provider`, `model` (required, non-empty): the call about to be
  made.
- [pinned] `affinity_model` (optional, defaults to `model`): the model the
  conversation's account affinity is bound to.
- [pinned] `conversation_key` (required, non-empty, at most 256
  characters): the runner's stable conversation id. One key stays on one
  account while that account is healthy, keeping the provider's prompt
  cache warm.
- [pinned] `admission_id` (required, non-empty, at most 128 characters):
  the idempotency key. The same id and body from the same runner returns
  exactly the first answer; a different body is `admission_conflict`.
  Bodies are compared with defaults applied
  (`AccountForRequest::same_body`): a member left out and the same member
  sent with its default value are the same body.
- [pinned] `mode`: `admit` (the default), `renew` (extend the current pin)
  or `reassign` (a different account after the current one failed).
  `renew` and `reassign` require `expected_pin {id, generation}`; without
  it the request is `invalid_request {field: "expected_pin"}`, and a stale
  generation is `pin_generation_conflict`. Any other mode is
  `invalid_request {field: "mode"}`.
- [pinned] `initial_input_bytes` (default 0) and `initial_input_estimated`
  (default false): the input's size, so the allocator avoids an account
  whose window cannot take it.
- [pinned] `candidates` (optional): `{view_digest, accounts: [{account_id,
  credential_id}]}`, the accounts the runner can use now. When present, the
  provider picks only from this list.
- [pinned] `requested_account` (optional): `{account_id?, credential_id?}`,
  a hint from an explicit pin; `account_id` wins when both are present.
- [pinned] `hold_wait_max_s` (optional, default 900): the longest the
  runner waits out a short hold instead of being refused.
- [pinned] `decision_id` (optional): with it, the provider honours its own
  selection instead of deciding twice. It is honoured only from the runner
  module that decision named, and only for that decision's provider and
  model. A `decision_id` that names a different runner, or a different
  provider or model, is refused `unknown_decision`, so it never reveals
  that the decision exists.

The answer (`account::AccountForAnswer`):

```
{
  admission: {id, state},                          // id is the request's admission_id
  account: {provider, account_id, credential_id, method},
  pin: {id, generation, affinity_expires_at_ms, admission_valid_until_ms,
        lease_valid_until_ms, mode},
  reason: {code, decision_id, policy_version, quota_fetched_at_ms?, quota_age_band}
}
```

- [pinned] The runner renews before `lease_valid_until_ms`, and reassigns
  after the account fails.
- [pinned] `reason.decision_id` is the request's `decision_id` when it
  carried one, and otherwise the provider's own id for this allocation.
- [pinned] `reason.code` is an open set, for logs; a reader never acts on
  it. This version names `affinity`, `decision`, `requested_account`,
  `headroom`, `renewed` and `reassigned`.
- [pinned] Refusals: `invalid_request {field}`, `unknown_decision`,
  `account_not_authorized`, `auth_required`, `account_hold {until_ms}`
  (timed), `account_quota_unknown`, `account_pool_exhausted
  {reset_at_ms?}` (timed when the reset is known),
  `account_pool_reserve_limited`, `account_selection_unavailable`
  (transient, not timed), `admission_conflict`, `pin_generation_conflict`.

### 9.2 `route.account_observe`

What a runner saw on an account, so the provider's headroom stays current.
`observe::AccountObserveRequest`, validated by
`observe::parse_account_observe_request`:

- [pinned] `event_id` (required, non-empty): the idempotency key, per
  calling runner. A runner uses a stable id for each event, such as a
  position in its own log.
- [pinned] `kind` (required): an open set, `admission_started`,
  `run_finished`, `attempt`, `scoped_not_found`, `account_limit_exhausted`.
  An unknown kind is kept as advisory.
- [pinned] Optional: `admission_id`, `run_id`, `served_account_id`,
  `credential_id`, `view_digest`, `observed_at_ms` (the original
  observation time, never a forwarding time), `upstream_origin`,
  `limit_window`, `resets_at_ms`, `resets_at_source`.
- [pinned] `headers`: the upstream provider's rate-limit headers, names
  lowercased, values raw, unknown names included. Absent means none were
  received. A name that is not lowercase is
  `invalid_request {field: "headers"}`.
- [pinned] The answer (`observe::AccountObserveAnswer`) is
  `{observation: {id, duplicate, bound, effects}}`. `bound` says whether
  the observation matched an admission; `effects` lists, in the provider's
  own words, the changes it applied: an account it stopped allocating for
  now, or a change to an account's remaining headroom.
- [pinned] The same `event_id` from the same runner again applies nothing
  and answers the first observation's `id` with `duplicate: true` and
  empty `effects`.
- [pinned] Refusals: `invalid_request {field}`, `account_not_authorized`.

## 10. Refusal codes

| Code | Answered by | Detail | Timed |
|---|---|---|---|
| `invalid_request` | every op except `role.describe` | `{field}` | no |
| `invalid_requirements` | `route.select` (§5.1) | `{field}` | no |
| `unknown_consumer` | `route.select` (§5.1) | `{consumer}` | no |
| `consumer_not_owned` | `route.select` (§1.1) | `{consumer}` | no (terminal) |
| `pin_unavailable` | `route.select` (§5.3) | `{pin, reason, reset_at_ms?}` | when `reset_at_ms` is present |
| `no_route` | `route.select` (§5.3) | `{reason, reset_at_ms?}` | when `reset_at_ms` is present |
| `unknown_decision` | `route.report` (§6), `route.account_for` (§9.1) | `{}` | no |
| `report_conflict` | `route.report` (§8) | `{}` | no |
| `account_not_authorized` | `route.account_for`, `route.account_observe` (§9) | `{}` | no |
| `auth_required` | `route.account_for` (§9.1) | `{}` | no |
| `account_hold` | `route.account_for` (§9.1) | `{until_ms}` | yes, by `until_ms` |
| `account_quota_unknown` | `route.account_for` (§9.1) | `{}` | no |
| `account_pool_exhausted` | `route.account_for` (§9.1) | `{reset_at_ms?}` | when `reset_at_ms` is present |
| `account_pool_reserve_limited` | `route.account_for` (§9.1) | `{}` | no |
| `account_selection_unavailable` | `route.account_for` (§9.1) | `{}` | no (transient) |
| `admission_conflict` | `route.account_for` (§9.1) | `{}` | no |
| `pin_generation_conflict` | `route.account_for` (§9.1) | `{}` | no |
| `routing_unavailable` | a consumer, for its own work (§1) | `{}` | no |
| `routing_ambiguous` | a consumer, for its own work (§1) | `{modules}` | no |

- [pinned] `Refusal::wait_until_ms` reads the time a timed refusal names,
  and `None` for every other refusal, an unknown code included.
- [pinned] `test-vectors/routing-v1/refusals.json` holds one body for every
  code in this table.

## 11. Not in v1

- [informative] Panels (selecting several routes at once) are not part of
  v1. A provider may keep serving an older panel op beside v1; a v1 op is
  added only when more than one consumer needs one.

## 12. Vectors and conformance

- [pinned] `test-vectors/routing-v1/` holds every op's requests and
  answers, the refusals the validators answer, one body per refusal code,
  unknown values for every open set, and the reply envelope's accepted and
  refused bodies. It also holds valid and invalid consumer declarations,
  including a byte-exact declaration round trip. `SHA256SUMS` pins every file.
- [pinned] `cortexkit-role-routing-conformance` drives a live provider
  through these rules. Each check names the rule it holds a provider to;
  the kit's own tests pair every check with a faulty provider that breaks
  only that rule and must fail it by name. Its README lists the checks and
  where the kit checks less than this contract says.
