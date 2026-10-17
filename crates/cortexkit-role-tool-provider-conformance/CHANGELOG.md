# Changelog

## 0.9.0

- Requires cortexkit-role-tool-provider 0.9.0 and subc-protocol 0.32 because
  `RoleDescribe` is now non-exhaustive and carries discovered tool capability
  tags.
- Add `catalog_capability_filter_matches_describe`; it reads the live
  `role.describe` advertisement, requests every listed tool capability, and
  checks that the returned catalog contains the same tag union.
- Add shared accept/refuse vectors for `params.capabilities`, vendor them in the
  conformance crate, and update its fake provider to advertise and apply the
  same capability filter.

## 0.8.0

- Requires subc-protocol 0.32 and cortexkit-role-tool-provider 0.8.0. Consumers
  must upgrade the conformance crate and provider together because public route
  errors and the `wire` re-export expose their protocol-bound types; no test
  adapter source changes are otherwise required.

## 0.7.2

- Require cortexkit-role-tool-provider 0.7.3 and verify content/v1 results are
  sent only on routes whose consumer declares that format.
- Exercise the same content/v1 tool call over legacy and content/v1 routes;
  reject content/v1 bodies sent to consumers that did not declare support.

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
