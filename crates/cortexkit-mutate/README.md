# cortexkit-mutate / ckdev-mutate

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
# features = ["test-support"]
# no_default_features = true
# all_features = true # instead of features/no_default_features
# ignored = "include" # or "only": also run (or run only) #[ignore]d tests
expect_red = ["list_agent_cannot_see_another_agents_flow"]
# expect_message = "assertion failed: own_flows_only" # or "/own.*flows/"
# signal_is_catch = "This guard intentionally aborts on invalid input"
only = true
# timeout_s = 600
# build_timeout_s = 1800
# equivalent = "Explain why this mutant computes exactly the same result"
# equivalent_guard = "path::symbol or the code fact preserving behaviour"
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

Cargo/nextest `expect_red` names may be plain when unique in the selected targets. If two
test binaries share a name, use `target::test_name`, for example
`ck-under-test::tests::cgroup_placement_override_requires_exact_disabled_value`
(using the stable target name the runner reports). Nextest retains its binary id,
such as `package::binary::tests::x`. Ambiguous plain expectations are validation
errors listing the qualified candidates within that selection.
Broad, package and explore results qualify repeated names and keep each binary's
independent red/green result; they never discard a binary to resolve ambiguity.
Unique names remain plain in reports, and qualified expectations still work in
a scoped replay observing only that one binary.

### Cargo feature selection (0.7.2)

| Row field | Default | Applies to | Meaning |
| --- | --- | --- | --- |
| `features = ["a", "b"]` | absent | cargo / nextest | Enable these Cargo features in addition to defaults. Dependency-qualified names such as `dep/seam` are allowed. |
| `no_default_features = true` | false | cargo / nextest | Disable default features; may be combined with `features`. |
| `all_features = true` | false | cargo / nextest | Enable every feature; mutually exclusive with `features` and `no_default_features = true`. |

Feature names must be nonempty, without whitespace, control characters, commas,
or a leading `-`. `check` rejects invalid combinations and names. Command rows
must omit all three fields, even empty lists or false values: their argv owns
its own build options. Do not put feature flags in `target`.

Every row build, test run, clean-tree baseline and name listing uses the same
feature selection, including `check`, `run --broad`, and package diagnosis.
Baseline and name-list caches distinguish features and default/all-feature flags.
`prove` and `explore` accept `--features a,b` (or repeated `--features a`),
`--no-default-features`, and `--all-features`, and retain them when appending.
Explore still runs the whole package (or workspace with `--workspace`).

For a row with `package = "example"`, `target = "--test contract"`,
`features = ["test-support"]` and `no_default_features = true`, the Cargo commands
are (both clean baseline and mutant use the build/run commands):

```sh
cargo test --locked -p example --test contract --features test-support --no-default-features --no-run
cargo test --locked -p example --test contract --features test-support --no-default-features -- --list
cargo test --locked -p example --test contract --features test-support --no-default-features --no-fail-fast -- --test-threads=1
```

Before 0.7.2 baseline name resolution used `cargo test --locked -p example -- --list`,
dropping the row's target (and lacking a way to select features). Now it uses the
second command above, the same target and features as the replay. `check` likewise
lists only selected targets. Broad audits add `--tests` before the row's target;
package diagnosis and explore omit the row's target, retaining its features.
Workspace explore replaces `-p example` with `--workspace`. Nextest uses the same
selection flags with `cargo nextest run --no-run`, `cargo nextest list
--message-format json`, and `cargo nextest run --no-fail-fast --retries 0` (plus
status/JSON output flags); feature flags always precede any harness separator.
Nextest's `cargo nextest run --help` capability probe is not a build/test/list and
does not take feature selection.

### Ignored-test selection (0.7.3)

Expensive guards, such as tests that need a real daemon, are often kept
`#[ignore]`d and run in a dedicated CI job. A cargo or nextest row selects them
with one typed field; there is no free-form way to pass harness arguments.

| Row field | Default | Applies to | Meaning |
| --- | --- | --- | --- |
| `ignored = "include"` | absent | cargo / nextest | Run `#[ignore]`d tests as well as the ordinary ones. |
| `ignored = "only"` | absent | cargo / nextest | Run only `#[ignore]`d tests. |

