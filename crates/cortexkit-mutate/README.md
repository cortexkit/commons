# cortexkit-mutate / ck-mutate

A standalone, unpublished runner for checked-in mutation proofs. Install a reviewed,
immutable revision (replace `<sha>` with a **full commit SHA**):

```sh
cargo install --locked --git https://github.com/cortexkit/commons --rev <sha> cortexkit-mutate
```

Run from anywhere inside the repository. Paths in the catalogue are relative to
its Git root, even when `--catalogue` overrides the default `mutations.toml`.
Cargo/nextest repositories must already have a current `Cargo.lock`: every build, test and
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
# hub = "Explain why multiple test targets intentionally guard this property"
# hub_targets = ["capacity_contract", "encoder_e2e"]
# platforms = ["macos"]
# desk_only = "TCC requires a real Mac with an interactive desktop session"
```

The two deadlines are separate. `timeout_s` (default 600) bounds only the test
run. `build_timeout_s` (default 1800) bounds the separate build of the mutant and
the compile inside `check`'s list mode. A slow compile on a loaded host is
therefore never read as a hung test, and a TIMED_OUT row says which deadline
expired. `prove` and `explore` take `--timeout-s` and `--build-timeout-s` for the
same two deadlines.

A `command` row has no separate build: its command builds and tests in one
process (an `xcodebuild test` or a Swift package test, for example), so its
`timeout_s` covers both, and `build_timeout_s` does not apply to it. To time or
bound a build separately, declare it as a prebuild step, which has its own
`timeout_s` and is reported as `prebuild_ms`.

IDs are unique nonempty `[a-z0-9-]+`. `guards`, `test_file`, `runner`
and `expect_red` are required. `runner` is `cargo`, `nextest`, or `command`.
Cargo/nextest rows also require `package`. Their `target` defaults
to all package tests; it is a whitespace-separated Cargo target selector, not
shell syntax or an arbitrary command/name filter. Selectors include `--lib`,
`--test name`, `--bin name`, `--example name`, `--bench name`, `--tests`, `--bins`,
`--examples`, and `--all-targets`. Quoted paths and shell expansion are not supported.

Cargo/nextest `expect_red` names may be plain when unique in the package. If two
test binaries share a name, use `target::test_name`, for example
`ck-under-test::tests::cgroup_placement_override_requires_exact_disabled_value`
(using the stable target name the runner reports). Nextest retains its binary id,
such as `package::binary::tests::x`. Ambiguous plain expectations are validation
errors listing the qualified candidates, even for a narrowly selected target.
Broad, package and explore results qualify repeated names and keep each binary's
independent red/green result; they never discard a binary to resolve ambiguity.
Unique names remain plain in reports, and qualified expectations still work in
a scoped replay observing only that one binary.

`equivalent` and `unreachable` are mutually exclusive recorded dispositions,
not runner detections. Each takes a non-empty reason string. An `unreachable`
reason must explain why no production caller exists (for example, all references
are unit tests); the runner does not infer dead code or verify a call graph.
UNREACHABLE rows may use `expect_red = []`: no guarding tests are claimed.
Their other row fields and edit anchors are still validated by `check`.

### Platform gates and desktop-only proofs

Use Rust **target-OS names**, not platform nicknames:

```toml
[[control]]
id = "desktop-rejects-untrusted-input"
guards = "the macOS input guard rejects untrusted events"
file = "src/macos.rs"
old = "event.is_trusted()"
new = "true"
test_file = "src/macos.rs"
runner = "cargo"
package = "desktop-guard"
target = "--lib"
expect_red = ["macos::tests::untrusted_input_is_rejected"]
platforms = ["macos"]
```

An absent `platforms` runs everywhere. A present list must be nonempty and contain
recognized Rust `target_os` values (for example `macos`, `linux`, `windows`, `ios`,
`android` or `freebsd`; `darwin` and `MacOS` are rejected). Matching uses the OS of
the runner's host, not a cross-compilation target. On another OS, replay reports
**SKIPPED_PLATFORM**, succeeds, and counts the row separately: no mutation, build,
test-name listing, command baseline or prerequisite is run for it. `check` still
validates its fields, paths and anchors, but cannot verify names that do not exist
on this host. Platform skips take precedence over EQUIVALENT and UNREACHABLE; HUB
never grants caught credit to a skipped row.

Some tests require more than the right OS: TCC, Accessibility or physical input
may require a real interactive Mac. Record that human-only proof with a reason:

```toml
# Inside the same control table, instead of other recorded dispositions:
desk_only = "TCC and physical input were verified on a real interactive Mac"
```

**DESK_ONLY** skips all automated execution, even on a matching host. It requires
a nonempty reason, may have `expect_red = []`, is exclusive with `equivalent`,
`unreachable` and `hub`, and is listed/counts separately, never as a catch or an
error. DESK_ONLY takes precedence over a platform mismatch, so the report retains
the reason automation cannot verify the proof. `platforms` describes the OS;
`desk_only` describes the need for a person and hardware. Neither is evidence of
an automated catch.

`prove` and `explore` accept repeatable `--platform <target_os>` and
`--desk-only REASON`. A platform skip never appends a proof; an explicit DESK_ONLY
reason can be appended without executing a mutant, like UNREACHABLE in explore.
`explore` also infers `platforms` for simple whole-file gates it can see:
`#![cfg(target_os = "macos")]` in the edited Rust file, or an adjacent conventional
`#[cfg(target_os = "macos")] mod macos;` declaration. All edited files must have
the same detectable gate. It does **not** guess compound `cfg`, `cfg_attr`, macros,
`#[path]`, gates on individual items, ancestor module gates or test-only gates.
For those, supply `--platform` or add `platforms` by hand. An explicit CLI field
overrides inference. `explore --append` preserves a detected gate in its row.

