# Tool-provider conformance

Version **0.15.8** pins provider **0.15.7**. Adapters declare
`Capability::SystemTextFacts` only when the module manifest provides the exact
`system-text-facts/v1` token. The facts cases use the existing
`system_text_catalog_arguments` fixture. A declarer must accept valid facts and
refuse malformed facts as `invalid_request` naming `facts`; the legacy request
case omits the member altogether, including for providers that reject unknown
request members. The token is a manifest fact, not inferred from a crate version
or another capability.

Run `run_suite` against a live provider using its `ToolProviderSubject`, real
`Harness`, and `ToolRoute` adapter. Each case is reported by name as `PASS`,
`FAIL`, `SKIP`, or `N/A`; missing capabilities never count as passes. Simulated
kills do not establish real-process crash conformance. The in-process fake
in this crate tests the runner, not any production provider.

The suite's case inventory and role contract are in the wire crate's
[`CONTRACT.md`](../cortexkit-role-tool-provider/CONTRACT.md). Version 0.14.3 pairs with
subc-protocol 0.34 and cortexkit-role-tool-provider 0.14.3. Version 0.14.1 pairs with
subc-protocol 0.34 and cortexkit-role-tool-provider 0.14.1. Version 0.14.0 pairs with
subc-protocol 0.34 and cortexkit-role-tool-provider 0.14.0. Versions 0.13.1 and 0.13.0
pair with cortexkit-role-tool-provider 0.13.0. Version 0.12.1
pairs with cortexkit-role-tool-provider 0.12.1. Version 0.12.0
pairs with subc-protocol 0.33 and cortexkit-role-tool-provider 0.12.0. Version 0.11.0
pairs with subc-protocol 0.33 and cortexkit-role-tool-provider 0.11.0. Version 0.10.1
pairs with subc-protocol 0.32 and cortexkit-role-tool-provider 0.10.4; version
0.10.0 with cortexkit-role-tool-provider 0.10.0. Version
0.9.2 pairs with cortexkit-role-tool-provider 0.9.2; versions 0.8.x pair with
subc-protocol 0.32; versions 0.7.x pair with protocol 0.31, and versions 0.6.x
pair with protocol 0.30.

## Late-result carrier custody (0.15.6)

Version 0.15.6 retains the exact `cortexkit-role-tool-provider =0.15.5` pin
and corrects 0.15.4's owner-custody assumptions to match contract §7. The
custodian is the carrier that raised the keyed call, even inside another
principal's scope. Late-result reads and acks use that carrier's verified
principal, not the scope owner's.

The existing `scoped_principals()` fixture and `Harness::route` stamps already
support a carrier in another owner's scope; no additional adapter hook is
needed. Supply distinct `carrier` and `scope.owner` identities so the kit can
distinguish custody from scope ownership. The reference fake uses distinct
identities and tests both carrier delivery and rejection of owner delivery.

`late_results_cursor_round_trip`, `late_results_ack`, both
`crash_after_*_not_started` cases and `tool_result_held_reply_route_gating`
require exactly one matching entry in the carrier's feed, with the carrier
as custodian and the original scope identity. When the owner differs, its
entire feed must exclude the call. These checks follow pagination so a later
page cannot hide duplicates or an owner leak. Probes that reject cursors from
another provider incarnation or past the log's end also use the carrier's feed.

## Session notices (0.15.4)

`session_notices_describe` runs only for the exact live module capability
`session-notices/v1` in `role.describe.capabilities`. Its shape must pass the
ordinary describe validator and still declare the token. An absent or unknown
token is `N/A`, not a pass or an error. No new duplicate-token or unknown-name
restriction is imposed. A deliberately faulty provider declaring the token
alongside a non-string capability member fails this case by name.

The provider's completion-only idle starts, progress steering and sender
identity promise live in the role contract; behavioral enforcement belongs
to runners and plan composers, not this provider kit. Consumers must accept
the token before providers declare it. The token is a provider capability,
not a per-tool tag, so routing's tag list is unchanged.