Any other value is a catalogue parse error, and command rows must omit the field
(their argv owns its harness options). The selection reaches every test listing
and run of the row, exactly as feature selection does: name resolution, the
clean-tree baseline, the mutant run, `check`, `run --broad`, package diagnosis
and `explore`. Builds select no tests and are unchanged. Baseline and name-list
caches are keyed by the selection, so a row running ignored tests never reuses a
baseline collected without them, or the reverse. `prove` and `explore` accept
`--ignored include` or `--ignored only` and retain it when appending.

| Runner | Mode | `ignored = "include"` | `ignored = "only"` |
| --- | --- | --- | --- |
| cargo | list | `cargo test ... -- --list --include-ignored` | `cargo test ... -- --list --ignored` |
| cargo | run | `cargo test ... --no-fail-fast -- --test-threads=1 --include-ignored` | `cargo test ... --no-fail-fast -- --test-threads=1 --ignored` |
| nextest | list | `cargo nextest list ... --message-format json --run-ignored all` | `cargo nextest list ... --message-format json --run-ignored only` |
| nextest | run | `cargo nextest run ... --no-fail-fast --retries 0 --run-ignored all` (plus output flags) | `cargo nextest run ... --no-fail-fast --retries 0 --run-ignored only` (plus output flags) |

Nextest's `--run-ignored` values are `default`, `only` and `all` in current
releases; older releases spelled `only` as `ignored-only`. CI installs the latest
nextest, which accepts `only`.

A row without the field that expects an `#[ignore]`d test fails `check` (and its
replay is an ERROR) with a message naming the field, for example `expect_red test
ignored_guard is #[ignore]d and this row does not run ignored tests; set ignored =
"include" (or "only") on the row`, instead of reporting NO_TESTS_RAN after a
baseline. To find them, a cargo row without the field lists the selection a
second time with `-- --list --ignored`, because libtest's plain `--list` does not
mark ignored tests; nextest marks them in its JSON listing. With `"only"`,
ordinary tests are not listed, so expecting one is reported as a name that no
longer exists. Rustdoc reports `ignore` doctests as ignored tests, so an
`"include"` or `"only"` row without a `target` also tries to run those.

### Multiline anchors on CRLF checkouts (0.7.2)

`check` and replay first match every edit's `old` anchor byte-exactly. Only if
there are zero exact matches, the file contains CRLF, and `old` contains LF but
no carriage returns, the runner retries with LF translated to CRLF in both
`old` and `new`. This also applies to every `edits` entry, including sequential
edits to one file. The chosen form must occur exactly once: exact and translated
matches are never combined, and two exact matches never trigger a retry.
Mixed-ending files therefore prefer the exact anchor without rewriting other
line endings. A row using any retry adds `"line_endings": "crlf"` to its report;
the field is absent for byte-exact matches. Restoration always copies the saved
file bytes, preserving even mixed line endings exactly.

### Failure identity and signal deaths (0.6.0)

An optional `expect_message` requires a **case-sensitive substring**, or a Rust
regex enclosed in `/.../`, in the failure output of **each expected red test**.
For example, `expect_message = "capacity exceeded"` or
`expect_message = '/capacity \d+ exceeded/'`. Invalid regexes and empty patterns
are refused. Matching another test's output, a build error, or the command row's
green baseline is not proof. A red test with missing or different output grades
**RED_FOR_ANOTHER_REASON**, fails the row, and reports that test's first eight
failure-output lines. Without this field, ordinary failures grade as before.
`prove` accepts `--expect-message` and preserves it in the appended row.

A test that aborts, segfaults or dies from another signal is **ERROR**, not a
catch; a normal assertion failure or unwinding panic is still red. Cargo's
signal exit diagnostic names the signal, and a binary missing its libtest
summary also fails closed, even if another binary printed a valid summary.
Nextest's signal status names the individual test. The report names the affected
test and signal (or reports that the signal/test identity is unavailable).

Use `signal_is_catch = "reason"` **only when termination by signal is itself the
intended contract**, such as a deliberate `std::process::abort()` guard. The
reason must be nonempty; `prove` accepts `--signal-is-catch REASON`. This opt-in
permits an identified test's signal death to count as red, not an unrelated
crash of Cargo/nextest itself. It does not waive expected-test identities,
executed-count checks, `expect_message`, or a green command baseline. An abort
usually has no failure output, so pairing it with `expect_message` requires
the runner to retain the intended message for that test (Cargo's captured
stdout is generally lost on abort). For Cargo, tests
after the abort do not run; missing expected tests still prevent a catch.

### Captured libtest reports (0.9.1)

