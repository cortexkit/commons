# Changelog

## 0.13.0

- Renamed the tool-result format from `content/v1` to `tool-result/v1`, with no alias. Providers and consumers that declared `content/v1` switch to `tool-result/v1` together.

## 0.12.1

- Add `system_text_default_served`, gated by the subject's manifest-fact fixture flag, with a named not-applicable result for undeclared subjects.
- Add a faulty provider that declares the fact but refuses preset-less text and prove it fails the case by name. Preserve the older optional-preset case.

## 0.12.0

- Breaking dependency update: the public `wire` re-export moves to
  cortexkit-role-tool-provider 0.12, whose `SystemTextItem.preset` is
  `Option<String>`. Migration: wrap existing presets in `Some(...)`, and
  treat `None` as the provider's default text.
- Add `system_text_preset_optional_served`: a system-text item without a
  preset must be answered, not refused, with a well-formed system-text item
  that a repeat returns again. When the subject names its default preset with
  the new `system_text_default_preset` hook, the text and `item_digest` must
  equal that preset's. The case requires the new subject-declared
  `Capability::SystemTextOptionalPreset`, so a provider on an older release
  skips it by name instead of failing.
- The fake provider serves a system-text item without a preset, with faults
  for refusing one as `invalid_params` and for answering another variant's
  text.

## 0.11.0

- Moves to subc-protocol 0.33; take subc-protocol 0.33, subc-transport 0.14, subc-control 0.33 and subc-client-rs 0.31 together.
- Breaking dependency update: `ObservedFrame::Error` contains subc-protocol
  0.33's `ErrorBody`. The public `wire` re-export moves to
  cortexkit-role-tool-provider 0.11, including its protocol-bound types.
- No behaviour changes; existing test-vector bytes are unchanged.

## 0.10.1

- Require cortexkit-role-tool-provider 0.10.4. Add
  `catalog_listed_presets_served`: when the provider's preset-less catalog
  answer lists `presets`, the list must pass `check_presets` and every listed
  preset's catalog must be served, not refused. A refused preset fails the
  case by name. `Capability::Presets` is inferred from the live catalog, so a
  provider listing no presets skips the case with missing `presets`.
- The fake provider lists the presets it serves, with faults for a listed
  preset it refuses and for listing none.

## 0.10.0

- Breaking: exhaustive matches on `subject::Capability` need a wildcard arm.

- Require tool-provider 0.10.0. Infer command cases from the live catalog and
  exercise shape, reply deadline, stored-repeat identity, codemode/flow scope
  refusals and accepted late results through real routes. Add a scope adapter
  hook because the shared harness's identity stamp does not carry run/flow ids.
- Add deliberately faulty provider self-tests for each named case.
- Mark `Capability` non-exhaustive so future capabilities are additive.

## 0.9.3

- Use `cortexkit-role-tool-provider` 0.9.3 and read call-key and schema-pin
  vector bytes through its `test-vectors` feature instead of keeping copies in
  this crate. The suite's conformance cases and results are unchanged.

## 0.9.2

- Require cortexkit-role-tool-provider 0.9.2 and add
  `tool_result_held_reply_route_gating`. A `HeldReplyV1` suite fixture exercises
  a provider advertising `late_results` on routes with absent, empty, single
  and both consumer declarations; only the both-declared route carries `held`.
  The case validates the interim body and checks the approved call's late answer.
- Add `held_reply_call` with a backward-compatible default and runner tests for
  incorrect gating, missing markers and malformed held replies.

## 0.9.1

- Add `catalog_emits_events_boolean`, which requires nothing and checks from
  the raw `tool.catalog` answer that every tool carrying `emits_events`
  declares a JSON boolean (`null` and other types fail). It cannot verify the
  promise a `false` makes. No public type changes, so the crate version and
  its cortexkit-role-tool-provider requirement stay as they are.

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