## Call keepalives

The wire crate defines opt-in `call-keepalive/v1`, its body and deadline helpers,
and shared vectors. This suite does not yet check keepalive timing or stopping
work on CANCEL: `ToolRoute` returns a buffered `Exchange` without frame timestamps
or an observation of work after cancellation. The suite's in-process fake does
not exercise a real route. A terminal `cancelled` reply alone would not prove
that the provider stopped working, so the existing terminal-frame cancellation
case does not establish the keepalive capability's promise.

## Opt-in `tool-result/v1` results

`tool_result_v1_valid` runs when the catalog declares at least one
tool with `result_format: "tool-result/v1"`. `Capability::ToolResultV1` is inferred
from that catalog, not controlled by the subject's capability flags. No
declaration produces a named `SKIP` with missing `tool-result/v1`, never a pass;
unknown format strings do not opt in.

| Case | Requires |
|---|---|
| `tool_result_v1_valid` | a live catalog tool declaring `tool-result/v1` |
| `tool_result_legacy_route_unchanged` | a live catalog tool declaring `tool-result/v1`, `Capability::LegacyReply` |

`tool_result_v1_calls()` supplies promptly completing `(tool name, arguments)`
probes for every declared tool. The default is `[quick_call()]`; override it
when other tools opt in, and supply multiple probes to check success and
tool-level error bodies. A missing probe fails by tool name. For every probe,
`tool_result_v1_valid` calls a route whose bind declares `tool-result/v1`;
the reply must decode as `ToolResult` and pass `validate`. Both the undeclared
and opted-in routes must list `result_format: "tool-result/v1"` in their catalogs. The case also
checks the entire serialized JSON body's size before decoding so unmodeled
extensions cannot evade the 3 MiB bound. Malformed replies fail by case, tool
and member, not as an opaque success.

To check legacy compatibility, implement
`legacy_reply(&self, tool: &str, arguments: &serde_json::Value) -> Option<serde_json::Value>`
and declare `Capability::LegacyReply` in `capabilities()`. Return the exact
legacy rendering for each `tool_result_v1_calls()` probe, preferably from a
pinned fixture or the legacy rendering code path, not another call to the live
provider. `tool_result_legacy_route_unchanged` sends each probe on a route
without the consumer declaration and requires JSON value equality with that
fixture. A mismatch fails by tool name and first differing JSON pointer
(sorted object keys, array order; the empty pointer means the root). An
MCP-shaped legacy body with `content` and `isError` is valid even if it decodes
as `ToolResult`: consumers never infer opt-in from a reply's appearance.

The hook defaults to `None`, so existing subjects still compile. Without
`Capability::LegacyReply`, the legacy half is **not checked**, reported as
`SKIP tool_result_legacy_route_unchanged (requires tool-result/v1, legacy_reply; missing legacy_reply)`
when a tool declares the format. The declared-route case still runs. The
suite is then `ConformingForDeclaredCapabilities`, not `Passed`, unless another
failure makes it fail. Declaring the capability but returning `None` for a
probe fails the legacy case by tool name.

Subjects with a catalog tool that opts into `tool-result/v1` must implement
`route_with_consumer_capabilities()` to open the declared route through the
real route-opening path. The method receives the exact consumer capability
list to send when opening the route; it must not add that declaration to a
call body.

## Held first replies: `held-reply/v1`

Declare the suite fixture capability `Capability::HeldReplyV1` and supply
`held_reply_call(marker)` for a person-gated tool opting into `tool-result/v1`.
The default reuses `held_call`. This suite capability is not a new provider
advertisement; the provider must advertise the existing `late_results`
session capability. The case also requires the scoped, keyed held-call and
approval-execution fixtures and a live `tool-result/v1` catalog declaration.