Cargo's libtest output is unescaped text: an assertion message can contain a
child's complete test run, even identical `---- name stdout ----` / `---- name
stderr ----` headers, `failures:` headings and `test result:` summaries. Those
headings cannot unambiguously delimit nested reports. Within each Cargo binary
(`Running ... (...)` or `Doc-tests ...`), the parser therefore uses the **first
`running N tests` (or `running 1 test`) header and the last `test result:` line**.
It counts per-test outcomes only before the first `failures:`, `successes:` or
captured-output header. A report heading never reopens outcome collection. The
final `failures:` followed only by blank lines and indented failing names up to
the outer summary ends captured output; earlier nested headings remain text.
Summary counts must still agree with the outer per-test events or the row is
**ERROR**. Minimal parser inputs without a running header remain supported.

Nextest's human results use `PASS` / `FAIL` / signal statuses with duration and
binary tokens, and a capitalized `Summary`, not libtest's `test ...` / `test
result:`. Its JSON events encode captured output as escaped strings. Both paths
are covered with embedded libtest text; no equivalent filtering is needed for
these child reports.

Nextest 0.9.138's own libtest-to-JSON conversion can truncate a failed test's
`stdout` at an embedded `test ...` line, though its outer status/count events
remain correct and the human output retains the child report. Message proofs
using text beyond that point may fail with `RED_FOR_ANOTHER_REASON`; this runner
does not reconstruct output missing from nextest's JSON event.

Cargo/nextest rows do not accept arbitrary harness arguments: `target` is
validated as a Cargo target selector. `--nocapture` and `--show-output` cannot be
requested through row config. The Cargo parser assumes captured output; raw
`--nocapture` output (including externally forced `RUST_TEST_NOCAPTURE`) is not
an unambiguous event stream and may fail strict accounting. `--show-output`
reports successes after the outcome lines, which the parser excludes just like
failure reports. Command rows own their argv and do not use this libtest parser.

### Equivalent mutants (0.7.0)

Use `equivalent` only when inspection establishes that the edit **cannot change
observable behaviour**, not merely because the current tests stay green. Record
both a nonempty explanation and a nonempty `equivalent_guard`: a `path::symbol`
or concrete code fact that makes the equivalence true. For example:

```toml
equivalent = "Reversing the comparison operands preserves strict ordering"
equivalent_guard = "src/guard.rs::guarded: value > 0 is exactly 0 < value"
```

The row is **still replayed**, including its clean baseline, mutation, build and
tests. A survivor grades **EQUIVALENT**, succeeds, and counts separately, never
as a catch. If **any newly red test** catches it (including an unexpected test),
it grades **EQUIVALENT_CAUGHT** and fails: remove the equivalence claim and make
it a normal row. Baseline errors, crashes and build errors are not equivalence
evidence. `check` requires the two fields together and validates their contents,
but cannot prove their semantic claim. Cargo/nextest equivalent rows may use
`expect_red = []` to replay the selected tests without claiming a particular catch.

`equivalent`, `unreachable` and `desk_only` are mutually exclusive, and cannot
be combined with HUB. `unreachable` is a recorded disposition, not a runner
detection, and takes a nonempty reason string. An `unreachable`
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

As of 0.7.0, sessions collect **all selected rows' baselines before the first
mutant**. Command test rows run each expected id; output-equality rows run twice.
Cargo/nextest build and run each selected target selection once, shared across
rows with the same runner, package and selector. `run --broad` baselines include
package tests plus explicit row targets. `prove` prepares the whole package
baseline in advance for its possible survivor diagnosis. Package-wide name
listing still validates expected test identities, including narrow rows.

An expected test already red grades **ERROR** with `baseline red: <tests>` and
its mutant is not run. Other baseline-red tests are recorded per stable target
in `baseline_red` and excluded from mutant grading and collateral, including
WRONG_TEST, CAUGHT_BROADLY and HUB. The report's `red` contains only newly red
tests. Baseline reds remain visible even if they turn green under a mutant:
the session summary names every one under **red at baseline (ignored)**. Never
use these ignored failures as evidence of a catch. Baseline data is never cached
between sessions. No clean-tree run can consume an earlier mutant's fixture.

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
(or `broad_baselines` for a broad replay, `package_baselines` for a package diagnosis), replay methods, then `finish`
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
`baseline_build_ms` and `baseline_test_ms` separate clean-tree compilation/name
listing and tests from mutant `build_ms`/`test_ms`. Shared baseline costs are
attributed only to the first row using that selection, not multiplied by the
number of rows. Stderr prints the session's baseline build, test and prebuild totals.
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

### Exact expected-test selection (0.9.0)

Cargo and nextest rows may opt into **`select = "expected"`** when a target
contains an expensive folded rig and only a few named tests guard this mutant:

```toml
[[control]]
id = "rig-isolates-sessions"
guards = "the daemon rig isolates sessions"
file = "src/session.rs"
old = "session.id()"
new = "default_session.id()"
test_file = "tests/rig.rs"
runner = "cargo" # or "nextest"
package = "my-package"
target = "--test rig"
ignored = "only"
select = "expected"
expect_red = ["sessions::isolates_sessions"]
```

Only the `expect_red` names execute on both the clean baseline and the mutant:
libtest receives the complete names as filters with `--exact`; nextest receives
an equality filterset (`test(=sessions::isolates_sessions)`, with OR for multiple
names). `ignored = "only"` or `"include"` still applies, but does not expand the
name selection. Use real full harness names, not binary-qualified report aliases,
and a target selector that identifies them unambiguously. Missing names, zero
executed matches or selected tests that only skip are ERROR naming the test. The
runner's count summary must agree with its per-test events; an unlisted executed
name is also ERROR. Builds and name listing keep their usual target/feature
selection; listing is not test execution. Without `select`, nothing changes.

This mode never observes breadth, **even under `run --broad`**: the row's target
and exact name selection stay narrow, `breadth_observed` is false, and the report
reason says `selection was expected-only; breadth not observed`. `only = true`,
`hub` and `hub_targets` are refused by name because they would claim breadth that
did not run. Command rows cannot carry `select`. Baseline sharing keys include
the exact expected names, so different name selections never borrow a baseline.
To audit collateral, run a copy of the row without `select` under `--broad`.

## Command rows

For Python, Bun, Xcode/Swift, or another runner with a per-test invocation, use
an **argv array**, never a shell string. With `catch_on` absent, the named-test
protocol below applies; `catch_on = "output_differs"` uses the output-equality
protocol later in this section instead:

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
id, it is replaced byte-for-byte with that id as **one argument**: interior
spaces, dots, `::`, Unicode and other punctuation are not split or normalized.
Since 0.9.0, ids may contain interior spaces. Ids must be nonempty, have no leading
or trailing whitespace, and contain no control characters (tabs, newlines and
NUL included). The command runs from the Git
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
On the mutant, every expected id exiting nonzero means CAUGHT (subject to
`expect_message`); any exiting 0 means SURVIVED. A spawn failure, exit 126 (not executable, or the command refused
to run), exit 127 (not found), death
by signal, interruption, or timeout is ERROR, **never red**, on either tree,
except an explicitly reasoned `signal_is_catch` on the mutant. For command rows,
`expect_message` matches that id's combined stdout/stderr, not another invocation.
The row's `timeout_s` applies separately to each baseline and mutant invocation.
There is no normal build phase; command test timeouts report `timed_out_phase: "test"`.
Declared prerequisites have their own build deadlines and timing fields.

Normally only the expected ids run: `collateral` is `{ "count": 0, "targets": [] }`
and `breadth_observed` is false. Without the optional JUnit fields below,
`run --broad` still runs just those named commands; it cannot discover collateral.
`explore` continues to refuse command rows: use `prove` to append a named proof.
`check` validates their fields, files, and anchors, but has no test-list protocol
to validate names; the fresh baseline during replay verifies the invocation.

### JUnit breadth audits (0.9.0)

Set **all three** optional fields on a command row:

- `broad_command = [argv…]`: a package-wide command, without `{test}` or a shell.
  Under `run --broad` it runs on the clean tree first, then on each mutant
  **after** the row's own expected tests have been graded. Normal runs and `prove`
  do not run it. `timeout_s` bounds each
  invocation separately, including any compile the external runner performs;
  there is no extra command-row build phase. A timeout is ERROR with phase `test`.
- `broad_report = "reports/junit.xml"`: a repository-relative path, deleted before
  every audit invocation. Absolute paths, `..` and symlink components are refused,
  including a symlink to an absent destination. The command must create a fresh
  report; create any necessary parent directory in your runner first.
- `broad_id = "{name}"`: maps XML-decoded testcase attributes to exact catalogue
  ids. `{classname}.{name}`, `{classname}/{name}` and literal separators are also
  supported. Only `{classname}` and `{name}` placeholders are allowed, and
  `{name}` is required. Values are never trimmed, split or recursively expanded.

A report must be UTF-8 JUnit XML rooted at `<testsuites>` or `<testsuite>`; nested
suites, XML entities and CDATA are supported. Direct `<failure>` and `<error>`
children mark red cases; skipped cases are not green. Missing, garbage, truncated
or zero-testcase reports are **ERROR naming the report**, never evidence of no
collateral. Duplicate mapped ids or absent expected ids also fail closed: choose
a template that uniquely matches your catalogue. DOCTYPE declarations are refused.
A nonzero command exit with no failed/errored cases is ERROR, as are spawn errors,
126/127, signals and interruption. Expected-test grading remains the named
command's verdict; the broad report cannot turn a named survivor into a catch.

Successful audits set `breadth_observed: true` and include collateral reds and
their JUnit failure/error messages in `red` and `failures`. As for Cargo, unlisted
failures in the expected tests' own target are reported as collateral but remain
CAUGHT. Failures in other targets grade **CAUGHT_BROADLY**. For command rows the
target is exactly the JUnit **`classname`**, not the file or `<testsuite name>`.
An empty classname is valid (Bun's top-level tests use it). A reviewed
`hub = "reason of at least 20 characters"` plus `hub_targets = ["OtherClass"]`
permits those cross-class failures as **HUB**; new classnames outside that list
remain CAUGHT_BROADLY. A successful audit need not have collateral.

Clean JUnit baselines are shared once per session by rows with identical
`broad_command`, `broad_report`, `broad_id` and `timeout_s` selections; their
timing is attributed once. Named-command baselines remain per row. Already-red
JUnit ids are recorded by classname in `baseline_red` and their messages in
`baseline_failures`, and excluded from mutant collateral just like Cargo's
baseline failures. The mutant's `failures` map still retains their diagnostics.
Every expected id must be present and green in the broad baseline. Missing,
unparsable or zero-case baseline reports make all rows sharing that selection
ERROR; a failed baseline never permits applying those rows' mutants.

#### Runner mappings and example rows

Inspect a clean report from your installed tool version before selecting ids:
JUnit naming is not the same as every runner's native selector syntax.

**Bun** writes JUnit at the end of `bun test --reporter=junit
--reporter-outfile=…` while retaining console output. A testcase's `name` is the
leaf test description, so **`{name}`** matches leaf catalogue ids, including
spaces; `--test-name-pattern` selects by a regex, not an exact full display id.
Choose unique leaf names and escape regex metacharacters in your named runner
when needed. For tests under one `describe`, **`{classname} > {name}`** maps to
the console's descriptive name if your named runner accepts that full id.
Bun 1.4.2 reports an empty classname for top-level tests and reverses the describe
order in nested classnames (`inner > outer`); that nested value cannot be reversed
by a template. Use unique leaf ids or normalize the XML in a runner when full
console ids are needed. Classnames group describes, **not test files**: top-level
tests in different files share the empty target.

```toml
[[control]]
id = "bun-isolates-routes"
guards = "cached routes isolate sessions"
file = "src/routes.ts"
old = "cache.get(sessionId)"
new = "cache.get(defaultSessionId)"
test_file = "tests/routes.test.ts"
runner = "command"
command = ["bun", "test", "tests/routes.test.ts", "--test-name-pattern", "{test}"]
test_count_pattern = "Ran {count} test"
expect_red = ["cached routes isolate sessions"]
broad_command = ["bun", "test", "--reporter=junit", "--reporter-outfile=reports/bun.xml"]
broad_report = "reports/bun.xml"
broad_id = "{name}"
```

**pytest `--junitxml`** emits dotted module/class `classname` and a method/function
`name`, preserving parameter suffixes in `name`. **`{classname}.{name}`** matches
dotted Python ids, including unittest-compatible tests run as shown below.
Pytest's native node ids use paths and `::` instead; a template cannot convert a
dotted classname back to a file path. For pytest-only tests, use a named runner
that resolves these dotted ids to collected node ids and reports a verified
executed count. Do not confuse collection counts with execution counts. Avoid
`--junitprefix` unless its prefix is also part of the catalogue ids.

```toml
[[control]]
id = "python-isolates-routes"
guards = "cached routes isolate sessions"
file = "app/routes.py"
old = "cache[session_id]"
new = "cache[default_session_id]"
test_file = "tests/test_routes.py"
runner = "command"
command = ["python3", "-m", "unittest", "{test}"]
test_count_pattern = "Ran {count} test"
expect_red = ["tests.test_routes.TestRoutes.test_isolated"]
broad_command = ["python3", "-m", "pytest", "--junitxml=reports/pytest.xml"]
broad_report = "reports/pytest.xml"
broad_id = "{classname}.{name}"
```

**SwiftPM XCTest** `swift test --parallel --xunit-output …` writes the XCTest
case type as `classname` (typically `Module.Class`) and method as `name`.
**`{classname}/{name}`** matches XCTest's filter/specifier form;
`{classname}.{name}` also works if that is your named runner's id convention.
The example's `swift-one.py` must invoke `swift test --filter ID`, preserve its
exit status and print exactly one `Executed N selected tests` line based on the
actual execution count. A wrapper is needed because XCTest repeats native
"Executed N tests" summaries and the count protocol refuses ambiguous counts.
Use this report for XCTest; Swift Testing's separate reports and ids vary with
the toolchain and may need conversion/merging before declaring a package audit.

```toml
[[control]]
id = "swift-isolates-routes"
guards = "cached routes isolate sessions"
file = "Sources/Routes/Cache.swift"
old = "cache[sessionID]"
new = "cache[defaultSessionID]"
test_file = "Tests/RoutesTests/CacheTests.swift"
runner = "command"
command = ["python3", "scripts/swift-one.py", "{test}"]
test_count_pattern = "Executed {count} selected tests"
expect_red = ["RoutesTests.CacheTests/testIsolated"]
broad_command = ["swift", "test", "--parallel", "--xunit-output", "reports/swift.xml"]
broad_report = "reports/swift.xml"
broad_id = "{classname}/{name}"
```

**Xcode** `xcodebuild test -resultBundlePath …` produces an `.xcresult` bundle,
not JUnit. `xcresulttool get test-results tests --path …` (modern Xcode) exposes
JSON; it does **not** directly export JUnit XML. Use one argv-invoked converter
that runs xcodebuild, extracts those results with xcresulttool, writes JUnit and
returns the build/test exit status. For a converter writing
`classname="RoutesTests.CacheTests" name="testIsolated"`, use
**`{classname}/{name}`**, as in the Swift row. Replace its `broad_command` with
`["python3", "scripts/xcode-junit.py", "--scheme", "Routes", "--report",
"reports/xcode.xml"]` and `broad_report` with `"reports/xcode.xml"`; the converter
must cover the entire intended scheme. Converter naming is not standardized:
inspect its output and make the named runner use identical ids.

Sources: [Bun reporters](https://bun.com/docs/test/reporters),
[pytest JUnit naming](https://github.com/pytest-dev/pytest/blob/main/src/_pytest/junitxml.py)
(`record_testreport`/`mangle_test_address`), and
[SwiftPM XUnitGenerator](https://github.com/swiftlang/swift-package-manager/blob/main/Sources/Commands/SwiftTestCommand.swift),
plus the [xcresulttool command reference](https://keith.github.io/xcode-man-pages/xcresulttool.1.html).

### Output-equality command rows (0.7.0)

Use `catch_on = "output_differs"` for a guard whose evidence is stable stdout
rather than a failing named test, such as a search-quality benchmark:

```toml
[[control]]
id = "search-quality-is-stable"
guards = "the ranked search benchmark output is unchanged"
file = "src/search.rs"
old = "score.with_quality_boost()"
new = "score"
test_file = "scripts/search-quality.py"
runner = "command"
command = ["python3", "scripts/search-quality.py", "--fixed-seed", "42"]
catch_on = "output_differs"
output_normalize = ['(?m)^elapsed_ms=\d+\r?\n', 'timestamp=[^\r\n]*']
expect_red = []
```

These rows run the argv once per invocation, with **no `{test}` placeholder**,
no `test_count_pattern`, and `expect_red = []`. Optional `output_normalize`
contains Rust regexes deleted, in order, from stdout before comparison; invalid
regexes fail `check`. Use normalization only for genuinely irrelevant data;
deleting the measured scores would make the proof vacuous. Stderr is retained
for diagnostics but never compared.

Both clean baseline invocations must exit 0 and produce identical normalized
stdout. Otherwise the row is ERROR (`baseline output is not deterministic`,
with the first differing line) and the mutant is skipped. Mutant stdout that
differs is CAUGHT even on exit 0; identical stdout survives. An ordinary nonzero
mutant exit still catches under the existing command exit rules; spawn errors,
exit 126/127, signals (even with `signal_is_catch`), interruptions and timeouts
are ERROR. `expect_message` does not apply and `check` refuses that combination.
Each invocation has its own `timeout_s`. `prove` supports `--catch-on output_differs`
and repeated `--output-normalize REGEX`, without `--expect-red` or
`--test-count-pattern`; put `--command` last as usual.

To prove and append a named-test row, put `--command` **last**. It consumes all
remaining argv elements, including options belonging to the test runner. The
runner defaults to `command` when `--command` is present (otherwise `cargo`):

```sh
ckdev-mutate prove --id rig-rejects-empty --guards 'the rig rejects an empty flow' \
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
ckdev-mutate check
ckdev-mutate --catalogue other.toml run --all --report mutations.json
ckdev-mutate run --diff origin/master --report mutations.json
ckdev-mutate run --all --shard 1/4
ckdev-mutate run --all --broad --report nightly-audit.json
ckdev-mutate run --only flow-list-agent-sees-only-own
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
ckdev-mutate prove --id rejects-zero --guards 'zero is rejected' \
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
ckdev-mutate explore --package my-package --file src/lib.rs \
  --old 'value > 0' --new 'value >= 0' --report explore.json
