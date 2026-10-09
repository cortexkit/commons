# Changelog

## 0.2.5

- `RunRequest` gains optional `network: Option<Network>`, with a `with_network`
  setter and getter. Absent requests remain offline and serialize to the same
  bytes as 0.2.4. Callers can request `Network::Outbound` (`"outbound"`) for
  internet access, for example to install packages. The runner, not these types,
  must enforce isolation: outbound access never grants access to the caller's
  machine or local network.
- `Accepted` gains optional `network: Option<Network>`, with a `with_network`
  setter and getter, for the access the runner actually granted. `None` means
  the runner predates the field, so callers cannot assume access. If a caller
  asked for `Outbound` and the acknowledgement is not `Some(Outbound)`, the job
  runs offline. Callers must not automatically rerun a job granted outbound
  access after an unknown outcome because it may have had outside effects.
- `Network` is a non-exhaustive string enum retaining future strings as
  `Unknown(String)`. Runners receiving an unrecognised network request must
  refuse before start, not downgrade or upgrade access. The new non-transient
  `RefusalReason::NetworkUnsupported` (`"network_unsupported"`) reports this
  refusal. Callers matching `Unknown("network_unsupported")` must switch to the
  known variant.
- Crate-local golden vectors cover outbound requests and grants, unknown network
  strings, the new refusal reason, unchanged offline bytes, and request/accepted
  decoders mirroring 0.2.4 that ignore the additive network fields.

## 0.2.4

- `StreamRecord::Started` reports the sequence number, runner-measured queue
  wait, and runner wall-clock start time when a queued job takes capacity. It
  follows `Accepted` and precedes output or the terminal; a refusal before start
  has no `Started` record. The runner's command timeout runs from this moment,
  not acceptance. Callers can report that the job is now running, but must not
  start their own timeout from this event. Older runners may omit it, so callers
  keep their existing behavior when it is absent. `Started` is progress, never a
  terminal record.
- A present `Started` sequence advances the attach resume cursor like every
  other sequenced stream record. Golden vectors cover its canonical wire shape,
  malformed known-tag rejection, and decoding as an unknown sequenced record by
  the 0.2.3 caller shape.

## 0.2.3

- `TerminalRecord` and `PrepareReply` gain optional `refusal_detail:
  Option<String>` and `retry_after_ms: Option<u64>`, with `with_*` setters and
  getters. Existing constructors are unchanged; absent fields decode as `None`
  and are omitted on serialization, preserving existing record bytes. The
  `RefusedBeforeStart { reason }` outcome variants are unchanged, and old decoders
  ignore the additive container fields.
- Callers may quote `refusal_detail` verbatim to explain a refusal, but must never
  parse it for decisions. Producers must send at most `REFUSAL_DETAIL_MAX_BYTES`
  (1024 UTF-8 bytes) and must never include secrets, tokens or credential material.
  The limit is a producer obligation: decoders retain longer received values
  without truncation or rejection.
- `retry_after_ms` hints that a refusal is transient. Callers must ignore it unless
  the outcome is `RefusedBeforeStart`; they may retry after waiting at most
  `min(retry_after_ms, their own remaining budget)` milliseconds. An absent hint
  means not reported, not necessarily permanent. The types do not enforce retry
  policy or cap the hint to a caller's budget.
- `RefusalReason::RunnerDraining` (`runner_draining`) and
  `RefusalReason::RunnerDiskFull` (`runner_disk_full`) are now known transient
  reasons. Callers who matched `Unknown("runner_draining")` or
  `Unknown("runner_disk_full")` must switch to the new variants.
- Crate-local golden vectors cover both container fields, unchanged older
  records, decoding by the 0.2.2 container shapes, and the two new reason strings.

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