`tool_result_held_reply_route_gating` opens scoped routes with absent, empty,
content-only, held-only and both consumer declarations. Each call carries a
fresh key; only the both-declared route must carry a valid `held` marker.
The case approves each call and checks that its `(carrier, call_key)` answer
arrives through `late_results`. Missing fixtures produce named skips, never
passes. Existing withdraw and crash cases still apply.

## Per-tool call modes

`catalog_modes_valid` runs for every provider, checking each tool with
`check_modes`: a present list must be nonempty, unique, and contain only
known modes. Absence means classic only.

To exercise both service and refusal, declare `Capability::CallModes` and
implement these `ToolProviderSubject` hooks:

- `call_mode_probe()` returns a `CallModeProbe`: one promptly successful
  `(tool name, arguments)` call, and one **known** `unlisted_mode` that its
  catalog entry does not list. The tool must serve at least one known mode.
  The suite chooses a served mode from that same catalog entry. For example,
  a tool listing `["flow", "classic"]` can supply `CallMode::Codemode` as its
  unlisted mode.
- `route_with_mode(handle, stamp, mode)` opens a real route to the provider
  whose daemon-stamped bind mode is the requested known mode. Arrange live
  plain, run, or flow scopes as needed: the mode is derived by the daemon,
  not put into the request body. The adapter must not replace the real
  route with an in-process call to provider code.

`call_mode_matches_catalog` sends the same call body on an unlisted-mode
route and a listed-mode route. The former must end with exactly one terminal
`ERROR`; the latter must end successfully with exactly one terminal frame.
The provider owns the refusal code and wording.

If there is no restricted fixture (for example, every tool lists all three
known modes), return `None` from `call_mode_probe()` or omit
`Capability::CallModes`. The report prints
`SKIP call_mode_matches_catalog (requires call_modes; missing call_modes)`;
the case never counts as passed. A malformed supplied probe fails the case.
Declaring the capability with a probe but without a working route adapter
also fails the case. This suite capability describes the available test
fixture, not the provider's manifest declaration of `call-modes/v1`.

The subc daemon writes the mode on a route bind itself, and today it only
ever writes one of the three known modes, so a live route cannot carry an
unknown one. Preservation and refusal of `CallMode::Unknown` are therefore
checked by the role crate's own tests of `serves_call_mode`, not by this
suite.

## Portable input schemas

`catalog_schemas_portable` runs for every provider. It passes each served
tool's `input_schema` to the role crate's `lint_input_schema` and fails when
any tool has a finding: a property, at any depth under `properties`, `items`,
`prefixItems` or an `additionalProperties` object, that declares no `type`
and is not an `anyOf`, `oneOf` or `allOf` combinator or a `$ref`. The failure
names every offending tool and JSON pointer, for example
`pick input_schema /properties/surface: declares no type; ...`. Such a
schema is valid JSON Schema, but some model providers refuse a request that
offers it, so fix it by adding the `type` (`{"type": "string", "enum":
["a", "b"]}`). Consumers do not refuse these catalogs; the case exists so the
provider hears about the problem before a model provider refuses it.

## Per-tool `emits_events`

`catalog_emits_events_boolean` runs for every provider. It reads the raw
`tool.catalog` answer and checks that every tool carrying `emits_events`
declares a JSON boolean: `null`, which the role crate decodes as absent, and
any other type fail by tool and member. Tools that omit it pass. The case
checks only the shape of the declaration. Whether a tool declaring `false`
really never makes the provider emit an event, directly or relayed from
upstream, cannot be observed from outside the provider, so a pass says
nothing about that promise.

## Catalog capability filtering

`catalog_capability_filter_matches_describe` runs only when the live
`role.describe` answer advertises `tool-catalog-capability-filter/v1`; otherwise
it is reported as a named skip. The case requests the full discovered
`tool_capabilities` list and checks that the filtered catalog's tag union is
exactly that list. It also checks that an empty list selects no tools,
that filtering does not change `generation`, and that a changed tool selection
changes `catalog_digest`. No subject flag can opt out of a live advertisement.

