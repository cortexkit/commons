# Tool-provider conformance

Run `run_suite` against a live provider using its `ToolProviderSubject`, real
`Harness`, and `ToolRoute` adapter. Each case is reported by name as `PASS`,
`FAIL`, `SKIP`, or `N/A`; missing capabilities never count as passes. Simulated
kills do not establish real-process crash conformance. The in-process fake
in this crate tests the runner, not any production provider.

The suite's case inventory and role contract are in the wire crate's
[`CONTRACT.md`](../cortexkit-role-tool-provider/CONTRACT.md). Version 0.13.0
pairs with cortexkit-role-tool-provider 0.13.0. Version 0.12.1
pairs with cortexkit-role-tool-provider 0.12.1. Version 0.12.0
pairs with subc-protocol 0.33 and cortexkit-role-tool-provider 0.12.0. Version 0.11.0
pairs with subc-protocol 0.33 and cortexkit-role-tool-provider 0.11.0. Version 0.10.1
pairs with subc-protocol 0.32 and cortexkit-role-tool-provider 0.10.4; version
0.10.0 with cortexkit-role-tool-provider 0.10.0. Version
0.9.2 pairs with cortexkit-role-tool-provider 0.9.2; versions 0.8.x pair with
subc-protocol 0.32; versions 0.7.x pair with protocol 0.31, and versions 0.6.x
pair with protocol 0.30.

## Opt-in `tool-result/v1` results

`tool_result_v1_valid` runs when the catalog declares at least one
tool with `result_format: "tool-result/v1"`. `Capability::ToolResultV1` is inferred
from that catalog, not controlled by the subject's capability flags. No
declaration produces a named `SKIP` with missing `tool-result/v1`, never a pass;
unknown format strings do not opt in.

`tool_result_v1_calls()` supplies promptly completing `(tool name, arguments)`
probes for every declared tool. The default is `[quick_call()]`; override it
when other tools opt in, and supply multiple probes to check success and
tool-level error bodies. A missing probe fails by tool name. For every probe,
the case makes the same call over a route with no consumer declaration and
over a route whose bind declares `tool-result/v1`. The first reply must not decode
as `ToolResult`; the second must decode and pass `validate`. The opted-in route
must still list `result_format: "tool-result/v1"` in its catalog. The case also
checks the entire serialized JSON body's size before decoding so unmodeled
extensions cannot evade the 3 MiB bound. Malformed replies fail by case, tool
and member, not as an opaque success.

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
exactly that list. It also checks that missing and empty lists select no tools,
that filtering does not change `generation`, and that a changed tool selection
changes `catalog_digest`. No subject flag can opt out of a live advertisement.

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
