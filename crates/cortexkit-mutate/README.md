# cortexkit-mutate / ck-mutate

A standalone, unpublished runner for checked-in mutation proofs. Install a reviewed,
immutable revision (replace `<sha>` with a **full commit SHA**):

```sh
cargo install --locked --git https://github.com/cortexkit/commons --rev <sha> cortexkit-mutate
```

Run from anywhere inside the repository. Paths in the catalogue are relative to
its Git root, even when `--catalogue` overrides the default `mutations.toml`.
The repository must already have a current `Cargo.lock`: every build, test and
list invocation uses `--locked`. The runner never stages or checks out files.

## Catalogue

```toml
[[control]]
id = "flow-list-agent-sees-only-own"
guards = "an agent's flow.list returns only its own flows"
file = "crates/basal-module/src/ops.rs"
old = "Caller::Agent { agent_id, .. } => Some(agent_id.as_str()),"
new = "Caller::Agent { .. } => None,"
test_file = "crates/basal-module/tests/list_contract.rs"
runner = "cargo"
package = "basal-module"
target = "--test list_contract"
expect_red = ["list_agent_cannot_see_another_agents_flow"]
only = true
# timeout_s = 600
# build_timeout_s = 1800
# equivalent = "Explain why this mutant computes exactly the same result"
# unreachable = "Explain why the mutated code has no production caller"
```

The two deadlines are separate. `timeout_s` (default 600) bounds only the test
run. `build_timeout_s` (default 1800) bounds the separate build of the mutant and
the compile inside `check`'s list mode. A slow compile on a loaded host is
therefore never read as a hung test, and a TIMED_OUT row says which deadline
expired. `prove` and `explore` take `--timeout-s` and `--build-timeout-s` for the
same two deadlines.

IDs are unique nonempty `[a-z0-9-]+`. `guards`, `test_file`, `runner`, `package`
and `expect_red` are required. `runner` is `cargo` or `nextest`. `target` defaults
to all package tests; it is a whitespace-separated Cargo target selector, not
shell syntax or an arbitrary command/name filter. Selectors include `--lib`,
`--test name`, `--bin name`, `--example name`, `--bench name`, `--tests`, `--bins`,
`--examples`, and `--all-targets`. Quoted paths and shell expansion are not supported.

`equivalent` and `unreachable` are mutually exclusive recorded dispositions,
not runner detections. Each takes a non-empty reason string. An `unreachable`
reason must explain why no production caller exists (for example, all references
are unit tests); the runner does not infer dead code or verify a call graph.
UNREACHABLE rows may use `expect_red = []`: no guarding tests are claimed.
Their other row fields and edit anchors are still validated by `check`.

For a multi-file break, use `edits` **instead of** `file`/`old`/`new`:

```toml
edits = [
  { file = "src/guard.rs", old = "value > 0", new = "value >= 0" },
  { file = "src/caller.rs", old = "guard(value)", new = "true" },
]
```

When a manifest edit changes dependency resolution, list `Cargo.lock` in `edits`
with Cargo's resolved lockfile so `--locked` builds succeed and the lockfile is
restored and verified like any other edit target.

TOML multiline strings work for multiline anchors. Each sequential edit must
replace **exactly one** non-overlapping occurrence. The replacement operation's
count is the anchor check; there is no separate grep or first-match fallback.
Edits to the same file are evaluated in order, in memory, before any writes.
Empty anchors and no-op edits are rejected. Source must be UTF-8, but saved and
restored bytes preserve line endings exactly. Paths must be regular files within
the repo; traversal, absolute paths and direct symlink targets are refused.

**Names are exact full libtest paths.** `history::some_test` is not `some_test`.
If a repo folds test binaries or changes module structure, update its rows.
Names duplicated across binaries are ambiguous and fail closed rather than
being silently combined. Ignored tests do not count as having run.

## Commands

```sh
ck-mutate check
ck-mutate --catalogue other.toml run --all --report mutations.json
ck-mutate run --diff origin/master --report mutations.json
ck-mutate run --all --shard 1/4
ck-mutate run --all --broad --report nightly-audit.json
ck-mutate run --only flow-list-agent-sees-only-own
```

`check` validates fields, files, anchors, and `expect_red` names via
`cargo test ... -- --list` or nextest's JSON list mode. It makes no source edit;
list mode may compile test binaries. Missing full names are reported explicitly.

`run --broad` is the **expensive audit pass**, intended for nightly or on-demand
use. Each mutant runs every test target of its row's package (`-p PACKAGE --tests`,
or the nextest equivalent), not the workspace. Explicit row targets are retained
as well, so selected examples or benches still run. The separate build uses the
same widened selection. It works with `--all`, `--diff`, `--only`, and sharding.
Only this opt-in pass grades CAUGHT_BROADLY from cross-target collateral.

