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

### 1.1 Catalog declarations and consumers

- [pinned] A module may declare its consumers, its runner support, or both in its manifest's daemon slot
  `role_declarations["routing/v1"]` (`ROLE_DECLARATIONS_KEY`). The daemon
  relays this JSON object unread; this role owns its schema
  (`ConsumerDeclarations`, validated by `parse_consumer_declarations`).
  Declarations come only from the daemon catalog, never from a call.
- [pinned] The object has two optional members, `consumers` and `runner`.
  Omitted `consumers` means an empty list, so a pure runner module need not
  declare consumers. When present, `consumers` must be an array (not `null`),
  including an explicitly empty array, with the shape `[{name, eq?, context_window?,
  max_output_tokens?, tools?, tool_capabilities?, modalities?,
  structured_output?}]`. `name` is required and uses the consumer id
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
  is terminal, never timed. Until a module declares, the provider keeps
  its configuration names.
- [pinned] A name found only in operator configuration is usable by any
  caller, unless that configuration names the name's owner. A configured
  owner is a module principal, and the same two arms apply to it: the
  caller's attested principal is that owner, or the request's
  daemon-stamped scope is owned by it. Every other caller is refused
  `consumer_not_owned {consumer}`. The shape of operator configuration is
  the provider's, so how an owner is written there is not part of this
  contract. A verified declaration of the same name still decides
  ownership, as above.
- [pinned] For each demand field, the first value present wins: operator
  configuration, then the request's own requirement, then the declared
  default. Omission is not a zero or false value. The request's required
  `iq` remains required; a declaration cannot set `iq` and does not relax
  request validation. An operator may still override `iq` in configuration.

### 1.2 Runner matching declarations

- [pinned] `runner` has the shape `{pairs: [{wire_api, auth_method}],
  follows_account_choice?: bool, tool_capabilities?: [string],
  standing_limits?: {explicit_providers?: [string]}, harnesses?: [token],
  default_harness?: token, starts_sessions?: bool}` (`RunnerDeclaration`).
  Unknown members in the slot, `runner`, any pair, or `standing_limits`
  invalidate the whole declaration. A refusal names the offending field
  path, such as `runner.pairs[0].wire_api`; no part is applied.
- [pinned] `pairs` is required, non-empty, and has at most 64 entries.
  Each `(wire_api, auth_method)` pair is unique. Both values are tokens
  matching `^[a-z][a-z0-9._-]{0,63}$`. These are open sets: validation
  checks their shape, never a fixed list of wire APIs or authentication
  methods.
- [pinned] Routing intersects the runner module's pairs with the vault's
  available provider credentials and the catalog's routes. A declaration
  promises which combinations the runner module can drive; it creates no
  credential or route and grants no authority to use one.
- [pinned] `follows_account_choice` defaults to `false`. When `true`, the
  runner module calls `route.account_for` and uses its answer. Absence or
  `false` makes no such promise; a routing provider must not infer it.
- [pinned] `harnesses`, when present, lists the exact programs the runner
  hosts sessions in: 1–16 unique tokens using the pair token grammar above.
  Empty is refused as `runner.harnesses`, not read as "no program". Invalid
  or duplicate entries name `runner.harnesses[index]`. Programs are independent
  of pairs: one runner can host several programs behind the same pair. A
  runner that launches an agent program must actually host the requested
  program, never merely relabel a session.
- [pinned] `default_harness`, when present, is a token listed in `harnesses`;
  without that list or membership it is refused as `runner.default_harness`.
