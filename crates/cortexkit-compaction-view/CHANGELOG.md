# Changelog

## 0.4.0 (breaking)

- **Behaviour change, `PLAN_VERSION` 4:** `clean_reply` no longer leaves a
  doubled space, or a space before closing punctuation, where it removes a
  well-formed tag from the middle of a reply. `"the file §4§ holds it"` cleans
  to `"the file holds it"`, and `"see §3§."` to `"see."` (closing punctuation
  is `.,;:!?)]`). Only a U+0020 space next to a removed well-formed tag is
  dropped. Newlines, other whitespace, the model's own spacing elsewhere,
  adjacent tags, broken tag forms, the split of leading tags and the outer trim
  are unchanged. As one JavaScript replace with `""`, after the leading tags
  are split off: `/(?: §\d+§)+(?=[.,;:!?)\]])|(?<= )§\d+§ |§\d+§/g`.
- New `clean::clean_reply_for_plan(plan_version, text) -> Option<CleanedReply>`
  (dispatcher function `clean_reply_for_plan`, input `{plan_version, text}`):
  plan version 3 and earlier clean byte for byte as before, 4 as `clean_reply`
  now does, and a newer version is `None`. Version 3 exists only to reproduce
  bytes recorded before a runner adopted version 4: a resumed run's requests,
  and offline checks that rebuild recorded requests. It is never for a new
  render or a served read.
- `PLAN_VERSION` is 4. Plan rendering is the same at 3 and 4: `plan_at` and
  `plan_view_at` accept 4 as well as 2 and 3. A v4 row pins the rendering
  outputs; v3's pins stay as history.
- **Runners that adopt this release step `history_version` once more:** served
  reply text cleaned through `clean_reply` changes for replies that hold such a
  tag, and so do compaction entries built from it and their digests.
- Vectors: `vectors/cases.json` (checked by `vectors/SHA256SUMS`) gains 23
  `clean_reply` cases for the new rule and the cases it must leave alone, and
  103 `clean_reply_for_plan` cases: every `clean_reply` input under plan
  versions 3 and 4, so the pinned difference is exactly the space rule, and one
  at version 2. Two earlier `clean_reply` outputs change as described above.
  JavaScript runners use the WebAssembly companion crate and stay current-only:
  they call `clean_reply`, because nothing on the JavaScript side replays
  recorded requests. A JavaScript implementation checked against the file reads
  it as a JSON array of `{name, function, input, output}`, calls each function
  it implements with `input` and must produce exactly `output`. It implements
  `clean_reply` and skips the `clean_reply_for_plan` cases. Every reply-cleaning
  output is `{"ok": {model_tag, text}}`, where `model_tag` is a JSON number or
  `null` and always fits in a double.
- Built on cortexkit-role-compaction 0.4.0, whose entries share content
  (`Text`), hold tool input as JSON text (`ToolInput`) and hold checksums as
  `Digest`. No function of this crate changes its signature, and apart from
  the reply cleaning above every vector and plan output is byte-identical.
- A replace of `input.<key>` parses the input, writes the member and stores the
  edited object's compact JSON text. Checking such an edit parses only the one
  member it names.
- A replace of `attachments` or `delivered` builds the kept list from the
  part's readers and sets it, since a tool part no longer exposes those lists
  as fields. This clones the kept items, which the published version kept in
  place.
- Replaced text and inserted texts are new `Text` values, one copy each as
  before. `Replace.expect` is compared with the part's `Digest` without
  formatting it.

## 0.3.0

- Signed floor input for compaction contract rule W3.5 (tool-set changes strip
  earlier signed reasoning). New `rules::Floor` (`Floor::none()`,
  `Floor::through(mid)`), a required parameter of `rules::invalid_signed_parts`,
  `rules::check`, `render::plan_view`, `plan_view_at`, `plan_view_with` and
  `apply_with_rules`, so a caller can't give the floor to one check and forget
  another. The JSON operations take an optional `signed_floor` member, the
  message the floor goes through.
  Signed parts at or before the floor count as an already-stripped oldest
  run: never invalid, never one side of a gap between kept signed parts
  (contract rule PR2), and a drop of one is not an edit to later parts. Edits
  before the floor still invalidate the signed parts after it. A floor that names no message
  covers nothing. Without this, a view editing before covered parts reported
  them as invalid, so a provider added needless drops and a runner checking
  with `plan_view` refused views the contract says it must accept.