Normal runs keep the row's original target selection and cost. They still report
collateral in whichever targets ran, but never grade CAUGHT_BROADLY, even if the
row itself selects multiple targets. Their report records `breadth_observed: false`
and catches print "breadth not observed"; absence of a warning is not evidence
that other package targets are unaffected.

`run --diff <base>` compares **committed** `<base>` to `HEAD`. A source edit file,
any `edits[].file`, `test_file`, or a changed/new catalogue row selects that row.
Selection cannot see an edit to a helper module or fixture used by a guarding
test if that path is neither an edit target nor `test_file`. Only the full
nightly replay catches regressions from those otherwise unselected changes.

Row changes are compared as parsed TOML fields (comments/formatting alone are
not changes to the proof). `--only` intersects this selection. Shards are 1-based
`i/n`, assigned in sorted-ID order **after** filtering. Within a shard, rows
execute grouped by package and then by edited file, so a package's rows share
one stretch of rebuilds; console lines and the report stay in sorted-ID order.
Separate CI jobs need separate checkouts: sharding is not permission to mutate
one tree concurrently.
An empty selection succeeds. Unknown IDs and invalid shards are errors.

```sh
ck-mutate prove --id rejects-zero --guards 'zero is rejected' \
  --file src/lib.rs --old 'value > 0' --new 'value >= 0' \
  --test-file src/lib.rs --package my-package --target=--lib \
  --expect-red tests::rejects_zero --only --report proof.json
```

`prove` appends a row on CAUGHT. It reports collateral but does not audit breadth;
replay the recorded row with `run --broad` for that audit. A SURVIVED replay is repeated without target
selection across the **whole package** (not the workspace). If broader tests
fail, it diagnoses omitted covering tests; otherwise it cannot distinguish a
real coverage gap from semantic equivalence and suggests inspecting the mutant
and supplying a justified `equivalent` reason. A proof inside a function says
nothing about callers reaching it: when a CAUGHT row mutates a function body,
`prove` names that function and suggests a second row removing a call to it. A
row that itself removes a call site, or that mutates something outside every
function body (a constant, a type, an attribute), prints no hint.

### `explore`: when nobody knows the guarding tests yet

```sh
ck-mutate explore --package my-package --file src/lib.rs \
  --old 'value > 0' --new 'value >= 0' --report explore.json
ck-mutate explore --package my-package --workspace --edits edits.toml
ck-mutate explore --package my-package --file src/lib.rs \
  --old 'value > 0' --new 'value >= 0' \
  --append --id rejects-zero --guards 'zero is rejected' --test-file src/lib.rs
ck-mutate explore --package my-package --file src/lib.rs \
  --old 'value > 0' --new 'value >= 0' \
  --unreachable 'This helper is referenced only by unit tests; no production caller exists' \
  --append --id unused-guard --guards 'zero is rejected' --test-file src/lib.rs
```

Use `prove` when you already know which test should catch a mutant: it runs
only that row's target and checks the names you give it. Use `explore` when you
don't (an age-selected sweep, a new mutant, auditing old code). It answers "does
anything catch this mutant?" and names the tests that do.

**Explore's answer is unscoped by design.** It ignores target selectors and runs
every test in `--package`, or every test in the workspace with `--workspace`, so
it costs a full package or workspace test run per mutant. It grades parsed
per-test results, never the command's exit status:

| Outcome | Meaning for explore |
| --- | --- |
| CAUGHT | At least one test went red. Every red test is listed by exact full name. |
| SURVIVED | Tests ran and none went red. Prints the survivor diagnosis below. |
| NO_TESTS_RAN | Passed plus failed is zero. |
| UNREACHABLE | A person supplied `--unreachable REASON`; records the reason without executing a mutant. Not a catch. |

Explore exits 0 on CAUGHT or a reasoned UNREACHABLE, 1 on other outcomes, and 2 on a preflight
error such as a dirty target.

ANCHOR_MISSING, DID_NOT_COMPILE, TIMED_OUT and ERROR mean exactly what they mean
for `run`. A SURVIVED explore prints the same three causes `prove` diagnoses,
worded for a run that was already unscoped: the mutant is equivalent; the
guarding test lives outside the scope run (another package, so rerun with
`--workspace`; an ignored test; a feature or cfg the run did not enable); or the
guard is missing.

