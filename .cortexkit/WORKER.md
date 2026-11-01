# Working in commons

## Delivery gate

- Before delivering, run `scripts/gate.sh fast` (seconds: formatting,
  lockfile, versions, vector checksums), the tests for the code and behaviour
  you changed, filtered by name or module
  (`cargo test -p <crate> --all-features <filter>`), and clippy for every crate
  you changed: `cargo clippy -p <crate> --all-targets --all-features -- -D warnings`,
  plus the same clippy with `--target x86_64-pc-windows-gnu`. A bug fix adds a
  test and shows once that it fails without the fix. When changing a
  conformance kit or its contract crate, run the kit cases for what you
  changed, filtered the same way.
- Never run a whole crate's test suite or a full gate (`scripts/gate.sh lint`
  or `test`). The head runs the full suites once, over several merged
  deliveries, because workspace builds unify features across crates and catch
  what package runs can't. A filtered run that takes more than about 15
  minutes means the filter is too broad: narrow it. If your brief names a
  narrower check, the brief wins.
- If a test outside your change fails, rerun that one test by name on the
  base commit, never the suite.
- Run tests on the build server with `runon: "linux"`. In the agent's bash
  tool, `runon` selects a Linux build host. Don't wait on remote jobs in long
  `bash_watch` loops: run a filtered job in the foreground with a timeout that
  fits it. Keep the normal build directory; a separate `CARGO_TARGET_DIR` per
  run means a cold build every time.
- Report each check's verdict, test counts, and every failing test name.
- Build and test remotely. Only macOS-only work runs on this Mac, at
  `nice -n 10`.

## Releases and contracts

- Every change to a published crate's `src/`, shipped public documentation
  (`README.md`, `CONTRACT.md`), or dependency pins needs a version bump and a
  CHANGELOG entry.
- A conformance kit pins its contract crate to one exact version. Releasing a
  contract moves that pin and bumps the kit too.
- A breaking change to a public type requires a minor bump on 0.x. Prefer
  `#[non_exhaustive]` types with constructors so later additions stay compatible.
- Copy test vectors byte for byte from their owner and update their `SHA256SUMS`
  in the same commit.

## Public repository

- Do not put private module names in code, comments, commits, READMEs, or test
  data. These are identifiers belonging to non-public repositories; ask the task
  giver if unsure, rather than recording a private-name list here. Internal
  documentation belongs in ignored `.cortexkit/internal-docs/`.
  Never force-add ignored `.cortexkit/` files.
- Never commit a file larger than 1 MB.

## Verification and decisions

- Do not create checklists of deliberate code breakages or tables of mutation
  results, or rerun unchanged checks just to demonstrate stability. A bug fix
  keeps one proof: show once that the new test fails without the fix. Each
  conformance check needs an intentionally incorrect implementation of the
  tested contract; a test must verify that the conformance runner rejects that
  implementation and reports the check's name.
- Pre-approved: regenerate a golden or fixture when its output differs only by
  the stated cause; update a test the intended change breaks while keeping what
  it checks; make necessary version bumps, CHANGELOG entries, and `Cargo.lock`
  refreshes. Do not ask for these routine changes.
- Never loosen a threshold, assertion, or timeout to get green. Never change
  code outside the scope of your assigned task without asking.
- Ask when a decision changes a contract's meaning, a wire shape, what a runner
  or provider must do, or a public API.