### Declared fixture prerequisites

Put an ordered `prebuild` list at the **catalogue root**, before any `[[control]]`:

```toml
prebuild = [
  { name = "daemon-fixture-binaries", command = ["cargo", "build", "-p", "subc-daemon", "--bins", "--features", "test-support", "--locked"], timeout_s = 1800 },
]

[[control]]
# ...the daemon's proof row...
```

Each step has a unique nonempty `name`, an argv-array `command` (no shell
expansion), and a positive `timeout_s` (default 1800). Commands run from the Git
root. Cargo prerequisites must explicitly include `--locked`. Use these steps
only to build fixture artifacts; they must not change edit targets or Cargo.lock.
The runner detects and restores such changes on the unmutated tree, and its
normal byte/lockfile restoration still covers mutated-tree failures, including
rows that deliberately edit Cargo.lock.

Once per CLI session, after selection/sharding and before **any** baseline or
mutation, all steps run on the unmutated tree. If a step fails, cannot spawn,
is interrupted or times out, the **whole replay aborts** with ERROR and that
step's name; no rows are graded. An empty selection or a selection containing
only platform skips/recorded dispositions runs no prerequisites.

The steps run **again after every successful mutant build, before its tests**.
For command rows, which have no build phase, they run immediately after the
edit and before mutant test commands. All steps rerun conservatively: the runner
builds only selected test binaries with `--no-run`, which does not guarantee that
fixture binaries (especially those built with different features) are refreshed.
An arbitrary prerequisite command has no reliable dependency graph, and even a
lockfile mutation can change its output. Reusing a baseline binary could therefore
hide a mutant. A mutated prerequisite failure is row **ERROR**, never CAUGHT or
DID_NOT_COMPILE, because the guarding tests did not execute.

As of 0.5.2, sessions collect **every selected command row's per-id baseline
before the first mutant**. A failing baseline is that row's ERROR and skips its
mutant; other baselines still run on clean fixtures. Cargo and nextest replays
have no green baseline runs; package-wide name listing validates expected test
identities on the clean tree before mutants. Their mutant build/test protocol is unchanged.
No clean-tree test run can consume an earlier mutant's fixture.

