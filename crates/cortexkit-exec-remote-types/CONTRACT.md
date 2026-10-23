# exec-remote/v1 caller contract

This crate defines caller-facing values for `exec-remote/v1`. It carries protocol
shapes; runners remain responsible for enforcing request limits, collecting files,
and emitting records in the required order.

## Hosted placement and billing

A hosted runner may add `TerminalRecord.hosted` with its placement identifier, such
as `ovh-1` or `boat:<sandbox_id>`. `billed_vcpu_ms: null` or an absent field means
billing is pending. Build-server jobs do not report `hosted`; absence is not an
indication of failed execution.

## Returning requested files

`RunRequest.return_paths` lists paths relative to the request's `cwd`. It is omitted
when empty. An empty list disables returned-file collection, and the runner must
not emit a `return_result` record for that request. A runner sends `return_result`
only when the request contains a non-empty `return_paths` and does not set
`return_chunked` (see "Returning files in chunks"); it sends the record
immediately before the terminal record. This request-field gate keeps the new
record out of streams for callers that cannot request returned files. The
exec-remote/v1 types define no capability handshake; a caller must send non-empty
`return_paths` only to a runner that advertises support. ck-motor advertises this
support through its own `Hello` capability.

`return_max_bytes` caps the total bytes in the returned files. Without
`return_chunked`, an absent value selects `RETURN_MAX_BYTES_CAP` (262,144 bytes),
and requests above that cap must be rejected with `invalid_params` identifying
`return_max_bytes`. The caller-facing types do not implement runner request
validation.

A successful `return_result` contains `returned_files`, possibly an empty list.
Each file's `path` is cwd-relative, `sha256` is the lowercase digest of the
contents, and `data` is standard padded base64 on the wire. A failed collection
contains `return_paths_failed` with measured and cap byte totals plus a cause.
Exactly one of `returned_files` and `return_paths_failed` must be present; callers
reject a record with both or neither. Known causes are `over_cap`, `missing`,
`escape`, `not_regular`, and `frame_limit`. The cause is open-ended: callers
preserve unrecognized strings rather than failing to decode the record.

A collection failure never changes the job's exit outcome. The runner must still
send the terminal record describing the command's actual outcome; callers must
not interpret a `return_paths_failed` cause as a process failure.

The `return_result` record is a known tag to this version of the types. Older
callers treat it as an unknown stream record, skip it, and retain its `seq` for
resume-cursor advancement. It is nonterminal, so only `terminal` ends the job
stream.

## Returning files in chunks

One `return_result` record carries at most 262,144 bytes of files. Chunked
return lets a runner send up to 10 MiB, split across records small enough for
its record size limit. Everything in this section is additive: `return_result`
keeps its exact shape and meaning, and a request without `return_chunked`
behaves as before.

### Opting in

`RunRequest.return_chunked` (default false, omitted from the wire when false)
asks for chunked return. A caller sets it only for a runner that advertises the
capability `RETURN_CHUNKED_CAPABILITY` (`exec-remote-return/v2`). ck-motor
advertises it in its `Hello`, as it does for `return_result` support.

With `return_chunked` set and `return_paths` non-empty, the runner sends zero or
more `return_chunk` records, then exactly one `return_manifest`, immediately
before the terminal record. It never sends `return_result` for that request.
With `return_paths` empty, `return_chunked` has no effect and no returned-file
record is sent.

With `return_chunked`, an absent `return_max_bytes` selects
`RETURN_CHUNKED_MAX_BYTES_CAP` (10,485,760 bytes). A value above it is rejected
with `invalid_params` naming `return_max_bytes`.

### `return_chunk`

`{type: "return_chunk", job_id, seq, path_index, offset, data}`. `path_index` is
the file's position in `return_paths`, from 0. `offset` is the position of
`data` in the file. `data` is standard padded base64 of 1 to
`RETURN_CHUNK_MAX_BYTES` (524,288) bytes. Chunks may be smaller than the maximum.

- Chunks come grouped by path, in ascending `path_index`. The runner never
  interleaves two paths' chunks and never returns to an earlier path.
- Each path's chunks are contiguous from offset 0: every chunk starts where the
  previous chunk for that path ended.
- A zero-byte file has no chunks.

### `return_manifest`

`{type: "return_manifest", job_id, seq, files, total_bytes, cap_bytes}`.
`files` has one entry per requested path, in request order. Each entry echoes
its requested path exactly as the caller sent it. An entry is either:

- returned: `{path, size, sha256, chunks?}`, where `sha256` is the lowercase
  hexadecimal SHA-256 of the file's bytes; or
- failed: `{path, cause, size?, chunks?}`, where `size` is the file's size when the
  runner knew it, for example for an over-cap or expired file.

