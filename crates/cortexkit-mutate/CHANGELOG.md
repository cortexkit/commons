# Changelog

## 0.9.8

- Tests: build every throwaway Cargo fixture into its own `target` directory.
  The fixture copies share names and layouts, so under an inherited shared
  `CARGO_TARGET_DIR` (as remote build runners export) parallel tests overwrote
  each other's fixture binaries, and Cargo's modification-time freshness check
  could reuse another test's mutated binary for a clean-tree baseline. Under CPU
  contention that surfaced as `baseline red: tests::guard_rejects_zero` and
  short collateral counts. The tests also drop an inherited
  `NEXTEST_TEST_THREADS`, which overrode a fixture's own `test-threads = 1`
  and let the order of that fixture's output depend on machine load. The
  runner itself is unchanged.

## 0.9.7

- Prepare report, broad-report, and catalogue output paths before running work;
  write JSON reports atomically so a late report error cannot leave a partial file.

## 0.9.6

- Use Git status to check target cleanliness, honoring line-ending and filter
  conversions so clean CRLF checkouts with `core.autocrlf=true` are accepted.
  Staged and unstaged edits still require `--allow-dirty`; restoration still
  writes back the saved local bytes unchanged.

## 0.9.5

- Move the per-worktree lock and stdout/stderr capture files out of `.git`
  into the host's temporary directory, so mutation proofs work when Git metadata
  is read-only (including Linux remote build jobs). Canonical worktree paths share
  one lock across aliases; captures are removed on success and error.

## 0.9.4

- Cache test listings for each runner, package, target, feature and ignored-test
  selection within a replay session, keeping clean-tree and mutated-tree
  listings separate. Reuse successful listings and failures in both `check` and
  `run`.

## 0.9.3

- Add `prove --select expected` for cargo and nextest. It reuses the row's exact
  expected-test selection for the clean baseline and mutant, preserves the
  selection in the appended row, and refuses command rows, `--only`, and names
  that do not execute. Selected proofs do not repeat a package-wide survivor
  diagnosis that cannot widen their selection.

## 0.9.2

- Use a testcase's non-empty JUnit `classname` as the collateral target, falling
  back to its `file`, then the enclosing testsuite's `file`. Bun top-level tests
  therefore report their source file instead of an empty target. Existing
  `broad_id` ids remain unchanged unless the template uses the new `{file}`
  placeholder, which maps from the testcase or enclosing testsuite file. Rows
  reviewed with `hub_targets: [""]` must be re-reviewed because their target
  values now use Bun's file paths.

## 0.9.1

- Ignore embedded child libtest runs in Cargo's captured stdout/stderr when
  collecting per-test outcomes and summary counts. Use the first `running N
  tests` header and the last `test result:` per binary, counting outcomes only
  before the captured-output report. Keep genuinely inconsistent accounting as
  ERROR and retain nested failure text for message proofs.
- Cover nested reports, duplicate outer/child names, stderr blocks and strict
  accounting with parser and replay fixtures. Confirm nextest's human status and
  JSON event formats do not mistake embedded libtest text for test results.

## 0.9.0

- Add typed optional Cargo/nextest `select = "expected"` to run only exact
  `expect_red` harness names on the clean baseline and mutant, including named
  ignored tests. Emit libtest `--exact` filters or nextest exact-name filtersets,
  check execution counts and refuse missing/zero-match names as ERROR. Preserve
  the narrow selection under `--broad`, report why breadth was not observed,
  and refuse `only = true`, HUB annotations, command rows and unknown values.
  Key shared baselines by expected names; rows without the field are unchanged.
- Accept command-row test ids with interior spaces, preserving them byte-for-byte
  as one argv element, in proofs, appended TOML, reports and failure maps. Empty
  ids, leading/trailing whitespace, controls (including tabs, newlines and NUL)
  are still refused with the offending id in the error.
- Add optional command-row `broad_command`, `broad_report` and `broad_id` fields
  for a real JUnit breadth audit under `run --broad`, after the named tests are
  graded. Delete stale reports before execution; missing, invalid and zero-case
  reports are ERROR. Map failed/errored cases to exact ids, retain failure output,
  and grade cross-class collateral as CAUGHT_BROADLY or reviewed HUB. Command
  `hub_targets` are JUnit `classname` values; successful audits record
  `breadth_observed: true`. Rows without these fields are unchanged.
- Share clean JUnit baselines by command/report/id/deadline selection, report
  baseline-red ids and messages separately, and exclude pre-existing failures
  from collateral. Invalid baseline reports make every sharing row ERROR.
