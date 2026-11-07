# Changelog

## 0.1.6

- `plan`, `plan_at` and `plan_with` refuse a view that removes the turn the
  request ends on (the compaction contract's rule PR6), with
  `unsupported_edit` naming the edit
  after which nothing that must stay is left. The request ends on the newest
  entry that has parts: entries with no parts, such as a step the runner has
  opened but not filled or a message whose tool results ride inside the
  calls, render nothing. When that entry has a tool call, its last call must
  stay (its output may still be replaced); otherwise at least one of its
  parts must stay, and a cut may not reach it. `check_final_turn` runs the
  same check on its own. Seven new vectors; every earlier vector,
  `PLAN_VERSION` and the pinned rendering checksums are unchanged.

## 0.1.5

- `first_change(before, after)` compares two render plans of the same
  transcript and returns where the second first differs: an inserted message
  or a transcript entry, by its index in the plan's items, the end of the
  shorter plan, or `Unchanged`. `FirstChange::from_first_message` says the
  difference starts at the request's first message. Then a runner can also
  change the system text or the tool list without losing any more of the
  provider's prompt cache. The JSON dispatcher (`vectors::call`, which the
  WebAssembly build exposes) takes `{before, after}` for `first_change`.
  Three new test vectors; every earlier vector, `PLAN_VERSION` and the
  pinned rendering checksums are unchanged.

## 0.1.4

- `plan` and `plan_with` no longer rebuild and reseal every kept part to work
  out its empty-result label. An unedited part's body is read as it stands,
  and an edited one has only its body computed. A 500-message window with no
  edits drops from about 20 ms to well under 1 ms per call. Plans and every
  vector are byte-identical, and `PLAN_VERSION` is unchanged.

## 0.1.3

- Plans list the sends that ride a tool result: each tool part's `rides`
  (`index` into its `delivered` list plus the `Delivered` record), in delivery
  order, left out when empty. Runners render them only from the plan, after
  the output through `cortexkit_role_compaction::rides`, or as a turn-scoped
  system message where the send's `forms` allow. The plan doesn't choose the
  form.
- `PLAN_VERSION` 3: a ride renders only while its carrying output does, so a
  view that replaces the output drops the result's rides unless it names
  `delivered` itself. Version 2's 56 rendering cases render byte for byte
  the same.
- `plan_at(version, …)` and `RideRule` render an applied view at the version
  a runner first served it under: version 2 keeps rides after a replaced
  output. The vector dispatcher, which the WebAssembly build exposes too,
  answers `plan_at` calls with `{version, entries, edits}`.
- `apply` shows the same rides as the plan on each tool part.
- Requires `cortexkit-role-compaction` 0.2.6.

## 0.1.2

- 24 more `clean_reply` vectors: reply-cleaning edge cases (adjacent and
  doubled section signs, section references, broken tag endings, tag zero,
  oversized and lone tags, non-breaking spaces). The library's behaviour is
  unchanged, and `PLAN_VERSION` stays 2.
- The rendering pin now covers growing prefixes of the vector cases: the
  latest version's row lists prefix checkpoints, and cases added at the end
  get a longer prefix in the same row, so new coverage needs no version bump.
  With the committed checksums unchanged, a changed output in any covered
  case, old or new, fails the pin; for an intentional rendering change,
  release review bumps `PLAN_VERSION` and adds a row.
- `vectors/SHA256SUMS` matches the new `cases.json`.

## 0.1.1

- Requires `cortexkit-role-compaction` 0.2.1.
- Inserted images (contract W6.5): `plan` refuses, as `unsupported_edit`
  naming the edit index, an image on a message whose role isn't user, a media
  type other than `image/png`, and data that isn't standard padded base64
  (standard alphabet, length a multiple of four, `=` only at the end, zero
  unused bits before it; PNG contents are not checked). The
  render plan lists each insert's images, and the neutral render adds one
  inline file part per image after the texts, with the decoded size.
- `split_leading` returns the first leading prefix's number only. When it
  doesn't fit a `u64`, the result is `None`; a later prefix no longer stands
  in for it (W4.6, T7).
- `PLAN_VERSION` is 2.

## 0.1.0

- First release of the shared compaction view library, with render plans, view
  application, provider-rule checks, reply cleaning, tag numbering, and JSON
  vectors.
