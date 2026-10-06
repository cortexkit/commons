# Changelog

## 0.6.0

- Treat cargo/nextest test signal deaths as ERROR, naming the test and signal,
  rather than crediting an abort or segfault as an assertion catch. Cargo also
  requires a summary from each completed binary. Nextest's human signal status
  is correlated with its libtest-json test identity: current JSON alone encodes
  aborts as `failed` with empty `stdout`. Check in actual runner-output fixtures
  and exercise aborts and ordinary assertion failures through both runners.
- Add optional `signal_is_catch = "reason"` for contracts whose intended failure
  really is termination by signal. Empty reasons are rejected; the exception
  does not bypass message/count/name checks or a command row's green baseline.
- Add optional `expect_message`, a substring or `/regex/` required in every
  expected red test's own failure output (Cargo stdout blocks, nextest stdout,
  or the per-id command's combined output). Mismatches fail the row with the new
  RED_FOR_ANOTHER_REASON outcome and the first lines of the actual failure.
  Rows without a pattern retain their ordinary failure grading.
- `prove` accepts and appends `--expect-message` and `--signal-is-catch REASON`.
  Keep the unversioned catalogue/report format; older runners reject new fields.

## 0.5.2

- Replay sessions now run prerequisites once on the clean tree, collect every
  command row's per-id baseline, then replay all mutants with their existing
  per-mutant prebuild. One final restored-tree prebuild leaves clean fixtures:
  N executable mutants need N+2 prerequisite runs, not 2N. This order covers
  `run` (including `--broad`, diff selection and shards), `prove` and `explore`.
  Cargo/nextest replay still has no green baseline; `check` remains clean-tree
  preparation followed by name listing, without mutants. Prove's conditional
  package diagnosis also runs before the single final refresh.
- Keep initial, mutant and final-restore prebuild timing separate. Final refresh
  failures abort the session by step name. Interruptions and mutant timeouts
  restore source bytes and run final fixture cleanup with its own deadlines,
  even after an interrupted build or failed mutant prerequisite.
- Clarify command exit 126 as `not executable, or the command refused to run`,
  including wrappers that refuse a zero-test invocation; 127 remains not found.
- Preserve `(binary, test name)` identities across Cargo and nextest results.
  Names shared by multiple binaries are reported as `target::test_name`, not
  rejected or merged. `expect_red` accepts qualified names; ambiguous plain
  expectations fail validation with candidate names. Package name validation
  runs on the clean tree before mutants, including narrow rows, and `check`
  still verifies that the expected names exist within the selected target.

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
