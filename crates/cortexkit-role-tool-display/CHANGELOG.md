# Changelog

## 0.1.1

- Contract text only (§3): a value of an `id` field may be passed to the
  app's own item opener, which opens only the ids the app already opens from
  chat links (questions `ask_`, artifacts `art_`, work items `wi_`) as the
  app's own card, or leaves the value as plain text. Every other use stays
  plain text, never a link.
- Contract text only (§6): a view's record schema and reads stay fixed for a
  pin, and the display server fetches a view again when the writer's
  `module_version` changes (tool-provider 0.14.2 `template_revision`).
- Contract text only (§7): a provider writes once its handler has produced
  the call's reply, off the reply path, and needn't prove the reply frame was
  flushed first. No wire or API change.

## 0.1.0

- Define independent `app-server/v1` reader and `tool-display/v1` storage
  capabilities: unambiguous live display-server discovery, asynchronous writes,
  attested identities, JCS idempotency, served limits and schema-pinned joins.
- Add flat record/completion schemas, typed field validators, free-form detail
  cut metadata and shared test vectors with checksum pins.
