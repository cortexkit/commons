# Changelog

## 0.2.0

- Take compaction-view 0.4. `clean_reply` follows `PLAN_VERSION` 4: it no
  longer leaves a doubled space, or a space before closing punctuation, where it
  removes a well-formed tag from the middle of a reply. The JavaScript side is
  current-only: it never reproduces version 3, which exists only to rebuild bytes
  a runner recorded before adopting version 4.

## 0.1.2

- Take compaction-view 0.3. `plan_view`, `plan_view_at`, `apply_with_rules` and
  `check_rules` accept an optional `signed_floor` member: the id of the newest
  message whose signed reasoning the runner leaves out of every request after
  a change to the session's tools. Signed parts at or before it no longer make
  a view invalid, because the runner strips them itself. Without the member,
  every function behaves as before.

## 0.1.1

- Take compaction-view 0.2. Its new `cleared_ride` function is callable
  through the shared JSON dispatcher; every existing function behaves the same.

## 0.1.0

- First release of the WebAssembly compaction view library and its JavaScript
  loader, using the shared Rust implementation and JSON vectors.
