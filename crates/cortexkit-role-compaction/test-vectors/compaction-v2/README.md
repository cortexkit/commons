# Compaction wire examples

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
