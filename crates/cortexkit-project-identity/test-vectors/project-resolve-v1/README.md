# Project resolution v1 samples

`resolve.json`, `resolve-root-records.json`, `resolve-project-id.json`, `resolve-remote.json`, and
`enumerate.json` hold replies captured from a running project-identity handler.
The handler's maintainer publishes the same five files and `SHA256SUMS` as the handler's own test data,
and they are copied here unchanged. The crate's checksum test checks each file
against `SHA256SUMS`. Checking the handler's live replies against these types
is the handler's own test, outside this crate.

Paths were normalized to `/fixture`, implicit ids re-derived from those paths,
and the generated registration epoch normalized to
`00000000000000000000000000000001`. The clock is fixed at `700` and process
incarnation at `0123456789abcdef`. Otherwise embedded replies preserve exact
UTF-8 bytes, including their `result` envelope and key order.

Each file contains an array of `{request, principal, reply}` records. `request`
is the full `{method, params}` envelope. `principal` is capture metadata, not an
operation parameter. `reply` is either `{result: ...}` or a top-level refusal.

- `resolve.json`: exact root, ancestor walk, external worktree, implicit path,
  removed registered root, and the missing-`canonicalRoot` refusal.
- `resolve-root-records.json`: root records in detail. An `unbound` root with
  two Git remotes, one owned and one not; an `unbound` root whose Git config
  can't be read, so it has no remotes and carries `remotesError`; the first
  root again, `bound` and approved with a journal sequence; then `replaced`;
  and a `retired` attached worktree found by `containment`, with `sourceRoot`.
  A retired identity appears only on an attached worktree.
- `resolve-project-id.json`: current, alias, and gone durable ids.
- `resolve-remote.json`: a unique match, no match, ambiguous project ids, an
  ASCII case-insensitive match, a root before its Git config becomes unreadable,
  no match after that root is skipped, another project found despite the bad
  root, and the missing-`owner` refusal.
- `enumerate.json`: an unfiltered request and two local projects in a workspace.

Tests compare each compact embedded reply byte for byte after decoding and
serializing through sorted-key JSON values. Reformatting a whole file would
still violate its SHA-256 even if all embedded replies remained equivalent.

```sh
sha256sum -c SHA256SUMS
```

The adjacent `../synthetic-forward-compatibility/` fixtures are invented decoder
inputs, **not captured handler replies**. They must not be substituted for
missing real-handler coverage. See the crate README for the real-handler sample
gaps.