ckdev-mutate explore --package my-package --workspace --edits edits.toml
ckdev-mutate explore --package my-package --file src/lib.rs \
  --old 'value > 0' --new 'value >= 0' \
  --append --id rejects-zero --guards 'zero is rejected' --test-file src/lib.rs
ckdev-mutate explore --package my-package --file src/lib.rs \
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
| RED_FOR_ANOTHER_REASON | An expected test went red, but its own failure output did not match `expect_message`. Not a catch; reports the first lines of that failure. |
| NO_TESTS_RAN | Passed plus failed is zero, or an expected full name did not run. |
| ANCHOR_MISSING | The replacement count was zero or greater than one. |
| DID_NOT_COMPILE | The separate build command exited nonzero. |
| TIMED_OUT | The build exceeded `build_timeout_s`, or the test run exceeded `timeout_s`. `timed_out_phase` is `"build"` or `"test"`, and the reason names the deadline. |
| EQUIVALENT | A replayed equivalent row survived. Recorded with its reason and guard; succeeds separately, not a catch. |
| EQUIVALENT_CAUGHT | A newly red test or output comparison caught a claimed equivalent. Fails; turn the row into a normal proof. |
| UNREACHABLE | Explicitly recorded with a reason explaining why no production caller exists. Listed separately, never counted as caught. |
| SKIPPED_PLATFORM | Host target_os is not in platforms. Counted separately, not executed. |
| DESK_ONLY | Explicit nonempty reason that this proof needs a real desktop; never automated or counted as caught. |
| ERROR | An expected test red at baseline, nondeterministic baseline output, signal death without a reasoned `signal_is_catch`, invalid/incomplete runner output (including a Cargo binary without a summary or zero-test command rows), prerequisite failure, interruption, or restoration/lockfile integrity error. |

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