- [pinned] `starts_sessions` defaults to `true`. When `false`, the runner is
  never answered by `route.select`, with or without a requested harness,
  including pinned requests and requests whose `runners` list only it. A
  gateway runner attaches to sessions started by a person or launcher rather
  than starting them. This flag affects no other operation: `role.describe`,
  `route.report`, `route.account_for` (including account choice for that
  runner's own turns without a select decision), and `route.account_observe`
  retain their existing authorization, ownership and admission rules.
- [pinned] `tool_capabilities` defaults to an empty list. It has at most 64
  unique names following the tool-provider role's capability-name grammar:
  role-defined unprefixed tags (such as `code.read/v1`), or
  `<namespace>:<name>/v<N>` tags. Routing uses these as the runner module's
  supported tool capabilities when matching tool capability requirements,
  not as module names. Namespaces are non-empty `-`-separated words of
  lowercase ASCII letters and digits; names are non-empty `.`-separated
  words of lowercase ASCII letters, digits and `_`; `N` is a positive
  decimal integer without a leading zero.
- [pinned] `standing_limits.explicit_providers`, when present, restricts the
  runner module to credentials for providers named in that list. Credentials
  for every other provider are ineligible for that runner module. The list
  is non-empty, has at most 64 unique entries, and each entry matches the
  same token grammar as `wire_api`. An empty list is refused rather than
  ambiguously meaning either no providers or no limit. This lets a gateway
  use a platform credential only when its registration names the provider.
  Absent `explicit_providers` means no provider limit. A present
  `standing_limits: {}` also means no limit and retains its empty object
  when serialized.
- [pinned] Optional runner members follow §2's null-as-absence rule.
  Absent `runner`, false `follows_account_choice`, and empty
  `tool_capabilities` are omitted when serialized. Absent `harnesses` and
  `default_harness`, and true `starts_sessions`, are omitted too. Slots without
  the new program members retain their exact 0.4.0 bytes, including an
  explicit `consumers: []`.
- [pinned] Rollout is readers first: a 0.3.x routing provider refuses a slot
  carrying `runner` as an unknown member, discarding even valid consumers
  in that slot. Runner modules **must not declare `runner` until the routing
  provider runs routing contract 0.4.0** (or a later version supporting it).
- [pinned] Program declarations also roll out readers first: a 0.4.0
  provider refuses a declaration carrying `harnesses`, `default_harness` or
  `starts_sessions`. A runner declares these members only after its routing
  provider serves 0.4.1 or later. A caller sends `harness` only when
  `role.describe` declares `select_harness`.

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
  observation's `kind`, a report's `eligibility.scope`, and an account
  answer's `reason.code`. An unknown
  value decodes and is kept as the string it was sent as.
- [pinned] An optional member is left out when it has no value. A member
  sent as `null` reads exactly as one left out.
- [pinned] Quota `pool` names in §§8–9 are opaque, 1 to 64 characters of
  `[a-z0-9._-]`, compared exactly. Absent means the account is a single
  pool. The provider keeps no pool registry: an unseen well-formed pool is
  never refused for being unseen; it has no recorded block. A malformed
  pool is `invalid_request {field: "pool"}`.
- [pinned] Rollout is provider-first: a 0.1.1 provider refuses unknown
  members. The routing provider adopts this release before any consumer
  sends the new members. A consumer sending them to an older provider gets
  `invalid_request {field}` naming the member, never silent omission.
- [pinned] Members that belong to an optional group (§4) follow the same
  rule per provider: a provider that does not declare the group refuses
  them as `invalid_request {field}` naming the member, exactly as it
  refuses any member it does not read. A caller sends them only to a
  provider whose `role.describe` declares the group.

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
  requirement_fields, detail_supported, capabilities?}`.
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
- [pinned] `capabilities` lists the optional groups the provider serves
  (`capabilities::KNOWN`), each once. It is left out when the provider
  serves none, and a reader decodes it left out as empty. A name a reader
  does not know is kept and means nothing to it. This version defines four
  groups:
  - `select_hold_control`: `route.select` reads `hold_ok` and
    `hold_wait_max_s` (§5.1) and decides blocked entries as §5.4 says.
    A provider without it refuses both members (§2). That refusal is why
    the group exists: a caller learns from `role.describe` whether its
    hold choice will be read, and is never answered with the choice
     silently ignored.
  - `select_harness`: `route.select` reads `harness` (§5.1), matching the
    runner's declared session programs, not its wire/auth pairs. A provider
    without it refuses the member (§2); a caller sends it only with the group.
  - `select_prefer`: `route.select` reads `prefer` (§5.1), an ordered list of
    route keys the caller would like. A provider without it refuses the member
    by name (§2); a caller sends it only with the group.
  - `select_exclude_providers`: `route.select` reads `exclude_providers`
    (§5.1), provider ids none of whose routes the caller will take. A caller
    sends the member only to a provider that declares the group. A provider
    without the group does not read the member, so it refuses it by name as
    it refuses any member it does not read (§2): a router older than the
    group, keeping that rule, fails the select loudly rather than silently
    answering a route from a provider the caller meant to rule out.
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
- [pinned] `pin_fallback`: `"route"` beside a pin. When the pin cannot be
  served, the provider routes normally instead of refusing
  `pin_unavailable`. Routing still decides, since only it knows why the pin
  failed, and the fallback answer is still the one selection for the work
  (§1): the caller never selects again. Any other value is
  `invalid_request {field: "pin_fallback"}`, as is a `pin_fallback`
  without a pin. Without the field behaviour is unchanged.
- [pinned] `exclude`: route keys the caller will not take. A route key is
  `provider/model`; the variant and the runner are separate from it.
  Excluding a route key excludes every variant of it. An entry that is not
  a route key is `invalid_request {field: "exclude"}`.
- [pinned] `exclude_providers`: provider ids the caller will not take (group
  `select_exclude_providers`). It is a member of its own beside `exclude`,
  whose meaning it does not change. A provider id is the provider part of a
  route key, everything before the first `/`: it is non-empty and has no
  `/`. No answer names a route whose provider is listed, whatever its model
  or variant (`select::check_answer`). A pin on a listed provider cannot be
  served, exactly as a pin on an excluded route key: `pin_unavailable` with
  reason `excluded`, or with `pin_fallback: "route"` a fallback that is
  itself never from a listed provider. When every route the request would
  otherwise get is from a listed provider, the select is refused with the
  same `no_route` (§5.3), under the same reason and `reset_at_ms` rules, as
  a request whose `exclude` lists each of those routes. The list has the
  limits `exclude` has: no length limit, and a repeated entry is not
  refused. An entry that is not a provider id, or a member that is not a
  list of strings, is `invalid_request {field: "exclude_providers"}`. A
  provider without the group refuses the member by that same field path
  rather than ignoring it. Null is absent.
- [pinned] `runners`: runner module ids the caller can send to. Absent: any
  session-starting runner the provider knows.
- [pinned] `harness`: optional token using §1.2's pair token grammar (group
  `select_harness`). Only runners whose own declaration lists that program
  in `harnesses` are candidates. The caller picks the program, never the pair.
  Without this member nothing is filtered by harness. A malformed value is
  `invalid_request {field: "harness"}`; a provider without the group refuses
  the member by that same field path rather than ignoring it. Null is absent.
- [pinned] `prefer`: an ordered list of at most 16 route keys (group
  `select_prefer`), each `provider/model` without a variant, with no
  duplicates. Among the routes the provider would already answer for the
  request it answers a listed one when one is adequate, in list order. It
  never admits a route the request would not otherwise get, so it is not a
  pin: a listed route that is excluded, fails the requirements, has no
  runner, or is otherwise unavailable is skipped. An entry that is not a
  route key, a list longer than 16, or a duplicate entry is
  `invalid_request {field: "prefer"}`. A provider without the group refuses
  the member by that same field path rather than ignoring it. Null is absent.
- [pinned] `affinity`: an opaque key of at most 128 characters (§7).
- [pinned] `correlation_id`: opaque, at most 128 characters, echoed in logs
  only.
- [pinned] `concurrency_slots`: an optional hint,
  `[{key, running, limit, queued?}]`.
- [pinned] `detail`: bool; asks for the provider's explanation object.
- [pinned] `hold_ok`: bool, default `true` (group `select_hold_control`).
  `false` asks the provider not to answer an entry the runner would have
  to hold on while another can serve now (§5.4).
- [pinned] `hold_wait_max_s`: non-negative integer seconds (group
  `select_hold_control`): the caller's own cap on a hold. `0` means no
  hold. It is valid together with `hold_ok: false`, because the fallback
  that request can take may still hold, and the cap applies to it.
- [pinned] `hold_ok` and `hold_wait_max_s` sent to a provider that does not
  declare `select_hold_control` are `invalid_request {field}` naming the
  member, `hold_ok` first when both are sent. `prefer` sent to a provider
  that does not declare `select_prefer` is `invalid_request {field: "prefer"}`,
  and `exclude_providers` sent to a provider that does not declare
  `select_exclude_providers` is `invalid_request {field: "exclude_providers"}`.
  The validator takes the
  provider's declared groups (`select::parse_select_request_serving`);
  `select::parse_select_request` declares none.
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
  runner: {module, model: {provider, model, variant?}, auth?: {pool?, credential_id?}, harness?},
  context_window, max_output_tokens,   // tokens, from the provider's own catalog read
  expires_at_ms,
  explanation,                         // one sentence, at most 200 characters
  detail?,                             // the explanation object, when asked for and supported;
                                       // on `fell_back` it carries `pin_reason`
  pin_outcome?                         // `"pinned" | "fell_back"`, exactly when the request pinned
}
```

