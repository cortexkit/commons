# Changelog

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