### Per-test failure output (0.7.1)

Every row includes `failures` and `baseline_failures`, maps from full test name
to that test's own failure output. `failures` includes every red in the mutant
run: expected, collateral, and tests still red from baseline (even though those
baseline reds are excluded from grading and `red`). `baseline_failures` holds
the clean-tree failures separately, including expected baseline reds that stop
replay. Repeated names use the same qualified binary identities as `red`.
Green tests are not included; runs without failures have empty maps.

Cargo entries come from each `---- name stdout ----` block. Nextest entries
contain that test's stdout and stderr, not the run's aggregate output. Command
test rows use each test id's combined output. Each entry is capped at **16 KiB**,
keeping the first 4 KiB and approximately the last 12 KiB, with a
`[… N bytes elided …]` marker counted within the cap. Cuts respect UTF-8
boundaries. `test_tail` remains the last 8,000 characters of the whole run;
failure maps retain early panics even when hundreds of PASS lines follow.

WRONG_TEST and RED_FOR_ANOTHER_REASON console rows print the first six lines of
each unexpected red's failure underneath the row. Unexpected reds include
collateral tests and expected tests whose output mismatches `expect_message`.

Every row includes `breadth_observed`: true only when an opt-in broad audit
finished and its complete per-test results were parsed. Normal replays, `prove`,
`explore`, explicit dispositions, and incomplete audits report false.

