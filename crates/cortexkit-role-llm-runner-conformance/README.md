# cortexkit-role-llm-runner-conformance

Version **0.17.1** pins `cortexkit-role-llm-runner =0.8.1`, the compatibility
patch for the breaking 0.8 release of the existing `llm-runner/v1` role. The runner's
[CONTRACT.md](../cortexkit-role-llm-runner/CONTRACT.md) is normative; the public
`CASES` registry names every executable case, its requirements and what it
checks.

The suite drives a **real runner** over its actual management route, through
its `cortexkit-role-harness` adapter, with a scripted model and tool provider.
The in-process fake under `tests/fake` tests the suite itself, never a runner's
production compliance. A production runner uses this crate as a dev-dependency.

## Changes from 0.16

- Remove compaction-provider/v1 fixtures, dependency and cases: Setup, descent,
  notices, views, due/pending promises, rides, ceiling-provider handling,
  checkpoint anchors and copied-checkpoint fork detection.
- Remove the tags/resolve and model-view cases and their adapter hooks.
  Compaction-runner conformance belongs to the compaction protocol's kit,
  not this one.
- Count active-path host records (excluding markers) from 1 under
  [compaction W4.1/W4.2](../cortexkit-role-compaction/CONTRACT.md#w4-the-neutral-part-model) and read the whole active path,
  including entries below native host markers. Zero is an alias for the
  first entry, not a distinct ordinal.
- Replace lineage-bound reads with required `history_version` and
  `incarnation` cache identity on heads and pages. Stale after-mid cursors
  fail `history_changed`.
- Keep no-provider archive pruning, with served part keys. Provider-enrolled
  sessions cannot prune until the compaction contract defines acceptance;
  the former Setup ready.accepts permission and post-prune Setup/view tests
  are gone.
- Add run-fixed routing-decision, route-bound user-command, both send-status
  names, and the 64 KiB tool-result replacement checks.

## Running the suite

Implement `LlmRunnerSubject` and its harness/route associated types. Then call
`run_suite(&subject, empty_work_directory)` or `run_suite_with` with progress
options. The directory must be empty. The suite owns a crash driver and
records which advertised kill points were actually reached.

The report has a named outcome for every case. Missing capability requirements
are skipped; an unsupported optional fixture is not applicable by name, never
passed. A simulated kill does not satisfy the suite's real-process-kill
requirement. Do not infer success only from the absence of an individual case
failure: inspect the report's verdict and kill ledger too.

The runner adapter supplies:

- attested owner and stranger route identities and one route per session;
- scripted model turns and exact provider tool answers, with held-call
  release/count observations kept outside the runner's process and stores;
- runner-owned send/plan fields, serializer-profile decoding, and native part
  replacement where a case requires it;
- the real persistence and restart path, never a test-only acknowledgement
  standing in for a durable write;
- optional native-history, marker, revert, retention-clock and pruning hooks
  only when the host can really create those situations.

Observe model requests at the model endpoint and provider calls at the
provider. Never rebuild expected observations from the runner's configuration
or its management replies. Native body decoding belongs to the adapter;
opaque part keys come from the served envelope, not a reader's invented index.

## New named cases

| Case | What an incorrect runner must not do |
|---|---|
| `history_version_on_read_pages` | Omit the rewrite version from a read page. |
| `stale_read_cursor_history_changed` | Answer a stale after-mid cursor instead of refusing `history_changed`. |
| `read_from_ordinal_zero` | Skip the first entry when the request starts at zero. |
| `incarnation_on_every_page` | Omit the required session-record identity or disagree with the head's cache pair. |
| `read_below_host_marker` | Hide or renumber entries from reads when a native marker is written. |
| `route_decision_id_fixed_per_run` | Accept a joining send with another decision id. |
| `user_commands_without_session_ref` | Require a session selector in either route-bound command operation. |
| `send_status_both_names` | Serve only one of the canonical and compatibility operation names. |
| `archive_pruning_part_keys` | Name a pruned part absent from its entry's served keys. |
| `archive_pruning_provider_blocks_pruning` | Prune a plan with a compaction provider before acceptance is specified. |
| `tool_result_too_large_is_error` | Cut provider output short or lose the synthetic reason, measured bytes, limit or discard text. |
| `tool_result_too_large_not_repeated` | Dispatch an oversized-result call again. |

Each new check is exercised against a deliberately faulty fake and verifies
rejection under the case's name. Existing unrelated admission, send receipts,
run results, usage, display/live events, attachments, retention, cancellation,
structured output, tool identity and crash guarantees remain in the registry.

The run-fixed routing-decision case uses `route_decision_id_fixture(session)`.
Its default literal suits runners without routing accounts. A routing-backed
adapter overrides it with a valid decision seeded in routing for the session's
provider and model; the test does not synthesize routing replies.

## Part numbers without the removed tags group

The retained `part_tag` template slot and immutable `output_refs` cases still
need the number actually shown for a message/part. Implement
`shown_part_number(session, mid, part)` using the runner's captured request
mapping, whose numbers follow
[compaction A3, Tag numbers](../cortexkit-role-compaction/CONTRACT.md#a3-tag-numbers). The kit does not compute numbers
from read ordinals or part keys and does not call `session.resolve`.
`part_digests` itself supplies descriptor keys in stored order; no duplicate
tag-row identity surface is required.

## Oversized results and historical snapshot fixtures

`oversized_tool_result` decodes the actual native stored size-error result,
including its reason, `bytes`, `limit`, error state and model-visible text.
The deliberately faulty fixtures either truncate a successful output or
actually dispatch the provider call twice. A completed result over 65,536
model-visible bytes must instead be discarded whole, without a retry.

The retained `output_refs_snapshot_size` and `output_refs_snapshot_budget`
cases test their original snapshot thresholds, which can exceed 64 KiB.
They use `fixture_seed_historical_tool_output`, a **fixture-only** import of
stored output that predates the provider-result limit or was imported. The
hook never dispatches a provider or exempts a newly returned result from size
enforcement. An adapter without it grades the affected case not applicable
by name. Do not lower the snapshot thresholds to make these cases reachable.

## Archive-pruning fixtures

`archive_pruning_fixture` supplies independently scripted original/served
native parts, with every declared kind in two messages and an unaffected
part. Its boolean chooses whether the plan names a compaction provider; it
installs no provider protocol. `prune_archived_session` exercises the real
maintenance path, including the active-run and queued-send exclusions.

Every affected raw/read and raw/display history page repeats
`pruned_reasoning`. `part_digests`, when present, publishes the stored-order
keys; otherwise `part_keys` does. Readers match the listed part to those
keys and never synthesize one. `reasoning_pruned` remains on head/read pages,
with a durable generation, highest touched ordinal and commit time.

## Scope and verdicts

The kit tests behavior through public routes and genuinely independent
observations. Native marker creation and imports are optional fixture hooks;
the kit does not claim provider-protocol conformance through them. Resident
memory and per-host integration still need the runner owner's measurements.
The public `NARROWINGS` inventory records boundaries of executable coverage.
