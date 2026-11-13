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

The baseline rules below remain unchanged without `consumer_declarations`.
That group's additive defaults, attested namespace and selector rules are in §16.1.

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

- [pinned] `runner` has the shape `{pairs: [{wire_api, auth_method, requires_wire_api?: bool}],
  follows_account_choice?: bool, requires_wire_api?: bool, tool_capabilities?: [string],
  standing_limits?: {explicit_providers?: [string]}, harnesses?: [token],
  harness_limits?: {<harness>: {pairs?: [{wire_api, auth_method}], explicit_providers?: [token]}},
  default_harness?: token, starts_sessions?: bool}` (`RunnerDeclaration`).
  Unknown members in the slot, `runner`, any pair, or `standing_limits`
  invalidate the whole declaration. A refusal names the offending field
  path, such as `runner.pairs[0].wire_api`; no part is applied.
  The known `harness_limits` member is the exception: it is retained raw and
  validated independently, so a malformed value does not discard the runner.
- [pinned] `pairs` is required, non-empty, and has at most 64 entries.
  Each `(wire_api, auth_method)` pair is unique. Both values are tokens
  matching `^[a-z][a-z0-9._-]{0,63}$`. These are open sets: validation
  checks their shape, never a fixed list of wire APIs or authentication
  methods.
- [pinned] Routing intersects the runner module's pairs with the vault's
  available provider credentials and the catalog's routes. A declaration
  promises which combinations the runner module can drive; it creates no
  credential or route and grants no authority to use one.
- [pinned] Each `wire_api` and `auth_method` value has one shared spelling.
  Decoding still checks only the token shape, so accepting a new value
  never needs a validation change. The tables below are a requirement on
  the systems that produce these values, not something a decoder enforces:
  a runner declares, the model catalog serves and the vault reports only
  spellings listed there. A new spelling is added to its table first, in a
  text release, before anything uses it. The tables are also exported as
  `spellings::WIRE_APIS` and `spellings::AUTH_METHODS`.
- [pinned] `wire_api` names the bytes on the wire: the endpoint shape and
  the request and reply format. When a backend's wire format differs because
  of the credential that reaches it, it has its own `wire_api`, and the pair
  says which credential that is.

  | `wire_api` | Wire format |
  |---|---|
  | `anthropic.messages` | Anthropic Messages API |
  | `openai.responses` | OpenAI Responses API, the public shape, also served by other providers |
  | `openai.chat_completions` | OpenAI-compatible Chat Completions, from any provider |
  | `openai.chatgpt_responses` | The ChatGPT backend's Responses variant, reached with a ChatGPT subscription |
  | `google.generate_content` | Gemini `generateContent` |
  | `google.code_assist` | The Code Assist envelope around `generateContent`, reached with a Google Antigravity sign-in |
  | `aws.bedrock_converse` | Amazon Bedrock Converse |

- [pinned] The model catalog serves a route's `wire_api` as a per-route fact
  named `wire.family`, a string whose value is one of the table's tokens, in
  the route's catalog reply. A token there means the
  route's request format is verified as that wire format; it says nothing
  about which runner can drive it. A route the catalog serves without
  `wire.family` has no verified wire format and matches no runner pair: a
  routing provider never infers one.
- [pinned] `auth_method` names how a request is authorised. The spellings
  are the vault's own names for its credential kinds:

  | `auth_method` | Credential |
  |---|---|
  | `apikey` | An API key |
  | `oauth` | A provider's OAuth sign-in, such as a Claude subscription |
  | `chatgpt` | A ChatGPT subscription sign-in |
  | `antigravity` | A Google Antigravity sign-in |
  | `gateway` | One vault credential supplies both the endpoint and its credential; the runner calls that endpoint, and the gateway holds the upstream provider key |

- [pinned] A `gateway` pair normally has an effective `requires_wire_api` of
  `false`: the runner uses the credential's own endpoint, not a catalog
  `wire.api`. The runner sends only the gateway credential, not a second secret
  for the upstream provider. Informatively, the vault and runner allow HTTPS,
  or plain HTTP only to a loopback gateway; routing does not validate endpoints.
- [pinned] For a credential whose `auth_method` is `gateway`, every
  `explicit_providers` check, runner-wide or per harness, compares the provider
  of the catalog row the gateway route borrows, which is the provider the vault
  credential's `serves` names, never the gateway credential's own provider id.
  Catalog rows keep their catalog provider ids. A gateway route is
  `<gateway provider id>/<model>`, borrowing the served provider's catalog row,
  never the borrowed provider's own route key. A provider without catalog rows
  borrows the rows of the one catalog provider its active gateway credentials'
  `serves` collectively names. If they name no provider or more than one, the
  provider stays unresolved: it gets no routes and nothing is selected from it.
  The vault sets `serves` at credential creation and never changes it. Other
  credential methods also carry `serves`; only `gateway` enables this borrowing.
  A gateway credential `anthropic-gw-acme` with `serves: ["anthropic"]` forms
  `anthropic-gw-acme/<model>` from `anthropic/<model>` and passes a limit of
  `["anthropic"]`; a gateway borrowing `openai/...` rows fails it. Listing
  `anthropic-gw-acme` in
  `explicit_providers` admits nothing extra: only the served provider counts.
  For every other auth method, the check compares the credential's own provider.
- [pinned] Readers first for gateway meaning: deploy routing 0.7.15 before a
  runner declares a `gateway` pair, with routing able to match the vault's
  `llm-gateway` credential as `gateway`. Older parsers accept the token and keep
  the whole declaration: `RunnerPair.auth_method` is a string and
  `validate_runner` checks only its token shape (`src/declarations.rs`,
  `RunnerPair` and `validate_runner`). Matching is by exact equality, so an
  older router can match a credential already reported as `gateway`, but cannot
  match a credential reported under a different method or without one. Parsing
  the token does not give a router the gateway credential's meaning or derive
  its method; the rollout rule is semantic, not a decoder compatibility rule.

- [pinned] The provider is never part of an `auth_method`: routing matches
  a credential's provider separately, so a Claude subscription and another
  provider's OAuth sign-in are both `oauth`, told apart by provider. A
  credential the vault reports without an `auth_method` is never routed. A
  spelling missing from the table still decodes, but nothing may use it
  until it is added to the table; a provider that meets one matches it only
  by exact equality, never as a near spelling of a listed one.
- [pinned] `follows_account_choice` defaults to `false`. When `true`, the
  runner module calls `route.account_for` and uses its answer. Absence or
  `false` makes no such promise; a routing provider must not infer it.
- [pinned] A runner pair's effective `requires_wire_api` is its own value,
  else the runner-level value, else `false`. When the effective value is
  `true`, the runner opens a catalog row for that pair only through its served
  `wire.api`, so routing excludes rows without that fact at runner-pair matching
  for `route.select`, panel selection and `route.head_choices`, and uses the
  same effective value for the `runner_requires_wire_api` explanation. A served
  fact is present with a value, regardless of its source (`from`); `wire.family` and
  `wire.npm` do not substitute for `wire.api`. An effective `false` preserves
  matching without this requirement. A non-boolean, non-null value refuses the
  whole declaration as `runner.requires_wire_api` or
  `runner.pairs[index].requires_wire_api`, like other boolean members; the raw
  `harness_limits` fallback is a separate exception.
- [pinned] Readers first: routers before 0.7.13 refuse unknown
  runner-declaration members and drop the runner, so a runner declares
  `requires_wire_api` only after its router runs 0.7.13 or later.
- [pinned] Readers first for pair overrides: a 0.7.13 or older reader refuses
  a pair carrying `requires_wire_api` as an unknown member and drops the whole
  runner declaration. No runner declares the member on a pair until routing
  runs 0.7.14 live.
- [pinned] `harnesses`, when present, lists the exact programs the runner
  hosts sessions in: 1–16 unique tokens using the pair token grammar above.
  Empty is refused as `runner.harnesses`, not read as "no program". Invalid
  or duplicate entries name `runner.harnesses[index]`. Programs are independent
  of pairs: one runner can host several programs behind the same pair. A
  runner that launches an agent program must actually host the requested
  program, never merely relabel a session.
- [pinned] `default_harness`, when present, is a token listed in `harnesses`;
  without that list or membership it is refused as `runner.default_harness`.
- [pinned] `harness_limits` optionally narrows the runner's offers for each
  named harness. `HarnessLimits` has optional `pairs` and `explicit_providers`;
  `RunnerDeclaration::with_harness_limits` writes the typed map, while
  `typed_harness_limits` reads and validates the raw member. A missing map,
  missing harness entry, or absent/null field inherits the runner-level value.
  Runner-level `pairs` remains the union of what the runner can run. Harness
  pairs must be a non-empty, unique subset, comparing only `wire_api` and
  `auth_method`. Harness-limit pairs keep the `{wire_api, auth_method}` shape:
  they only narrow the pair set and never change a pair's requirement, which
  comes only from `runner.pairs` and the runner-level value. A
  `requires_wire_api` member in a harness-limit pair is ignored like other
  unknown members and has no effect. Harness `explicit_providers` is a non-empty, unique provider
  allow-list with the standing-limit token grammar and 64-entry bound. For
  gateway credentials, it compares the served catalog provider as pinned in
  the gateway rules above. When
  a runner-level allow-list exists, every harness provider must be in it;
  when absent, a harness may introduce an allow-list. Neither field may broaden
  what the runner offers. The union remains safe for readers that ignore the
  restrictions: they over-offer for a harness, never beyond the runner's ability.
- [pinned] The harness-limits accessor returns `HarnessLimitsError`, naming a
  missing `harnesses` list, a key outside that list, empty pairs, a pair outside
  the runner-level set, a provider outside the standing allow-list, or malformed
  fields. Unknown fields inside a harness entry or its pairs are retained and
  ignored. A refused value means **use the runner-level values for every
  harness**, even entries that would be valid alone. Selection and head-choice
  construction log a warning naming the runner and the error; they do not
  remove the runner or apply part of the map. `limits_for_harness` resolves
  inheritance and returns the same error for callers to log and fall back.
- [pinned] Without `decision_harness` (§16.3), for `route.select`, resolve the session harness from the request,
  declared default, or sole declared harness before matching models, then use
  that harness's effective pairs and provider allow-list. If no harness is
  resolved, use runner-level values. This applies to pins and reused choices
  as well as ordinary model selection; a pin cannot bypass the limits.
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
  when serialized. For gateway credentials, this list compares the served
  catalog provider as pinned in the gateway rules above.
- [pinned] Optional runner members follow §2's null-as-absence rule.
  Absent `runner`, false `follows_account_choice` and `requires_wire_api`, and empty
  `tool_capabilities` are omitted when serialized. Absent `harness_limits`, `harnesses` and
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
- [pinned] Harness limits also roll out **readers first**: routing must run
  contract 0.7.11 or later before a runner declares `harness_limits`.
  `parse_consumer_declarations` in 0.7.10 and earlier refuses this unknown
  member, discarding the whole runner declaration rather than ignoring it.

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
- [pinned] Most enumerations a reader decodes are open sets: a report's
  `outcome`, `pin_unavailable`'s and `no_route`'s `reason`, an observation's
  `kind`, and an account answer's `reason.code`. An unknown value decodes and
  is kept as the string it was sent as.
- [pinned] A few are closed, because a router acts on their value and a
  guess would be a wrong fact: `ContinueAnswer.kind` (§9.1.1), a report's
  `eligibility.scope` and `refusal_class` (§8), and an observation's
  `upstream_origin` and `resets_at_source` (§9.2). In a request, a value a
  closed set doesn't name is `invalid_request` naming the member. In an
  answer (`ContinueAnswer.kind`), a caller that meets a value it doesn't name
  treats the whole answer as malformed. Adding a value needs a describe
  group, because a reader that doesn't know it refuses it.
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
  selects on a route bound to that work's scope: the scope of the session or
  run that will spend the decision. When that scope doesn't exist yet, the
  module creates it first and selects under it. Where no session or run will
  exist at all, it uses a scope made for that one operation and ended after
  it. A scope that names only a project, with no session or run in it, isn't
  a work scope, so this rule never asks for one.
- [pinned] A select on an unscoped route is attributed to the calling module
  itself.
- [pinned] Per-project pins and cost attribution follow the scope of the
  route, so they follow the work, not the module that asked.
- [pinned] The caller of every op is the route's principal, never a body
  member.
