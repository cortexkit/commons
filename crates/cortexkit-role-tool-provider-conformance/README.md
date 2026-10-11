# Tool-provider conformance

Run `run_suite` against a live provider using its `ToolProviderSubject`, real
`Harness`, and `ToolRoute` adapter. Each case is reported by name as `PASS`,
`FAIL`, or `SKIP`; missing capabilities never count as passes. Simulated
kills do not establish real-process crash conformance. The in-process fake
in this crate tests the runner, not any production provider.

The suite's case inventory and role contract are in the wire crate's
[`CONTRACT.md`](../cortexkit-role-tool-provider/CONTRACT.md). Versions 0.7.x
pair with subc-protocol 0.31; versions 0.6.x pair with protocol 0.30.

## Opt-in `content/v1` results

`tool_result_content_v1_valid` runs when the catalog declares at least one
tool with `result_format: "content/v1"`. `Capability::ContentV1` is inferred
from that catalog, not controlled by the subject's capability flags. No
declaration produces a named `SKIP` with missing `content/v1`, never a pass;
unknown format strings do not opt in.

`content_v1_calls()` supplies promptly completing `(tool name, arguments)`
probes for every declared tool. The default is `[quick_call()]`; override it
when other tools opt in, and supply multiple probes to check success and
tool-level error bodies. A missing probe fails by tool name. The case uses
the real route, decodes each reply as `ToolResult`, and runs `validate`;
it also checks the entire serialized JSON body's size before decoding so
unmodeled extensions cannot evade the 3 MiB bound. Malformed replies fail
by case, tool and member, not as an opaque success.

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