Every row also includes `collateral: { "count": N, "targets": [...] }`. For proof
replays, it counts every newly red test not named in `expect_red` and lists their sorted,
de-duplicated targets, including same-target failures. With no collateral it is
`{ "count": 0, "targets": [] }`. Explore discovers its expected names from all
red tests, so its collateral is empty. Cargo target identities are stable executable
names from `Running ... (<binary>)` headers, without the hash suffix or Windows `.exe`;
doctests use `doc:<crate>`. Human nextest uses its binary token; JSON nextest uses
the `crate::binary` prefix. Attribution is retained by the same per-test parser.
Console output includes collateral on catches with extra reds, a warning summary
for CAUGHT_BROADLY, and separate lists and counts of UNREACHABLE and HUB rows.

These formats have no explicit version field: 0.7 requires `equivalent_guard`
alongside existing `equivalent` claims, adds `catch_on`/`output_normalize` for
command output comparisons, EQUIVALENT_CAUGHT, per-target `baseline_red`, and
separate baseline build/test timing. Existing equivalent rows must supply their
guard and are no longer skipped. Other existing rows remain readable.
0.6 adds optional `expect_message`
and `signal_is_catch` row fields and the RED_FOR_ANOTHER_REASON outcome; existing
rows remain readable. 0.5 retained the unversioned TOML
`[[control]]` schema and JSON array, adding `platforms`, `desk_only`, root `prebuild`,
the required command-row `test_count_pattern`, skip outcomes and preparation timing.
Cargo/nextest catalogues remain readable unchanged; existing command catalogues
must add an executed-count pattern. Older runners rejecting new fields is expected. Since 0.4,
`collateral.targets` uses stable Cargo names rather than hash-suffixed executables.