- [pinned] The one settings-lookup exception is `settings` (§5.6), also on
  `route.select_panel`. Its `project_id` and `tier_consumer` choose settings,
  never scope. Pins, cost attribution and consumer ownership still follow the
  route's stamp. A stamped project different from `settings.project_id` is
  `invalid_request {field: "settings.project_id"}`.

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
  does not know is kept and means nothing to it. This version defines these groups:
  - `consumer_declarations`: consumer `iq`, `ladder`, `placement`, `selectors`
    and `kind`, including the attested module-root rule (§16.1).
  - `runner_harness_facts`: per-harness `starts_sessions`, `capabilities`
    and `exclude` (§16.2).
  - `decision_harness`: decisions bind `{module, harness}` in the existing
    `runner` object (§16.3); no member changes type.
  - `select_kind`: workload `kind` on selects (§16.4).
  - `select_eligible_routes`: the per-call route subset (§16.4).
  - `select_price`: value-estimate rates on select answers (§16.5).
  - `report_attempt_outcomes`: authoritative attempt statuses on reports (§16.6).
  - `service_tier`: tier-aware selection and tier metadata in report usage
    (§5.5). A caller sends these members only to a declaring provider.
  - `account_continue`: `route.account_for` reads `mode: "continue"` for a
    runner's durably paused run (§9.1.1). A non-declaring provider refuses
    `invalid_request {field: "mode"}`.
  - `account_continue_signed_out`: only together with `account_continue`,
    accepts `exhausted.limit_window: "signed_out"` for a run paused for
    sign-in (§9.1.1). Unknown capability support counts as no support.
  - `account_warm_credential`: `route.account_for` reads `warm_credential`
    on admission (§9.1). A non-declaring provider refuses the member by name.
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
  - `select_task_ref`: `route.select` reads `task_ref` (§5.1) for attribution,
    never selection. A caller sends it only to a provider declaring the group;
    a provider without the group refuses the member by name (§2).
  - `select_panel`: `route.select_panel`, including seat replacements (§11.1).
  - `budget_quote`: `route.budget_quote` (§11.2).
  - `explain`: `route.explain` (§11.3).
  - `account_status`: `route.account_status` (§11.4).
  - `head_choices`: `route.head_choices` (§11.5).
  - `consumer_models`: `route.consumer_models` (§11.6).
  - `select_engine`: `route.select_engine` (§13).
  - `engine_reports`: `route.engine_report` (§14); requires `select_engine`.
  - `engine_evals`: `route.engine_eval` (§15); requires `select_engine`.
  - `select_pin_slot`: operator-policy slot pins (§5.6).
  - `select_available_routes`: caller-reachable route restriction (§5.6).
  - `select_prefer_prompt_cache`: prompt-cache tie-breaking (§5.6).
  - `select_settings`: project and service-tier settings lookup (§5.6).
  - `observe_backoff`: account-local backoff, three observation members and
    decision-owner authorization (§9.2).
  - `observe_credential_refused`: credential-wide refusal observations (§9.2);
    requires `observe_backoff`.
  - `report_outcomes`: additional report outcomes and their effects (§8).
  - `report_usage_run_id`: run-attributed report usage (§8).
- [pinned] Each of the nine optional op groups is declared exactly when its op is
  in `ops` (`capabilities::OP_GROUPS`). `describe::check_describe` refuses
  either mismatch as `DescribeProblem::OpGroupMismatch {group}`. The five
  `REQUIRED_OPS` are unchanged; a 0.6.0 describe answer still passes. A caller
  calls a new op only on a declaring router. Whole ops have no undeclared-group
  refusal case. The `cortexkit-role-routing-conformance` cases that send a new
  operation are not applicable without its group.
- [pinned] A consumer checks the answer with `describe::check_describe`
  before routing anything to the provider.
- [pinned] Declaring `observe_credential_refused` without `observe_backoff` is
  `DescribeProblem::MissingCapabilityDependency {group: "observe_credential_refused",
  requires: "observe_backoff"}`.
- [pinned] Declaring `engine_reports` or `engine_evals` without `select_engine`
  is `DescribeProblem::MissingCapabilityDependency {group, requires: "select_engine"}`.

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
    For a consumer whose router does not rank by iq (such as a classifier
    consumer), callers send `0` and the router ignores it; it remains
    required on the wire.
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
  - `questions` (integer ≥ 1): the most questions any single provider
    request will carry; the model's served question limit must be at least
    this value.
  - `input_tokens` (non-negative integer): estimated total input tokens the
    decision will spend, not the largest single request. Without a cap it
    is optional, and `0` means no estimate.
  - `max_cost_usd` (number > 0): an input-spend cap. A model whose stated
    input price multiplied by `input_tokens` (converting price units to
    tokens) exceeds the cap is ineligible. A model with no stated input
    price is ineligible under a cap. A cap requires `input_tokens` ≥ 1;
    missing or zero is `invalid_requirements` naming `input_tokens`.
    These three members are usable by any consumer, but callers send each
    only to routers listing it in `requirement_fields`.
    Existing `context_window` covers the largest single request, and
    `modalities: ["image"]` covers image inputs.
- [pinned] A requirement member the provider does not read is refused
  `invalid_requirements {field}` naming it as sent; it is never ignored. A
  malformed requirement (a missing `iq`, a demand above 100, a `tools` or
  modality value this version does not define) is refused the same way,
  naming it.
- [pinned] `pin`: `"provider/model[:variant]"`. The provider is everything
  before the first `/`; a variant is what follows the last `:` after it. A
  pin that cannot be served is `pin_unavailable`; a malformed one is
  `pin_unavailable` with reason `invalid`.
  With no fallback, routing chooses only an account for that model and
  refuses if none can serve it; it never substitutes another model.
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
  a route key, including a pin with a variant (`provider/model:fast`), is
  `invalid_request {field: "exclude"}`. The variant is read after the last
  `:` in the model part, so a provider id may itself contain `:`.
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
- [pinned] `task_ref`: an optional opaque task reference (group
  `select_task_ref`), 1 to 128 bytes, each printable ASCII `0x21`–`0x7E`.
  Spaces, control characters and non-ASCII are invalid; punctuation such as
  `:` `_` `-` `/` `.` and `#` is allowed. A malformed value is
  `invalid_request {field: "task_ref"}`. Null is absent and an absent member
  is not serialized. A caller sends it only to a provider declaring the group;
  a provider without the group refuses it by that same field path.
  The provider stores `task_ref` with the decision for attribution and per-task
  cost profiles; its retention follows the decision row's. It never changes
  which route is answered: a select with `task_ref` is answered exactly as
  the same select without it. It never appears in any answer and is never
  returned to any other caller. `route.report` needs no new member because
  it already names its decision. `correlation_id` remains log-only.
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
  Waiting on a blocked account is worth it only while the session's
  prompt cache on that account would still be warm when the account resets.
  After that the cache is lost whichever account serves. So a runner that
  knows when its cache expires sends at most the time left until then
  (its last accepted response on that account plus the cache lifetime in
  force), and sends `0` when it has no accepted response there. The provider
  keeps no cache state; the cap is the only place this bound reaches it.
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
  pin_outcome?,                        // `"pinned" | "fell_back"`, exactly when the request pinned
  project_settings?                    // exactly when settings.project_id was sent (§5.6)
}
```

- [pinned] `runner.module` is one of the request's `runners` when the
  request sent them, and the route is never one the request excluded nor
  from a provider it excluded (`select::check_answer`).
- [pinned] `runner.model` names the model as the runner names it.
- [pinned] `auth` is present only when the provider's policy names a key
  pool. It names the key by public id only, never secret material. The
  runner sends with that key, still under its own admission check, and a
  runner refuses a key-pool send without it, except for a sign-in method the
  runner lists in `routed_accounts` (§9.3). For a listed method, the runner
  admits the account itself; it passes `auth.pool` as `account_for.pool` (or
  omits the pool), and `auth.credential_id` only as
  `requested_account.credential_id`. The account admission answer, not the
  select hint, names the credential used. Unlisted methods and runners without
  the group keep the original rule unchanged.
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
  `credential_billing_exhausted`, `cooldown`, `quota_exhausted`, `backoff`,
  `unknown_slot` (§5.6).
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
- [pinned] The provider keeps, per provider, `max_hold`: a safety ceiling
  on any hold, which the operator may set. The contract fixes no number;
  a router documents its own default. The operator may also set
  `hold_when_reset_within`, which has no default. The hold limit is the
  smaller of `max_hold` and the request's `hold_wait_max_s`.
- [pinned] For a blocked entry the provider decides, in this order:
  1. `hold_ok: false`: move.
  2. The reset is unknown: move. When the operator has set
     `hold_when_reset_within` and the reset is further away than it, move
     too. Unset, it adds no threshold of its own.
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

### 5.5 `service_tier`: provider service levels

- [pinned] A router declares the optional `service_tier` group in
  `role.describe.capabilities`. A caller sends `route.select.service_tier`,
  `route.account_for.service_tier`
  and either tier member in report usage **only** to a router declaring
  that group. A non-declaring older router does not know the select member
  and could otherwise silently answer standard. A new decoder refuses an
  undeclared select member by name. Older account-admission parsers may
  silently ignore the member and pin as though `standard` was requested;
  callers must never interpret a missing `service_tier_chosen` as confirmation
  of a requested non-standard tier. Request decoding remains lenient about
  unknown members. Report usage remains strict: 0.5.5
  accepts the two defined members, and still refuses every other unknown
  member. The capability gate protects older strict report readers.
- [pinned] `route.select` and `route.account_for` accept optional `service_tier`: `standard`, `fast`
  or `ultrafast`. On `route.select` and `route.select_panel`, absence means
  the request sets no level: the router's service-tier settings for the
  consumer (§5.6) may choose one, and without such a setting the level is
  standard. A requested level always caps the answer. On `route.account_for`,
  absence means standard, or the decision's chosen tier when the request
  carries a `decision_id`. Only accounts whose plans
  include the tier are eligible. Plan inclusion is the router's knowledge;
  this contract does not prescribe where plans come from. When no eligible
  account can serve at the requested or settings-chosen level, try
  `ultrafast` then `fast` then `standard`, starting at that level and never
  upgrading it. A
  decision or account admission records top-level `service_tier_chosen`. An explicit tier request carries
  that value in the answer, including a fallback to standard. Without a tier
  request, standard may be omitted to retain existing answer bytes. A runner
  receiving the decision applies the chosen level, never the original level
  that the router fell back from; account admission for the decision also
  honours that level's plan eligibility.
  `service_tier::fallback` supplies the same descending levels for both ops.
  A request carrying a `decision_id` continues to honour that decision's
  recorded chosen tier instead of deciding twice (§9.1).
  An `account_for` carrying `decision_id` returns `service_tier_chosen` equal
  to the decision's chosen tier when that tier is non-standard, whether or
  not the request repeats `service_tier`.
- [pinned] Report usage gains optional `service_tier_applied` and
  `service_tier_served`. Applied is the abstract level actually requested,
  present for non-standard and omitted for standard. Served is present only
  on a reported wire: a runner discovery entry with `served: "reported"`,
  permitting metadata only when that response states it (runner CONTRACT §9.3.2).
  Map known values
  to `standard`, `fast` or `ultrafast`, otherwise keep the value verbatim.
  Never copy applied into served. Prefer a provider-stated served tier; its
  price stays unconfirmed if that tier or its rate is unrecognised. Without a
  served statement, applied gives only an unconfirmed estimate; absent applied
  means standard. Keep unfamiliar strings rather than inventing a rate.
  Both new members are omitted when absent, preserving today's report bytes.

- [pinned] Requests use routing's local `ServiceTier` enum. Unknown tier-aware select or account-for
  levels are refused `invalid_request`, detail
  `{field: "service_tier", reason: "unsupported_tier"}`, not as a generic
  decode error. All tier values in records, including `service_tier_chosen`
  and report usage, are strings: readers preserve unfamiliar levels. Producers
  use `ServiceTier::as_str()` and producer-side validation of the known level
  set; applied usage permits only `fast`/`ultrafast`, with standard omitted.
  `select::check_answer` is stricter than record decoding: it also refuses an
  unrecognised `service_tier_chosen` with `AnswerProblem::ServiceTier`, as well
  as a missing chosen level on an explicit request or an upgrade. Strict report
  member names remain unchanged; accepting a new level is not accepting a new
  member. Drift tests compare request spellings and the wire API list against
  the runner contract, which is a dev-dependency only.
  `account::parse_account_for_request_serving` validates tiers with the same
  named-field refusal helper when `service_tier` is declared. Without it,
  `parse_account_for_request` and the serving parser keep legacy behaviour:
  that member is ignored. Shared `service_tier::check_chosen` validates a
  produced chosen tier: it must be a recognised level, a requested level caps
  it, and without a requested level any recognised level passes, because tier
  settings may choose it. An `account_for` producer passes standard, or the
  decision's chosen tier, as the effective request. Record decoding preserves
  unfamiliar tier strings.

### 5.6 Independently gated select members

- [pinned] Each member below is sent only with its named group. A 0.7.0
  serving parser without the group refuses `invalid_request {field}` naming
  that member; null means absent. `select::parse_select_request_serving`
  takes the declared groups; the legacy parser supplies none.

| Member | Group | Meaning and validation |
|---|---|---|
| `pin_slot` | `select_pin_slot` | A §1.2 token naming an operator-policy slot. Sending both `pin` and `pin_slot` is `invalid_request {field: "pin_slot"}`. |
| `available_routes` | `select_available_routes` | Non-empty list of `provider/model` route keys reachable by the caller's runners. Malformed entries or an empty list are `invalid_request {field: "available_routes"}`; repeated entries are allowed. |
| `prefer_prompt_cache` | `select_prefer_prompt_cache` | Boolean; breaks ties among adequate routes only, never admits a route. |
| `settings` | `select_settings` | `{project_id?, tier_consumer?}`; any other member is `invalid_request {field: "settings.<member>"}`. |

- [pinned] `pin_fallback` requires `pin` or `pin_slot`, otherwise it is
  `invalid_request {field: "pin_fallback"}`. `pin_fallback: "route"` and
  `pin_outcome` apply equally to both pin forms. `pin_outcome` is present
  exactly when either was sent, and `fell_back` carries `detail.pin_reason`.
  A `pin_unavailable` refusal names the slot as `pin`; its reason is
  `unknown_slot` for an unknown slot, otherwise the reason pinning the slot's
  route would get.
- [pinned] `available_routes` narrows routes independently of `runners` and
  `exclude`. The answered route key must be in the list.
- [pinned] `settings.project_id` is an identity-service project id, not a
  folder. The router finds the project's settings itself. A different project
  stamp is refused as §3 says. `tier_consumer` is 1–128 bytes and changes only
  whose service-tier settings are looked up, not the selecting consumer.
- [pinned] `SelectAnswer.project_settings` is present exactly when the request
  sent `settings.project_id`. It is an open set: `applied`, `none` (no project
  settings), `unresolved` (the project could not be found). An unresolved project
  never refuses selection: select as if it had no settings.
- [pinned] `select::check_answer` holds the reachable-route, pin and
  `project_settings` presence rules. Prompt-cache tie-breaking is router policy
  and cannot be checked from an answer.

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
pool?, eligibility?, request_input_tokens?, refusal_class?}`.