Exactly one of `sha256` and `cause` is present. Decoding refuses an entry with
both or neither, and a `sha256` without `size`. `total_bytes` is the sum of the
returned entries' sizes. `cap_bytes` is the effective cap.

`chunks`, when present, is a `u32` count of the `return_chunk` records originally
sent for that path, regardless of their sizes. A producer that sets it sets it
on every entry. It is 0 for an empty file or a path refused before its first
byte, the full count for a returned file, and the count sent before failure for
`read_error`. Expired replay preserves every entry's original count. Older
producers omit `chunks`; it is not serialized as null when absent. A manifest
with counts on only some entries is malformed.

Per-path failures use `ReturnPathsFailureCause`. In chunked mode the causes
mean:

- `missing`: no such path.
- `not_regular`: the path is not a regular file.
- `escape`: the path, or a symlink it resolves through, leads outside the job
  root.
- `over_cap`: the file would take the total past the cap. This is decided
  before any of its bytes are sent.
- `read_error`: reading failed, or the file's bytes changed while it was read.
- `expired`: a replay after the runner stopped keeping the job's returned bytes
  (see Replay).
- `frame_limit` does not occur in chunked mode.

Unrecognised causes still decode as `Unknown`, keeping the raw string.

### Runner rules

These rules bind the runner; the types do not enforce runner policy.

- The runner decides the cap for each file before sending any of its bytes and
  reads at most the size it admitted. The total bytes sent never exceed the cap.
- A file whose bytes change while it is read, ending up with fewer or more bytes
  than admitted, ends `read_error`. Chunks may already have been sent for it.
  `read_error` is the only cause that may follow chunks for its path.

### Caller rules

A caller places a returned file only after the manifest verifies it: the bytes
received for the path must match the entry's `size` and `sha256`. For example,
the caller writes chunks to a temporary file and renames it into place after
verification. The bytes received for a failed path are discarded. A failed entry
whose cause this version does not recognise also discards its chunks without
an error, because a newer runner may define a cause that can follow chunks.

`ReturnAssembler` applies these rules without doing IO, so a caller and a
producer's tests check streams the same way. `ReturnAssembler::new(return_paths,
cap_bytes)` starts a check. `accept_chunk` checks each chunk's `path_index`
range, the grouping, contiguity, the 1 to 524,288 byte chunk size and the
running total against the cap, hashes the bytes, and returns them for the caller
to write. `finish` checks the manifest's entry count, that each entry echoes
its path exactly, each entry's `chunks` against the received record count when
present (including `read_error`), each returned entry's size and SHA-256,
`total_bytes`, and that only `read_error` or an unrecognised cause follows chunks. It returns one
`VerifiedReturn` per requested path. On any `ReturnStreamError`, which names
the path index and, for a chunk, its offset, the caller discards every byte
received for the job's returned files.

### Replay

While the runner keeps the job's returned bytes (ck-motor keeps them for one
hour), a replay through `exec.attach` re-sends the same `return_chunk` records
and `return_manifest` at the same `seq` values.

After that, a replay sends no `return_chunk` records. It sends the manifest at
its original `seq`, with every returned entry replaced by a failed `expired`
entry carrying its original `size`. Failed entries keep their causes, and
`total_bytes` is 0. Retention is all or nothing per job, never part of a path's
chunks.

Because the chunks are no longer sent, the replay has a gap in `seq` where they
were: between the last record before the chunks and the `return_manifest`. A
caller that set `return_chunked` must accept a gap in exactly that place, so a
strict gap check cannot stall the replay. The gap can only be where
`return_chunk` records were, before the manifest; every other gap keeps its
existing meaning. The resume cursor advances to the manifest's `seq` as for any
other record.

When every entry carries `chunks`, the number of missing sequence numbers
between the job's last output record and the replayed manifest must equal the
sum of `chunks` over **all** entries, whatever the chunk sizes. In other words,
`manifest.seq - last_output_seq - 1 == sum(chunks)`, including chunks from paths
that ended `read_error`. This distinguishes expired chunks from lost output.
Without counts, the older rule still assigns that gap to return chunks, but
the gap is unverified.

Use `ReturnAssembler::finish_expired_replay(manifest, last_output_seq)` for this
replay instead of `finish`. It refuses any received chunks, requires counts on
all entries or none, and checks the gap against their sum when present, returning
`ReplayGapMismatch` on a mismatch. It still checks paths, outcomes and totals,
but does not compare historical per-path counts to the replay's zero received
chunks. Use normal `finish` for live streams and retained replay; it refuses
`ChunkCountMismatch` for any entry whose count differs from the chunks received.

### Compatibility

`return_chunk` and `return_manifest` are known tags to this version of the
types. Older callers never request them, and would treat them as unknown stream
records, skip them, and retain their `seq`. Both records are nonterminal.
