# Changelog

## 0.2.10

- Add chunked returned files, so a runner can return up to 10 MiB of files when
  one `return_result` record would be too large. A caller opts in with
  `RunRequest.return_chunked` (`with_return_chunked`, omitted when false), and
  only for a runner advertising `RETURN_CHUNKED_CAPABILITY`
  (`"exec-remote-return/v2"`). The cap then defaults to
  `RETURN_CHUNKED_MAX_BYTES_CAP` (10,485,760 bytes); larger requests are refused
  as `invalid_params`. `RETURN_MAX_BYTES_CAP` and `return_result` are unchanged
  for requests without the flag.
- Add `StreamRecord::ReturnChunk` (`return_chunk`): up to
  `RETURN_CHUNK_MAX_BYTES` (524,288) bytes of one file, grouped by path and
  contiguous from offset 0.
- Add `StreamRecord::ReturnManifest` (`return_manifest`), sent once before
  `terminal`, with one `ReturnManifestEntry` per requested path: returned (size
  and SHA-256) or failed (cause and optional size), exposed as
  `ReturnManifestOutcome`. Decoding refuses an entry with both or neither of
  `sha256` and `cause`, or `sha256` without `size`.
- Add `ReturnPathsFailureCause::ReadError` (`read_error`) and `Expired`
  (`expired`). After the runner's retention window, a replay sends the manifest
  at its original seq with returned entries turned into `expired`, and no
  chunks; a chunked caller must accept the seq gap this leaves before the
  manifest.
- Add `ReturnAssembler`, a pure checker shared by callers and producers'
  tests. It checks chunk order, contiguity, size and the cap, then the
  manifest's paths, sizes, digests and total, and returns `VerifiedReturn`
  values or a `ReturnStreamError`. A chunk error names the path index and
  the chunk's offset, a manifest error names the path index, and
  `TotalMismatch` concerns no single path.
- `sha2` becomes a normal dependency (it was a dev-dependency).
- Add 21 vectors under `test-vectors/exec-remote-v1/return-chunked/`. Existing
  vectors are unchanged.
## 0.2.9

- Add optional `TerminalRecord.hosted` placement and billed virtual-CPU
  milliseconds for hosted jobs; pending billing and build-server jobs omit it.
- Add `RunRequest.return_paths` and `return_max_bytes`, plus the 262,144-byte
  `RETURN_MAX_BYTES_CAP`. Runners must refuse an excessive cap as `invalid_params`
  naming `return_max_bytes`; non-empty paths gate the new result record.
- Add `StreamRecord::ReturnResult` and returned-file/failure types. Returned bytes
  decode from base64, failure causes remain open-ended, and exactly one result
  payload is required. A collection failure does not change the terminal outcome.
- Add vectors for two returned files and each known and unknown failure cause;
  older stream decoders keep `return_result` as an unknown sequenced record.

## 0.2.8

- Add `RefusalReason::LostToRestart` (`"lost_to_restart"`) for an admission that
  had not started and was lost during runner recovery. The caller may resubmit
  fresh under its own time budget; only the refused-before-start contract, not
  the tag alone, guarantees that the job never started.
- Add `RefusalReason::RequestRejected` (`"request_rejected"`) for an operation or
  parameters that are not permitted. Correct or avoid the request rather than
  retry it unchanged. `refusal_detail` carries the specific human-only code,
  such as `prepare_unsupported` or `direct_timeout_refused`.
- Add `RefusalReason::RunnerAuthenticationFailed`
  (`"runner_authentication_failed"`) when the executor cannot authenticate the
  job to its runner. The job's caller cannot fix this, and a retry is not expected
  to help. `refusal_detail` carries a safe human-only fault code, never credential
  material.
- Existing variants, constructors, wire bytes and `Unknown(String)` round-trips
  are unchanged. Older readers retain all three tags as unknown strings. Three
  appended golden vectors pin the new refusals without changing existing vectors;
  the types describe the reasons but do not enforce retry policy.

## 0.2.7

- `Outcome::WindowsExit` carries the full unsigned Windows process exit code.
  NTSTATUS failure codes identify crashes and should be displayed in hexadecimal;
  callers that do not recognize the tag must grade it as `outcome_unknown`, not
  as proof the command did not run.
- `Started` gains an optional `PathMapping`, omitted when absent to preserve
  existing wire bytes. A runner that rewrites paths reports the caller-side
  `host_prefix` and runner-side `guest_prefix`; caller paths map by appending the
  path with `/` changed to `\`. Jobs are refused if a path cannot be mapped;
  output paths inside the mapped folder are translated back and other paths pass
  through unchanged.

## 0.2.6

- `RunRequest` gains optional `platform: Option<Platform>` with a
  `with_platform` setter and getter. Absent means Linux and is omitted during
  serialization, preserving existing request bytes. `Platform::Linux` and
  `Platform::Windows` use the wire strings `"linux"` and `"windows"`; unrecognised
  strings round-trip as `Platform::Unknown`.
- `Accepted` gains optional `platform: Option<Platform>` with a `with_platform`
  setter and getter. A runner that understands platform requests always reports
  the platform it will actually use. Callers who requested a non-Linux platform
  must cancel without trusting the result if acceptance does not acknowledge the
  exact requested platform, including when an older runner omits the field.
- A runner must refuse an unavailable or unrecognised requested platform before
  start with `RefusalReason::PlatformUnsupported` (`"platform_unsupported"`),
  reporting the requested platform wire name in `refusal_detail`. Golden vectors
  pin Linux and Windows acknowledgements, unsupported and unknown refusals. Tests pin absent-platform bytes.

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