- [pinned] `route.report` has no `limit_window` member: an account limit wall
  (a quota window or a `credit` wall) is reported only through
  `route.account_observe` with `account_limit_exhausted` (§9.3), and a runner
  does not report it through `route.report`.
- [pinned] `outcome` is an open set: `completed`, `refused`, `failed`,
  `timed_out`, `cancelled`. `refused`, `failed` and `timed_out` are
  failures; `completed` and `cancelled` are not. An unknown outcome is kept
  and counts as a failure, so a provider never keeps preferring a route on
  a report it cannot read (`Outcome::is_failure`).
- [pinned] With `report_outcomes`, four additional values are named:
  `contract_error` (a pre-send refusal naming an invalid caller field),
  `session_error`, `interrupted`, and `launch_failed` (the run never reached
  the provider for another reason, such as runner admission or setup failure).
  A provider's refusal at launch is `failed` with its `runner_code`, not
  `launch_failed`. Failures before a provider send must not burn candidates.
  `Outcome::effect()` pins this matrix; `is_failure()` equals its availability
  penalty:

  | Outcome | Availability penalty | Affinity (§7) |
  |---|---|---|
  | `completed`, `cancelled` | no | kept |
  | `refused`, `failed`, `timed_out` | yes | ended |
  | `contract_error`, `session_error`, `interrupted`, `launch_failed` | no | kept |
  | unknown string | yes | ended |

- [pinned] `report::parse_report_request_serving(params, capabilities)` decodes
  those four strings as named variants only with `report_outcomes`; without
  it they remain `Outcome::Other` and failures as in 0.6.0. The legacy parser
  supplies no groups. Callers send `cancelled` instead of these four values
  to non-declaring routers. Declaring routers apply the effect matrix for
  availability and affinity; unknown strings retain the old failure effect.
- [pinned] `runner_code`: the runner's typed refusal or error code,
  verbatim, so the provider can tell "this route cannot serve this" from
  "this attempt failed". It is a record, not a vocabulary a router must
  parse: a refusal the router must act on travels as `refusal_class`.
- [pinned] `refusal_class` (`report::RefusalClass`), with `report_outcomes`:
  what kind of refusal this is, mapped by the runner from its provider's own
  wording. A closed set:

  | Value | Meaning | Router use |
  |---|---|---|
  | `credential_refused` | The provider refused this credential for any use: an entitlement or attestation refusal, or billing, credit or quota exhausted on the account. A subscription's spending wall reported as §9.3's `credit` is not this. | Blocks the credential. |
  | `request_too_large` | The request was over the provider's size limit. Requires `request_input_tokens`. | Records a refusal for that request size, not for the account. |

  It goes only with outcome `refused` or `failed`, and never with
  `eligibility`; either combination is `invalid_request {field:
  "refusal_class"}`. `request_too_large` without `request_input_tokens` is
  `invalid_request {field: "request_input_tokens"}`. Without the group it is
  `invalid_request {field: "refusal_class"}`. When `refusal_class` is
  present, the router acts on it and on nothing it might infer from
  `runner_code`. When it is absent, anything a router infers from
  `runner_code` is its own policy, outside this contract.
- [pinned] `credential_id` and `reset_at_ms`: when the runner reported them.
- [pinned] `pool`: the pool the refused or served model drew from (§2).
- [pinned] `eligibility`: `{scope: "model" | "account", code: string}`,
  sent instead of exhaustion when the model provider says the account
  cannot serve this model (`model`) or cannot serve at all (`account`).
  `scope` is a closed set (§2): any other value is
  `invalid_request {field: "eligibility"}`, never read as `model`. Routing
  never treats eligibility as exhaustion and never holds on it.
- [pinned] Eligibility with an exhaustion `runner_code` or with
  `reset_at_ms` is `invalid_request {field: "eligibility"}`. Exhaustion
  codes are `account_limit_exhausted` (§9.2), `account_pool_exhausted`
  (§9.1), `quota_exhausted` and `credential_billing_exhausted` (§5.3), when
  relayed as `runner_code`. A reset time implies a hold.
- [pinned] `request_input_tokens`: non-negative integer, the request's
  input size in tokens. A size-shaped refusal fences only that request-size
  class, not smaller requests that can pass.