- [pinned] `runner.module` is one of the request's `runners` when the
  request sent them, and the route is never one the request excluded nor
  from a provider it excluded (`select::check_answer`).
- [pinned] `runner.model` names the model as the runner names it.
- [pinned] `auth` is present only when the provider's policy names a key
  pool. It names the key by public id only, never secret material. The
  runner sends with that key, still under its own admission check, and a
  runner refuses a key-pool send without it.
- [pinned] `runner.harness` is the exact program the answered runner hosts
  the session in. With a requested `harness`, the runner must list it in its
  declaration and the answer echoes that requested value. Without one, the
  answer carries the runner's `default_harness`, else its single listed
  harness, else leaves the member out. It is never from anything but the
  runner's declaration or the request. A caller never binds a runner session
  on an answer without it. A runner declaring `starts_sessions: false` is
  never answered, regardless of harness.
- [pinned] `context_window` and `max_output_tokens` are the answered model's
  figures from the provider's own catalog read, so a consumer sizes its
  work without a second lookup.
- [pinned] `expires_at_ms` is in the future when the answer is given (§6).
- [pinned] `explanation` is one sentence of at most 200 characters.
- [pinned] `detail` is non-normative; a consumer never acts on it, except that
  on `pin_outcome: "fell_back"` it carries `pin_reason` naming the pin
  refusal's reason, so the caller knows why the pin was skipped.
