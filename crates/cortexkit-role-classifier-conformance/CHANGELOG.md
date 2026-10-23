# Changelog

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
