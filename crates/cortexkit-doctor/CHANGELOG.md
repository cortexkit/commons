# Changelog

## 0.3.0

- Add a read-only full check with a 30-minute limit and support full-mode
  rechecks when fixing a finding. The check, fix, and manifest document
  formats are unchanged.
- Pass the check mode to module callbacks and require it when invoking runner
  checks and fixes. The runner reports `no_doctor` for older helpers that
  reject a full check so the caller can fall back to quick.

## 0.2.0

- Report a missing `doctor` subcommand when `check` exits 2 with empty stdout,
  and retain the I/O error kind when a launch fails.
- Distinguish invalid GitHub repository URLs from other manifest shape errors.
- Build local-only reports from validated check output even without a manifest.
- Document how checks inspect SQLite stores without writing and which storage
  roots the runner exposes. Existing `doctor/v1` wire and report bytes are unchanged.

## 0.1.1

- No output change; drop `serde_jcs` as a normal dependency so dependents don't
  get `serde_json`'s `float_roundtrip` feature.

## 0.1.0

- Add strict validation for doctor check, fix and manifest documents, with fixed
  failure-stage codes and canonical SHA-256 digests.
- Share log field-kind rules and secret-pattern redaction across doctor tools.
- Add a Unix module-side command helper that keeps callback output out of the
  JSON document, confirms through the OS with a terminal fallback, and repeats
  the check and preflight before applying a fix.
- Add a Unix runner for check, fix and manifest with a cleared environment,
  bounded stdout and process-group cleanup, and typed failures without output text.
  The non-default `runner-test-fixture` feature builds a fixture executable
  for integration tests; it is not part of the doctor contract.
- Build canonical, size-bounded reports on all platforms, with allow-listed log
  templates, typed evidence projection, per-report aliases, and secret redaction.
- Document the `doctor/v1` contract, including adopter duties for crash-consistent
  fixes, test-only confirmation hooks, log-path confinement, and report review.
- Add an opt-in `conformance` feature for the kit's isolated, non-prompting
  test fix invocations.