- [pinned] `pin_outcome` is present exactly when the request pinned:
  `"pinned"` when the pin was served, `"fell_back"` when
  `pin_fallback: "route"` routed instead (`select::check_answer`). A
  fallback answer is still the one selection for the work.

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
  `quota_or_cooldown`, `all_inadequate`, `accounts_blocked` (§5.4),
  `harness_unavailable`. When no session-starting runner lists the requested
  program, answer `no_route` with reason `harness_unavailable`, naming the
  requested harness in the message; this cause is not timed. Other existing
  refusals (including `pin_unavailable` for an unservable pin) are unchanged.
- [pinned] **A timed cause carries `reset_at_ms`**: a cooldown, a backoff, or
  exhausted quota whose reset is known. A consumer waits until then and
  asks again, rather than failing its work. A refusal without `reset_at_ms`
  is not timed. `reset_at_ms` alone decides whether a consumer waits.

### 5.4 Holds

This section binds a provider declaring `select_hold_control` (§4).

- [pinned] `route.select` never waits and has no hold answer. A hold
  happens only at `route.account_for`, as `account_hold {until_ms}`
  (§9.1). What select decides is whether to answer an entry whose
  eligible accounts are all blocked, so that the runner holds there, or to
  move.
- [informative] Switching models costs a full prompt-cache rebuild, so
  staying on an entry is worth a short wait.
- [pinned] An entry is one ladder entry: one route (`provider/model`, with
  its variant) of a consumer the operator routes by ladder
  (`mode: "ladder"`), in the ladder's order. A consumer routed by demand
  (`mode: "demand"`) has no entries to hold on: a model whose eligible
  accounts are all blocked is skipped for the next-cheapest adequate one.
- [pinned] An entry's accounts are every account the provider's own
  account policy lets it use for that route and consumer. A per-token
  (metered) account counts only where that policy allows falling back to
  one when the subscription accounts are blocked; while such an account can
  serve, the entry is not blocked. An entry is
  blocked when every one of them is blocked by a window. Its reset is the
  time the first of those windows ends, and is unknown when no window's
  end is known.