Explore uses the same machinery as `run` for every mutant: the tree lock, the
dirty-target refusal (`--allow-dirty` opts in), the exact-once anchors, the
separate build, per-test parsing, byte restoration of targets and `Cargo.lock`,
and signal and timeout handling. `--edits` takes inline TOML or JSON (an array of
`{ file, old, new }`, or a document with an `edits` array), or a path to a file
holding it (relative to the Git root, like `--catalogue`). `--report` writes the same JSON array `run` writes. Test names repeated
across test binaries fail closed as ERROR, as they do for `run`. That happens more
often with `--workspace`.

`--append` requires `--id`, `--guards` and `--test-file`, and acts on
CAUGHT or an explicitly assigned UNREACHABLE disposition. It checks the id, guards and test file before the run. After a catch it
builds a row for the whole package (no `target`, `only = false`) that names every
red test as `expect_red`. It validates that row with `check` and then appends it
with the same writer `prove` uses. UNREACHABLE appends its reason and an empty
`expect_red`; no test run occurs. On other outcomes nothing is appended. With
`--workspace`, red tests outside `--package` cannot be named in the row: `check`
rejects it and nothing is written.

## Outcomes and evidence

CAUGHT, CAUGHT_BROADLY, and explicitly recorded EQUIVALENT or UNREACHABLE rows
succeed. Only the first two count as catches:

| Outcome | Meaning |
| --- | --- |
| CAUGHT | Every expected test failed; with `only`, no other test failed. |
| CAUGHT_BROADLY | In a `run --broad` audit, every expected test failed, but a collateral red test belongs to a target outside all expected-test targets. A warning, not a failing row. |
| SURVIVED | An expected test passed, with no unrelated failure. |
| WRONG_TEST | An expected test passed while another failed, or `only` forbids an extra failure. |
| NO_TESTS_RAN | Passed plus failed is zero, or an expected full name did not run. |
| ANCHOR_MISSING | The replacement count was zero or greater than one. |
| DID_NOT_COMPILE | The separate build command exited nonzero. |
| TIMED_OUT | The build exceeded `build_timeout_s`, or the test run exceeded `timeout_s`. `timed_out_phase` is `"build"` or `"test"`, and the reason names the deadline. |
| EQUIVALENT | Explicitly skipped with the catalogue's reason. |
| UNREACHABLE | Explicitly recorded with a reason explaining why no production caller exists. Listed separately, never counted as caught. |
| ERROR | Invalid/incomplete runner output, interruption, or restoration/lockfile integrity error. |

**Current policy:** CAUGHT_BROADLY warns, succeeds, and still records a proof.
A later release will make it a failing outcome, after the catalogues that use
this runner have rewritten or dispositioned their broad rows. There is no numeric
threshold or configuration knob. Collateral confined to the expected tests'
target(s) is reported without changing CAUGHT. `only = true` still rejects any
extra red test as WRONG_TEST before broad-catch grading, including in `--broad`.

The JSON array holds each ID, outcome, `timed_out_phase` (`"build"`, `"test"`,
or null when the row did not time out), red and green full test names, build/test
milliseconds, reasons, and up to 8,000 characters of each output tail. Summary
lines are uppercase outcomes. Exit status is nonzero on any failing row or hard
preflight error. A dirty-target refusal is a preflight error (no mutation/report
row); normal row failures are included in the report.

Every row includes `breadth_observed`: true only when an opt-in broad audit
finished and its complete per-test results were parsed. Normal replays, `prove`,
`explore`, explicit dispositions, and incomplete audits report false.