`catalog_filter_preserves_plan_item` runs under the same live filter token.
Supply `catalog_arguments()` as an ordinary plan-item request: no
`params.capabilities`, the provider's documented params when supported, and
optionally a preset it serves. The case requires a non-empty catalog and fails
on a refusal, including one naming a documented `params.<key>`. This keeps
discovery's filter requests separate from ordinary plan items.

`catalog_filter_model_refused` also requires only the live filter token. It
sends `capabilities` with a well-formed `model` and requires `invalid_request`
naming `params.model`, even if the provider also advertises model support.

## Catalog model selectors

`catalog_model_malformed_refused` runs only when the live `role.describe`
answer advertises `tool-catalog-model/v1`. It sends the malformed selectors
from the shared `catalog-model.json` samples on ordinary requests and requires
`invalid_request` naming `params.model` for each. A selector is exactly an
object with non-empty `provider` and `model` strings, with no `variant` or
other member. Consumers send model selectors only to providers advertising
this token, and never on filter requests. How a provider treats a model it
does not recognise remains its own documented rule. The case is skipped by
name without the live token, not opted out by a subject flag.

## Served presets

`catalog_listed_presets_served` runs when the live catalog answer to the
subject's `catalog_arguments()`, sent without `preset`, `digest_only` and
`system_text`, lists `presets`. `Capability::Presets` is inferred from that
answer, not set by the subject; a provider listing no presets gets
`SKIP catalog_listed_presets_served (requires presets; missing presets)`. The
case checks the list with `check_presets`, then requests the catalog once per
listed preset. Every listed preset must be answered with a catalog that
carries no `presets` list of its own; a refusal fails the case naming the
preset.

## System text without a preset

`system_text_preset_optional_served` runs for a subject declaring both
`Capability::SystemText` and `Capability::SystemTextOptionalPreset`. A
provider on a release that still requires `system_text.preset` leaves the
latter undeclared and gets
`SKIP system_text_preset_optional_served (requires system_text, system_text_optional_preset; missing system_text_optional_preset)`.
The case sends `system_text_catalog_arguments()` without the top-level
`preset` and without `system_text.preset`. A refusal, such as
`invalid_params`, fails the case naming the refusal code. The answer must
carry a system-text item that passes the `system_text_digests_match_text`
checks, and a repeat must return the same `item_digest`. When
`system_text_default_preset()` names the preset the default text belongs to,
the preset-less answer's `text` and `item_digest` must equal that preset's.
The hook defaults to `None`, for a provider whose default is not a named
preset; the case then cannot tell which variant answered.

## User commands

The live catalog, not an adapter capability flag, enables command cases. The
catalog case always runs, including the reserved-tool-name check. Implement
`user_command_route` with actual daemon scope attributes for Session, Codemode
(`run_id`) and Flow (`flow_id`), independent of the route's call-mode field.
`user_command_input` supplies a safe probe for each declared command; an accepted
probe must settle within the fixture's 30-second observation window. Every
listed command is exercised. A synchronous reply has no late-result obligation.

`user_commands_repeat` requires `ToolRoute::request_with_response_bytes`: retain
the actual RESPONSE body before decoding JSON. Re-encoding a decoded value would
hide a whitespace-only or member-order change. The other cases cover catalog
and reply shape, the five-second first-reply deadline, codemode/flow refusals and
accepted progress followed by a terminal command reply. Deliberately faulty
fakes prove every named command case, including a bytes-only repeat defect.

## Default system-text manifest fact