- [pinned] The operator sets, per provider, `max_hold` (default 900
  seconds, the `route.account_for` default) and `hold_when_reset_within`
  (default 900 seconds). The hold limit is the smaller of `max_hold` and
  the request's `hold_wait_max_s`.
- [pinned] For a blocked entry the provider decides, in this order:
  1. `hold_ok: false`: move.
  2. The reset is unknown, or further away than `hold_when_reset_within`:
     move.
  3. The wait until the reset exceeds the hold limit: move.
  4. Otherwise it answers the entry, and the runner holds at
     `route.account_for`.

  Moving is going on to the next entry in ladder order.
- [pinned] With `hold_ok: false`, the provider answers the next entry in
  ladder order that has an account usable now. If none has one, it falls
  back to the `hold_ok: true` behaviour, because work that cannot proceed
  otherwise is held rather than failed: it answers the blocked entry
  whose reset is soonest among those steps 2 and 3 let through, and the
  runner holds at `route.account_for`.
- [pinned] When every entry is blocked and none resets within the limit,
  the provider refuses `no_route` with reason `accounts_blocked`.
  `reset_at_ms` is the earliest known reset across those entries, and is
  left out when none is known (§5.3).
- [pinned] The provider records the request's `hold_wait_max_s` on the
  decision. A `route.account_for` naming that decision applies it too
  (§9.1).

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
`{decision_id, outcome, runner_code?, credential_id?, reset_at_ms?, usage?,
pool?, eligibility?, request_input_tokens?}`.

- [pinned] `outcome` is an open set: `completed`, `refused`, `failed`,
  `timed_out`, `cancelled`. `refused`, `failed` and `timed_out` are
  failures; `completed` and `cancelled` are not. An unknown outcome is kept
  and counts as a failure, so a provider never keeps preferring a route on
  a report it cannot read (`Outcome::is_failure`).
- [pinned] `runner_code`: the runner's typed refusal or error code,
  verbatim, so the provider can tell "this route cannot serve this" from
  "this attempt failed".
- [pinned] `credential_id` and `reset_at_ms`: when the runner reported them.
- [pinned] `pool`: the pool the refused or served model drew from (§2).
- [pinned] `eligibility`: `{scope: "model" | "account", code: string}`,
  sent instead of exhaustion when the model provider says the account
  cannot serve this model (`model`) or cannot serve at all (`account`).
  `scope` is open on decode (§2). Routing never treats eligibility as
  exhaustion and never holds on it.
- [pinned] Eligibility with an exhaustion `runner_code` or with
  `reset_at_ms` is `invalid_request {field: "eligibility"}`. Exhaustion
  codes are `account_limit_exhausted` (§9.2), `account_pool_exhausted`
  (§9.1), `quota_exhausted` and `credential_billing_exhausted` (§5.3), when
  relayed as `runner_code`. A reset time implies a hold.
- [pinned] `request_input_tokens`: non-negative integer, the request's
  input size in tokens. A size-shaped refusal fences only that request-size
  class, not smaller requests that can pass.
- [pinned] `usage`: `{input_tokens?, cached_input_tokens?,
  cache_write_tokens?, output_tokens?, reasoning_tokens?, duration_ms?,
  first_token_ms?, cost_usd?}` (`report::Usage`). The token field names and their meaning
  match `cortexkit-role-llm-runner`'s usage: `input_tokens` never includes
  cache reads or writes, which are the separate `cached_input_tokens` and
  `cache_write_tokens` counts; `reasoning_tokens` is a breakdown of
  `output_tokens`, never an addition to it; an absent count means unrecorded,
  never zero. `duration_ms` is the wall-clock
  run time and `first_token_ms` the time to the first output token, both in
  milliseconds. Every present count is a non-negative integer up to 2^53-1
  (`report::MAX_USAGE_COUNT`); `cost_usd` is the caller's own claim, a
  non-negative finite JSON number. The provider may ignore it, since it prices usage
  from its own catalog. An unknown member is refused as
  `invalid_request {field: "usage"}`. `usage` is part of the report body for
  idempotency: a repeat with different usage is a different report.
