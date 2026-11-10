# Changelog

## 0.1.5

- Accept `subc-protocol >=0.34.1, <0.37` so consumers pinned to 0.36 can share
  one protocol copy. The re-exported `Principal`, principal-bearing display
  and wake APIs, and `ErrorBody` results use whichever protocol version the
  build resolves. No display wire-format or writer-admission change.

## 0.1.4

- Add opt-in wake records on `display.write`, with verified writer identity,
  immutable keyed-send records, bounded items and excerpts, and open reader kinds.
- Define session-aware lookup for moved rows, ambiguity fallback, and 180-day
  retention; app read and watch operations remain the display server's protocol.

## 0.1.3

- Contract §6: ask `tool.views` before judging current catalog membership,
  including for old pins, and retry once pins previously marked "not in
  catalog". Accept a view's pinned `tool` definition only when it recomputes
  to the requested pin, using its input schema without a catalog check;
  views without `tool` retain the catalog check. A definition that doesn't
  recompute to the pin is `unknown` and cannot authorize a join. A matching
  pin fixes the name, structural input schema and `semantics`, not
  `description` text or which provider owns the tool.
- Contract §6: for one call, if writes from more than one writer each have an
  accepted view for the runner's stored pin, none of them is used; this
  covers providers listing identical tools.
- Contract §3: a call has at most one completion, for the end of its deferred
  work, not approval of a hold. Approved background work completes when it
  finishes, with person-wait time in `heldMs` (omitted when there was none);
  a refusal before the action starts completes as an error; work the
  provider stops and confirms stopped completes with `reason: "killed"`.
- Add shared view-definition vectors for matching, mismatching and legacy views.

## 0.1.2

- Contract text only (§6): the display server fetches a writer's views again
  when its `module_version` or, where the catalog carries it, declared build
  changes, and may also fetch again under its own cache-age or
  template-validation policy, as tool-provider 0.14.5 §2.1 describes.

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
