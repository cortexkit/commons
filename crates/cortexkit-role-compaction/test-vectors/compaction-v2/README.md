# Compaction wire examples

`entries-numbers-unknown.json` and `notice-numbers-unknown-request.json` serve
tagless numbered-kind parts with `numbers_unknown: true` to a provider declaring
`compaction-numbers-unknown/v1`. `numbers-unknown-transition.json` pairs a
previously numbered entry with its marked form: role, notice and part digests
stay the same, while the entry digest changes because the tags are absent.
`checksum-numbers-unknown-marker.json` isolates the marker's exclusion from
checksums; its unmarked form is a checksum input, not a served entry.
`refusal-numbers-unknown.json` permanently refuses the whole tag selection
covering the requested numbers in its detail; `view-cut-before-numbers.json`
names the last unknown-number entry that an accepted view must cut through.
The view library's `vectors/cases.json` includes a plan that cuts out tagless
unknown-number entries and keeps the numbered entries after them.

`entries-host-parts.json` includes a mixed user entry with a whole-block
interrupt marker and a host-only retry entry, marked by stable part id. The
retry entry has no code. `entries-host-parts-legacy.json` carries the same entries
without facts for a provider not declaring `compaction-host-parts/v1`. Part and
entry digests are byte-identical between the two pages; host text is never the
person's prompt, and the host-only entry is not a user turn or summary boundary.

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

`refusal-transient-index-filling.json` and
`refusal-transient-index-filling-retry-after.json` are `session.entries`
refusals while the requested history's part-number index is filling, without
and with an optional retry delay. Once the runner can answer the refused read,
`notice-transient-read-wake-request.json` wakes the session with the next `seq`
after `notice-request.json`, unchanged conditions, newest cursor and applied
view, and no entries or events. It is the only notice allowed without a change;
the provider still reads any gap on every notice (N4).

`view-rule-final-turn.json` is the `rule` refusal a runner sends for a view
that removes the turn the request ends on (PR6).

`notice-undo-behind-cut-request.json` reports an undo that removed the message
version 2's applied cut ran through (`message-cut-removed`, W6.6). The runner
drops that view and its window: `applied_view` is absent. `view_refused` names
the removed cut message with `unknown_message`, and `transcript_changed`
starts from the newest retained entry (`message-a`, ordinal 1), which is
also the notice's `newest`. Until a new view arrives, moved-in sessions
(MV2, MV6) and sessions past the host's compaction marker take no step and
end `compaction_unavailable`; other sessions render unreduced. Non-cut edits
still follow W6.6's filtering when the cut's message remains.

`notice-stale-window-request.json` is an independent step-boundary refusal
of version 3: the window names `message-b`'s missing `part-missing`, so the
runner reports `view_refused` with `unknown_part`. No view applies, so
`applied_view` is absent. Unlike the undo case, no transcript change is
reported and `entries` is absent. Both examples use invented identifiers
and the existing notice and refusal shapes.

The `notice-render-failed-*-request.json` examples are independent notices to
a provider declaring `compaction-render-failed/v1`, not three reports for one
version (W6.5/W6.6a):

- `notice-render-failed-part-request.json` means version 7 is served minus
  every replacement of `message-b`'s `part-b`. The `attachments` field only
  narrows the diagnosis; it does not narrow the skipped replacements.
- `notice-render-failed-message-request.json` means no view is served for
  version 7 and the runner knows the message (`message-b`).
- `notice-render-failed-unlocated-request.json` means no view is served for
  version 7 and the runner cannot locate the failure.

The runner decides its first-step fallback before reporting, so a one-skip
candidate that still fails uses one of the no-view shapes, never the part
shape. A later escalation from a reported one-part skip to no view sends
exactly one more report, also in one of the no-view shapes. No third report
is sent, and non-declaring providers receive none of these reports.

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
`ride-statement-seq-request.json` carry the same epoch, state stamp (`42`) and
signed floor for `compaction-statement-seq/v1` providers. Equal stamps are
processed normally; the notice also carries next-request surface counts.
Within that epoch, lower stamps give no runner state, including size readings,
but their entries and events are still processed. The restored-session Setup in
`setup-statement-seq-restored-request.json` replaces the epoch, resets the held
stamp to `7` and reports no floor. A late statement from the replaced epoch is
dropped whole, including its notice number; a dropped notice answers
`setup_required`, and a dropped ride is refused `unknown_session`. Older Setup,
notice and ride vectors omit both epoch and stamp for non-declaring providers.
`notice-statement-seq-restored-request.json` uses the new epoch with stamp `8`
and restarts the notice number at `1`, unaffected by a late old-epoch notice.

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
