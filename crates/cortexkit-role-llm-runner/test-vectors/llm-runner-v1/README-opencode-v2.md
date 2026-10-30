# OpenCode 2 stored-session and display vectors

These vectors cover the additive `opencode-v2` profile, not the older
`opencode-aisdk` profile. The original captures used invented content with
OpenCode **2.0.26** and **2.2.3** on Linux, against a loopback mock provider.
They were recorded in disposable home, configuration and working directories,
without real credentials. The recorder inspected only its own SQLite stores,
read-only, and kept native SQL snapshots separate from provider projections.
The image is a generated one-pixel PNG. No database is distributed.

## Provenance and extraction

| Version | Associated OpenCode source commit | Original capture SHA-256 |
| --- | --- | --- |
| 2.0.26 | `9b4ec5714d481559990db0a816d5dec19541a814` | `e093940553cf318c15d8b87015586e95e52bb133e4394bb2f5b34e050400a350` |
| 2.2.3 | `082ac6281fd395c2d0c17fd3c38214db2f89621f` | `9a889986eb00906d6f0ef65dd761970486f23ac386d984515385ed5d1f2d68ec` |

The runner-side reinjection follow-up uses the same version and source pins:

| Version | Original runner-side `reinjection.json` SHA-256 |
| --- | --- |
| 2.0.26 | `97c0ba81844f568a506a7b92e9ad0570416c1455f2a1f48fce335525dc3c38db` |
| 2.2.3 | `7ede021d49bfeaad7320f76243c7e51ea5621173cdd3a1b608925e6437dec16f` |

The recorder used exact npm versions with installation scripts disabled. The
associated source commits are provenance pins, not an independent attestation
that the npm binaries were built from those commits. Full captures include
package checksums and recorder configuration; those are not needed by a public
serializer vector and are deliberately not copied here.

`extract-opencode-v2.py` checks the complete original capture hashes above,
checks version/commit identity, and extracts only the `session_message` arrays
from `stored_snapshots.before_cut`, `.after_cut` and `.marker_next`. It preserves
all stored row fields, IDs, sequence gaps, times and replay state. It performs
no sanitization, provider-message conversion or normalization. Run:

```sh
python3 extract-opencode-v2.py CAPTURE_ROOT OUTPUT_DIRECTORY
```

`CAPTURE_ROOT` contains `2.0.26/capture.json` and `2.2.3/capture.json`.
Those original captures aren't distributed, so the recipe can only be rerun
by someone holding them; the hashes above let such a holder check they have
the same files. The shipped rows-only outputs are what the vectors pin. The two
`opencode-v2-rows-VERSION.json` files are the deterministic outputs. They carry
version/commit pins and stored rows only, not packaging/configuration data.
For the runner-side follow-up, pass `--reinjection` and a root containing the
two versioned `reinjection.json` files. This extracts only stored SQL rows from
`previous_marker`, `before_cut`, `current_marker`, `after_injection`, and the
`before`/`committed` snapshots of both revert branches (`between` and `below`).
The outputs are `opencode-v2-reinjection-rows-VERSION.json`; revert stages keep
the captured names `between.before`, `between.committed`, `below.before` and
`below.committed`. Provider requests and plugin/configuration data are excluded.
Separate provider-only captures stay internal and are not vendored as vectors.
`SHA256SUMS` pins all four rows-only outputs, the extractor and the expectations.

## Version compatibility and `keepID`

The profile follows OpenCode 2.2's loader. OpenCode 2.0.x support is best effort.
OpenCode 2.0.x's own compactions store `data.recent`, not a generated `keepID`:
2.0.26 `packages/schema/src/session-message.ts:246-266` defines that shape, and
`packages/core/src/session/compaction.ts:643-656` publishes the recent text.
Without `keepID`, the profile uses the compaction row as boundary, matching
2.0.x's native history selection. A plugin can supply additional metadata
through its compaction hook (`compaction.ts:595-608`); 2.0.26 stores that metadata
but never resolves `keepID` in its loader (`history.ts:83-98`).

The recording used a test-only OpenCode plugin to supply summaries and `keepID`s
through a compaction hook. Both non-self `keepID`s in the original 2.0.26 captures
came from that plugin, not from OpenCode's own boundary policy:

| Stored compaction id | Sequence | Plugin-supplied `keepID` | Target sequence |
| --- | --- | --- | --- |
| `msg_18cea7a00001uGSrQnmayUYzGO` | 66 | `msg_18cea7872001a57Hi9OX9dPCdA` | 16 |
| `msg_18cea818b00146hdOK7u8gnt6u` | 16 | `msg_18cea80db001ORNj2BNXhbh9Ci` | 4 |

For example, this is a minimal excerpt of the first stored SQL row (all other
fields remain in the rows-only vector):

```json
{
  "id": "msg_18cea7a00001uGSrQnmayUYzGO",
  "type": "compaction",
  "seq": 66,
  "data": {
    "status": "completed",
    "metadata": {"keepID": "msg_18cea7872001a57Hi9OX9dPCdA"}
  }
}
```

On these plugin-bearing 2.0.26 sessions the profile's boundary serves earlier
native rows that 2.0.26 did not load/send as those messages; some text can appear
inside its checkpoint instead. These recordings do not establish exact native
model-context parity for 2.0.x sessions carrying plugin-written `keepID`.