- Checks only: covered parts stay in the plan and the rendered messages, and
  the runner's own strip removes them. Every earlier vector and plan output is
  byte-identical; six vectors are added and the rendering pin covers 91 cases.
- Breaking: the Rust functions gain a parameter. `Floor::none()` behaves
  exactly as 0.2.2.

## 0.2.2

- Bind signed thinking for Claude Fable 5.2, Mythos 5.1 and Mythos 5.2.
  Anthropic's preserved-thinking page applies the prefix check to "Claude 5.1
  and later models"; Mythos 5.1, Fable 5.2 and Mythos 5.2 enforce it on every
  account, whatever its age. `rules::defaults` now declares
  `signed_parts: bound` for all three. Without the rows a runner taking the
  defaults declared `none`, and a provider could plan an edit that breaks the
  signed thinking after it, which the API refuses with a 400. No plan bytes,
  `PLAN_VERSION` or vectors change.

## 0.2.1

- Require compaction 0.3.3 for optional entry-level agent attribution. Providers
  read attribution from `Entry`; rendered messages carry only what the model
  request needs. Model bytes, render plans and `PLAN_VERSION` stay unchanged.
- Patch bump: the dependency's added member is on non-exhaustive `Entry`; this
  library adds no public fields or signatures. The operator approved the
  carried-agent attribution contract for release.
- Readers first: Magic Context deploys reading `Entry.speaks_for` and declares
  `compaction-entry-speaks-for/v1` before any runner sends the member. Core sends
  `notice` with `speaks_for` only to runners declaring `notice_with_speaks_for`.

## 0.2.0

- Take compaction 0.3.0; the minor bump reflects the compaction types exposed
  by the public render-plan API. Existing plans and vectors are byte-identical.
- Added `render::cleared_ride(form, route_clears)`, the reference rule for
  `turn_system_cleared` rides: rendered cleared on a route that clears at the
  next user-role message, omitted on any other route, never as a persistent
  `turn_system`. Also available as the `cleared_ride` dispatcher function with
  vectors.

## 0.1.8

- Require compaction 0.2.10. Add view-aware `plan_view`, `plan_view_at`,
  `plan_view_with` and `apply_with_rules` entry points, with JSON dispatch for
  WebAssembly. They honor checksummed `built_through` and `new_signed: strip`
  only for invalidated signed parts strictly newer than an active-path anchor.
  Temporary drops never change the stored view, bypass running-tool checks,
  or remove the final protected tool part. Unknown policy values stay strict.
- Keep edit-only APIs strict. Add stripping, anchor, refusal and checksum
  vectors without changing any earlier vector or plan bytes. `PLAN_VERSION`
  stays 3; the WebAssembly adapter passes the new members through unchanged
  and needs no code or version change.
- Roll out providers that decode the new members before runners send them.

## 0.1.7

- Extend PR6 (compaction 0.2.9) in `check_final_turn` and all render-plan paths:
  when the newest entry with parts is system-role, also protect the newest
  non-system entry with parts, skipping trailing system entries. The newest
  entry remains protected. Selection uses role, not the optional notice mark;
  a trailing user notice leaves the old behavior unchanged. Refusals keep the
  existing `FinalTurnRemoved` zero-based edit index.
- Add vectors for one and two trailing system entries and a trailing user
  notice, including allowed edits and refusals. `PLAN_VERSION` stays 3: accepted
  render plans keep the same output bytes, and all earlier vector outputs and
  rendering checksum pins are unchanged. The WebAssembly adapter delegates to
  the same planner and needs no code change.

## 0.1.6

- `plan`, `plan_at` and `plan_with` refuse a view that removes the turn the
  request ends on (the compaction contract's rule PR6, compaction 0.2.7),
  with the `rule` refusal `final_turn_removed` naming the edit
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
