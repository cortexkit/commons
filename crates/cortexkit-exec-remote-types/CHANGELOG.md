# Changelog

## 0.2.2

- Producers: `TerminalRecord` gains optional `git_state_changed:
  Option<GitStateChange>`, `untracked_files: Option<UntrackedFiles>`, and
  `ignored_writes: Option<IgnoredWrites>` reports for server state that is not
  copied back. Use the matching `with_*` setters without changing existing
  constructor calls. Reporting runners send all three reports, including equal
  Git before/after values, empty untracked paths, or a zero ignored-write count.
- Consumers: absent fields decode as `None`, meaning **not reported**, never
  "nothing changed". Unknown fields inside the new reports are tolerated. Use
  `GitStateChange::changed()` to detect differences in commit IDs, symbolic refs,
  index trees, or stash counts; unavailable IDs and detached refs remain nullable.
  A remote commit, staged change, or generated file is not evidence it exists
  locally.
- Producers should cap `UntrackedFiles.paths` at 100 entries and mark omitted
  paths with `truncated`; cap `IgnoredWrites.sample_paths` at 20 entries while
  retaining the total `count`. Ignored writes exclude `target/`, `node_modules/`,
  and `dist/`. These data-only types do not enforce caps or exclusions.
- Six crate-local canonical vector pairs cover all reports, an older runner,
  nested unknown fields, detached HEAD, truncated untracked paths, and unchanged
  reports. Existing vectors and public constructors retain their wire shapes.

## 0.2.1

- `Accepted`, the first record of an `exec.run` stream, gains an optional
  `env_not_forwarded` list: the names, never the values, of caller-supplied
  environment variables the runner did not forward. Set it with
  `Accepted::with_env_not_forwarded`. It is omitted when absent, and replies from
  older runners decode as absent, which means "not reported".

## 0.2.0

- Every enum in a caller reply or stream record now preserves unrecognised tags
  in a documented catch-all: `StreamRecord`, `Killed`, `Ran`, `OutputStream`,
  `PrepareOutcome`, and `RebuildResult` join `RefusalReason` and `Outcome`.
- Unknown stream records retain an optional `u64` sequence number for advancing
  the caller's resume cursor. A present, non-`u64` sequence remains a decode error.
  Callers skip these records, never treat them as terminal, and seek the outcome
  through status or attach if the stream ends without a known terminal record.
- Known tags with malformed or missing required fields remain decode errors;
  known wire shapes are unchanged. Six new crate-local vector pairs pin unknown
  tags and their canonical round-trips.
- Breaking: `Killed`, `Ran`, `OutputStream`, and `RebuildResult` are still
  `Clone`, but no longer `Copy`, because their catch-alls own a `String`. Code
  that copied these values must clone them. 0.1.0 is yanked: it decoded these
  six enums strictly, so a new server-side value would fail a caller's decode.

## 0.1.0

- Initial caller-facing `exec-remote/v1` request, reply, and stream types, with
  raw-byte output, explicit nullable availability fields, and non-exhaustive
  public records and enums.
- Catch-alls for refusal reasons and terminal outcomes, with conservative
  caller grading and strict validation of malformed known outcomes.
- Packaged canonical JSON and SHA-256 golden vectors with contract tests.