OpenCode 2.0.26's fork copies `metadata.keepID` verbatim while giving rows new
ids (`packages/core/src/session/projector.ts:217-225`). A fork of a compacted
session with a resulting stale reference is refused by this profile with the
named `opencode_v2_malformed_session` error. The 2.2.3 fork projector rewrites
those references (`projector.ts:226-238`). Captured rows are never repaired.

## Expected results

`opencode-v2-session.json` specifies every ordinal, part count, projected native
unit, native cut and ID outside the served range for each captured snapshot.
The helper's test compares every field, checks standalone counts and repeats
with input order reversed. In `after_cut`, the kept assistant tool row has
ordinal 4, not 0. `marker_next` is a separate session: its stored `keepID`
names the first user row, so its profile boundary is ordinal 0 even though the
provider request starts with a checkpoint. Neither compaction row nor summary
is a served entry. These are the profile's agreed boundaries, not a claim that
both upstream provider-history renderers preserve a structured historical tail.

The runner-side follow-up provides real stored-row coverage of the markers
`msg_s0_previous_marker` and `msg_s0_current_marker`, each with self-referencing
`keepID`. They are reported separately as `runner_marker`, never as a native
cut or inferred served range. Expected rows remain fully ordinal-bearing on
both sides of a marker. On 2.2.3, the `between` revert removes the current marker
and falls back to the previous one; the `below` revert removes both, leaving
whole surviving history. The 2.0.26 `between.before`, `between.committed` and
`below.before` fork snapshots instead pin named stale-reference refusals. Its
`below.committed` snapshot is valid again because both markers are gone.
The runner's checkpoint (`message_count` and the head cut,
CONTRACT.md §4.8/§9.6) determines a runner-written cut, not native row placement.
No marker or revert vector is synthetic. The recording is request-side
reinjection evidence, not production qualification or proof that every model
request path replays a restored tail.
`outside_served_range: null` in the expected results means a runner marker
requires the runner's own checkpoint; it does not assert an empty excluded
prefix. With neither a native cut nor a runner marker, that list is empty.

`display-opencode-v2.json` gives decoded bodies (`data` plus `id` and `type`)
and literal display rows for both versions: user text/image, reasoning and raw
assistant text, successful/failed tool slots, system notices and synthetic text.
Tests compare every field through `display::project`, including tool limits and
UTF-8 truncation. A tool's two display parts still occupy one native slot.

Cases marked `synthetic` are not recordings. They include an instruction-entry
system notice with a `<context key>` body, location-switched, model-switched,
agent-switched, skill, foreground/background shell, running/failed compactions,
a completed compaction without `keepID`, an unfinished assistant with streaming
tool input, and a rendered template notice. Additional synthetic native units
cover supplied/unsupplied skills, a file-image location prefix, unsupported
media and duplicate inline images: provider expansion/dropping/dedup never
changes the stored count. The unfinished assistant reserves an ordinal and
native slots but is not eligible for reads or history until finished (§4.2).
Decoder tests are not permission to serve streaming rows.

The captured assistants store content inline, including in 2.2.3; these
recordings do not exercise separate `session_part` or blob storage. Runners
hydrate those before calling the SQL-row helper. Synthetic hydrated-content
and replay-state tests do not claim a database integration test. The recordings
do not check provider signature validation on reasoning or tools executed by
the provider rather than a local tool handler. They do not cover every native
control variant. No reasoning archive or pruning policy is defined for this
profile.

## Source read

The following files were fetched from the pinned public commits and these
ranges read. Paths below are relative to
[`anomalyco/opencode`](https://github.com/anomalyco/opencode).

- **2.0.26:** `packages/core/src/session/history.ts:28-60,62-108` selects a
  completed compaction in sequence order and decodes `{...data,id,type}`;
  `packages/schema/src/session-message.ts:73-236` defines native user/notices,
  shell, assistant slots and replay state;
  `packages/core/src/session/runner/to-llm-message.ts:253-325` projects controls,
  user units, notices, shell and checkpoints.
  `packages/schema/src/session-message.ts:246-266` defines compaction `recent`;
  `packages/core/src/session/compaction.ts:595-608,643-656` carries hook metadata
  separately from recent text; `packages/core/src/session/projector.ts:192-232`
  copies forked rows under new ids without rewriting their data.
- **2.2.3:** `packages/core/src/session/history.ts:93-135,152-179,183-224`
  hydrates ordered parts/blobs, adds identity and selects sequence-ordered
  history; `packages/schema/src/session-message.ts:76-246` defines native
  messages, tool lifecycle state and `native` replay data;
  `packages/core/src/session/runner/to-llm-message.ts:76-103,240-313` expands
  attachments, deduplicates inline images and projects native row types.
  `packages/schema/src/tool.ts:67-84` defines tool text/file outcomes, and
  `packages/schema/src/shell.ts:25-26,77-84` defines shell status/output.
  `packages/core/src/session/history.ts:231-265` resolves `keepID` to `start.seq`
  and reads `start.seq <= seq < compaction.seq` (239-255). A self-reference makes
  that interval empty. The 2.0.26 loader at history.ts:83-98 does not read
  `keepID`; it loads only the compaction and later rows. Neither loader restores
  an older structured tail from a self-referencing marker.
  `packages/core/src/session/projector.ts:189-242` copies forked rows and
  rewrites compaction `keepID` to the copied target's id (226-238).

The contract deliberately counts native slots rather than the expanded
provider messages. Ordinal policy, model-switched's stable zero count and the
unserved checkpoint rule belong to this profile, not to upstream numbering.