- Document Bun, pytest, SwiftPM XCTest and converted Xcode report mappings. Keep
  command exploration refused; `prove` uses the same canonical append path as
  `explore --append`. Add pinned quick-xml 0.42.0 without optional features.

## 0.8.0

- **Breaking:** rename the CLI binary from `ck-mutate` to `ckdev-mutate`; the
  `cortexkit-mutate` crate and library names are unchanged. Migrate by replacing
  `ck-mutate` with `ckdev-mutate` in CI and scripts.

## 0.7.3

- Add an optional Cargo/nextest row field `ignored = "include"` or `"only"` to
  run `#[ignore]`d tests too, or only them. libtest receives `--include-ignored`
  or `--ignored` after `--`; nextest receives `--run-ignored all` or
  `--run-ignored only`. The selection applies to test listing and name
  resolution, clean-tree baselines, mutant runs, `check`, package diagnosis,
  `run --broad` and `explore`; builds are unchanged. Baseline and name-list
  caches are keyed by it, so selections never share a baseline.
- Refuse an `expect_red` test that is `#[ignore]`d on a row without the field,
  in `check` and at baseline, with a message naming the field, instead of a later
  NO_TESTS_RAN. Name resolution no longer counts ignored tests the row skips.
- Refuse the field on command rows and any value other than `include` or `only`.
- Add `--ignored include|only` to `prove` and `explore`, preserved in appended rows.

## 0.7.2

- Add optional Cargo/nextest row fields `features`, `no_default_features`, and
  `all_features`. Use the same feature selection in clean-tree baselines,
  separate builds, test runs, test listing/name resolution, `check`, package
  diagnosis, and `--broad` audits. Feature selections never share baseline or
  name-list caches. Refuse malformed feature names and command-row feature fields.
- Add matching `--features`, `--no-default-features`, and `--all-features` options
  to `prove` and `explore`, preserving them in appended rows.
- List names using the replay's target selector instead of compiling every
  package target. Plain expectations only need qualification when ambiguous in
  the selected targets; unrelated targets no longer block narrow proofs.
- Keep the unversioned catalogue format; existing rows retain default features.
- Match edit anchors byte-exactly first. If an LF-only multiline anchor is absent
  in a CRLF source, retry with both anchor and replacement translated to CRLF.
  Keep the exactly-one-occurrence rule for the chosen form, never combine exact
  and translated matches, and report `line_endings: "crlf"` when the retry was
  used. Single and multi-edit rows still restore the saved bytes exactly.

## 0.7.1

- Add `failures` and `baseline_failures` report maps containing every red test's
  own output, including collateral and baseline reds excluded from grading.
  Cargo retains each libtest stdout block; nextest retains per-test stdout and
  stderr; command rows retain each test id's combined output. Keep `test_tail`
  unchanged, so early failures remain readable even after hundreds of passes.
- Bound each failure entry to 16 KiB including an elision marker, retaining the
  first 4 KiB and approximately the last 12 KiB on UTF-8 boundaries.
- Print the first six lines of each unexpected red's failure beneath WRONG_TEST
  and RED_FOR_ANOTHER_REASON rows, including message-mismatched expected tests.

## 0.7.0

- Collect Cargo/nextest clean-tree baselines before any mutant, sharing one
  build/test run per runner/package/target selection. Broad runs baseline package
  tests; prove prepares its package diagnosis baseline while fixtures are clean.
  Expected tests already red grade ERROR (`baseline red: <tests>`) without
  applying the mutant. Other baseline reds are recorded per target, named in the
  session summary, and excluded from WRONG_TEST and broad/collateral grading.
  Report baseline build/test/prebuild costs independently of mutant costs.
- Replay equivalent rows instead of skipping them. Require both the semantic
  `equivalent` explanation and `equivalent_guard` code fact. Surviving rows grade
  EQUIVALENT, counted separately; any new catch grades EQUIVALENT_CAUGHT and fails
  the claim. Existing equivalent rows must add a guard before replay/check.
- Add command `catch_on = "output_differs"`, comparing stdout after optional
  ordered regex deletions in `output_normalize`. Run the baseline twice and
  refuse nondeterministic output, naming its first differing line. Different
  mutant output catches, identical output survives; existing process-error and
  nonzero-exit rules still apply. Reject `expect_message` on these rows. Prove
  supports the new options without requiring a named test or test count.

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
  Rows without a pattern grade as before: any red expected test is a catch.
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
