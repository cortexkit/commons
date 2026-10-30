# Pi stored-session and display vectors

`pi-1.3.0-1.4.0.jsonl` is one byte-for-byte copy of two identical recordings
made with `@earendil-works/pi-coding-agent` **1.3.0 and 1.4.0**. Its SHA-256 is
`794b083f752b6f1d33f5a92044637dcf57e5fb87d90f6c4add0ac876f6f93d67`.
`SHA256SUMS` pins this recording and the expected-result files beside it.

| Package version | npm tarball SHA-256 |
| --- | --- |
| 1.3.0 | `ae2785e870b81a8c8e1d44510a57b5ee0bfc9e974679ac2faaada4aabee3d3c0` |
| 1.4.0 | `d155c16ef6fa00960fbf58b2911536e80b05f33700e0b14fbd08967ede0c7428` |

## How the recordings were made

Pi ran in RPC mode on Linux with Node 24 and npm 11, a loopback mock provider,
and a test-only extension, using invented prompts and results. Home, agent,
session, npm and working directories were disposable, without real credentials
or inherited Pi settings. The recorder compared the journal against RPC
`get_entries`. It normalized only IDs and their references, paths, timestamps,
and elapsed `durationMs`; it did not rebuild entry shapes, roles, content,
usage, flags, compaction fields or parent relationships. Paths are `/fixture/...`.
Pi's own token estimate is retained. Transitive npm dependencies were resolved
at recording time; the two Pi tarballs themselves were checksum-pinned.

There is one version-3 session header and 24 Pi-written entries. A branch from
`entry-005` abandons `entry-006` through `entry-015`. The final entry is a label,
`entry-024`, and is the durable leaf. After recording that label, an in-memory
switch to `entry-005` added nothing; reloading selected `entry-024` again.
The latest native cut, `entry-021`, keeps `entry-019` onward. A real context edit
changes `entry-019`'s projected assistant text. Excluded and included shell runs
were observed before abandoning that branch; the excluded output was absent
from the subsequent provider request, while the included output was present.

## Expected results and synthetic coverage

`pi-session.json` independently specifies the leaf, full active path, all
ordinal-bearing entries, projected content and part counts, served bodies, the
native cut, and abandoned IDs that must not appear. The helper's test compares
every field. The retained tail has ordinals 6, 7 and 8, not a new zero-based
sequence. Each entry's `served_body` is what a runner serves on `session.read`:
in the recorded journal, `entry-019`'s `/message/content` carries the recorded
context edit's replacement, and every other entry is its stored row's JSON
value unchanged. A test checks that the single-row count of every served body,
recorded and synthetic, equals its part count. The synthetic case
`null-removal-shapes-synthetic` removes a user message, a custom message, a
branch summary and a shell run with null edits, pinning each removal shape.
`display-pi.json` gives display projections of stored bodies for each retained
entry, as well as recorded entries before navigation and synthetic coverage.
They test the display function itself; inside the served window a runner
projects the served body, so `entry-019` displays the edited text there, and
its original text appears only on host-history pages below the window.

Cases marked `synthetic` are not recordings. Shell cases copy the recorded
`bashExecution` shape and change fields for empty output, cancellation,
nonzero exit and truncation. The Oh My Pi shell/image case is synthetic, but
its one text block plus one image block is checked against public source.
Other synthetic cases cover a branch summary (stored `summary`, not an invented
`content` field), a late edit of an earlier message, last-edit-wins, null
replacement, and a rendered template send stored as `custom_message` with
`customType: "cortexkit:notice"` and `display: true`. The template example's
`template_id` annotation identifies the source template; it is not a
`session.send` request (which also needs its delivery and identity fields).
The recordings do not exercise these synthetic cases or the runner's send
storage rule, nor can a journal alone prove step-boundary lineage transitions.

## Source checked

Both npm tarballs were downloaded and their full SHA-256 values above verified.
Their `dist/core/messages.js` (MSG) files are byte-identical, SHA-256
`8688b3f6eb28865f779cac998bd4754d1a4f08703200dfe0dd5a799aa0d42ef6`;
`dist/core/session-manager.js` (SM) is also identical across the two versions.
The following source ranges were read in those files:

- SM:124-145: parent-chain path selection; SM:732-753: reload chooses the last
  non-header entry, including labels. SM:815-820,1034-1046: append advances the
  leaf, including labels. SM:1155-1174: branch/reset only changes memory.
- SM:166-191: message/custom/branch-summary runtime projection, including
  missing/null content normalization. Branch summaries use `entry.summary`.
- SM:193-229: latest compaction and retained tail.
- SM:235-280: context edits, null removal, roles accepting content replacements,
  last edit per target, and provenance-preserving projection.
- MSG:21-38: shell command/output and status text; MSG:75-122: model conversion,
  shell exclusion, and pass-through roles.
- MSG:40-46,48-55,57-66: `branchSummary`, `compactionSummary` and `custom`
  runtime roles. MSG:79-87: `bashExecution`; MSG:89-115: conversion to user
  authority versus pass-through system/user/assistant/toolResult roles.

The shell/image shape was checked in public Oh My Pi source at revision
`95db18ef9936bff96fbd33268b2363ab25a9bb9d`,
[`packages/coding-agent/src/session/messages.ts`](https://github.com/can1357/oh-my-pi/blob/95db18ef9936bff96fbd33268b2363ab25a9bb9d/packages/coding-agent/src/session/messages.ts#L1120-L1133):
lines 962-976 render shell text, and 1120-1133 append `images` as distinct
content blocks after that text. The vector is synthetic, not an Oh My Pi
capture; it makes no claim about other Oh My Pi extensions or truncation metadata.
