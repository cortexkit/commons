# Changelog

## 0.7.1

- Require cortexkit-role-tool-provider 0.7.1 and add the catalog-gated
  `tool_result_content_v1_valid` case. No declaring tool yields a named skip,
  never a pass; declarations cannot be opted out of by a subject flag.
- Add `Capability::ContentV1` (inferred from the catalog) and additive
  `content_v1_calls` probes, defaulting to `quick_call`. Validate each declared
  tool's probed replies and the whole serialized body's 3 MiB bound.
- Test a deliberately malformed provider reply failing by case/tool/member
  and legacy opaque tools skipping the case.

## 0.7.0

- Requires subc-protocol 0.31 and cortexkit-role-tool-provider 0.7: use 0.7.x
  with protocol 0.31, and 0.6.x with protocol 0.30.
- Add unconditional `catalog_modes_valid` and opt-in
  `call_mode_matches_catalog`. The latter opens mode-stamped routes with a
  subject-supplied restricted probe and checks both refusal and service.
  No restricted fixture means a named skip, never a pass.
- Add `Capability::CallModes`, `CallModeProbe`, and the subject's
  `call_mode_probe` and `route_with_mode` hooks. Test a provider stand-in
  that ignores bind modes and one that wrongly refuses a listed mode.

## 0.6.0

- Requires subc-protocol 0.30 and cortexkit-role-tool-provider 0.6. No change to
  the cases.

## 0.5.2

- Add the unconditional `catalog_reply_valid` conformance case, refusing
  invalid per-tool reply metadata by tool and member with `check_reply`.
- Test the new case against a deliberately broken provider stand-in and
  depend on the optional reply deadline types and validator added in
  `cortexkit-role-tool-provider` 0.5.2.
