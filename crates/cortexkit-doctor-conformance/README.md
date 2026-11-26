# cortexkit-doctor-conformance

A Unix kit for the [module doctor contract](../cortexkit-doctor/CONTRACT.md).
Implement `Adapter` and pass it to `run(&mut impl Adapter) -> Vec<RuleResult>`.
Each `RuleResult` has a `name` and an `Outcome`: `Passed`, `Failed { reason }`,
or `Skipped { reason }`. Reasons are fixed codes, not child output or adapter
error messages. A skip is not a pass.

The adapter supplies `program(&self) -> &Path` and
`seed(&mut self, data: &Path, config: &Path) -> io::Result<()>`. Seed the two
empty directories with representative data and settings; do not replace the
directories themselves. The kit creates fresh homes for each check or fix rule
and fresh homes for the shared manifest invocation.
`output.valid` validates the quick check document, and `check.read_only` compares
both home trees before and after a separate quick check. `check.full` validates
the full check document and runs the same before-and-after comparison. The
comparison includes relative paths, entry types, exact symlink targets, file
sizes, and SHA-256 of regular files. It does not follow symlinks. The two
manifest rules inspect the same manifest invocation: `manifest.digest` is
skipped if `manifest.valid` fails. A stale templates digest fails only
`manifest.digest`.

The adapter has optional `fixable_id()`, `full_fixable_id()`, and `needs_stop_id()`
getters (returning `Option<&FindingId>`) and `liveness()` (returning `Option<&Liveness>`).
`Liveness::PidFile(PathBuf)` and `Liveness::FlockFile(PathBuf)` specify a path
relative to the data home with no `..`. Use only a module test build for fix
checks. Its test-only switch must check `CORTEXKIT_DOCTOR_CONFORMANCE=1` before
reading `CORTEXKIT_DOCTOR_TEST_PRESENCE`; when the switch is `no_presence`,
inject a presence function returning `NoPresence`. Leave the switch out of
release builds. Declare that behavior with `non_prompting() -> bool`; it
defaults to `false`. The runner supplies the switches in a new session for
kit fixes only. Ordinary check, manifest and fix invocations do not get them.

When `non_prompting()` returns `true`, `fix.stale_digest` and `fix.no_terminal`
use the fixable id to check that a changed digest and a missing terminal each
cause the expected refusal. `fix.module_running` uses the needs-stop id and
holds the liveness signal while checking for a `module_running` refusal. It
skips if the stand-in makes that finding disappear or changes its digest. A
configured id must appear in the baseline check with a plan, and the needs-stop
id must have `needs_stop` set.

`fix.full_recheck` uses `full_fixable_id()` and skips if no id is supplied.
Seed a finding with a plan that appears only in the full check. The rule passes
its digest to `doctor fix --full` and expects `refused`/`no_terminal`, showing
that the helper found the full-only finding before attempting confirmation.

`fix.no_change_on_refusal` compares both homes across every attempted fix,
regardless of the outcome. For the running-module rule, the stand-in is already
active at the first snapshot and stays active through the second. If no fix
runs, the comparison rule skips. Without the non-prompting declaration all
fix rules skip and the kit never invokes `doctor fix`.

The kit uses scratch homes and isolates process groups; it is not a filesystem
sandbox. Only run programs you trust. A before/after comparison does not detect
changes that were fully undone before the child exited.

The `test-fixture` feature builds the kit's fixture executable for its own
integration tests. Adopters do not need that feature.
