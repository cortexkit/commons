# tool-display/v1 vectors

`wakes.json` contains tagged wake writes covering every kind, an exact 16 KiB /
64-item record with `omittedItems`, a notice delivered with a tool result under
the same key, an awaited-work wake with a display-only `notice_record` pointer,
legacy wakes without that pointer whose canonical bytes are unchanged, refusal
cases, and same-session, moved-row and ambiguous lookups. All agents and names
are invented.
Wake refs and identities use snake_case; `omittedItems` keeps the display marker's
existing spelling. Lookup examples describe server behavior, not an app read op.

- `canonical.json`: equivalent bodies with differing spacing, key order and
  number spelling, with the exact RFC 8785 JCS bytes.
- `field-kinds.json`: valid and invalid scalars for all nine scalar field kinds,
  including traversal, drive/UNC paths, uppercase or suffixed origins and app paths.
- `records.json`: required and fixed fields, held reasons, location, declared
  scalars and arrays, forbidden nested and undeclared fields, and completions
  with valid and invalid `text_lines` values and completion-field declarations.
- `details.json`: free-form detail objects and standard cut metadata.
- `writes.json`: limits, wire params, stored/already-stored/conflict/too-large
  answers and invalid params, with the retryable not-ready code.
- `messages.json`: exact JCS `display.write` requests for `message`, all media
  kinds and diff selectors, malformed-member paths, writer admission, and
  identical/conflicting retries across the writer rename. The maximal request
  has a 16 KiB body with 8 media items and 200 diff files. Refusal cases name a
  request and a JSON pointer to replace, remove or append to. `clear_padding`
  empties the maximal sample's padding title before a malformed-member edit,
  so the byte limit does not mask the member refusal.
- `reads.json`: a management-surface pull request, its unchanged write params
  reply, final denied/unknown refusals, invalid params, retryable not-ready, and
  the identifier-only record-ready event without a reader suffix.
- `views.json`: copied byte for byte from
  `crates/cortexkit-role-tool-provider/test-vectors/tool-provider-v1/views.json`,
  including a manifest and an unknown pin. Both packages validate these bytes,
  and the provider's `tests/views.rs` checks the two copies are identical.
- `view-tools.json`: copied byte for byte from the same provider-owned vector
  directory. Pins matching and mismatching tool definitions, and a legacy view
  whose missing `tool` keeps its serialized bytes and requires a catalog check.
- `notices.json`: notice writes and their served shapes, including finished,
  failed, still-running and watch notices, a folded pair, and a record-key-only
  notice whose `send_id` is never sent to the agent. A capped write reaches the
  name, version, key, send id and body limits. Refusals modify the first write
  unless they name the capped write; an `append` adds bytes to the string at the
  given JSON pointer. `malformed` cases fail structural decoding.

`SHA256SUMS` covers every JSON file and this README. Tests read every JSON file;
`scripts/gate.sh fast` at the repository root verifies every checksum.