Source restoration between mutants does not rebuild fixtures: the next mutant
refreshes its own prerequisites as above. After all mutants, the session runs
prerequisites **once on the restored source**, leaving the developer's fixtures
clean. N mutants that reach their prerequisite phase require **N+2** builds
(initial, N mutated, final), instead of 2N. A failed or interrupted mutant build
also requires final cleanup, since builds may produce artifacts before failing.
A failing final refresh aborts the session by step name: the tree's fixtures
are suspect. If no mutant is applied, the initial clean preparation is sufficient.
Mutant builds must not rely on a previous row's generated fixture; prerequisites
that generate compile-time inputs should handle refreshing those inputs within
the build itself. Declared mutant prerequisites still run after `--no-run`.

`run --broad`, command rows, `prove` (including its survivor diagnosis), and
`explore` all use the same session prerequisites. `prove` and `explore` read them
from `--catalogue` even without appending. Append preserves the existing root
list and does not run it again for explore's name check. Standalone `check` runs
preparation once before list-mode compilation. Library clients with catalogue
prerequisites use `ReplaySession::prepare`, `baselines` for all selected rows
(command runs or Cargo/nextest name validation), replay methods, then `finish`
(also on early errors); plain
`run_row`/`explore_row` have no catalogue context.

Full prerequisite output, step names and elapsed milliseconds go to stderr logs.
JSON rows add `prebuild_ms` and `prebuild_tail` for mutant preparation, separate
from `build_ms` and `test_ms`. Session preparation is attributed **once**, to the
first executable row, in `baseline_prebuild_ms` and `baseline_prebuild_tail`;
other rows have zero/empty baseline fields. Output tails are bounded like other
runner output. `restore_prebuild_ms` and `restore_prebuild_tail` separately record
the final restored-tree refresh, attributed once to the last reported row.
Report order is unchanged, regardless of the internal grouped execution order.
A prerequisite timeout uses
`timed_out_phase: "prebuild"` and ERROR.

### HUB: reviewed cross-target catches

Use `hub = "reason"` with `hub_targets = ["target_name", ...]` when a broad
catch is legitimate: several test targets intentionally assert the same property,
and narrowing the mutant would assert less. HUB, EQUIVALENT, and UNREACHABLE are
mutually exclusive. A HUB reason must contain **at least 20 characters after
trimming**, and `hub_targets` must be present and nonempty. Supplying targets
without a reason is refused at load, with the row's id.

HUB does **not** skip a mutant. Only under `run --broad`, after all expected tests
fail (and `only` is satisfied), the runner compares only collateral targets outside
every expected-test target to `hub_targets`. This list approves the **other targets**
that made the catch broad; it need not include the expected tests' own targets.
The same cross-target set or a subset grades HUB and counts as caught, with its own
summary count and a listing of reviewed reasons. Any cross-target collateral outside
the recorded set grades CAUGHT_BROADLY and the reason names those new targets for
review. Same-target collateral, including newly failing tests, never needs HUB
approval and remains in the report's full `collateral.targets` list.
A plain run ignores HUB and grades the row normally,
because it cannot observe package breadth. Command rows cannot observe breadth
and therefore never grade HUB.

Copy **stable target names** from a broad report's `collateral.targets`, not Cargo
executable hashes: for example, `encoder_e2e`, not `encoder_e2e-3f9a1c0123456789`.
Nextest uses its binary id (the binary token for human output, `crate::binary` for
JSON output). Include only targets outside every expected-test target in the
reviewed list. A smaller observed cross-target set needs no catalogue change; a
new other target needs another review. Write HUB by hand after reviewing a broad
catch: neither `prove` nor `explore --append` may create it, and the shared append
writer refuses HUB.

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

**Cargo/nextest names are exact full libtest paths.** `history::some_test` is not `some_test`.
If a repo folds test binaries or changes module structure, update its rows.
Names duplicated across binaries are ambiguous and fail closed rather than
being silently combined. Ignored tests do not count as having run.

## Command rows

For Python, Bun, Xcode/Swift, or another runner with a per-test invocation, use
an **argv array**, never a shell string:

```toml
[[control]]
id = "rig-rejects-empty"
guards = "the rig rejects an empty flow"
file = "script/flows_rig.py"
old = "if not flows:"
new = "if False:"
test_file = "script/tests/flows_rig.py"
runner = "command"
command = ["python3", "-m", "unittest", "{test}"]
test_count_pattern = "Ran {count} test"
expect_red = ["script.tests.flows_rig.RigChecks.test_rejects_empty"]
# timeout_s = 600
```