Version 0.12.1 adds `system_text_default_served`. Set
`Capability::SystemTextDefault` only when the subject's module manifest lists
`system-text-default/v1` in `capabilities.provides`. The route has no daemon
catalog access, so this declaration is an adapter fixture flag. Supply
`system_text_catalog_arguments` and, when the default is also named,
`system_text_default_preset`. The case removes both presets, requires valid
system text rather than a refusal, checks repeat stability and compares with
the named default where available. Undeclared subjects receive `N/A`, never
`SKIP` or `FAIL`. A faulty provider declaring the fact but refusing a
preset-less item fails this case by name. The older
`system_text_preset_optional_served` fixture and its skip behavior remain
available for compatibility. `SystemTextOptionalPreset` is a conformance
fixture flag, not a manifest fact; composers use `system-text-default/v1`.

## Reply custody (0.15.4)

Version 0.15.4 pins cortexkit-role-tool-provider exactly to 0.15.4. Declare
`Capability::ReplyCustody` only for a provider advertising the `reply_custody`
session capability. Every custody case requires that declaration; without it,
the report prints named skips, never passes. Core cases need no additional
fixture declaration:

- `reply_custody_dependencies` checks the catalog's `late_results` and
  `call_key` dependencies.
- `reply_custody_immediate`, `reply_custody_route_close`,
  `reply_custody_restart`, `reply_custody_fresh_key` and
  `reply_custody_retention_ack` check a non-Pure keyed call's entry, survival
  after ordinary route closure, observed or unknown outcome without re-execution after a
  real restart, fresh-key dispatch, and retention across reads until ack.

Additional declarations unlock probes not every provider can supply:

| Fixture capability | Cases |
|---|---|
| `ReplyCustodyHeld` | `reply_custody_held` |
| `ReplyCustodyRefusal` | `reply_custody_refusal_after_dispatch` |
| `ReplyCustodyPure` | `reply_custody_pure_direct`, `reply_custody_pure_route_close` |
| `ReplyCustodyPure` and `ReplyCustodyHeld` | `reply_custody_pure_held` |
| `ReplyCustodyCatalogChange` | `reply_custody_frozen_catalog` |

Implement `reply_custody_fixture(state_root, scenario)` for each enabled case.
A declared fixture that is missing or fails to open fails the case, not skips
it. Each fixture owns a fresh real provider instance, opened on the supplied
unused state root. Its call route declares `reply_custody`, `late_results` and
`call_key` at bind; held probes additionally declare `tool-result/v1` and
`held-reply/v1`. The catalog is fetched before dispatch and frozen for the
probe. Only the Pure scenarios use a tool declaring `execution_mode: "pure"`.
Supply pinned expected terminal result/error values and the expected outcome
string for entries that retain only the outcome rather than the full result.
Never obtain expected values by another call to the live provider.

`ReplyCustodyControl::finish` observes dispatch and controls only the fixture's
lifecycle. It releases held work after its first reply, closes only the call
route for RouteClose, or kills the actual provider after DispatchStarted and
restarts it on the same root for Restart. Pin the observed result when it can
be recovered, or set `expected_reduced_outcome: "unknown"` for a fixture whose
outcome cannot be determined. Both recovery paths are accepted; neither may
re-dispatch the call. For
FrozenCatalog it changes the live execution mode to pure after dispatch,
then closes the call route before releasing the work. It returns a fresh
late-results route as the same custodian, after settlement. All entries and
acks are checked over real routes, not synthesized by the hook.
`dispatch_count` observes actual dispatches/effects independently of the
returned bodies; it must survive the restart. Dropping the fixture must stop
its provider. Restart probes must really kill the process, not just drop a route.
`restart_kill_report` must report DispatchStarted and a real-process kill
mechanism; missing reports and simulated kills fail the restart case.

The Pure direct case accepts either omission or a truthful retained entry.
The Pure route-close case permits stopping but requires an entry when no
direct terminal reply was sent. The Pure held case always requires an entry.
The retention case verifies that
reading does not delete an entry and that ack removes it, not a 24-hour wall
clock wait. Custodian-side durable commits, outcome deduplication and Pure
restart re-dispatch policy are responsibilities described by the contract,
not claims made by this provider kit. Its deliberately faulty in-process
providers test rejection of every case by name; they do not establish any
production provider's conformance.