Every row also includes `collateral: { "count": N, "targets": [...] }`. For proof
replays, it counts every red test not named in `expect_red` and lists their sorted,
de-duplicated targets, including same-target failures. With no collateral it is
`{ "count": 0, "targets": [] }`. Explore discovers its expected names from all
red tests, so its collateral is empty. Cargo target identities are executable
file names from `Running ... (<binary>)` headers (including Cargo's hash suffix);
doctests use `doc:<crate>`. Human nextest uses its binary token; JSON nextest uses
the `crate::binary` prefix. Attribution is retained by the same per-test parser.
Console output includes collateral on catches with extra reds, a warning summary
for CAUGHT_BROADLY, and a separate list and count of UNREACHABLE rows.

These formats have no explicit version field: 0.2 retains the unversioned TOML
`[[control]]` schema and JSON array, adding `collateral` and `breadth_observed` to report rows and the
optional `unreachable` reason to controls. Existing 0.1 catalogues/proofs remain
readable unchanged. Older runners rejecting new `unreachable` rows is expected.

Builds are separate: Cargo uses `cargo test -p PACKAGE TARGET --no-run --locked`;
nextest uses `cargo nextest run ... --no-run --locked`. Only build exit status
determines DID_NOT_COMPILE. Test exit status never determines catches.
Cargo results use per-test `test NAME ... ok|FAILED|ignored` lines and require
summary counts to agree; each summary count is located by its following word,
not a column. All failures (`0 passed; 2 failed`) are a valid catch, not an empty
run. `test result:` is never a test named `result:`. Both runners disable
fail-fast so later binaries are not omitted.

For nextest, the runner probes `run --help` for `libtest-json` and uses that
machine-readable event stream when available (enabling its experimental feature).
Nextest's `crate::binary$` prefix is removed to retain the exact libtest path.
It checks terminal suite counts and disables retries to avoid conflating failed
attempts with final results. Older nextest versions use color-free `PASS`/`FAIL`
status lines with all statuses enabled; unknown formats cannot establish a catch.
List mode always uses nextest JSON. Current nextest is exercised in CI.

## Safety and traps

An OS advisory lock at Git's `ck-mutate.lock` path excludes concurrent runs,
including when `.git` is a worktree pointer. The lock file remains but the lock
is released when its handle closes. A deleted or renamed edit target reports ANCHOR_MISSING for its row, naming the
missing file; subsequent rows still run. Every existing target is compared **byte for byte to
HEAD**, including staged changes, before any mutation. `--allow-dirty` opts in
explicitly, and restoration still uses saved local bytes, not HEAD or the index.

Every path restores saved source and verifies its bytes. Unless it is an edit
target, root `Cargo.lock` is compared before/after; a change fails the row and its
original bytes/existence are restored too. RAII restoration covers unwinding
panics. SIGINT and SIGTERM set an interruption flag; the process group is killed,
reaped, and restoration finishes before exit. Timeouts kill the whole Unix group,
not just Cargo. On Windows a Ctrl-C handler triggers restoration and
`taskkill /T /F` kills the child tree. Windows does **not** offer Unix SIGTERM
semantics; closing a console,
TerminateProcess, power loss, SIGKILL, aborting panics, or OS failure cannot be
covered by in-process restoration. Detached processes that escape a Unix process
group are not covered. Run proofs in disposable CI checkouts, not production.

**Never `git checkout` a target mid-run.** It removes the mutation before tests
execute and fakes SURVIVED. Do not edit targets or their tests while a replay is
running. After a SIGKILL the mutated file is left in place, and the next run
refuses it as dirty. Once the killed runner and its children are no longer
running, restore each affected file with `git checkout -- <file>` and rerun.
This intentionally discards the mutant; if the interrupted run used
`--allow-dirty`, recover your original local edits from a separate backup rather
than checking them out. The lock coordinates runner instances, not editors or external Git
commands. A restoration failure is a hard error naming the file: investigate
before using that checkout again.

## CI

For a consuming repo, replace `<sha>` with a reviewed full SHA:

```yaml
name: Mutation proofs
on:
  pull_request:
  schedule:
    - cron: '0 3 * * *'
concurrency:
  group: mutations-${{ github.event_name }}-${{ github.ref }}
  cancel-in-progress: true
jobs:
  mutations:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        shard: [1, 2, 3, 4]
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install --locked --git https://github.com/cortexkit/commons --rev <sha> cortexkit-mutate
      - run: ck-mutate check
      - if: github.event_name == 'pull_request'
        run: ck-mutate run --diff '${{ github.event.pull_request.base.sha }}' --shard ${{ matrix.shard }}/4 --report mutations.json
      - if: github.event_name == 'schedule'
        run: ck-mutate run --all --broad --shard ${{ matrix.shard }}/4 --report mutations.json
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: mutations-${{ matrix.shard }}
          path: mutations.json
```

Keep `${{ github.event_name }}` in the group: a group keyed on `${{ github.ref }}` alone with `cancel-in-progress: true` (common in a repository's main CI workflow) puts the nightly run in the default branch's group, so any push to that branch cancels the hours-long scheduled run.

The cortexkit/commons repository's continuous integration runs this runner's fixture controls on pushes and PRs touching this crate or its workflow, on Linux,
macOS and Windows, including actual nextest replay. Unix signals have dedicated
controls. The workspace test gate includes these controls too. See
`tests/controls.rs` and the library's panic restoration test.

## Dependencies

No internal workspace crates or async runtime are used. `clap` provides strict CLI validation
and help; `serde` derives the catalogue/report schema; `toml` reads/writes the
catalogue; `serde_json` writes evidence and reads nextest events; `fs2` supplies
portable advisory locks; `ctrlc` supplies Unix interruption/termination and
Windows Ctrl-C handling; Unix-only `rustix` with its `process` feature supplies safe process-group and test signal APIs. The library, binary and integration tests forbid unsafe code. `tempfile` is
only a dev dependency, isolating real Git/Cargo fixture repos for tests.
