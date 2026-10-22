# Changelog

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
