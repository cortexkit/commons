# Changelog

## 0.4.0

- Add reviewed HUB dispositions using `hub = "reason"` and `hub_targets = [...]`.
  Refuse missing reasons, reasons shorter than 20 trimmed characters, missing or
  empty targets, and combinations with EQUIVALENT or UNREACHABLE at load.
- Under `run --broad`, count a catch as HUB only when every collateral target
  outside all expected-test targets is in the recorded set. `hub_targets` lists
  only these other targets, the ones that made the catch broad. Smaller sets
  remain HUB; new other targets grade CAUGHT_BROADLY and are named for review.
  Same-target failures need no approval and remain in the full collateral report.
  Expected tests must still fail and `only` still holds.
  Normal runs ignore HUB. Report a separate HUB count and listing with reasons.
- Refuse HUB in the shared append writer: it must be written by a person after
  reviewing a broad catch, never by `prove` or `explore --append`.
- Report stable Cargo collateral target names without executable hashes or the
  Windows `.exe` suffix, so reviewed target sets survive rebuilds. Keep nextest's
  binary ids. Older runners reject the new catalogue fields.

## 0.3.1

- `--old` and `--new` accept values starting with `-` or `--`, such as a
  shell flag or a SQL comment, without the `--old=` form.

## 0.3.0

- Add `runner = "command"` catalogue rows and `prove --command` for argv-based
  test runners outside Cargo/libtest and nextest. Each expected id runs unchanged
  as one argument, first on a fresh green baseline and then on the mutant.
- Refuse mixed runner fields, malformed argv templates, and `only = true` on
  command rows at load. Existing cargo/nextest catalogues remain readable.
- Command process failures, reserved exit codes 126/127, signals, and timeouts
  are ERROR, never catches. Reuse the shared replay/restoration and per-test
  timeout machinery, with no build phase, collateral, breadth audit, or explore.

## 0.2.0

- Report collateral red-test counts and sorted, unique binary/target identities
  on proof rows, including zero collateral. Preserve Cargo headers alongside
  test events and retain nextest binary attribution in the shared parser.
- Add `run --broad`, an opt-in audit that replays each row's mutant against
  every test target of its package. Only this audit grades a catch with
  collateral outside all expected-test targets as `CAUGHT_BROADLY`; it warns in
  row and summary output while still recording proofs and succeeding.
  Same-target collateral retains `CAUGHT`, and a row with `only = true` still
  fails as `WRONG_TEST` on any extra red test.
- Report `breadth_observed` on every row. Normal runs and `prove` report
  collateral among the targets they ran but never grade `CAUGHT_BROADLY`, and
  print "breadth not observed" on caught rows.
- Record reasoned `UNREACHABLE` dispositions in catalogues, exploration results,
  and reports, with a separate console listing and no caught credit.
- Keep the existing unversioned TOML catalogue and JSON report-array formats;
  `collateral` is additive report data. Previous catalogue rows remain readable.

## 0.1.0

- Replay, prove, and explore checked-in mutation controls with exact test names,
  separate build/test deadlines, and verified source and lockfile restoration.