- [pinned] A report with `outcome: failed` and a future `reset_at_ms` fences
  only that decision's route on that decision's account until the reset:
  never the whole provider, never other accounts, never other routes. A
  caller-supplied reset further than 24 hours away is capped at 24 hours
  from now (`report::RESET_FENCE_CAP_MS`). A reset already in the past is
  not refused: the report is recorded as a failure with no fence, because a
  "retry in 5 s" landing 6 s later is a normal race. Only the decision's
  own consumer may report, as §6 says.
- [pinned] The answer (`report::ReportAnswer`) is
  `{decision_id, report_id, outcome}`: the provider's id for the recorded
  report and the outcome recorded.
- [pinned] A report is idempotent on `decision_id`: the same body again
  returns exactly the first answer; a different body after the first is
  `report_conflict`. Bodies are compared as decoded, so key order, and a
  member sent as `null` instead of left out, never make bodies differ.
  `pool`, `eligibility`, `request_input_tokens` and `usage` are part of the body.
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
- [pinned] `parent_conversation_key` (optional): the same string rules as
  `conversation_key` (`CONVERSATION_KEY_MAX_CHARS`). The provider prefers an
  eligible account the parent conversation is not on, and falls back to
  the parent's account when no other is eligible. The parent conversation
  is on an account while its conversation key holds a live pin there: the
  account answered by the latest `admit`, `renew` or `reassign` for that
  key, whether or not `admission_started` has been observed. The pin stops
  counting when it lapses: an admission never started at
  `admission_valid_until_ms`, a started one at `lease_valid_until_ms`.
- [pinned] `pool` (optional): the quota pool the requested model draws from,
  as the runner names it (§2).
- [pinned] `admission_id` (required, non-empty, at most 128 characters):
  the idempotency key. The same id and body from the same runner returns
  exactly the first answer; a different body is `admission_conflict`.
  Bodies are compared with defaults applied
  (`AccountForRequest::same_body`): a member left out and the same member
  sent with its default value are the same body.
  `pool` and `parent_conversation_key` are part of the body.
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
  runner waits out a short hold instead of being refused. When the
  request's `decision_id` names a decision that recorded a caller's
  `hold_wait_max_s` (§5.4), the provider holds only within the smallest of
  its `max_hold`, that recorded cap and this member.
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
  `account_pool_reserve_limited`, `account_selection_disabled` (not timed),
  `account_selection_unavailable`
  (transient, not timed), `admission_conflict`, `pin_generation_conflict`.
- [pinned] `account_selection_disabled` means account selection is off on
  this router; use your own credential. The switch is module-wide, never
  per provider or per pool. The refusal is untimed and carries no
  `reset_at_ms`. A caller must not treat it as exhaustion or a hold, and
  must not retry against another provider expecting a different answer.

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
- [pinned] `pool` (optional): the quota pool for the reading or used-account
  report (§2). Both use this flat observation shape.
- [pinned] `headers`: the upstream provider's rate-limit headers, names
  lowercased, values raw, unknown names included. Absent means none were
  received. A name that is not lowercase is
  `invalid_request {field: "headers"}`.
- [pinned] In `headers` and `limit_window`, a disabled usage window is left
  out of a reading, never sent with a stale fraction.
- [pinned] `account_observe {kind: run_finished}` releases the runner's
  account pin; `lease_valid_until_ms` is only the backstop.
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
| `account_selection_disabled` | `route.account_for` (§9.1) | `{}` | no (module-wide; use your own credential) |
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
  including a byte-exact declaration round trip. `route-select-hold-control.json`
  holds the `select_hold_control` members with and without the group, the
  `accounts_blocked` refusal, and an answer naming its runner's harness.
  `route-select-prefer.json` holds the `select_prefer` members with and
  without the group, and `route-select-exclude-providers.json` the
  `select_exclude_providers` members. `SHA256SUMS` pins every file.
- [pinned] `cortexkit-role-routing-conformance` drives a live provider
  through these rules. Each check names the rule it holds a provider to;
  the kit's own tests pair every check with a faulty provider that breaks
  only that rule and must fail it by name. A check of an optional group's
  rules reports not applicable against a provider that does not declare
  the group, and leaves its verdict a pass. Its README lists the checks and
  where the kit checks less than this contract says.
