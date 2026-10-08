# cortexkit-test-support

Portable CortexKit test helpers. **Use as a dev-dependency only**, not in production.
Licensed under MIT. This crate contains no application-store dependencies.

```toml
[dev-dependencies]
cortexkit-test-support = "0.1.0"
```

## Scratch fixtures

`ScratchDir::new("label")` creates a unique directory beneath
`temp_dir()/cortexkit-tests/k<PID>-<kernel-start-identity>`. Normal drop removes
it. `keep()` and panic unwinding preserve it and print its path. Failed tests
therefore retain their evidence. Unsupported process-identity platforms use an
unknown start marker; such roots are never reaped on uncertain evidence.

Initialization sweeps process roots only when `subc_os::process_identity::liveness`
positively reports Dead. Live owners, unknown or incompatible identities,
malformed process-root names and legacy encodings survive. Non-process fixtures
retain the 24-hour age rule; directories with young descendants and symlinks
survive. The `stable-bin` executable directory is exempt from the entire sweep.
`wait_until_gone` uses the same liveness API;
unknown is never considered gone (including unsupported platforms).

## Executables and spawn guard

`ckdev_binary(built)` publishes a content-addressed, read-only **copy**, never a
hard link. `ckdev_file_name` maps `ck-example` to `ckdev-example`, `ck` to
`ckdev-ck`, and preserves `.exe`. Windows staging ensures an `.exe` suffix.
`stage_test_binary` is the narrower compatibility helper: only `ck-*` names
are staged; other names pass through.

Unix staging uses a per-user root under `/tmp`, never `$TMPDIR`; Windows uses
the system temp directory. A `cp` child writes Unix staging files so the test
process holds no writable descriptor to an executable. Windows uses `fs::copy`.
A private staging directory is atomically renamed into its content address;
concurrent requests reuse one published file. Digests are checked before use,
including reuse. Publications older than three days are pruned on a new build.
Do not modify a publication. A source rebuild receives a different path.

```no_run
use cortexkit_test_support::{ckdev_binary, dev_command, checked_output};
let placed = ckdev_binary("target/debug/ck-example");
let output = checked_output(dev_command(placed).arg("--version"))?;
# Ok::<(), std::io::Error>(())
```

`dev_command` refuses production names (`ck` and `ck-*`, case-insensitively),
except Cargo's hashed test harnesses. Its return type is a standard `Command`:
use `checked_output` rather than plain `.output()` when a failed child should
be an error naming its exit code or Unix signal number and name. Explicit
production-name exemptions take a caller-owned exact `(test, stem)` table and
verify the calling test thread as well.

Consumers should call `assert_test_binary_spawns(&[test_directory, ...])` from
one test. This recursive lexical scan ignores comments and catches direct
production artifact spawns, adjacent unwrapped spawns, and simple raw-path
bindings. It is not a Rust data-flow analyzer: also use the runtime name guard.
Missing directories and an empty Rust scan fail instead of silently passing.

On Unix, `stable_executable` and `stable_executable_dir` provide reusable
content-addressed fake executable files in the exempt stable-bin directory.

## Fenced daemon lifecycle

`TestDaemonCommand` refuses programs not named `ckdev-*` at spawn and refuses
`kill_on_drop(false)`. It clears inherited credentials and module identity,
allows only tool/locale variables, and redirects HOME, all XDG roots, and TMPDIR
into scratch. Call `.fenced_root("EXAMPLE_STORAGE_ROOT", "storage")` to add a
caller-selected directory variable; fixed roots never name a consuming repository.
These roots, including caller additions, are reapplied immediately
before spawning: `.env` and `.env_remove` cannot bypass the daemon fence.

Unix consumers require **`python3` on PATH** for lifecycle supervision, plus `ps`
and `cp`. If `python3` cannot be spawned, the error names that requirement.
A pipe watcher owns the daemon's process group, terminates descendants on EOF
(even when the harness is killed), and reaps the daemon on normal exit or
panic. Drop is synchronous and bounded. Windows uses Tokio child kill-on-drop;
it does not promise Unix process-group or orphan-descendant supervision.
`SubcDaemonBinary` specifically builds the platform daemon `ck-subc` from the
`subconscious` sibling checkout. Its `build` and `try_build` methods take the
caller-selected dependency checkout names to include in the cache key.
`SubcDaemonBinary::command` returns only the guarded builder; its path is private.
`fenced_command` provides the ambient environment fence for other tools, but
as a standard command its roots can subsequently be overridden by the caller.
`fenced_command_with_roots(program, scratch, &[("EXAMPLE_STORAGE_ROOT", "storage")])`
adds caller-selected roots to that standard command. Extra subdirectories must be
plain names, built-in variables cannot be replaced, and symlink roots are refused.

`sign_test_binary(path, identity)` signs best-effort on macOS. An explicit identity
wins over `CORTEXKIT_TEST_SIGNING_IDENTITY`; no configured identity means no
signing. Callers can explicitly request ad-hoc signing with `Some(OsStr::new("-"))`.
Ad-hoc signing uses no certificate or keychain signing identity. Already-valid
signatures are reused. Sign built binaries **before** staging,
not read-only publications. `warm_exec_fenced` runs `--version` through the fence
and reports failures without making warm-up mandatory.

## Sibling builds and privileged tests

Sibling cache helpers key builds by clean/dirty source content and caller-selected
dependency revisions. `sibling_build_revision(repo, revision, dependency_checkouts)`
and `build_sibling_binary(repo, sibling, bin, dependency_checkouts)` take the
adjacent dependency checkout names explicitly; pass `&[]` for no dependencies.
An OS file lock serializes building and publication across processes;
the four most recently used revisions are retained. `CachedSiblingBinary`
separately exposes the published build target and the staged executable path.
`sibling_cargo_build` always uses `--locked`, so a stale sibling lock fails
without rewriting it. The shared cache lives in the user's
`.cache/cortexkit-tests/sibling-target` directory.

`sibling_checkout` resolves an adjacent repository, including from linked
worktrees. Workspace resolution is runtime-based, not this dependency's source
location: `git rev-parse --show-toplevel` in the current directory, or explicit
`CORTEXKIT_TEST_WORKSPACE_ROOT`. `sibling_target_dir` uses that workspace's
`target/sibling-target`, not the sibling's active target directory.

`privileged_tests_enabled(test, needs)` opts in only when
`CORTEXKIT_PRIVILEGED_TESTS=1`, otherwise prints the skip reason. Callers return
early when it is false. No privileged operations are performed by this crate.