For Cargo/nextest, builds are separate: Cargo uses `cargo test -p PACKAGE TARGET --no-run --locked`;
nextest uses `cargo nextest run ... --no-run --locked`. Only build exit status
determines DID_NOT_COMPILE. Test exit status never determines Cargo/nextest catches.
Signal deaths veto catches unless explicitly accepted as described above.
Command rows instead use the baseline-gated exit-status rules above.
Cargo results use per-test `test NAME ... ok|FAILED|ignored` lines and require
each binary's summary counts to agree; each summary count is located by its following word,
not a column. All failures (`0 passed; 2 failed`) are a valid catch, not an empty
run. `test result:` is never a test named `result:`. Both runners disable
fail-fast so later binaries are not omitted.

Cargo captures each `---- NAME stdout ----` failure block; nextest JSON captures
the test event's `stdout`. These are the per-test sources for `expect_message`.

For nextest, the runner probes `run --help` for `libtest-json` and uses that
machine-readable event stream when available (enabling its experimental feature).
Nextest's `crate::binary$` prefix is removed to retain the exact libtest path.
It checks terminal suite counts and disables retries to avoid conflating failed
attempts with final results. Older nextest versions use color-free `PASS`/`FAIL`
status lines with all statuses enabled (and their failure-output blocks);
unknown formats cannot establish a catch. Nextest 0.9.138 emits an abort as
`{"type":"test","event":"failed",...,"stdout":""}` in libtest-json, **not**
as a dedicated JSON signal event. Its simultaneous human status supplies
`SIGABRT [ ... ] ... BINARY TEST`, matched to the JSON binary/test identity.
The captured Cargo/nextest abort runs are checked in as
`tests/fixture/cargo-abort.txt` (only the temporary root is normalized) and
`tests/fixture/nextest-abort.txt`. Cargo 1.99.0 emitted
`(signal: 6, SIGABRT: process abort signal)` without a summary; nextest emitted
`SIGABRT [   0.010s] (5/6) mutation-fixture tests::waits`.
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
      - run: ckdev-mutate check
      - if: github.event_name == 'pull_request'
        run: ckdev-mutate run --diff '${{ github.event.pull_request.base.sha }}' --shard ${{ matrix.shard }}/4 --report mutations.json
      - if: github.event_name == 'schedule'
        run: ckdev-mutate run --all --broad --shard ${{ matrix.shard }}/4 --report mutations.json
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
Windows Ctrl-C handling; Unix-only `rustix` with its `process` feature supplies safe process-group and test signal APIs. The library, binary and integration tests forbid unsafe code. `regex` matches the optional per-test assertion pattern. `tempfile` is
only a dev dependency, isolating real Git/Cargo fixture repos for tests.
