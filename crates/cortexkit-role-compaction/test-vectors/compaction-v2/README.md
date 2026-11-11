# Compaction wire examples

`view-anchor-shapes.json` pins Setup's absent, false and true `view_anchor`
flags and sealed view bytes. Without true, both policy members are absent and
the view retains the 0.2.9 shape and checksum.

`delivered-rides.json` shows legacy and new ride entries with optional
`send_id` and ordered `forms`, plus a later ride-text `tokens` count that
leaves part and entry digests unchanged.

These invented requests, answers and checksum examples round-trip through the
crate's wire types. `entries-token-counts.json` and
`notice-token-counts-request.json` include optional counts in one token family,
separate tool input/output counts, a file count, both reasoning measurement
states and both surface-count exactness values. The page's `token_family_exact`
is `true` and the notice's is `false`. Digests inside those examples
are unchanged when count metadata is removed (W7).

`view-rule-final-turn.json` is the `rule` refusal a runner sends for a view
that removes the turn the request ends on (PR6).

`setup-child-harness-request.json` names the harness for the next child on
the same runner. `applied-view-attributed.json` carries the recorded applying
module id; the older applied-view examples omit it. The signed-policy apply
request carries both optional view members inside its checksum; older views
omit them and retain their checksums.

`notice-notes-declared-request.json` separates pending next-request plan and tool
notes from system text for `compaction-notes-tokens/v1` providers.
`notice-notes-legacy-request.json` omits the separate note count and keeps those
notes inside `system_tokens` for non-declaring providers.

`setup-statement-seq-request.json`, `notice-statement-seq-request.json` and
`ride-statement-seq-request.json` carry the same state stamp (`42`) and signed
floor for `compaction-statement-seq/v1` providers. Equal stamps are processed
normally; the notice also carries next-request surface counts, while size
readings retain their separate staleness rule. The restored-session Setup in
`setup-statement-seq-restored-request.json` authoritatively resets the held stamp
to `7` and reports no floor. After a restore the runner sends Setup first and
sends no more statements built before the restore. Older Setup, notice and ride
vectors omit the stamp for non-declaring providers.

`setup-limits-request.json` advertises runner-owned page, named-read and window
bounds, plus `window_over_budget` detection for a `compaction-limits/v1`
provider. `setup-limits-legacy-request.json` carries the same bounds but omits
the capability-gated cause: the page ceiling applies to every provider.
`notice-window-over-budget-request.json` reports the soft window state, and
`ride-window-over-budget-request.json` asks for a view with that cause alone,
without implying cache loss. `entries-unanswered-mids.json` and
`entries-unanswered-tags.json` preserve the remaining held names for a declaring
provider to read next. `refusal-select-too-many.json` pins the named-read cap's
machine-readable detail. Legacy named reads stay full and omit `unanswered`.

Regenerate JSON from the repository root after an intentional wire change:

```sh
UPDATE_COMPACTION_VECTORS=1 cargo test -p cortexkit-role-compaction --all-features --test vectors wire_vectors_round_trip -- --exact
```

The generator preserves unchanged files. JSON uses sorted object keys, pretty
printing and one trailing newline. Refresh the byte checksums from this directory:

```sh
LC_ALL=C sha256sum *.json README.md | LC_ALL=C sort -k2 > SHA256SUMS
sha256sum -c SHA256SUMS
```

`SHA256SUMS` covers every file here except itself; these byte checksums are
distinct from the wire's selected-content JCS digests.
