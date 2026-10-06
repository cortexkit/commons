# Changelog

## 0.5.1

- A command row's report shows a green baseline's output under `baseline output:`. The words "baseline was not green" now appear only when a baseline actually failed.
- The README states that a command row's `timeout_s` bounds its whole command and that `build_timeout_s` does not apply to it; time a separate build with a prebuild step.

## 0.5.0

- Add per-row `platforms` using validated Rust target-OS names. Nonmatching hosts
  report SKIPPED_PLATFORM with a separate count, never a catch, survivor or error.
  Check still validates anchors but skips unavailable test-name discovery.
- Add reasoned `desk_only` dispositions for real-desktop proofs (TCC,
  Accessibility, physical input). DESK_ONLY takes precedence over platform
  mismatch, skips automation and prerequisites, and has a separate count/listing.
  It is exclusive with other recorded dispositions and HUB.
- Add catalogue-root named argv `prebuild` steps. Run once before any baseline or
  mutant, then conservatively rerun after each successful mutant build (after
  editing command rows) because test-only builds cannot refresh every fixture
  binary or infer arbitrary prerequisite dependencies. Unmutated failures abort
  the session by name without grading; mutated failures are ERROR, never catches.
  Log output and separate baseline/mutant prebuild timings, including timeouts.
  Refresh prerequisites on the restored tree before the next executable row of
  any kind, with separate restore timing; a refresh failure aborts the session.
- Share both features across broad replay, command rows, prove, explore and check.
  When `explore --append` writes a row, it copies a single target OS into the
  row's `platforms` only when the whole file is gated literally: an inner
  `#![cfg(target_os = "...")]`, or a `#[cfg(target_os = "...")] mod name;` in the
  conventional parent file. Compound cfgs, `#[path]`-redirected modules and
  gates on single items are left for the author to add. Diff selection includes rows
  affected by changed root prerequisites. Existing lockfile restoration remains.
- Require command rows to declare `test_count_pattern`, a literal pattern with
  one decimal `{count}` placeholder. Baseline and mutant invocations must each
  report exactly one nonzero executed count; missing/zero/ambiguous counts are
  ERROR regardless of exit status. Existing command catalogues and prove commands
  must add this field/option; Cargo and nextest count parsing is unchanged.

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