`{test}` must occur exactly once, as a complete argv element. For each expected
id, it is replaced byte-for-byte with that id as **one argument**: dots, `::`,
and other punctuation are not split or normalized. Ids must be nonempty and
contain no whitespace or control characters. The command runs from the Git
root, without shell expansion. `package` and `target` must be absent, even if
empty; cargo/nextest rows must not carry `command`. `only = true` is refused
because command rows cannot observe extra failing tests.

Command rows must demonstrate that tests actually executed, on **both** the
baseline and mutant. `test_count_pattern` is required: a literal output pattern
with exactly one `{count}` placeholder for an unsigned decimal count, with
nonempty literal text on both sides. It is **not a regex**. For Python unittest,
`"Ran {count} test"` matches both `Ran 1 test` and `Ran 2 tests`; use the executed
count in your runner's summary, not a discovered/planned/skipped-test count.
Each invocation must produce exactly one matching count and it must be nonzero.
Zero, missing, overflowing or ambiguous counts are ERROR, regardless of exit
status. A runner without a count summary needs a wrapper that reports the number
it actually executed. This prevents a zero-test success or unrelated nonzero
exit from establishing a false pass/catch. Cargo/nextest already parse their
per-test counts and cannot carry `test_count_pattern`.

**Every replay first runs every expected id on the unmutated tree.** Each must
exit 0 before any mutant in the session is applied. A non-green baseline is ERROR
and names the id. Baselines are collected afresh each session, never cached across
sessions: otherwise an always-failing command, a
stale environment, or a pre-existing broken test could masquerade as a catch.
On the mutant, every expected id exiting nonzero means CAUGHT; any exiting 0
means SURVIVED. A spawn failure, exit 126 (not executable, or the command refused
to run), exit 127 (not found), death
by signal, interruption, or timeout is ERROR, **never red**, on either tree.
The row's `timeout_s` applies separately to each baseline and mutant invocation.
There is no normal build phase; command test timeouts report `timed_out_phase: "test"`.
Declared prerequisites have their own build deadlines and timing fields.

Only the expected ids run. `collateral` is always `{ "count": 0, "targets": [] }`
and `breadth_observed` is always false. **Command rows have no breadth audit:**
`run --broad` runs them normally and cannot discover other tests, despite the
generic console suggestion to audit breadth. `explore` refuses command rows.
`check` validates their fields, files, and anchors, but has no test-list protocol
to validate names; the fresh baseline during replay verifies the invocation.

To prove and append the same row, put `--command` **last**. It consumes all
remaining argv elements, including options belonging to the test runner. The
runner defaults to `command` when `--command` is present (otherwise `cargo`):

```sh
ck-mutate prove --id rig-rejects-empty --guards 'the rig rejects an empty flow' \
  --file script/flows_rig.py --old 'if not flows:' --new 'if False:' \
  --test-file script/tests/flows_rig.py \
  --expect-red script.tests.flows_rig.RigChecks.test_rejects_empty \
  --test-count-pattern 'Ran {count} test' \
  --report proof.json --command python3 -m unittest '{test}'
```

Repeat `--expect-red <id>` to name multiple expected tests. `prove` appends only
on CAUGHT, in the same canonical TOML format as cargo rows; a command survivor
has no unscoped second replay because there is no package-wide test discovery.

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
| SKIPPED_PLATFORM | The host OS is outside the row's explicit or reliably inferred platform list. No tests or prerequisites run for this row. |
| DESK_ONLY | A person supplied `--desk-only REASON`; no automated replay is possible. Not a catch. |

Explore exits 0 on CAUGHT, SKIPPED_PLATFORM or a reasoned UNREACHABLE/DESK_ONLY, 1 on other outcomes, and 2 on a preflight
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
CAUGHT or an explicitly assigned UNREACHABLE/DESK_ONLY disposition. It checks the id, guards and test file before the run. After a catch it
builds a row for the whole package (no `target`, `only = false`) that names every
red test as `expect_red`. It validates that row with `check` and then appends it
with the same writer `prove` uses. UNREACHABLE/DESK_ONLY appends its reason and an empty
`expect_red`; no test run occurs. On other outcomes nothing is appended. With
`--workspace`, red tests outside `--package` cannot be named in the row: `check`
rejects it and nothing is written.

