# Changelog

## 0.3.1

- This release moves the complete `step-transform-provider-v1` vector set
  from the workspace root into this crate. It ships in the package behind
  `test-vectors`, with a public `vectors` module and a SHA256SUMS test in
  CI. Old pinned commits keep the old path; new consumers use the
  crate's vector bytes instead of reaching into a workspace checkout.


- Requires cortexkit-role-llm-runner 0.4.0. Nothing this crate exports names
  an llm-runner type (its runner-code constants are `&str` with unchanged
  values), so this is a patch release.
- The `tagger` epoch names the tagging rules: `claude_code::COMPUTED_TAGGER_EPOCH`
  (2) for llm-runner 0.4.0's computed tags, 1 for the earlier never-reused
  tags (CONTRACT.md §14.1). The synthetic Claude Code fixture now advertises
  2; admission still accepts any value, the epoch being advertised data.

## 0.3.0

- Requires cortexkit-role-llm-runner 0.3.0, a breaking release (was 0.2.0). Nothing this crate exports changes type; its `errors::runner_codes` constants keep their values. The minor bump follows the role contract: compaction-provider 0.3.0 removed `compaction.step`, `compaction.ready`, the request fence and `wait`, its host runner lane (former §18) and its native Claude Code lane (former §19), on which this contract's §13 host runner lane and §14 Claude Code capability lean. CONTRACT.md now marks each such reference as removed, with a note at the head of §13.1 and §14.1; the lanes themselves are kept as written until they are re-specified on advance notices and views, and a caller relying on them can no longer pair them with a compaction-provider 0.3.0 provider.

## 0.2.3

- Contract wording only (§13.2), no code or wire change: the host advances
  `served_through_ordinal` past an ordinal only after a model request carrying
  it completed cleanly at the provider, never when the ordinal is committed
  before sending or on response headers. Published from the same commons
  revision as cortexkit-role-compaction-provider 0.2.4, because the Claude Code
  capability bundle records one revision for both roles.

## 0.2.2

- Update the dev-only capability-identifier parity test to subc-protocol 0.32.
  The provider's public API and wire behavior are unchanged, so consumers need
  no source changes. Published from the same commons revision as
  cortexkit-role-compaction-provider 0.2.3, which records this pair's shared
  Claude Code capability revision.

## 0.2.1

- No code change. Published from the same commons revision as cortexkit-role-compaction-provider 0.2.2, because the Claude Code capability bundle records one commons revision for both roles.

## 0.2.0

- Add optional Claude Code capability publication to describe/declare, frozen connection-scoped admission, daemon-compatible capability identifier validation and declared payload-cap checks. The gateway requires both a daemon-attested principal and the selected bundle; body fields never grant authority. Existing absent-field wire bytes and vector checksums are unchanged.
- Extend host admission to the attested Claude Code gateway, including answer observation and bounded host paging without transcript reads or a ready service.
- Add native hook raw-message sidecars, required only on the admitted lane and counted in payload caps. Require all codec/raw-provenance/render/tagger/memory/compartment epoch pins and advertise typed bootstrap completion support instead of a bare format string.
- Keep the capability identifier checker transport-independent, with a dev-only parity test against subc-protocol 0.31.0 so changes to the daemon grammar cannot silently diverge.
- Minor package bump: adding an optional field to the previously exhaustive public `Declaration` breaks Rust struct literals. Wire major remains v1; no existing wire member changes meaning. New vectors use synthetic epochs, not released codec epoch claims.

## 0.1.2

- Hook requests gain four optional host-lane fields: `served_through_ordinal` (the highest message ordinal the host has durably served), `unserved_subjects` (hook answers the host never served), `subject_part` (which tool part of a message the hook is about; non-empty, at most 256 bytes) and `pass_complete` (marks the last hook of a pass). Tests now load the host-lane vectors.
