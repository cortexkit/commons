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
only when the request contains a non-empty `return_paths`; it sends the record
immediately before the terminal record. This request-field gate keeps the new
record out of streams for callers that cannot request returned files. The
exec-remote/v1 types define no capability handshake; a caller must send non-empty
`return_paths` only to a runner that advertises support. ck-motor advertises this
support through its own `Hello` capability.

`return_max_bytes` caps the total bytes in the returned files. If absent, the cap
is `RETURN_MAX_BYTES_CAP` (262,144 bytes); requests above that cap must be rejected
with `invalid_params` identifying `return_max_bytes`. The caller-facing types do
not implement runner request validation.

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