## Outcomes and evidence

CAUGHT, CAUGHT_BROADLY, HUB, SKIPPED_PLATFORM and explicitly recorded EQUIVALENT, UNREACHABLE or DESK_ONLY rows
succeed. The first three count as catches:

| Outcome | Meaning |
| --- | --- |
| CAUGHT | Every expected test failed; with `only`, no other test failed. |
| CAUGHT_BROADLY | In a `run --broad` audit, every expected test failed, but a collateral red test belongs to a target outside all expected-test targets. A warning, not a failing row. |
| HUB | In a `run --broad` audit, every expected test failed and every collateral target outside the expected tests' own targets is in the row's reviewed `hub_targets`. Same-target collateral needs no approval. Counts as caught, with the recorded reason. |
| SURVIVED | An expected test passed, with no unrelated failure. |
| WRONG_TEST | An expected test passed while another failed, or `only` forbids an extra failure. |
| NO_TESTS_RAN | Passed plus failed is zero, or an expected full name did not run. |
| ANCHOR_MISSING | The replacement count was zero or greater than one. |
| DID_NOT_COMPILE | The separate build command exited nonzero. |
| TIMED_OUT | The build exceeded `build_timeout_s`, or the test run exceeded `timeout_s`. `timed_out_phase` is `"build"` or `"test"`, and the reason names the deadline. |
| EQUIVALENT | Explicitly skipped with the catalogue's reason. |
| UNREACHABLE | Explicitly recorded with a reason explaining why no production caller exists. Listed separately, never counted as caught. |
| SKIPPED_PLATFORM | Host target_os is not in platforms. Counted separately, not executed. |
| DESK_ONLY | Explicit nonempty reason that this proof needs a real desktop; never automated or counted as caught. |
| ERROR | Invalid/incomplete runner output (including zero-test command rows), prerequisite failure, interruption, or restoration/lockfile integrity error. |

**Current policy:** CAUGHT_BROADLY warns, succeeds, and still records a proof.
A later release will make it a failing outcome, after the catalogues that use
this runner have rewritten or dispositioned their broad rows. There is no numeric
threshold or configuration knob. Collateral confined to the expected tests'
target(s) is reported without changing CAUGHT. `only = true` still rejects any
extra red test as WRONG_TEST before broad-catch grading, including in `--broad`.

The JSON array holds each ID, outcome, `timed_out_phase` (`"build"`, `"test"`,
`"prebuild"`, or null when the row did not time out), red and green full test names, build/test/prebuild
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
red tests, so its collateral is empty. Cargo target identities are stable executable
names from `Running ... (<binary>)` headers, without the hash suffix or Windows `.exe`;
doctests use `doc:<crate>`. Human nextest uses its binary token; JSON nextest uses
the `crate::binary` prefix. Attribution is retained by the same per-test parser.
Console output includes collateral on catches with extra reds, a warning summary
for CAUGHT_BROADLY, and separate lists and counts of UNREACHABLE and HUB rows.

These formats have no explicit version field: 0.5 retains the unversioned TOML
`[[control]]` schema and JSON array, adding `platforms`, `desk_only`, root `prebuild`,
the required command-row `test_count_pattern`, skip outcomes and preparation timing.
Cargo/nextest catalogues remain readable unchanged; existing command catalogues
must add an executed-count pattern. Older runners rejecting new fields is expected. Since 0.4,
`collateral.targets` uses stable Cargo names rather than hash-suffixed executables.

For Cargo/nextest, builds are separate: Cargo uses `cargo test -p PACKAGE TARGET --no-run --locked`;
nextest uses `cargo nextest run ... --no-run --locked`. Only build exit status
determines DID_NOT_COMPILE. Test exit status never determines Cargo/nextest catches.
Command rows instead use the baseline-gated exit-status rules above.
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

After an interrupted mutant, the final restored-tree prebuild still runs: the
latched interrupt does not cancel cleanup, but each prerequisite's own timeout
still applies. A second interruption does not bypass those cleanup deadlines.

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
