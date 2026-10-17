# Tool-provider conformance

Run `run_suite` against a live provider using its `ToolProviderSubject`, real
`Harness`, and `ToolRoute` adapter. Each case is reported by name as `PASS`,
`FAIL`, or `SKIP`; missing capabilities never count as passes. Simulated
kills do not establish real-process crash conformance. The in-process fake
in this crate tests the runner, not any production provider.

The suite's case inventory and role contract are in the wire crate's
[`CONTRACT.md`](../cortexkit-role-tool-provider/CONTRACT.md). Version 0.9.0
pairs with subc-protocol 0.32 and cortexkit-role-tool-provider 0.9.0. Versions
0.8.x pair with subc-protocol 0.32; versions 0.7.x pair with protocol 0.31,
and versions 0.6.x pair with protocol 0.30.

## Opt-in `content/v1` results

`tool_result_content_v1_valid` runs when the catalog declares at least one
tool with `result_format: "content/v1"`. `Capability::ContentV1` is inferred
from that catalog, not controlled by the subject's capability flags. No
declaration produces a named `SKIP` with missing `content/v1`, never a pass;
unknown format strings do not opt in.

`content_v1_calls()` supplies promptly completing `(tool name, arguments)`
probes for every declared tool. The default is `[quick_call()]`; override it
when other tools opt in, and supply multiple probes to check success and
tool-level error bodies. A missing probe fails by tool name. For every probe,
the case makes the same call over a route with no consumer declaration and
over a route whose bind declares `content/v1`. The first reply must not decode
as `ToolResult`; the second must decode and pass `validate`. The opted-in route
must still list `result_format: "content/v1"` in its catalog. The case also
checks the entire serialized JSON body's size before decoding so unmodeled
extensions cannot evade the 3 MiB bound. Malformed replies fail by case, tool
and member, not as an opaque success.

Subjects with a catalog tool that opts into `content/v1` must implement
`route_with_consumer_capabilities()` to open the declared route through the
real route-opening path. The method receives the exact consumer capability
list to send when opening the route; it must not add that declaration to a
call body.

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

## Catalog capability filtering

`catalog_capability_filter_matches_describe` runs only when the live
`role.describe` answer advertises `tool-catalog-capability-filter/v1`; otherwise
it is reported as a named skip. The case requests the full discovered
`tool_capabilities` list and checks that the filtered catalog's tag union is
exactly that list. It also checks that missing and empty lists select no tools,
that filtering does not change `generation`, and that a changed tool selection
changes `catalog_digest`. No subject flag can opt out of a live advertisement.
