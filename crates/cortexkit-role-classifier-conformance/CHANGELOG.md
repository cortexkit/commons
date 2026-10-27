# Changelog

## 0.6.0

- Depends on `cortexkit-role-classifier` 0.5.0, re-exported as `wire`, whose
  item image may be a reference `{handle}` (breaking: `Image` gains a
  variant and is non-exhaustive).
- New optional subject hook for image references:
  `ClassifierSubject::issue_image_reference` (hand bytes to the image
  source the module resolves through, get the handle back) and
  `set_image_source_down`. Both have defaults, so a subject without the
  hook keeps compiling.
- Five new checks: `image_reference_follows_declaration` (an unissued
  handle is `image_unavailable` on a module that declares
  `image_reference`, `invalid_params` on one that does not),
  `image_reference_resolved_and_answered`,
  `image_reference_oversized_refused_like_inline` (the inline refusal at
  `items[0].images[0]`), `image_source_down_refused_transient` (with
  `retry_after_ms`), and `image_reference_identity_is_its_handle` (the same
  handle is a replay; the same picture inline is `batch_id_reuse` naming
  `items[0]`). The last four are not applicable without the hook or the
  declaration.
- The image narrowing now says byte limits are checked through a resolved
  reference, and a new narrowing says the stand-in does not show the bytes
  a resolved reference reached the provider with.

## 0.5.0

- Moves to subc-protocol 0.33; take subc-protocol 0.33, subc-transport 0.14, subc-control 0.33 and subc-client-rs 0.31 together.
- Breaking dependency update: `Reply::Error` contains subc-protocol 0.33's
  `ErrorBody`. The `wire` re-export moves to cortexkit-role-classifier 0.4.3.
- No behaviour changes; existing test-vector bytes are unchanged.

## 0.4.1

- `README.md` explains that the adapter behind `ClassifierRoute` opens its
  route with `role_versions` naming `classifier/v1`; the conformance kit does
  not open routes itself.

## 0.4.0

- Depends on `cortexkit-role-classifier` 0.4.0, which it re-exports as
  `wire`; that release changes `request::check_catalog_row`'s signature.
- The named kindless-row check rejects a catalog row with no `kind` as
  `model_not_classifier` and requires the refusal to omit `detail.kind`. A
  faulty module that answers `model_unknown` for that row fails that check.

## 0.3.0

- Depends on `cortexkit-role-classifier` 0.3.0, which it re-exports as
  `wire`; that crate moved to `cortexkit-model-batch` 0.2.0. The checks are
  unchanged.

## 0.2.0

- Depends on `cortexkit-role-classifier` 0.2.0, which it re-exports as
  `wire`. The checks are unchanged: they script errors by HTTP status only,
  so a module's mapping of Workers AI codes is checked by the role crate's
  vectors, not by this suite.

## 0.1.0

- First published release of the `classifier/v1` conformance suite. It drives
  a live classifier module over its real management route against a
  call-counting stand-in provider, and checks every reply against the
  `cortexkit-role-classifier` wire types. A module adds it as a
  dev-dependency.
- The role's vectors ship in `cortexkit-role-classifier` behind its
  `test-vectors` feature; this suite reads no files outside its own package.
- `ServedModel::context_tokens` is an `Option<u64>`: `None` when the catalog
  states no exact context, and `describe_complete` then requires the
  `role.describe` entry to leave `context_tokens` absent.