- [pinned] `usage`: `{run_id?, input_tokens?, cached_input_tokens?,
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
  idempotency: without `run_id`, a repeat with different usage is a different
  report and conflicts with the first, as specified below.
- [pinned] `usage.run_id`, with group `report_usage_run_id`, is an opaque id:
  1–128 characters, with no control characters. Without the group, a report
  carrying the member is `invalid_request {field: "usage.run_id"}`, never
  silently ignored. With the group, an invalid id is refused by that same
  field path. Absent means the existing per-decision reporting rule below.
  For routers that keep usage per run, different run ids on one decision each
  count, including when one decision carries several runs. A repeated
  `(decision_id, run_id)` counts once: the first report stands; later reports
  for that pair are accepted and ignored, even if their body differs. Among
  a decision's reports that carry `usage`, either all carry `usage.run_id` or
  none do: once one of each kind would exist, the later one is
  `report_conflict`. A report without `usage`, such as an outcome-only
  `cancelled` after runs that reported usage, takes no part in that rule and
  never conflicts with run-attributed reports. Reports without `usage` keep
  the per-decision idempotency rule below among themselves.
- [pinned] Where a router's usage comes from. A decision owner's report
  `usage`, when it sends one, is the only per-decision token and timing total
  this contract gives a router. Statistics meant to cover every run, such as
  per-task or per-workload token profiles, are built from it. A decision with
  no report, or a report without `usage`, is unrecorded, never zero. This
  contract doesn't oblige a decision owner to report usage. With `run_id`, the
  totals are attributed per run as above instead of treating the decision as
  a single run.
- [pinned] Token totals never establish that an account is exhausted or when
  it resets. That evidence comes from account observations (§9.2), the
  router's own provider readings, and this section's report members: an
  exhaustion `runner_code`, `refusal_class`, and `reset_at_ms`.
- [pinned] A router may also read the live step hints of runners that
  declare `step_events` (`cortexkit-role-llm-runner` §14.3). A hint may carry
  `route_decision_id` and the model call's content timings, each only when
  the runner knows it. Hints aren't stored or replayed on the bus, so a router
  that isn't listening never receives the ones it missed, and this contract
  gives it no other read of them. A router therefore uses hints only for its
  own statistics where missing some hints lowers precision without making a
  result wrong, such as a speed estimate, and never in place of report usage.
  This contract defines no speed estimate and no use of one in selection.
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
- [pinned] Without `usage.run_id`, a report is idempotent on `decision_id`:
  the same body again returns exactly the first answer; a different body after the first is
  `report_conflict`. Bodies are compared as decoded, so key order, and a
  member sent as `null` instead of left out, never make bodies differ.
  `pool`, `eligibility`, `request_input_tokens` and `usage` are part of the body.
- [pinned] Refusals: `invalid_request {field}`, `unknown_decision`,
  `report_conflict`.

## 9. Runner ops

- [pinned] `route.account_for` and `route.account_observe` are called only
  by attested runner modules or §13's attested media providers, except the decision-owner arm of §9.2's
  observation authorization. A caller is an attested runner module when its
  route carries a principal the daemon verified itself (`reserved:<module>`,
  proven by the launch secret the daemon handed that module), and the
  provider knows that module as a runner. Outside the §9.2 exception, any other
  caller, a `direct` client included, is refused `account_not_authorized`.
  They are separate from select, with
  their own refusals, so a consumer that only selects never depends on
  them.
- [pinned] The calling runner comes from the route; neither op carries a
  `caller` member.
- [pinned] `select_engine.media_admission`: An attested media provider has a
  daemon-verified route principal and a valid, nonce-verified
  `media-provider/v1` catalog slot. It may call `route.account_for` and
  `route.account_observe`. Its `account_for` is limited to the `(provider, model)`
  of a remote engine in that slot; any other pair is `account_not_authorized`,
  checked before decision lookup. Within the slot, an admission without
  `decision_id` is allocated by the router itself, as for a runner.
- [pinned] Accounts and credentials are named by public id only, never
  secret material.

### 9.1 `route.account_for`

The account a runner uses for a model call on a provider whose accounts
the routing provider allocates (subscription logins).
`account::AccountForRequest`, validated by
`account::parse_account_for_request`:

- [pinned] `provider`, `model` (required, non-empty): the route about to be
  called, as routing names it (§2), never the runner's own name for the
  vendor. A runner acting on a select answer sends exactly that answer's
  `route.provider` and `route.model`. The route's provider owns the accounts
  (the answer's `account.provider`), and `decision_id` binds to it below. On a
  key-pool route that provider is routing's pool owner, which can differ from
  `runner.model.provider`. `runner.model` names what the runner sends the
  model call with (§5.2) and never goes in this request.
- [pinned] `service_tier` (optional): `standard`, `fast` or `ultrafast`.
  Sent only to a router declaring `service_tier`, with the exact plan
  eligibility, ordered step-down and named refusal rules of §5.5. Only an
  account whose plan includes the chosen level is pinned. With a
  `decision_id`, the decision's already chosen level is honoured, not
  independently replaced. Without a tier request, standard may be omitted
  from the answer to preserve legacy bytes.
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
  One exception, for a run moved by `account_continue` (§9.1.1), on a router
  that declares `account_continue_resume` in `role.describe.capabilities`
  (only together with `account_continue`): once an offer for that admission
  is used, an `admit` repeating the started admission's id (a runner resuming
  the paused run) answers the run's current account, the last used offer, not
  the stored first answer, which names the original account the run paused
  on. An admission never moved replays its first answer byte for byte. A
  runner relies on this only with a router that declares it; against any other
  router it keeps the moved account from its own durable record.
  Bodies are compared with defaults applied
  (`AccountForRequest::same_body`): a member left out and the same member
  sent with its default value are the same body.
  `pool`, `parent_conversation_key`, `service_tier` and `warm_credential` are
  part of the body; an absent tier and explicit `standard` compare equal.
  The same id with a different hint is `admission_conflict`. Omitting
  `warm_credential` leaves existing request bodies and fingerprints unchanged.
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
- [pinned] A router never admits a fresh run onto a credential the vault
  reports as needing sign-in. Affinity, an explicit pin or a warm hint cannot
  override this eligibility rule.
- [pinned] `requested_account` (optional): `{account_id?, credential_id?}`,
  a hint from an explicit pin; `account_id` wins when both are present.
- [pinned] `warm_credential` (optional): a public vault credential id, using
  the non-empty opaque string rule of §9.2's `credential_id`. It is valid
  only in `mode: "admit"` (including the default mode); in any other mode it
  is `invalid_request {field: "warm_credential"}`. A runner sends it only to
  a router declaring `account_warm_credential` in `role.describe.capabilities`.
  A non-declaring router refuses it as an unknown member with that same
  named refusal: v1 refuses unknown members (§2).
- [pinned] The router reads `warm_credential` only when it holds no affinity
  for `conversation_key`, in practice the conversation's first admission.
  The hint never overrides an existing affinity, an explicit pin, a hold or
  a reserve. The hinted credential must pass the same hard filters and quota
  check as a cold pick. Otherwise the router ignores the hint and records
  its rejection; an unknown or ineligible hinted credential is never itself
  a refusal. When honoured, the hinted account becomes the conversation's
  affinity exactly as a cold pick would.
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

Informative runner guidance: pass the credential the host or plugin last used
for the session when the runner has no routing history for the conversation,
typically when a plugin first starts asking routing for accounts.

The answer (`account::AccountForAnswer`):

```
{
  admission: {id, state},                          // id is the request's admission_id
  account: {provider, account_id, credential_id, method},
  pin: {id, generation, affinity_expires_at_ms, admission_valid_until_ms,
        lease_valid_until_ms, mode},
  reason: {code, decision_id, policy_version, quota_fetched_at_ms?, quota_age_band},
  service_tier_chosen?: string
}
```

- [pinned] `admission.state` is an open string for logs, chosen by the router;
  neither `admitted` nor any other state word is required. Successful admissions
  are identified by the matching admission id, account provider, and decision id
  when the request carries one.
- [pinned] The runner renews before `lease_valid_until_ms`, and reassigns
  after the account fails.
- [pinned] Top-level `service_tier_chosen` records the admitted tier, as an
  open string like select's record, not an additional pin member. An explicit
  tier request always receives it, including fallback to `standard`. A
  caller applies the chosen tier, never the original tier when it stepped
  down. With no tier request, absence still means standard on a declaring
  router; absence on a non-declaring router does not confirm a tier request.
  `AccountForAnswer` and `AccountPin` are non-exhaustive Rust types in 0.6.0;
  construct both types with `new` instead of literals.
  `with_service_tier_chosen` is available on `AccountForAnswer` only.
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

### 9.1.1 `account_continue`: continuing a paused run

- [pinned] A router opts in by declaring `account_continue` in
  `role.describe.capabilities`. A runner sends `route.account_for` with
  `mode: "continue"` only to a declaring router. A non-declaring router
  refuses `invalid_request {field: "mode"}`, as for an unknown mode, never
  as a continuation answer. Legacy account requests, replies and their parser
  remain unchanged. The additive `account_continue::ContinueRequest` and
  `parse_continue_request(params, describe)` handle only this mode.
- [pinned] A router opts into sign-in continuation by also declaring
  `account_continue_signed_out`, only together with `account_continue`.
  A runner sends a `signed_out` continuation only to a router declaring both;
  unknown support counts as no. A router without the additional capability
  refuses `invalid_request {field: "exhausted.limit_window"}`, exactly as for
  any unknown window.
- [pinned] Only the runner itself, on its own routing route, sends this
  request, for a run it has durably paused on an account usage limit or,
  under `account_continue_signed_out`, for sign-in. The
  session's owner and plan composer never send it; the router answers it.
  The route authenticates the runner under §9; no body member supplies
  caller identity.
- [pinned] Sign-in continuation requires a definite signal: the vault reports
  that the credential needs sign-in, or the provider refuses with an
  authentication-class 401 or 403 after a freshly loaded token. These are
  the same signals that pause a run for sign-in. An unreachable vault,
  transient error or timeout never triggers it. A run already paused for
  sign-in may ask once per pause, with a new `selection_id`, when the runner
  next re-checks that pause; transport retries still keep the same id.
- [pinned] The request is `{mode: "continue", run_id, admission_id,
  conversation_key, previous_decision_id, exhausted, selection_id}`.
  `admission_id` always stays the original admission, not a new one.
  `previous_decision_id` names the decision the run is currently using:
  the admitted decision, or, after an earlier switch, the offered decision
  it switched to. The router
  binds these to the paused run and takes provider, auth method, model and
  owner scope only from that decision, never changing them.
- [pinned] Owner scope is the owner derived under §3 and stamped on the
  previous decision, together with that decision's scope stamp. An offered
  decision must keep the same owner and an exactly equal scope stamp: the
  same present or absent project id, workdir, session and run values.
  Unscoped selection keeps the selecting module as owner (§3); the runner's
  request body never replaces the stamped owner or scope.
- [pinned] `exhausted` is `{credential_id, account, limit_window, pool?,
  resets_at_ms?, retry_after_s?, observed_at_ms}`. `account` is the public
  account id, with the meaning and string type of §9.2's `served_account_id`;
  `credential_id` likewise uses that report's public string id. Window and
  pool are strings and times are unsigned integers, reusing §9.2's evidence
  types. `limit_window` must be `five_hour` or `seven_day`, or `signed_out`
  under `account_continue_signed_out`; `observed_at_ms`
  is the original observation time in Unix milliseconds. `resets_at_ms` is
  Unix milliseconds, and `retry_after_s` is a duration in seconds. The router
  records this evidence too, not merely the selection request. `signed_out`
  is the one value of `limit_window` that is not a usage window: it names why
  the account cannot serve. For `signed_out`, `resets_at_ms`, `retry_after_s`
  and `pool` must be absent, not even `null`; a present member is
  `invalid_request {field: "exhausted.<field>"}`. `observed_at_ms` remains
  required. The router treats that credential as unusable for new selections
  and offers until the vault reports it usable again, not until a reset.
  This record is only for quota exhaustion or the definite sign-in signals
  above, not arbitrary observations: a `credit` wall (§9.3) still does not
  trigger continuation.
- [pinned] Required ids are non-empty opaque strings. `selection_id` has
  the same syntax as `admission_id`: at most 128 characters, no parsing or
  case folding. `admission_id` and `conversation_key` retain §9.1's bounds.
  Pools retain §2's syntax. Malformed requests are `invalid_request` naming
  the field (nested evidence uses `exhausted.<field>`).
- [pinned] `selection_id` is stable across transport retries of the same question,
  scoped to the calling runner. A new id means a fresh eligibility check.
  A transport failure is not any continuation answer; retry it with the
  same id.
- [pinned] The same question means equal parsed `run_id`, `admission_id`,
  `conversation_key`, `previous_decision_id` and `exhausted`
  (`ContinueRequest::same_question`). Reusing a selection id with any of
  these different is refused before replaying a cached answer, using the
  ordinary refusal envelope: `invalid_request {field: "selection_id"}`.
  The first answer stands and is not replaced. Comparing after parsing means
  absent and `null` optional quota evidence members are equal (§2); those
  members are forbidden for `signed_out`. A different `limit_window` is a
  different question, including changing a usage window to `signed_out`.
- [pinned] A valid, authorized continuation request has exactly one of three
  `account_continue::ContinueAnswer` outcomes. The wire tag is `kind`:

  ```json
  {"kind":"offer","credential_id":"credential-b","account":"account-b","decision_id":"decision-b","valid_until_ms":1900000000000}
  {"kind":"hold","routing_code":"account_pool_exhausted","detail":{},"until_ms":1900000000000}
  {"kind":"refuse","routing_code":"account_selection_disabled","detail":{}}
  ```

- [pinned] An `offer` authorizes switching only this run to that credential
  and account pair. It always has the same provider, auth method, model and
  owner scope as the previous decision. Never cross API-key/subscription
  boundaries, providers, models or owner scopes; never change the run's
  frozen session plan (the llm-runner contract's §10.1), tool set or renderer.
  Never offer the exhausted account for this selection. For quota exhaustion,
  never offer it before its reset; for `signed_out`, never offer it until the
  vault reports the credential usable again. Never offer an account under
  a known hold for the relevant window or pool.
- [pinned] Reserve an offer once per `selection_id`. A retry with the same
  id returns the identical offer, including its decision and expiry, and
  never reserves twice. The offered `decision_id` links to
  `previous_decision_id`, so pricing and per-task records follow the switch.
- [pinned] The runner validates that `valid_until_ms` has not passed and
  that it can load the offered credential for the decision's provider and
  auth method, then durably records the switch before any send. The runner
  must not use an expired offer, and asks again with a new id instead.
- [pinned] The runner marks the offer used in one of three ways, and the
  router treats the first one it receives before `valid_until_ms` as use:
  - a `route.report` under the offered `decision_id`;
  - a `route.account_observe` `admission_started` under the run's original
    `admission_id` and `run_id`, naming the offered `credential_id` and
    `served_account_id`, sent after the switch is durably recorded and
    before any `attempt` observation on the new account;
  - as a backstop for a lost frame, the first `attempt` observation under
    that admission and run naming the offered account.
  Marking used is idempotent. An `admission_started` or `attempt` naming an
  account with no open offer for that admission and run (expired, released
  or superseded) is recorded as an observation only. It never creates or
  revives a reservation, and never marks a released offer used. An offer
  not used by `valid_until_ms` is released.
- [pinned] Observations after a move stay under the original `admission_id`
  and name the account in use; they carry no decision id. One `run_finished`
  under the original admission ends the run's lease and every reservation
  the run holds, the original one and each used offer alike.
- [pinned] `hold` is not an error: no compatible account is available right
  now. It carries `{until_ms?, routing_code, detail}`. The run stays paused;
  the runner asks again at `until_ms` or on its own wake, with a new selection
  id for a fresh check. Absent `until_ms` means unknown, never "now".
  `ContinueAnswer::wait_until_ms` preserves that distinction.
  For `signed_out`, the run waits for another compatible account, never for
  the signed-out account; it asks again at `until_ms` with a new id.
- [pinned] `refuse` carries `{routing_code, detail}`: the router will not
  offer a switch for this pause. Examples are a caller-pinned account—an
  admission whose `requested_account` came from an explicit pin—
  (`routing_code: "pin_unavailable"`, `detail.reason: "caller_pinned"`),
  router policy forbidding failover (`"pin_unavailable"`, `"failover_forbidden"`),
  an unknown or mismatched previous decision (`"unknown_decision"`), or
  selection disabled (reuse `"account_selection_disabled"`). These are
  continuation outcomes, not the legacy refusal envelope. The run stays
  paused on the original account. For quota exhaustion it resumes only after
  that account's own reset passes; for `signed_out` it stays paused for
  sign-in, waiting for a person as before. The runner does not ask again for
  this pause. A later
  pause of the same run may ask again with a new selection id.

### 9.2 `route.account_observe`

What a runner saw on an account, so the provider's headroom stays current.
`observe::AccountObserveRequest`, validated by
`observe::parse_account_observe_request`:

- [pinned] `event_id` (required, non-empty): the idempotency key, per
  calling principal (per runner without `observe_backoff`). A runner uses a
  stable id for each event, such as a position in its own log.
- [pinned] `kind` (required): an open set, `admission_started`,
  `run_finished`, `attempt`, `scoped_not_found`, `account_limit_exhausted`.
  An unknown kind is kept as advisory.
- [pinned] Optional: `admission_id`, `run_id`, `served_account_id`,
  `credential_id`, `view_digest`, `observed_at_ms` (the original
  observation time, never a forwarding time), `upstream_origin`,
  `limit_window`, `resets_at_ms`, `resets_at_source`.
- [pinned] For `account_limit_exhausted`, `limit_window` can name a quota
  window (for example, `five_hour` or `seven_day`) or `credit`, an
  account-level spending or credit wall (§9.3). For `credit`,
  `resets_at_ms`, `resets_at_source` and
  retry hints (such as Retry-After) remain optional; quota-window reset
  evidence rules are unchanged. Observation readers must accept `credit`
  before runners send it (readers first); it is not a continuation window.
- [pinned] `upstream_origin` (`observe::UpstreamOrigin`) is a closed set
  naming where the observation's headers came from. The router uses it for
  one decision: whether a header reading counts as evidence that the account
  is exhausted.

  | Value | Meaning | Evidence |
  |---|---|---|
  | `upstream` | The provider's own response headers, as the runner received them, whatever the transport. Send it when the runner can't say more. | yes |
  | `direct` | The provider's response, received straight from its HTTP API. | yes |
  | `http_relay_upstream` | The provider's response, passed through unchanged by an HTTP relay or proxy in front of the provider. | yes |
  | `ws_response_start` | The headers on a WebSocket stream's response-start frame. | yes |
  | `optimistic_wrapper` | Headers a relay or wrapper made up itself, such as a placeholder sent before the real response. | never |

  For a genuine origin, a utilization reading of 1.0 or more can confirm the
  window is exhausted. Without `upstream_origin`, header readings are
  recorded but can't confirm anything. The value names a kind of source,
  never a URL or host.
- [pinned] `resets_at_source` (`observe::ResetsAtSource`) is a closed set
  naming where `resets_at_ms` came from.

  | Value | Meaning | Router use |
  |---|---|---|
  | `header` | The provider stated the window's reset time in a response header. | The reset that ends the block. |
  | `body` | The provider stated an absolute reset time in a refusal body. | The reset that ends the block, like `header`. |
  | `retry_after` | Worked out from a relative delay, such as a Retry-After value or a body's seconds-until-reset. | Only a hint for when to try again. It never starts a short hold or ends a block. |

  Without a source, the router reads `resets_at_ms` like `retry_after`.
- [pinned] Both sets are closed: a value the set doesn't name is
  `invalid_request` naming `upstream_origin` or `resets_at_source`, so a
  runner learns at once. A runner building the request with this crate's
  types can't send one. Adding a value needs a describe group, because a
  router that doesn't know it refuses it.
- [pinned] `pool` (optional): the quota pool the observation is about (§2):
  for a reading, the pool its headers describe; for
  `account_limit_exhausted`, the pool that ran out.
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
- [pinned] The same `event_id` from the same calling principal again applies
  nothing and answers the first observation's `id` with `duplicate: true` and
  empty `effects`.
- [pinned] Refusals: `invalid_request {field}`, `account_not_authorized`.

- [pinned] `observe_backoff` adds `kind: "transient_backoff"` and optional
  `decision_id`, `retry_at_ms` (Unix ms), and `provider_code` (1–128 characters,
  verbatim). `retry_at_ms` is required for `transient_backoff`, allowed for
  `account_limit_exhausted`, and refused for every other kind, including
  `credential_refused`, by that field.
  `observe::parse_account_observe_request_serving(params, capabilities)`
  refuses each new member by name without the group, keeps the new kind as
  advisory `ObserveKind::Other`, and applies no retry rule. The legacy parser
  supplies no groups. A 0.6.0 parser ignores unknown observe members instead;
  callers send none of these members or the new kind to either non-declaring
  router.
- [pinned] `transient_backoff` fences the admission's account when an
  `admission_id` is sent, otherwise `served_account_id`. With neither, refuse
  `invalid_request {field: "admission_id"}`. New admissions on that account
  stop for every runner until `retry_at_ms`. Past times are recorded with no
  fence; times over 24 hours out are capped at 24 hours (`RESET_FENCE_CAP_MS`).
  The current pin stays; only `run_finished` releases it.
- [pinned] `observe_credential_refused` adds `kind: "credential_refused"`, with
  the same meaning as report `refusal_class: credential_refused` (§8). It names
  `credential_id`, or else the admission's credential through `admission_id`;
  with neither, refuse `invalid_request {field: "admission_id"}`.
  `provider_code` is allowed verbatim; `retry_at_ms` is refused by that field.
  The serving parser keeps the kind as advisory `ObserveKind::Other` without
  the group and applies nothing; the legacy parser supplies no groups.
  It blocks that credential for every runner until an operator clears it or
  a router-chosen, bounded re-probe horizon passes. The router documents the
  horizon; the next selection after it may try the credential again. A later
  `credential_refused` for the same credential extends the block and never
  shortens it. A refusal such as an exhausted prepaid balance or a revoked
  key does not clear by waiting, so it is never treated as a resettable
  limit. A subscription's spending or credit wall, which renews on the same
  credential, uses `account_limit_exhausted` with `limit_window: "credit"`
  instead (§9.3).
- [pinned] On a declaring router, resolve `ObserveArm` from trusted state and
  call `observe::check_observe_authorization(arm, request)`:
  - `Runner`: the caller holds the admission or is otherwise authorized by
    §9's runner rule. All kinds are allowed. `decision_id` is informative and
    never refused: it may belong to another consumer or be an `account_continue`
    offer the admission never carried. Authorization follows the admission,
    not decision ownership, so `run_finished` can still release the pin.
  - `DecisionOwner {decision_id: D, admission_id, carried}`: the caller owns
    a select or panel decision D. Only the kinds that block allocation
    (`account_limit_exhausted`,
    `transient_backoff`, `credential_refused`) are allowed, only naming D and
    that admission, and only if the admission's `account_for` carried D. A
    decision owner cannot release or reshape a runner's pin.
  - `Other`: neither arm, including foreign or non-select decisions from a
    non-runner; refused.
  Every authorization refusal is `account_not_authorized`, never
  `unknown_decision`. Non-declaring routers retain runner-only authorization.
  The key is the calling principal and `event_id`, never the role. So a
  runner and a decision owner that are different principals don't collide
  on the same `event_id`, but one principal acting in both roles must use a
  different `event_id` for each observation, or the second is a duplicate.

### 9.3 Pooled API keys and runner-owned account routing

- [pinned] Pooled API keys need no second lease mechanism. The select answer's
  `runner.auth` (`select::RunnerAuth`) carries pool and credential hints, never
  an account lease pin. The runner admits, renews (including keeping prompt-cache
  affinity), and releases its own account through the existing operations:

  | Account action | Routing request |
  |---|---|
  | acquire | `route.account_for`, `mode: "admit"`, with `pool` |
  | renew | `route.account_for`, `mode: "renew"`, with `expected_pin` |
  | release | `route.account_observe`, `kind: "run_finished"` |
  | block (stop allocating an account or credential) | `route.account_observe`, `account_limit_exhausted` or, with their groups, `transient_backoff` or `credential_refused` |

- [pinned] `account_limit_exhausted` with `limit_window: "credit"` reports
  a subscription account's spending or credit wall with no known reset. It
  clears when credits renew or the user tops up, using the same valid
  credential. An exhausted prepaid API-key balance is not this; it stays
  `credential_refused`. Routing treats it as a non-quota account fence for at most one hour, then
  allows one probe. A quota reading never lifts this fence: a credit wall
  can coexist with an unused quota window. It is never a credential refusal,
  since the credential remains valid. Readers must accept `credit` before
  runners send it (readers first).
- [pinned] A runner declaring `routed_accounts` in its runner-role describe
  lists the sign-in methods it admits, renews and releases itself. Entries
  use §1.2's `auth_method` spelling and token grammar. The list is non-empty,
  unique, and at most 64 entries (`check_routed_accounts` in the runner
  contract). Any listed method requires `follows_account_choice: true` in the
  routing runner declaration. That flag without the group keeps its 0.6.0
  meaning.
- [pinned] A runner declaring `routed_accounts` that learns the provider refused
  the admitted credential for any use sends `credential_refused`, naming the
  admission, before releasing it with `run_finished`. It sends it only to a
  router declaring `observe_credential_refused`, and only from the provider's
  structured error (a code or type), never inferred from message text, which
  can quote a user. When a provider signals the case only by a fixed message
  under one structured type, the runner may match that message, provided the
  match is checked only under that type (and HTTP status), covers the whole
  message as an exact string or a fixed template whose variable parts are
  provider-generated values (a reset time, a key prefix) parsed strictly, and
  never searches for a substring or keyword. A message that does not match
  exactly yields no blocking observation. Limits that reset stay
  `account_limit_exhausted` or `transient_backoff`. For an API key, an authentication refusal that
  persists after the runner's own retry (the key is invalid or revoked)
  counts as refused for any use. For a sign-in method such as OAuth, the same
  refusal means the sign-in must be renewed: the runner pauses for sign-in and
  does not send `credential_refused`. Under `account_continue_signed_out`,
  the runner may instead move the paused run via §9.1.1's continuation
  exchange, only on that section's definite sign-in signals.
- [pinned] Blocking outcomes are reported by the party holding the admission.
  A runner that holds an admission (through `routed_accounts`, or by calling
  `route.account_for` itself) sends `transient_backoff` and
  `account_limit_exhausted` for it, and `credential_refused` as above. When the
  provider's structured error gives no retry time, `retry_at_ms` for
  `transient_backoff` is the final attempt's receipt time plus 5,000 ms. The
  runner contract's `admission_reports` describe group (llm-runner 0.8.28)
  tells a session owner which runners do this. For their runs the session
  owner sends no observation of the runner's outcome and keeps no local account
  block of its own; the `DecisionOwner` arm (§9.2) remains for what only the
  decision owner observes.
- [pinned] For listed methods the runner forwards the select pool and
  credential hint as §5.2 specifies, loads the public-id credential answered by
  `account_for`, and sends with it. For unlisted methods, or without the group,
  the caller keeps naming the account as the runner accepts it today and §5.2's
  original key-pool requirement applies. No credential secret travels here.

## 10. Refusal codes

| Code | Answered by | Detail | Timed |
|---|---|---|---|
| `invalid_request` | every op except `role.describe` | `{field}` | no |
| `invalid_requirements` | `route.select` (§5.1), `route.select_panel` (§11.1), `route.select_engine` (§13) | `{field}` | no |
| `unknown_consumer` | `route.select`, `route.select_panel`, `route.select_engine` | `{consumer}` | no |
| `consumer_not_owned` | `route.select`, `route.select_panel` (§1.1), `route.select_engine` | `{consumer}` | no (terminal) |
| `pin_unavailable` | `route.select` (§5.3) | `{pin, reason, reset_at_ms?}` | when `reset_at_ms` is present |
| `no_route` | `route.select` (§5.3) | `{reason, reset_at_ms?}` | when `reset_at_ms` is present |
| `no_engine` | `route.select_engine` (§13) | `{filter}` | no |
| `engine_not_declared_by_caller` | `route.engine_report`, `route.engine_eval` | `{}` | no |
| `unknown_decision` | `route.report` (§6), `route.account_for` (§9.1), panel replacement (§11.1), `route.budget_quote`, `route.explain` (§11) | `{}` | no |
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

- [pinned] `refusals::retry_class(code, detail)` classifies an answered
  refusal as `RetryAt(t)` exactly when `Refusal::wait_until_ms` returns
  `Some(t)`: every timed row in the table above retries at the time it names,
  an absolute Unix timestamp in milliseconds. A missing or malformed
  timestamp is untimed and permanent. `account_selection_unavailable` is
  `Transient`; every other untimed refusal, unknown codes included, is
  `Permanent`. Both helpers use the same timed-code logic.
- [pinned] `Refusal::wait_until_ms` reads the time a timed refusal names,
  and `None` for every other refusal, an unknown code included.
- [pinned] `test-vectors/routing-v1/refusals.json` holds one body for every
  code in this table.
- [pinned] `select_engine.no_engine_code`: `no_engine` has the §10 row above
  and a body in `test-vectors/routing-v1/refusals.json`; it is never timed.

## 11. Optional selection and inspection operations

- [pinned] Each operation below requires its own §4 group. New wire records
  are non-exhaustive with constructors; open sets keep unknown strings.
- [pinned] `route.budget_quote` and `route.explain` accept only decision ids
  minted by `route.select` or `route.select_panel`, replacement seats included.
  An `account_for` allocation's own `reason.decision_id` without a caller
  decision, and an `account_continue` offer's `decision_id` even after switching,
  are `unknown_decision`. This changes no id accepted by a 0.6.0 operation.
- [pinned] Ownership follows §6's selecting `(principal, consumer)`. Unknown,
  foreign and past-retention decisions are answered alike as `unknown_decision`.
  Retention is independent of `expires_at_ms`: expired but retained decisions
  can still be explained, quoted, and replaced through their panel. Displaced
  seat decisions are retained like any other decision.

### 11.1 `route.select_panel` — group `select_panel`

New-panel params (`panel::SelectPanelRequest`):

| Member | Meaning |
|---|---|
| `class` | Required open string naming the kind of review, mapped to router policy. Named values: `audit`, `design_review`, `evaluate`, `diagnose`, `explain`, `spec`, `lab`; they fix the spelling, and each router's own policy says what a class selects. `role.describe` doesn't list the classes a router serves: one it has no policy for is `invalid_request {field: "class"}`. |
| `consumer`, `runners?`, `available_routes?`, `exclude?`, `harness?`, `service_tier?`, `correlation_id?`, `task_ref?`, `settings?` | As on select, under the same groups, applying to every seat. |
| `requirements?` | `PanelRequirements {context_window?, modalities?}`: each seat has a known window at least the floor and accepts every modality. No `iq`; class policy owns quality. |
| `temperatures?` | At most 8 finite numbers in `0..=2`, indexed by slot; otherwise `invalid_request {field: "temperatures"}`. |
| `package_tokens?` | Unsigned evidence-package size; every seated route's package ceiling (§11.2) must cover it. Absent means unconstrained. |
| `seed?` | String of at most 256 bytes. Random draws depend only on seed and candidates, so identical seed and candidates give the same panel. |

- [pinned] Other members, including select's `pin`, `pin_slot`, `prefer`,
  `prefer_prompt_cache` and `affinity`, are `invalid_request {field}` naming
  them. Unknown requirements, including `iq`, are `invalid_request
  {field: "requirements.<member>"}`. A non-unsigned window or undefined
  modality is `invalid_requirements {field}` naming it, as on select.
- [pinned] Router policy owns size (1–8), ranking, extra draws and quality
  floor. Never seat the same `provider/model` twice, nor two models of one
  family; models without a known family count as one family.

Answer (`SelectPanelAnswer`):

| Member | Meaning |
|---|---|
| `panel_id` | Router-minted id used for replacement; only the panel's caller may replace. |
| `policy` | `{size, top, extra}`, size in 1–8, `top + extra = size`; policy counts, not seated counts. Slots below `top` have phase `top`, others phase `extra`. Optional `ranking`, `extra_ranking`, `floor`, `seed` are arbitrary informative JSON, never parsed for decisions. |
| `seats` | `[{slot, phase, answer, family?, temperature?}]`, ascending distinct slots below size, gaps allowed. `phase` is open (`top`, `extra`). `answer` is a full `SelectAnswer` with its own decision and report. Temperature equals `temperatures[slot]`, absent beyond that list. |
| `shortfall?` | `{code, detail?}`, present exactly when fewer than size seats were seated, including zero seats (an answer, not a refusal). Open code: `package_unmeetable`, `floor_unmeetable`, `diversity_unmeetable`, otherwise `size_reduced`. |

- [pinned] Shortfall `detail` remains arbitrary JSON on the wire and
  `Option<serde_json::Value>` in Rust for compatibility. The typed evidence
  shape is `PanelShortfallDetail {message, package_tokens?, causes}`: `message`
  is prose, `package_tokens` is an unsigned request package size (absent or null
  when unconstrained), and `causes` is an array of `PanelRejectedCandidate
  {model, reason, detail?, reset_at_ms?}`. `model` names a route or model pin;
  `reason` is an open string, not a decision discriminator. `reset_at_ms` is an
  optional unsigned Unix timestamp in milliseconds for quota/backoff causes.
  Readers ignore unknown fields in all typed evidence records.
- [pinned] Known candidate reasons emitted today are `package_too_large`,
  `package_exceeds_ceiling`, `excluded` (caller exclusion),
  `excluded_by_operator`, `unscored_awaiting_placement`, `unavailable`,
  `quota_exhausted`, `backoff_active`, `capability`, `below_iq_floor`,
  `missing_fact`, `family_unlabeled`, `family_duplicate`, and
  `deprecated_upstream`. `below_iq_floor` means the route's quality grade is
  under the router policy's quality floor for the seat (see the policy rule
  above). `family_duplicate` also covers a family held by a standing seat, and
  sibling collapse: two routes serving the same canonical model (one model
  offered by several providers or accounts) count once, and the others are
  rejected with this reason. `capability` covers context
  window or modality requirements; `unavailable` also covers routes absent from
  `available_routes` or unserved by the requested runners. Availability and
  final admission retain the exact refusal code as the reason, including
  `accounts_blocked`, `account_hold`, `auth_required`, or any other route
  refusal code, and retain its diagnostic payload; they must not be relabeled
  as diversity failures. Shortfall codes
  summarize the panel; they are not a closed vocabulary for candidate reasons.
- [pinned] Candidate `detail` remains arbitrary JSON: older package causes use
  prose strings, typed package causes use `PanelPackageFit`, and final admission
  may carry a full refusal object `{code, message, detail}`. `PanelPackageFit`
  has unsigned `package_tokens`, `budget_tokens`, `context_window`,
  `output_reserve_tokens`, `prompt_reserve_tokens`, boolean
  `context_window_known`, and nullable unsigned `catalog_max_output_tokens`.
  The window is the effective window, including the conservative 128,000-token
  default when not known. A missing or zero catalog output ceiling is unknown
  (`null`); output reserve is `min(catalog_max_output_tokens or 32,000, 64,000)`
  and prompt reserve is 16,000. The existing half-window rule is
  `budget_tokens = context_window.saturating_sub(output_reserve_tokens +
  prompt_reserve_tokens) / 2`, using integer division: the other half of the
  remaining window is reserved for panel discussion. Evidence reports the
  package calculation; it does not change seat eligibility.
- [pinned] `PanelShortfall::with_typed_detail(PanelShortfallDetail)` stores typed
  evidence in the existing JSON slot. `typed_detail()` returns
  `Option<Result<PanelShortfallDetail, serde_json::Error>>`, distinguishing
  absence from malformed or differently shaped evidence. The same pair on
  `PanelRejectedCandidate` stores/reads `PanelPackageFit`. Prose package detail
  from older providers decodes as a candidate but returns an error from the
  fit accessor; readers must treat that as no typed fit, not an invalid cause.
  All three typed records are non-exhaustive and have constructors.

Replacement params and answer:

| Body | Shape |
|---|---|
| Request (`PanelReplacementRequest`) | `{replace: {panel_id, slot}, exclude?}`, where `exclude` holds route keys as in `route.select`'s `exclude` (§5.1), never with a variant; any other member is `invalid_request {field}`. |
| Answer (`PanelReplacement`) | `{panel_id, slot, seat?, cause?, detail?}`, exactly one of seat or cause. Seat is the same shape as above with a decision id never answered by this panel. Cause is open: `no_candidate`, `package_unmeetable`. `detail` is allowed only with a cause and uses the same `PanelShortfallDetail` evidence shape; its Rust slot is `Option<serde_json::Value>`, omitted when absent. A replacement either seats a member or explains why not, never both. |

- [pinned] A replacement with a seat and non-null detail is invalid even without
  a cause. Older replies without detail decode unchanged; replacement readers
  ignore unknown fields so older readers can ignore the added detail member.
- [pinned] Unknown, foreign and past-retention panels all get `unknown_decision`.
  `replace.slot >= policy.size` is `invalid_request {field: "replace.slot"}`.
  Any lower slot may be replaced, even if never seated or last answered a cause.
- [pinned] A replacement has the slot's original phase and temperature. It
  uses a route the panel never seated before, excludes the original plus new
  exclusions, shares no family with standing seats in other slots, is available
  now, has a runner, and covers `package_tokens`. Original `runners`,
  `available_routes`, `requirements`, `harness`, `service_tier`, `settings`,
  class and package size still apply. A displaced route is never seated again.
  Keep the panel replaceable at least as long as its decisions remain explainable.
- [pinned] New-panel refusals are `invalid_request`, `invalid_requirements`,
  `unknown_consumer`, `consumer_not_owned`; replacement adds `unknown_decision`.
  Insufficient candidates produce a shortfall or cause, not `no_route`.
- [pinned] Caller helpers are not wire members:
  `SelectPanelRequest::seat_request(extra_exclude)` copies the common select
  constraints, merges exclusions, uses `Requirements::iq(0)` plus the panel
  window/modalities, and sends no pin or preference. The inert IQ is not checked
  by `select::check_answer`. `PanelState::new(request, answer)` records the
  original request, standing slots, and every seated route and decision id;
  `apply(&PanelReplacement)` installs a checked seat and extends both histories.
  A cause changes nothing. Validate before constructing or applying state.
- [pinned] `check_panel_answer(request, answer)` checks policy bounds, slot
  order/phase, temperature, unique decisions/routes/families (at most one unknown
  family), exact shortfall presence, all seat answers against `seat_request([])`,
  equal project-settings results, and package ceilings. When a window floor
  was sent, both panel helpers require non-zero seat windows at least that
  floor; modalities cannot be checked from answers.
- [pinned] `check_panel_replacement(state, replace, answer)` checks the echoed
  panel and slot, exclusive seat/cause and detail only with cause,
  phase/temperature, combined exclusions,
  no historically seated route or decision id, family diversity with other
  standing slots, and the seat answer against `seat_request(replace.exclude)`,
  window floor and package ceiling.

### 11.2 `route.budget_quote` — group `budget_quote`

| Body | Members |
|---|---|
| Request (`BudgetQuoteRequest`) | `{decisions, prompt_tokens?}`. 1–16 distinct decisions; empty, repeated or longer lists are `invalid_request {field: "decisions"}`. Prompt tokens must be unsigned, otherwise `invalid_request {field: "prompt_tokens"}`. |
| Answer (`BudgetQuote`) | `{package: {tokens, limiting: {decision_id, route_key}, context_window, context_window_known}, seats: [{decision_id, route_key, package_tokens, output_limit?}]}`; seats in request order. |
| `output_limit` | `{status: "send", tokens, catalog, suspect}` or `{status: "unknown", reason}`, present exactly when prompt tokens were sent. Status and reason are open sets. |

- [pinned] Refusals: `invalid_request {field}`, `unknown_decision` for the
  entire request if any decision is unknown, foreign or past retention (§11).
  Only the decisions' selecting owner may quote them.
- [pinned] Use capacities recorded in each select answer, not a fresh catalog
  read. `context_window` and `max_output_tokens` are required u64 fields; 0
  means unknown. `budget::quote(seats, prompt_tokens)` is the pure shared
  arithmetic helper, taking `{decision_id, route_key, context_window,
  max_output_tokens}` in request order. It also validates the decision list.
- [pinned] With saturating arithmetic, window is the known context window or
  128,000. Output reserve is min(max output or 32,000 when unknown, 64,000).
  Seat package ceiling is floor((window − output reserve − 16,000) / 2).
  Panel ceiling is the smallest seat ceiling. A limiting tie takes the
  bytewise-smaller route key, then earlier decision in request order. Package
  context window and known flag are that limiting seat's window and whether
  its capacity was known.
- [pinned] When prompt tokens are present, room is window − prompt tokens −
  4,096, saturating at zero. The first matching unknown-output reason wins:
  `no_output_limit` (unknown max output), `no_context_window` (unknown window),
  `placeholder` (max output at least the window), `no_room` (room zero).
  The caller sends no limit and the runner applies its own. Otherwise send
  min(max output, room), record max output as `catalog`, and always include
  `suspect`, true exactly when max output exceeds 131,072. A suspect limit
  is still sent. Package ceilings do not depend on prompt tokens.
- [pinned] `check_budget_quote` requires exact equality with `budget::quote`
  over the capacities the caller received for those decisions.

### 11.3 `route.explain` — group `explain`

| Body | Members |
|---|---|
| Request (`ExplainRequest`) | `{decision_id}` |
| Answer (`ExplainAnswer`) | `{decision_id, route, runner, serving: {provider, account_id?, credential_id?}, policy_version, reason_code, note?}` |

- [pinned] Only the selecting owner may explain (§11); refusals are
  `invalid_request {field}` and `unknown_decision`.
- [pinned] `serving.provider` is the route provider. Account and credential
  bindings come from the latest `account_for` admission carrying this decision,
  otherwise select's auth; unbound members are absent. An `account_continue`
  switch does not move the binding: it carries `previous_decision_id`, not
  this decision, and its offer is not explainable. Serving stays as admitted.
- [pinned] `policy_version` is a string; `reason_code` is open (`pinned`,
  `fell_back`, `affinity`, `ranked`), for logs only. `note` is informative,
  never parsed. `check_explain(select_answer, request, answer)` checks the
  decision echo, route and runner equality, and serving provider equality.

### 11.4 `route.account_status` — group `account_status`

| Body | Members |
|---|---|
| Request (`AccountStatusRequest`) | `{provider, account_id}` |
| Answer (`AccountStatusAnswer`) | `{provider, account_id, limit_state, reset_at_ms?}` |

- [pinned] Any caller may ask; the answer contains only public ids and times.
  Unknown accounts get `limit_state: "unknown"`, never an existence refusal.
  Malformed params get `invalid_request {field}`.
- [pinned] `limit_state` is open: `available`, `limited`, `unknown`. Reset is
  Unix ms, only with `limited`, absent there when unknown.
  `check_account_status(request, answer)` checks both id echoes and that reset
  rule, preserving unknown states as strings.

### 11.5 `route.head_choices` — group `head_choices`

| Body | Members |
|---|---|
| Request (`HeadChoicesRequest`) | `{runner_module, harness?}` |
| Answer (`HeadChoicesAnswer`) | `{catalog_version, runner_module, models: [{provider, model, variants, default_variant?, service_tiers, pin_only?, billing?}]}` |

- [pinned] Any caller may ask. Malformed params are `invalid_request {field}`;
  unknown runners and runners declaring `starts_sessions: false` get empty
  models, not a refusal, unless a harness was explicitly requested as below.
  The answer is unranked; empty models is valid.
- [pinned] Optional `harness` uses the same token validation and hosted-program
  check as `route.select`. A named harness not hosted by the requested
  session-starting runner is `no_route` with reason `harness_unavailable`, and
  the refusal message names the harness. A known harness uses its effective
  `harness_limits` pairs and provider allow-list. Absent/null `harness` means
  the runner-level union, not its default harness. Use routing 0.7.11 or later
  when relying on harness-specific choices; earlier parsers ignore the member
  and continue to return runner-level choices.
- [pinned] Models are bytewise sorted by `(provider, model)`, each pair unique.
  Variants and tiers are bytewise sorted and unique; a default variant must be
  in variants. Every pair is a route the named runner runs. Listing once under
  a preferred provider or once per provider is router policy; callers assume
  neither.
- [pinned] `catalog_version` identifies the catalog used. Variants and tiers
  are the router's admitted subset at that version. Limits and prices still
  come from the catalog. `check_head_choices(request, answer)` checks the
  runner echo, sorting, uniqueness and default-variant membership.
- [pinned] `pin_only` absent means false. True means the router uses the model
  only when a caller pins it explicitly, never by its own choice.
- [pinned] `billing` is the closed set `subscription` or `per_token`
  (`billing::Billing`); absent means unknown, not a refusal. An answer with
  another value is malformed. Billing is checked when decoding the answer
  before `check_head_choices`; unknown answer members remain tolerated.
  Both optional members are omitted when not set; no new group is required.

### 11.6 `route.consumer_models` — group `consumer_models`

| Body | Members |
|---|---|
| Request (`ConsumerModelsRequest`) | `{consumer}` |
| Answer (`ConsumerModelsAnswer`) | `{catalog_version, consumer, models: [{provider, model, elo?, eq?, excluded, billing?}]}` |

- [pinned] The request refuses unknown members as `invalid_request {field}`;
  `consumer` has the non-empty string grammar of §5.1. Admission is exactly
  select's §1.1 rule: a declared consumer is admitted from its declaring module
  or under a scope owned by that module. A name only in operator configuration
  is usable by any caller unless configuration names an owner. Other callers
  get `consumer_not_owned {consumer}`; unknown consumers get
  `unknown_consumer {consumer}`. Callers send this operation only to routers
  declaring its group; like the other whole optional operations, it has no
  undeclared-group refusal case (§4).
- [pinned] The answer echoes `consumer`; `catalog_version` identifies the
  catalog used. Models are bytewise sorted by `(provider, model)`, each pair
  unique. An empty list is valid. `elo` and `eq` are optional finite `f64`
  numbers, the router's own stored scores for this consumer; absent means no
  score, not zero. `excluded` is true when operator configuration excludes the
  model for this consumer. `billing` has §11.5's closed set and absence rules;
  any other value makes the answer malformed. Unknown answer members are
  tolerated. `check_consumer_models(request, answer)` checks the echo, sorting,
  uniqueness and finite scores; decoding enforces the billing set.
- [pinned] This read carries the router's own judgments only. Prices, context
  windows, modalities and model lifecycle come from the model catalog, and a
  caller reads them there.

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
  `select_exclude_providers` members. `route-select-task-ref.json` holds
  task references and field-named refusals with and without `select_task_ref`.
  `route-report-usage-run-id.json` holds run-attributed usage and group/field
  refusals. `route-consumer-models.json` holds stored judgments, consumer
  admission refusals and malformed answers. Its non-finite cases name the typed
  score to inject because JSON has no non-finite numeric literal.
  `route-account-continue.json` holds opt-in continuation requests, all three
  outcomes, an identical-offer retry, refusal reasons and malformed or
  undeclared requests (§9.1.1). `SHA256SUMS` pins every file.
- [pinned] `cortexkit-role-routing-conformance` drives a live provider
  through these rules. Each check names the rule it holds a provider to;
  the kit's own tests pair every check with a faulty provider that breaks
  only that rule and must fail it by name. A check of an optional group's
  serving rules reports not applicable against a provider that does not
  declare the group; an undeclared-group refusal check reports not
  applicable when the provider does declare it. Either leaves its verdict a
  pass. Its README lists the checks and where the kit checks less than this
  contract says.

## 13. `route.select_engine` — group `select_engine`

- [pinned] `select_engine.opt_in`: This operation is optional and declared
  together with `select_engine`. It does not add fields to `route.select` or
  to the `routing/v1` declaration slot.
- [pinned] `select_engine.declarations`: Engines are learned only from the
  daemon catalog's `role_declarations["media-provider/v1"]` slot, using
  `cortexkit-role-media` 0.1's per-engine type. The slot is an object whose
  `engines` member is required and lists the engines; a provider may also
  carry describe's other members, and readers use `engines` and ignore any
  other member. Only nonce-verified entries count. The daemon relays slots
  unread. A valid, nonce-verified slot identifies a media provider; older
  routers never read this slot and retain that module's separate routing
  declaration.
- [pinned] `select_engine.declarations_atomic`: `parse_engine_declarations`
  rejects the whole slot when `engines` is absent or an engine entry is
  malformed, including an unknown member of an engine, applying none and
  reporting the problem on health. It applies the media checker's per-engine
  checks: `duplicate_engine` for repeated `(kind, provider, model)`, `fact_too_long` for
  provider, model or list entries over 256 Unicode scalar values,
  `local_realtime_engine`, `bias_limit_invalid` for a max below 1, and
  `formats_empty`. Group-level
  describe checks do not apply to a slot.
- [pinned] `select_engine.request`: Params are `{consumer, kind, requirements,
  privacy, caller_local?}`. `consumer` follows §5.1's ownership rules:
  `unknown_consumer` and `consumer_not_owned` apply. No consumer `priced_only`,
  demand requirement or operator pin restricts engine selection. `kind` is the
  open media kind (`transcribe`, `synthesize`, `image`, `video`,
  `stream_transcribe`, `realtime`). `privacy` is closed: `local_only` or
  `remote_allowed`. Unknown top-level members or malformed non-requirement
  members are `invalid_request {field}`.
- [pinned] `select_engine.requirements_valid`: `requirements` is an object
  with optional `language`, `voice`, `format`, `duration_ms`, `input_bytes`,
  `reference_formats`, `bias`, `size`, `resolution`, `quality`, and
  `latency_class`, and `metric`. Strings, unsigned integer byte/duration counts, a MIME
  string list and a boolean bias flag respectively are required. An unknown
  member or malformed value, including an unknown latency class, is
  `invalid_requirements {field}`. `format` for `realtime` is also
  `invalid_requirements`, because realtime formats are never matched.
- [pinned] `select_engine.caller_local`: Each `caller_local` entry has the
  media per-engine shape, `local: true`, and passes the same per-engine checks
  (including unique `(kind, model)` within the list). Otherwise it is
  `invalid_request {field}` naming a dotted, indexed path such as
  `caller_local[0].local`.
- [pinned] `select_engine.answer`: The answer is `{decision_id, expires_at_ms,
  engine: {provider, model, local, caller_local, module?, index?}}`.
  `expires_at_ms` is exactly ten minutes after selection. Local means on the
  user's hardware. A module engine has `module`, `caller_local: false`, and
  no `index`. A caller-local engine has `caller_local: true`, `local: true`,
  and `index`, its position in the request list, and no `module`; the caller
  runs it in-process without a job.
- [pinned] `select_engine.filter_order`: Filters apply to the whole remaining
  set in order: `kind`, `requirements`, `privacy`, `reachability`. When a set
  becomes empty, refuse `no_engine {filter}` naming that filter; use `kind`
  when no engines are known at all. Never time this refusal.
- [pinned] `select_engine.match_requirements`: Every stated requirement must
  be met. MIME matching ignores type/subtype case and requires every parameter
  the engine's entry states to be equal. Language matching ignores case and
  accepts an engine tag equal to the requested tag or extended by it at a
  subtag boundary (`en` serves `en-GB`, not the reverse). Voice, size,
  resolution and quality require equal list entries. An absent fact list or
  limit fails a stated requirement. `duration_ms <= max_duration_ms` and
  `input_bytes <= max_input_bytes`, counted as in media: decoded audio bytes
  (cumulative for streams), UTF-8 text bytes for synthesis, summed reference
  bytes for image/video. Each `reference_formats` entry matches a declared
  reference MIME. `bias: true` requires `bias_limit`; false adds no constraint.
  Latency must be equal to or faster than requested, in order `realtime`,
  `interactive`, `batch`; an unknown engine class meets only a request with
  no latency requirement.
- [pinned] `select_engine.privacy`: `local_only` keeps only local engines,
  including caller-local and local module engines. `remote_allowed` keeps
  both local and remote engines.
- [pinned] `select_engine.reachability`: Caller-local and local module engines
  are reachable. A remote module engine is reachable unless its provider/model
  route is fenced by a cooldown, backoff, exhausted quota or recorded failure
  under §7.
- [pinned] `select_engine.ranking`: Rank by measured accuracy for the kind,
  descending, with unmeasured engines below measured ones; then catalog cost,
  ascending, with engines lacking cost facts below costed engines; then `(module,
  provider, model)` ascending, absent strings read as empty; then index ascending.
  Speed reports never rank engines. Accuracy comes only from §15's reference-set
  evaluations, never production jobs or catalog accuracy fixtures. Without
  `engine_evals`, every engine ranks as unmeasured. Cost remains a catalog fact.
- [pinned] `select_engine.metric`: `requirements.metric` is an optional open
  string overriding the kind's default metric. The default table is:

  | Kind | Metric |
  |---|---|
  | `transcribe` | `wer` |

  A kind with no default and no requested metric ranks every engine as
  unmeasured. Arrival order of evaluations never supplies a default.
- [pinned] `select_engine.comparison_set`: For the remaining candidate engines'
  evaluations of `(kind, metric)`, choose the `set.id` whose newest evaluation
  has the latest `measured_at_ms`; ties choose the bytewise ascending `set.id`.
  Compare only scores on that `(metric, set.id)`. An engine without an evaluation
  on the chosen set ranks as unmeasured. For each `(kind, provider, model, metric,
  set.id)`, use the newest `measured_at_ms`; an equal timestamp replaces the
  previous score with the last accepted evaluation. Provider and model match
  the selection answer exactly: a new model id starts unmeasured. Caller-local
  engines receive no evaluations and remain unmeasured.
- [informative] The engine-declaring media provider reporting evaluations keeps
  one canonical set per kind and
  re-runs every engine when it changes the set, so the newest set is canonical.
- [pinned] `select_engine.report`: A decision records its consumer and is bound
  to the selecting principal. Only that principal may `route.report` it,
  joined by `decision_id` and required `outcome`; report needs no consumer or
  runner member. A foreign principal receives `unknown_decision`.
- [pinned] `select_engine.decision_admission`: After §9's slot authorization,
  `route.account_for` honours an engine decision only from `engine.module`
  for the decision's provider/model and only for a new `admission_id` before
  `expires_at_ms`. Another module, a new admission at or after expiry, and
  every caller-local decision receive `unknown_decision`. A replay of an
  admission recorded before expiry, same id and body, returns its first answer
  even after expiry; §9.1's admission conflict rule is unchanged.
- [pinned] `select_engine.explain`: `route.explain` returns `unknown_decision`
  for every engine decision in this version. Scope follows §3.

## 14. `route.engine_report` — group `engine_reports`

- [pinned] `engine_report.request`: Params are `{decision_id, kind, engine:
  {provider, model}, outcome, input: {duration_ms?, bytes?, chars?}, wall_ms,
  first_output_ms?}`. Counts and times are unsigned integers in milliseconds
  where named. `kind` and `outcome` are open strings; known outcomes are
  `done`, `failed`, `cancelled`, `ended`, matching media usage outcomes. Unknown members or malformed values are
  `invalid_request {field}`, naming the member (including nested paths).
- [pinned] `engine_report.authorization`: Only the module that declared the
  exact `(kind, provider, model)` in its nonce-verified media-provider describe
  may report it, matched by attested principal, not a caller-supplied module
  name. Refuse every other caller with `engine_not_declared_by_caller`.
  The decision must have selected that module's engine for that kind;
  otherwise it is `unknown_decision`. The reporting module need not be the selecting principal.
- [pinned] `engine_report.speed`: Send once when the job ends. Routing retains
  a rolling window per `(kind, provider, model)` of wall and first-output latency
  and derives realtime factor as `wall_ms / input.duration_ms` when duration is
  positive. Missing or zero duration has no realtime factor. The router
  documents its bounded window and exposes samples to apps through its own
  inspection interface; this version adds no inspection op. These measurements
  feed realtime filtering, never accuracy ranking. With no speed samples, the
  declared latency class remains the filter's only evidence; samples may exclude
  an engine that cannot meet the requested realtime latency. Outcome is retained
  with each sample; failed or cancelled jobs never establish successful speed.
- [pinned] `engine_report.answer`: The success answer is `{}`. Repeated reports
  for the same decision apply no additional sample. They do not release account
  leases: §9.3 still requires `route.account_observe {kind: "run_finished"}`.

## 15. `route.engine_eval` — group `engine_evals`

- [pinned] `engine_eval.request`: Params are `{kind, engine: {provider, model},
  metric, score, set: {id, items}, measured_at_ms}`. `kind` and `metric` are open
  strings. `set.id` identifies the reference set and `items` is its unsigned
  item count. `measured_at_ms` is unsigned Unix milliseconds. Unknown members
  or malformed values are `invalid_request {field}`, naming nested paths too.
  `score` must be a finite number in `0..=1`, higher is better; refuse an
  out-of-range score as `invalid_request {field: "score"}`. Report a WER-style
  metric as `1 - wer`, not raw word error rate.
- [pinned] `engine_eval.authorization`: Apply §14's attested declaration rule
  independently of decisions. A non-declaring caller is refused with
  `engine_not_declared_by_caller`. Caller-local entries confer no authority.
- [pinned] `engine_eval.reference`: Accuracy comes only from evaluation on a
  reference set: production transcription has no reference text. Keep the
  newest evaluation per `(kind, provider, model, metric, set.id)` as §13 says;
  neither speed reports nor reports of job success supply accuracy. The success
  answer is `{}`.

## 16. Consumer defaults and runner placement (independent optional groups)

- [pinned] These groups are reader-first. No module declares a new member until
  its live router advertises the corresponding group in `role.describe`.
  Callers send request members only toward a declaring router. Non-declaring
  routers refuse unread request members by name, never silently apply a weaker
  constraint. Omission (or `null`) leaves every existing body, serialization
  and report fingerprint unchanged. The baseline types and members keep their
  existing types and meanings unless the corresponding group is declared.

### 16.1 `consumer_declarations`

- [pinned] The consumer slot remains the flattened object in §1.1, not a new
  nested `defaults` object. It additionally admits `iq?: 0..100`,
  `ladder?: [pin]`, `placement?: "caller_runner" | {module: string}`,
  `selectors?: [module-id]`, and `kind?: string`. Members use the usual optional
  null/omission convention. Module ids are attested ids, never caller-supplied
  identities; an empty module id is invalid. Placement objects have only `module`.
- [pinned] For typed declarations, `ConsumerDeclarations::validate()` validates
  a reader without the new group, preserving the old refusal of `iq`, even
  `iq: null`. `validate_serving(capabilities)` is for declaring readers:
  with `consumer_declarations`, a valid iq default is accepted and null is
  absent. Serde presence metadata preserves this distinction without adding
  a wire member or changing serialization or equality of absent defaults.
- [pinned] A declared name is honoured only if it equals the declaring module's
  id or starts with `<module id>.`. Any other name is ignored with a health
  warning naming the module. This is an identity check after schema validation;
  it does not discard that module's other, valid names. The catalog's nonce
  verification still applies. Different attested modules cannot own one root,
  so the baseline conflicting-owner rule cannot arise for these declarations.
- [pinned] Admission succeeds when the route's attested identity is the owner
  or a listed selector, or the select comes from a daemon-stamped scope the
  owner owns. Absent `selectors` means the owner only (plus the owned-scope
  arm). A selector's own scope is not the owner's scope. No request body can
  assert either identity or scope ownership. Other callers get
  `consumer_not_owned {consumer}`.
- [pinned] A call's explicit `iq` and `eq` override the corresponding declared
  defaults, including an explicit zero. Omission is not zero. A caller may
  omit `requirements.iq` only toward a declaring router for a consumer with a
  declared `iq` default. Dotted children inherit defaults within the owner's
  namespace; underscores do not inherit. Settings override defaults by name.
  The legacy parser and `Requirements.iq: u8` are unchanged:
  `select::parse_select_request_with_defaults` resolves admitted, inherited
  defaults before validating the call. Without a declared default, omission
  retains today's refusal. Admission and inheritance precede that helper.
- [pinned] `ladder` is the default ordered list in ladder mode, replaced by an
  operator's ladder when configured. Each entry is a pin accepted by
  `RouteRef::parse_pin`: `provider/model` or `provider/model:variant`. Entries
  that do not parse are invalid; no second ladder grammar is introduced.
- [pinned] `"caller_runner"` means the same runner **module** as the caller's
  session, on any harness of that module that can host the task. It does not
  bind the caller's harness. Resolve the module from `route.select`'s attested
  `reserved:<m>` caller when `<m>` is a catalog runner (it declares the routing
  runner slot). Otherwise use the single entry of the request's `runners`
  filter, which must name a catalog runner. With no filter, zero or multiple
  entries, or a non-runner entry, refuse `no_route` with
  `{reason: "caller_runner_unresolved"}`; this cause is not timed. The router
  never guesses. The attested runner takes precedence over the filter, but the
  filter still narrows eligible runners: a filter excluding that runner yields
  no route. Settings and the resolved runner's own declaration still apply.
- [pinned] `caller_runner` only narrows where a task of a consumer the caller
  is admitted to select will run. The consumer's owner chose who may select it,
  and declarations never widen settings, so a misstated runner can only
  misplace that task inside what settings allow. The value is not an authority
  input.
- [pinned] `{module}` limits the task to that module for a
  selector opening the session itself. Settings cannot move a consumer away
  from a declared placement. A head keeps its current placement; these facts
  govern session creation, not live head moves.
- [pinned] Declarations narrow and never widen settings. No declared demand,
  ladder, selector, kind or placement can admit a model, provider, paid key or
  runner that settings exclude. Settings still override defaults by name;
  dotted children inherit. An undeclared settings name produces a warning,
  not a refusal. An explicit settings `owner` still creates an operator-only
  consumer using the existing attested owner/owned-scope admission rule.
- [pinned] `kind` is an open list: `llm` (the default), `transcribe`,
  `stream_transcribe`, `synthesize`, `realtime`, `image`, `video`, `search`.
  Unknown well-formed tokens decode and are retained, not silently treated as
  `llm`. A consumer routes only within its kind. Catalog type words map with
  `transcription` ↔ `transcribe` and `speech` ↔ `synthesize`. Embedding and
  reranking are not routing kinds defined here.

### 16.2 `runner_harness_facts` and shared exclusion specs

- [pinned] Existing `runner.harness_limits.<harness>` entries additionally admit
  `starts_sessions?: bool`, `capabilities?: [token]`, `exclude?: [spec]`.
  Missing `starts_sessions` inherits the runner-level value; missing capabilities
  asserts no new task capability; missing exclusions adds no exclusion.
  `capabilities` is open: defined names are `one_off` (tool-free one-off session),
  `fork_answer`, and `output_cap` (can enforce the requested output cap).
  Unknown tokens are retained and confer no known capability.
- [pinned] A harness with `starts_sessions: false` never hosts a new session.
  Exclusions reject catalog routes even though the catalog lists them. Per-harness
  facts only narrow runner-wide offers, never widen them. Existing independent
  malformed-map warning and fallback rules still apply.
  `typed_harness_limits_serving` gates new facts for a reader; without the group,
  such a map is not applied. The ungated typed accessor validates producer data.
- [pinned] Shared exclusion specs have forms `provider`, `provider/model`,
  `-provider`, `-provider/model`. Each entry is trimmed. Consumers drop an empty
  entry, a bare `-`, or an empty slash segment; producers must refuse these
  entries, never emit them. The first slash separates provider from model;
  the rest belongs to the model, so `openrouter/anthropic/example-model` names
  provider `openrouter`, model `anthropic/example-model`.
- [pinned] A provider spec matches all keys under that provider at a slash
  boundary. A model spec matches that exact key or its slash descendants, never
  a substring or a prefix inside a segment. Model-level matches beat
  provider-level matches regardless of order; within each level, the later
  matching entry wins. A positive entry uses the list's base value; `-` flips
  it. A list with no matching entry leaves the caller's value unchanged.
  For harness `exclude`, the base is `true` (excluded), with no match admitting
  the route. `route_spec::is_route_spec`, `matches` and `resolve` share these
  rules; `resolve` returns `None` when there is no match.
- [pinned] There are no globs, version ranges or special `:variant` semantics
  in exclusion specs: a colon is part of the literal model id. A harness
  refusing old model generations lists those model ids explicitly. Ladder pins
  use their own parser, not this exclusion grammar.

### 16.3 `decision_harness`

- [pinned] A decision names both the chosen runner module and harness, using
  the existing `runner: {module, model, auth?, harness}` object. No member
  changes type; the already optional `runner.harness` is required on decisions
  produced under this group. A creating send uses that harness; a runner refuses
  a contradictory harness instead of silently substituting. The router selects
  a harness that can host the task and route, not merely a runner's default.
- [pinned] Callers' `runners` and `harness` only narrow allowed placements.
  If nothing remains, the answer is `no_route` with reason `harness_unavailable`.
  Settings remain the source of allowed placements; shipping these groups does
  not by itself move any workload.

### 16.4 `select_kind` and `select_eligible_routes`

- [pinned] `select_kind` adds `kind?: string` to `route.select`. It uses the
  same open kind list as declarations; omission means `llm` unless the admitted
  consumer declares another kind. A call cannot cross a consumer's declared
  kind. Unknown kinds are retained and have no implicit fallback to another kind.
- [pinned] `select_eligible_routes` independently adds
  `eligible_routes?: [{provider, model, variant?}]`, using `RouteRef`. These are
  routes, not pins, provider globs or exclusion expressions. The caller computes
  the subset for this call (format, duration, filters or other eligibility).
  Routing ranks within it and never adds to it, even for a pin or ladder.
  An explicitly empty subset is `no_route`, never `invalid_request`. Omitted
  means no additional subset restriction. Other policy and request restrictions
  still intersect it. Repeated entries do not grant any additional eligibility.

### 16.5 `select_price`

- [pinned] A select answer may carry:
  `price: {basis, multiplier?, rates_per_mtok: {input, output, cache_read?,
  cache_write?, cache_write_ttl?: {<lifetime>: rate}}, estimated_task_usd?}`.
  Rates, multiplier and estimates are JSON numbers (`serde_json::Number`, as
  the existing `max_cost_usd` uses), not decimal strings or integer micros.
  Rates and estimates are finite and non-negative; a multiplier is finite and
  positive. `input` and `output` are required, other rate roles are optional.
- [pinned] Producer bases are exactly `metered` and `subscription_equivalent`.
  Metered carries the catalog's list rates and **no multiplier**.
  Subscription equivalent carries those list rates divided by the value
  multiplier, and **must state that multiplier**. The multiplier is the router's
  measured subscription value multiplier when available, otherwise the operator's
  configured value. Plan pressure is a scarcity signal, not a price, and stays
  out of rates. Readers may retain unfamiliar record bases; producers must not
  emit them.
- [pinned] Every rate is already the rate the caller multiplies reported usage
  by: `cost = Σ(tokens × rate / 1_000_000)` for either basis. Reasoning tokens
  are inside `output`, never added twice. The caller multiplies and settles;
  the runner reports usage only and never derives a charge from this answer.
- [pinned] `cache_write_ttl` keys are the catalog's lifetime strings verbatim,
  not a closed enum. A usage bucket with a lifetime is priced only by the
  matching lifetime entry. An unmatched lifetime is unpriced, **never borrowed
  from plain `cache_write`**. Samples include `5m`/`1h` and `30m`/`12h`/`24h`.
  Catalog `.above_context.<n>` conditions are resolved by routing for the
  selected request: the answer carries the applicable rate, not the condition.
- [pinned] `estimated_task_usd` is routing's per-task estimate, suitable for a
  caller's pre-call reserve. All these numbers are an **estimate of value, not
  money spent**; every reader must present and interpret them that way.
- [pinned] Routing omits `price` if it cannot price the route. A caller holding
  a dollar cap must refuse to send without a price (`price_unknown` at that
  caller). This is the caller's rule, not a routing refusal. Neither the runner
  nor an absent estimate is an alternative rate source.

### 16.6 `report_attempt_outcomes`

- [pinned] `route.report` additionally carries
  `attempt_outcome?: {status: string, retry_after_ms?: u64}`. It is authoritative
  over legacy `outcome` when the router declares this group. Producers emit
  exactly this closed set; decoding is open and an unknown status counts
  **nothing**, including no route penalty, score change, cooldown or affinity
  change. This overrides the legacy unknown-`outcome` failure rule only when
  `attempt_outcome` is present. Without it, all old report effects remain.

  | `status` | Meaning | Companion legacy `outcome` |
  | --- | --- | --- |
  | `served` | Work served | `completed` |
  | `served_empty` | Served with an empty result | `completed` |
  | `quota` | Route quota exhausted | `failed` |
  | `throttled` | Retryable throttling | `failed` |
  | `credential` | Credential cannot serve | `failed` |
  | `unavailable` | Provider unavailable | `failed` |
  | `rejected` | Caller's own bad request | `cancelled` |

- [pinned] `rejected` never counts against a route. There is no faithful legacy
  value for it: `cancelled` is the mildest no-penalty approximation. Likewise
  `served_empty` loses the empty-result distinction when mapped to `completed`;
  quota/throttled/credential/unavailable lose their specific cause in `failed`.
  Callers send the table's companion, so readers never infer contradictory
  effects. Known statuses with a different companion are `invalid_request
  {field: "outcome"}`. A future unknown status is read without imposing a
  legacy-companion check; a producer of this version never emits it.
- [pinned] `retry_after_ms` is optional for `throttled` and means **the provider
  named this wait**. It is an unsigned integer duration, including zero. Absent
  means unknown, and routing applies its own backoff; a runner must not invent
  a duration when the provider supplied no wait or reset time. The member is
  invalid on every other status: `invalid_request
  {field: "attempt_outcome.retry_after_ms"}`. Malformed values, including
  non-integers and negative numbers, are refused by that same field path.
