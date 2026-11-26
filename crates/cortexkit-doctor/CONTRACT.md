# `doctor/v1` — module diagnostics contract

A module diagnoses its own on-disk state. `ck doctor` finds the module program
in the daemon configuration and runs it directly, without a daemon connection.
The module can apply an approved fix, and `ck` can build a sanitized report
for public review. Module callbacks own any store access; the doctor library
does not read stores or vaults, or use the network.

Rules marked **[pinned]** are binding requirements of `doctor/v1`. The module
helper, runner helper, and conformance kit are Unix-only. The wire types,
digests, sanitizer, report builder, and separate `cortexkit-log-manifest`
generator are platform-independent. `cortexkit-log` is unchanged and does not
emit template ids.

## 1. Commands and documents

- [pinned] `<program> doctor check --json` emits one `CheckOutput` document:
  `{"schema":"ck.doctor-check/v1","findings":[Finding]}`. A check changes
  nothing, even when the module is running. It opens stores read-only, never
  takes a lease or lock the running module could need, never migrates a store,
  and never creates or writes a file, including SQLite `-wal` and `-shm`
  files. If a store needs the running module's lock to be read, the check
  returns the `info` finding `store.in_use` instead of waiting. A check reports
  on-disk state, not live process state, and must finish within 10 seconds.
  For a SQLite store, the check reads `user_version` from header bytes 60–63
  (zero-based) and opens SQLite only with `immutable=1` when neither `-wal` nor
  `-shm` exists; otherwise it returns `store.in_use` because even a `mode=ro`
  reader in WAL mode can write its read mark to `-shm` while the module runs.
  The runner clears the environment and sets only `XDG_DATA_HOME` and
  `XDG_CONFIG_HOME`, leaving `HOME` unset; checks must not look for stores
  outside those roots, including a module's old storage location.
- [pinned] A `Finding` has `id`, `severity`, `summary`, `evidence`, and `fix`.
  `FindingId` accepts `[a-z0-9_.-]{1,96}` subject to the secret check. An id is
  stable within the module across versions and unique in a `CheckOutput`.
  `Severity` is `error`, `warning`, or `info`. A summary contains 1 to 200
  Unicode scalar values and no characters in U+0000–U+001F or U+007F–U+009F.
  One plain sentence is guidance, not an additional validation rule.
- [pinned] The evidence member of `Finding` is required. It contains 0 to 16 `Evidence`
  entries, each an object with `name`, `kind`, and `value`. Names are unique
  within the list and match `[a-z0-9_]{1,64}` subject to the secret check.
- [pinned] A `Fix` has `class`, a boolean `needs_stop`, and an optional `plan`.
  `FixClass` is `none`, `safe`, or `risky`. A plan is absent for `none` and
  required for `safe` or `risky`. A `Plan` carries `steps`, `digest`, and
  `timeout_ms`. There are 1 to 16 steps, each subject to the summary rules.
  The timeout is an integer from 1,000 to 600,000 milliseconds.
- [pinned] `plan_digest` hashes the RFC 8785 canonical JSON
  of `{id, steps, needs_stop, class, timeout_ms}` with
  SHA-256. `is_digest` accepts only `sha256:` followed by 64 lowercase hex
  digits. `CheckOutput::decode` recomputes the digest, so even a timeout-only
  change invalidates it. The digest binds the described operation, not its
  evidence. The fix flow binds the confirmed finding.
- [pinned] `<program> doctor fix <id> --digest <digest>` emits one `FixOutput`
  document: `{"schema":"ck.doctor-fix/v1","result", "code"?, "evidence"?}`.
  `FixResult` is `fixed`, `refused`, or `failed`. A code matches
  `[a-z0-9_.-]{1,64}` subject to the secret check. It is absent for `fixed`
  and required for `refused` and `failed`. Evidence follows the finding rules;
  the module helper omits the member when the list is empty.
- [pinned] The module helper writes `check` and `fix` documents with one final
  LF, and writes the embedded `Manifest` bytes unchanged for
  `<program> doctor manifest`. It exits 0 for a document, including `refused`
  or `failed`. It exits 2 with empty stdout for a usage error: a missing
  `--json`, malformed `--digest`, invalid `FindingId` (including a secret
  pattern), or any unknown or extra argument.
- [pinned] At entry the module helper decodes the embedded `Manifest`, takes
  its `ModuleId`, and checks that its bytes equal `Manifest::to_canonical_bytes`.
  A bad or noncanonical embedded manifest exits 1 with empty stdout. The
  adopter embeds the generator's file with `include_str!`. `manifest` opens no
  store, daemon connection, or network.
- [pinned] Dispatch `doctor` before any normal module initialization,
  including setting up logging or opening stores. At entry the module helper
  duplicates fd 1 close-on-exec (`F_DUPFD_CLOEXEC`),
  redirects fd 1 to `/dev/null`, and never restores it. It writes the document
  on the saved descriptor. Callback output, even if buffered until exit, does
  not enter the document. A terminal prompt goes only to the terminal.

## 2. Validation order and evidence kinds

- [pinned] `CheckOutput::decode`, `FixOutput::decode`, and `Manifest::decode`
  return only the first applicable `DecodeError` stage code, never input text.
  The order is:
  1. `too_large`: input exceeds `MAX_OUTPUT_BYTES` (1,048,576 bytes), counting
     any trailing whitespace. The runner enforces this limit while reading.
  2. `not_json`: input does not begin with one JSON value.
  3. `extra_output`: bytes other than RFC 8259 JSON whitespace (space, tab, CR,
     LF) follow that value.
  4. `shape`: the wire shape, member names, grammars, ranges, uniqueness, order,
     kinds, or structural-string secret checks fail. An unknown or repeated
     member at any depth is invalid. JSON `null` is never valid, including
     in place of an optional member. Integer kinds and members require an
     integer token; `10.0`, `1e3`, and `-0` are invalid. Malformed digests and
     template ids outside `[0-9a-f]{12}` fail here, not at a digest stage.
  5. `repository` (`DecodeError::Repository`): a manifest repository string
     is not an accepted GitHub repository URL (see §5). Missing or non-string
     repository members still fail at `shape`.
  6. `plan_digest`: a well-formed plan digest does not match `plan_digest`.
  7. `template_id`: a well-formed id does not match `template_id`.
  8. `templates_digest`: a well-formed digest does not match `templates_digest`.
- [pinned] A `FindingId`, evidence or manifest field name, fix-result code,
  or `ModuleId` that matches `redact_secrets` is invalid (`shape`). In a
  decoded manifest, a matching repository owner or repo segment is invalid
  (`repository`). The `sk-` credential pattern starts at a word boundary
  (see §7). `task-queue_backlog` is a valid finding id; `sk-example` and an id
  containing a 32-hex run are not. `FindingId` and `ModuleId` enforce these
  checks wherever those types are constructed.
- [pinned] A wrong-kind `EvidenceValue` invalidates a whole document, not just
  that entry. `Evidence`, `EvidenceKind`, `EvidenceValue`, and `PathCategory`
  implement the evidence shapes below. The report builder uses the same kinds
  for admitted log fields, but drops an invalid field value and counts it in
  `dropped_values` instead of rejecting the report. Numeric and boolean log
  values parse the decoded field text as JSON, including the integer-token rule.

| Kind | Accepted value | Report value |
| --- | --- | --- |
| `count`, `bytes` | Integer from 0 to 2^53 − 1 | Value as given |
| `duration_ms` | Finite non-negative number | Value as given |
| `ratio` | Finite number | Value as given |
| `bool` | Boolean | Value as given |
| `code` | `[a-z0-9_.-]{1,64}` | String after `redact_secrets` |
| `version` | `[0-9A-Za-z.+-]{1,64}` | String after `redact_secrets` |
| `timestamp_ms` | Integer from 0 to 2^53 − 1 | Relative age |
| `path` | `{category, path}` with a path of 1 to 4096 UTF-8 bytes and category `data`, `config`, `project`, `temp`, or `other` | `<category>/#<alias>` |
| `ref` | Opaque string of 1 to 256 UTF-8 bytes | `<prefix>#<alias>` |

## 3. Applying a fix (Unix module helper)

Use `cortexkit_doctor::module::run` in the module's CLI and return its
`ExitCode` from `main`. The argument slice excludes `doctor` itself:

```rust,ignore
fn run(
    args: &[impl AsRef<OsStr>],
    check: impl FnMut() -> Result<Vec<Finding>, CheckError>,
    preflight: impl FnMut(&Finding) -> Result<(), PreflightError>,
    fix: impl FnOnce(Finding) -> FixOutcome,
    manifest: &str,
) -> ExitCode
```

`PreflightError` is `ModuleRunning`, `FilesInUse`, or `Failed`. Construct
the callback result with `FixOutcome::fixed(Vec<Evidence>)` or
`FixOutcome::failed(code: impl Into<String>, Vec<Evidence>)`, which return
`Result<FixOutcome, DecodeError>`, or with
`FixOutcome::refused(FixRefusal) -> FixOutcome`. `FixRefusal` is
`ModuleRunning`, `FilesInUse`, or `PlanChanged`.

- [pinned] The check callback returns findings or an error. Preflight gets
  a `&Finding` and returns success, `module_running`, `files_in_use`, or an
  error. The fix callback gets the confirmed `Finding`. It returns `fixed`
  with evidence, `refused` with one of `module_running`, `files_in_use`, or
  `plan_changed`, or `failed` with a code and evidence. The callback result
  type admits no other refusal. A callback's own failure code is the module's.
- [pinned] On `check`, a callback error, invalid output (including duplicate
  finding ids), or panic exits 1 with empty stdout. The runner reports
  `crashed` with `exit_code: 1`. On `fix`, a check error or invalid result
  gives `failed`/`check_failed`. A preflight error other than a refusal gives
  `failed`/`preflight_failed`. A panic in any callback gives
  `failed`/`callback_panicked`; the helper catches unwinding panics with
  `catch_unwind`. No refusal or failure calls a later callback.
- [pinned] The module helper measures time from its own entry and follows
  these steps in order:
  1. Run the check callback. Refuse `unknown_finding` if the id is absent.
     Refuse `plan_changed` if the plan has no digest matching `--digest`,
     including a class-`none` finding.
  2. Run preflight with the verified `Finding`. Refuse `module_running` when
     `needs_stop` is true and the module is alive; refuse `files_in_use`
     when another process holds a file the fix changes. Preflight always runs
     before asking the person to confirm.
  3. If more than 10 seconds have elapsed since entry, refuse `out_of_time`
     without touching either confirmation step. Otherwise confirm as below.
  4. Run the check callback again. Refuse `unknown_finding` if the finding
     disappeared, or `plan_changed` if its digest or summary changed. Run
     preflight again with the original, confirmed `Finding`.
  5. If more than 140 seconds have elapsed since entry, refuse `out_of_time`.
  6. Call the fix callback with the original confirmed `Finding` and its plan.
     Changes to evidence alone on the second check do not replace it. The
     callback applies that plan and does not resolve the id again.
- [pinned] Confirmation goes through the OS so a person can approve the fix
  even when the daemon is down. The helper never connects to the daemon, and
  launching the program from a shell is not approval. The first confirmation
  step calls `subc_presence::prompt(text, publish)` from `subc-presence` 0.2.x
  on a separate thread off any async executor. The prompt is exactly
  `CortexKit <module_id>: <summary> Fix: <steps joined with "; ">`, using the
  embedded manifest's module id and the verified finding's fixed summary and
  steps. Only the first 400 Unicode scalar values are sent to the OS, without
  a truncation marker. The helper keeps the withdrawal handle published by the
  prompt. After 120 seconds it withdraws the handle if published, refuses
  `not_confirmed` without joining the thread, and ignores a late result.
- [pinned] Only `Outcome::Approved` confirms. `Declined`,
  `LocalAuthenticationError` with failure `Declined`, and
  `AuthorizedWithoutPrompt` refuse `not_confirmed` without fallback; a policy
  grant is not a person's approval. `NoPresence`, `UnsupportedPlatform`,
  `SetupRequired`, `ProviderError`, `ProviderErrorWithReason`, any other
  `LocalAuthenticationError`, and unknown outcomes fall back to the terminal.
- [pinned] Before opening the terminal the helper rechecks the 10-second
  gate. If more time has elapsed, it refuses `out_of_time`. Otherwise it
  opens `/dev/tty` by default and refuses `no_terminal` if it cannot open it.
  It writes the untruncated prompt text above, a newline, and
  `Type <module_id> to confirm:` followed by a space. It reads one line
  terminated by LF within 120 seconds, strips that LF and one optional CR,
  and accepts only the exact module id. A wrong line, EOF, or timeout gives
  `not_confirmed`. The fallback proves only that a line was typed on the
  controlling terminal, not who typed it. A program that controls the
  terminal can type the public module id.
- [pinned] `Timing` defaults to the 120-second prompt and terminal limits and
  the 10-second and 140-second gates. The helper takes an injected monotonic
  clock, presence function, terminal opener, and `Timing` for tests. Gates
  read the injected clock. The prompt and terminal limits are real waits of
  the configured lengths; tests shorten them. The terminal seam writes a
  prompt and reads a line, EOF, or timeout by a deadline. Module helper tests
  use the injected presence and terminal rather than making real OS prompts
  or opening `/dev/tty`. None of the seams is a runtime flag or environment
  setting in production. `run_with` takes the same arguments and final
  `hooks: Hooks<'_>`, returning `ExitCode`. `Hooks` contains `now: &dyn Fn() ->
  Instant`, `presence: Arc<Presence>`, `open_terminal: &dyn Fn() ->
  Option<Box<dyn Terminal>>`, and `timing: Timing`. `Presence` is
  `dyn Fn(&str, PublishWithdraw) -> Outcome + Send + Sync`; `PublishWithdraw`
  is `Box<dyn FnOnce(Arc<dyn Withdraw>) + Send>`. `Terminal` implements
  `write_prompt(&mut self, &str) -> io::Result<()>` and
  `read_line(&mut self, Instant) -> io::Result<TerminalRead>`. `TerminalRead`
  is `Line(Vec<u8>)` (including the line ending), `Eof`, or `TimedOut`.
  Production code calls `run`. Wire the conformance kit's test-only presence
  switch to `run_with` only in the module's test binary (see §9). Compile the
  switch out of release builds; a runtime marker check alone is not enough.
- [pinned] Preflight is advisory. The fix callback must hold the same lock
  or lease the running module takes for the entire apply. If it cannot
  obtain that exclusion, it refuses `module_running` or `files_in_use`
  without writing. If the target depends on evidence, it revalidates that
  target under exclusion and refuses `plan_changed` without writing if the
  target no longer holds. Keep writes crash-consistent: a fix can be killed
  at any point, so use transactions, temporary-file replacement by rename,
  or another method that leaves stores consistent. The report records
  `timed_out` when the runner kills a fix at its deadline; it cannot tell
  whether the fix took effect.

## 4. Running a doctor program

The Unix-only `cortexkit_doctor::runner` exposes:

```rust,ignore
fn check(program: impl AsRef<Path>, options: &Options) -> Result<CheckOutput, RunError>;
fn manifest(program: impl AsRef<Path>, options: &Options) -> Result<Manifest, RunError>;
fn fix(
    program: impl AsRef<Path>,
    id: &FindingId,
    digest: &str,
    timeout_ms: u64,
    options: &Options,
) -> Result<FixOutput, RunError>;
```

Start with `Options::new()`. `with_data_home` and `with_config_home` take an
absolute path (`impl Into<PathBuf>`); `with_force_setsid(bool)` disables the
terminal handoff for fixes when set to `true`. `with_limits(Limits)` overrides
the contract defaults for tests. For `fix`, use the digest and `timeout_ms`
from the selected plan. The runner must own waiting for its child; don't have
a separate SIGCHLD handler reap it or enable automatic reaping. Serialize
foreground fixes on any one terminal.

- [pinned] The runner clears the environment and sets absolute
  `XDG_DATA_HOME` and `XDG_CONFIG_HOME`. It uses the supplied homes, or resolves
  them with `cortexkit_store_types::resolve_data_home` and
  `cortexkit_store_types::resolve_config_home`. A relative result refuses to
  run with `home_relative`; there is no state home. The runner passes no other
  variables to `check` or `manifest`. For `fix` it also preserves these prompt
  variables when set: on Linux `DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR`,
  `DISPLAY`, and `WAYLAND_DISPLAY`; on macOS none. macOS itself can add
  `__CF_USER_TEXT_ENCODING` at process startup; the runner does not pass it.
  The contract defines `SystemRoot`, `windir`, `USERPROFILE`, `LOCALAPPDATA`,
  and `TEMP` as the prompt allowlist for Windows, but this crate ships no
  Windows runner. A missing variable can prevent the OS prompt and lead to
  terminal fallback; it cannot confirm a fix.
- [pinned] The runner connects stdin to `/dev/null` and discards stderr.
  `check` and `manifest` start with `setsid` and have no controlling terminal.
  For `fix`, the child gets its own process group and becomes foreground on
  the terminal before exec only if the runner has a controlling terminal, is
  foreground on it, and `force_setsid` was not set. It blocks `SIGTTOU` for
  `tcsetpgrp` and restores the inherited signal mask before exec. The runner
  restores its own foreground group with `SIGTTOU` blocked on every return
  path, including timeout and kill. Otherwise the child uses `setsid`; the
  terminal fallback cannot open `/dev/tty`. Either way the child pid equals
  the managed group id.
- [pinned] `Limits` defaults to 10 seconds for `check`, two seconds for
  `manifest`, and `Plan::timeout_ms` plus 150 seconds for `fix`. That allowance
  covers five seconds before helper entry, ten seconds to start confirmation,
  one full 120-second confirmation step, ten seconds for rechecking, and five
  seconds to write the result and exit. The module must dispatch `doctor`
  immediately. The helper does not know the launch time; the full callback
  budget is guaranteed only if helper entry is within five seconds of launch.
  A slower start can end in `timed_out`.
- [pinned] `Limits` also defaults to a two-second post-exit read grace and a
  two-second interval between `SIGTERM` and `SIGKILL`. The runner observes the
  direct child with `waitid(WEXITED | WNOWAIT)` and reaps it only after its last
  signal to the group, so its group id cannot be reused while signalling.
  If the direct child has not exited by the deadline, the runner stops
  reading, sends `SIGTERM` to the group, waits the interval, sends `SIGKILL`
  even if the child died in between, and reaps the child. Otherwise it reads
  to stdout EOF or the end of the read grace after exit, whichever is earlier,
  even if the grace crosses the deadline. It then sends `SIGKILL` and reaps.
  With the defaults, the call returns by the deadline plus two seconds and
  reaping time. The runner sends `SIGKILL` to the managed group before it
  returns; the system reaps orphaned descendants. A descendant that escapes
  the group with its own `setsid` is outside the teardown guarantee.
- [pinned] The runner decides the result after reading, not at the provisional
  exit notification. Bytes received during the grace count. Cleanup after the
  decision does not change it. The first applicable result wins:
  1. Spawn failed: `launch_failed`, with the I/O error kind available to the
     caller.
  2. Stdout exceeded 1 MiB before reading ended: `invalid_output` with reason
     `too_large`. Reading stops and teardown starts immediately; no decoding
     is attempted.
  3. The direct child had not exited by the deadline: `timed_out`.
  4. The child died from a signal: `crashed` with the integer `signal`.
  5. `manifest` exited nonzero with empty stdout: `no_doctor`.
  6. `check` exited 2 with empty stdout: `no_doctor`.
  7. Any other nonzero exit: `crashed` with the integer `exit_code`.
  8. Exit 0 with invalid stdout: `invalid_output` with the first
     `DecodeError` stage as its reason. Otherwise the runner returns the
     parsed document. No failure carries child output text.

Unlike `manifest`, `check` treats only the usage exit (2) as evidence of a
missing subcommand, so a panic or other failed check with empty stdout still
surfaces as `crashed`.

`RunError` has variants `LaunchFailed(std::io::ErrorKind)`, `HomeRelative`,
`Crashed(Crash)`, `TimedOut`, `InvalidOutput(DecodeError)` and `NoDoctor`.
`Crash` is either `ExitCode(i32)` or `Signal(i32)`. `RunError::as_str()` returns
the failure name above without details, and reports omit the launch error kind.

## 5. Manifest and template generator

- [pinned] `Manifest` has `schema: "ck.doctor-manifest/v1"`, `module_id`,
  `repository`, `logs`, `templates`, and `templates_digest`.
  `Manifest::to_canonical_bytes` writes RFC 8785 canonical JSON and exactly
  one final LF, all within `MAX_OUTPUT_BYTES`.
- [pinned] `ModuleId` accepts `[a-z][a-z0-9-]{0,63}`. The repository is
  `https://github.com/<owner>/<repo>` with owner `[A-Za-z0-9-]{1,39}` and repo
  `[A-Za-z0-9._-]{1,100}`. The repo is not `.` or `..` and does not end in
  `.git`. No extra path, query, or fragment is allowed. The module id, owner,
  and repo are subject to the structural-string secret check.
- [pinned] `logs` contains 1 to 8 `LogSource` objects. Each has `dir: "data"`,
  `format: "cortexkit-log/v1"`, and a `glob` of 1 to 256 bytes relative to
  the module's data directory. Every `/`-separated segment is nonempty,
  consists of `[A-Za-z0-9._*-]`, and is not `.` or `..`. Absolute paths and
  `**` are invalid. Syntactic validation does not constrain filesystem
  resolution: `ck` must confine resolved matches to the module's data
  directory, including when resolving symlinks, before reading any log.
- [pinned] `templates` is an array sorted by unique id. A `Template` has
  `id`, `target`, `text`, and `fields`. A target takes 1 to 256 UTF-8 bytes;
  text takes 1 to 1024. Neither permits a control character. `TemplateField`
  objects have `name` and `kind`, sorted by unique `[a-z0-9_]{1,64}` names.
  An unkinded name, a different kind from `field_kind`, or a secret match
  is invalid even if the digests agree. Templates carry no log level.
- [pinned] `template_id` is the first 12 lowercase hex digits of SHA-256 over
  the target, one NUL byte, and the string literal's value after resolving
  escapes and collapsing `{{` and `}}`. The id is a key in the manifest and
  report, never a field on a log line. `templates_digest` is `sha256:` plus
  the lowercase hex SHA-256 of the canonical JSON of the template array.
  `Manifest::decode` recomputes the ids and then the digest.
- [pinned] `field_kind` applies the following rules in order. Prefixes win
  over exact names and suffixes. An unlisted or invalid name is never kept.
  1. Prefixes `is_` and `has_` give `bool`; `n_` gives `count`.
  2. Exact names: `ok` gives `bool`; `elapsed_ms`, `duration_ms`, and
     `timeout_ms` give `duration_ms`; `bytes`, `count`, and `ratio` give
     their own kinds; `code`, `reason`, `state`, `cause`, `outcome`, `level`,
     and `kind` give `code`; `version` gives `version`; `path` gives `path`;
     `session`, `agent`, `project`, and `scope_ref` give `ref`.
  3. Suffixes, tried in order: `_at_ms` gives `timestamp_ms`; `_ms` gives
     `duration_ms`; `_bytes` gives `bytes`; `_count` gives `count`; `_ratio`
     gives `ratio`; `_code`, `_reason`, or `_state` give `code`; `_version`
     gives `version`; `_path`, `_dir`, or `_file` give `path`; `_id` or `_ref`
     give `ref`. For example, `has_file` is `bool` and `n_bytes` is `count`
     by the prefix rules.

### Generating the file

- [pinned] Generate the file with `cortexkit-log-manifest` rather than writing
  templates by hand. The generator scans one crate's lib and bin targets as
  Cargo resolves them. It uses explicit `[lib]` and `[[bin]]` names and paths
  and auto-discovers `src/lib.rs`, `src/main.rs`, `src/bin/<name>.rs`, and
  `src/bin/<name>/main.rs` as applicable, honoring `autobins = false`.
  It excludes `tests/`, `benches/`, `examples/`, and `#[cfg(test)]` items,
  but includes other cfg-gated modules. Other crates contribute no templates.
- [pinned] Recognition is syntactic: the generator looks for `trace!`,
  `debug!`, `info!`, `warn!`, `error!`, and `event!`, bare or
  `tracing::`-qualified. It does not resolve names. It includes a local macro
  of the same name and misses a renamed tracing import. An `event!` level
  must be a path ending in `Level::TRACE`, `Level::DEBUG`, `Level::INFO`,
  `Level::WARN`, or `Level::ERROR`.
- [pinned] The target is an explicit string-literal `target:` or the path
  that `module_path!()` would give. The generator rebuilds it from the lib
  or bin name (hyphens become underscores) and the module tree, including
  file modules, `mod.rs`, `#[path]`, and inline modules. `name:` and `parent:`
  do not affect templates.
- [pinned] The generator records the field name tracing records for `name = v`,
  `name = ?v`, `name = %v`, `?name`, `%name`, `?name = v`, bare `name`, a
  quoted key with or without a value sigil, and dotted keys and shorthand
  such as `%self.session_id`. It keeps only names with a `field_kind`, so a
  dotted name can be recognized without being listed.
- [pinned] In both modes the generator skips an event with format arguments
  (including inline captures), no message, a `message =` field, a nonliteral
  or invalid target, an invalid message literal, a literal starting with `[`,
  or a nonconstant `event!` level. A leading bracket is ambiguous with the
  log format's bound fields. It also skips a call inside `macro_rules!` and
  a supported macro name followed by `!` inside another invocation's tokens.
  Each skip is printed to stderr as `path:line: <reason>` and does not fail
  `--check`. Adopting the contract requires no rewrite of logging calls.
- [pinned] Calls with equal target and text merge, even across levels. The
  merged field list is the intersection of their kinded fields. If two
  unequal target-and-text pairs get the same id, write mode exits 1 and
  names both call sites without touching the file; `--check` also fails.
- [pinned] The command accepts `--check`, `--crate <dir>`, `--out <path>`,
  `--module-id <id>`, `--repository <url>`, and repeated `--log-glob <glob>`.
  The crate defaults to `.` and the output to `doctor-manifest.json` at the
  crate root. In write mode, flags replace the corresponding metadata;
  otherwise the file supplies it. Creating a manifest without this metadata
  is a usage error (exit 2). An invalid or oversized result exits 1 without
  touching the file. `--check` validates the checked-in file, then compares
  its exact bytes to the regenerated manifest with the same metadata. In this
  mode, metadata flags assert the stored values rather than replacing them.
  `--check` exits 1 on a difference, including a missing final LF. The generator
  shares `Manifest`, `field_kind`, `redact_secrets`, `template_id`, and
  `templates_digest` from `cortexkit-doctor`.

## 6. Sanitizing log tails

- [pinned] Split each tail at LF. An empty tail and the empty piece after a
  final LF emit nothing and do not count as omissions. Pass every other
  piece, including an interior empty line or a line ending in CR, to
  `cortexkit_log::parse_line`. The parser checks the UTC millisecond timestamp,
  logger, and level, and separates the bound bracket. Admit the module id
  itself, or that id followed by dot-separated `[a-z0-9_-]` segments. A bad line or
  another logger contributes one to `omitted_lines`. The logger itself is
  not reported. Rust-path targets use the bare logger; explicit dotted
  targets use `<module>.<target>`, and dependency events use
  `<module>.dep.<crate>`. All are admitted under that same rule. The input
  has already passed through the built-in `cortexkit-log` redaction (see §7)
  and any module-specific log redactor.
- [pinned] Match the template text against the start of the line's body,
  with each backslash in the template doubled as the log writer does. The
  match must end at the end of the body or just before the space introducing
  the first field. The remaining text must lex as fields. Prefer the longest
  matching text. If several templates share that text, use the lowest id and
  keep only fields listed with the same kind by every matching template.
- [pinned] Fields lex as a sequence of ` key=value`. The key is a nonempty
  run of characters other than space, `=`, and `"`. An unquoted value is a
  nonempty run of characters other than space and `"`. A quoted value
  supports `\\`, `\"`, `\r`, `\n`, and `\uXXXX` with four lowercase hex
  digits. Its closing quote must be followed by a space or the end of the
  body. A malformed remainder does not match the template.
- [pinned] A matched line emits `{age, level, template, text, fields}`. Use
  the line's lowercase level and the chosen template id and text. `fields`
  is an object, empty if nothing is kept. A repeated field keeps its first
  occurrence, even if that value is invalid. Count a drop only when a value
  admitted by the template match fails validation. Unlisted, non-shared,
  repeated, bound, span, and unmatched-line fields do not count. An admitted
  line with no match emits only `{age, level, text: "text omitted"}`.
- [pinned] Events from the module's other crates supply no templates. Their
  admitted lines normally appear as `text omitted`, as do other messages with
  no matching template. Matching observes rendered bytes, not call sites:
  formatted, dependency, and redaction-altered messages can collide with a
  template and be reported as that template, with only its allowed fields.
  For example, a formatted message rendering `check n_bytes=2` can match the
  literal `check` with a listed `n_bytes`.

## 7. Redaction and value projection

- [pinned] `redact_secrets` copies the built-in patterns from `src/redaction.rs`
  in `cortexkit-log` 0.3.7. It runs them in the same order: Authorization,
  Bearer, JWT, `ckh_`, `sk-`, GitHub tokens, URL userinfo, and credential query
  values. Next it redacts PEM private-key blocks from
  `-----BEGIN [A-Z ]*PRIVATE KEY-----` through the first following
  `-----END [A-Z ]*PRIVATE KEY-----`,
  regardless of label. Without an END line it redacts through the end of the
  string. Text after the END line is preserved.
- [pinned] After PEM blocks, `redact_secrets` matches email addresses with
  `[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+`. Last, it considers
  each maximal run of `[A-Za-z0-9+/_=-]`. It redacts the entire run if it
  has at least 40 characters and contains a digit, uppercase letter, and
  lowercase letter, or if it contains a run of 32 or more hex digits in
  either case. A digest in free text becomes `sha256:[redacted]`.
- [pinned] Each match becomes `[redacted]`. As in `cortexkit-log`, the Bearer,
  URL userinfo, and credential-query rules retain their captured prefixes.
  The report builder runs the pass on the title, description, summary, each
  step, retained template text, and projected `code` and `version` values.
  It does not redact generated aliases, template ids, or numbers. Structural
  strings that match a secret pattern are rejected at decode instead.
- [pinned] `ReportContext` takes a 32-byte `report_key`, `report_time_ms`,
  `data_home`, `config_home`, and `temp_dir`. The caller draws a fresh random
  key for each report. Fixtures
  may supply fixed keys and times. An alias is eight lowercase hex digits
  of HMAC-SHA256 keyed by `report_key` over the ASCII kind (`path` or `ref`),
  one NUL byte, and the value's UTF-8 bytes. The path category is not hashed.
  The same kind and value reuse the same alias within a report. If a
  different pair already holds the first eight-digit window anywhere in the
  report, across both kinds, use the next free consecutive eight-digit window.
- [pinned] Assign aliases in encounter order: findings then fix results
  (each evidence list in order), then log lines in report order before
  eviction, with fields in line order. Eviction does not recompute aliases.
  A ref prefix is its name with a trailing `_id` or `_ref` removed, or `ref`
  if empty. The same ref under two names shares the alias, not the prefix.
- [pinned] Project a log path from the decoded field text. A whole-component
  prefix match (`Path::starts_with`) against `data_home`, `config_home`, or
  `temp_dir`, in that order, gives category `data`, `config`, or `temp`.
  Otherwise use `other`. Only module evidence can supply `project`.
- [pinned] Relative age uses `report_time_ms` minus the timestamp. A
  nonnegative difference gives `before`, and a negative one gives `after`.
  Using the absolute difference, use `s` below 60 seconds, `min` below 60
  minutes, `h` below 48 hours, and `d` otherwise. Round the count down in the
  chosen unit. A zero difference is `0 s before`; −500 ms is `0 s after`.
- [pinned] Strict decoding validates untrusted output's shape, not where
  its text came from. Trust the module publisher to supply fixed source
  text for summaries, steps, and templates, not user or store data. The
  person supplies the title and description. The secret pass cannot remove
  all private text: for example, a lowercase file or host name can pass as
  `code` or `version`, and rendered log text can collide with a template.
  Module-chosen finding ids and fix codes that pass validation are published
  as written. `ck` must show the person the full report before filing.

## 8. Report

- [pinned] `build_report` takes a `ReportContext`, the selected `ModuleId`,
  the manifest, title and description, a `CheckOutput` or runner failure,
  fix results paired with their `FindingId`, log tails, and `max_body_bytes`.
  A manifest with a different module id gives `module_mismatch` without a
  report. On success it returns RFC 8785 canonical JSON with
  `schema: "ck.doctor-report/v1"`, `module_id`, `repository`, `title`,
  `description`, `check`, `findings`, `fix_results`, `logs`, `dropped_values`,
  `omitted_lines`, and `evicted_lines`. The caller renders the report.
- [pinned] `check` contains `outcome`, which is `ok` or the runner failure
  name. For `invalid_output` include the stage code as `code`. Only a crash
  includes an integer `exit_code` or `signal`. `findings` is empty unless
  the check outcome is `ok`. Findings stay in check order and keep `id`,
  `severity`, `summary`, projected `evidence`, and `fix` with `class`,
  `needs_stop`, and `steps`. Class `none` has empty `steps`. The report
  includes neither the plan digest nor timeout.
- [pinned] `fix_results` stay in input order and contain `finding_id`,
  `outcome`, optional `code`, `exit_code` or `signal`, and projected
  `evidence`. The outcome is `fixed`, `refused`, or `failed` from a
  `FixOutput`, or the runner failure name. The code is the fix-result code
  or an `invalid_output` stage. Only a crash includes `exit_code` or
  `signal`; absent evidence is an empty array. Projected evidence stays in
  `{name, kind, value}` entries. No runner failure carries output text.
- [pinned] `logs` are sorted oldest first by line timestamp, with ties in
  input tail order and then line order. Measure `max_body_bytes` on the
  final redacted encoding. Evict the oldest lines first until the report
  fits, incrementing `evicted_lines`. `dropped_values` includes drops from
  lines that were evicted. If the report is too large with no log lines,
  return `too_large`; do not truncate findings.

The Rust API is `build_report(&ReportContext, &ReportInput)`, with the selected
module, manifest, text, check result and budget supplied to `ReportInput::new`.
Use `with_fix_results` and `with_log_tails` to include those inputs. The returned
bytes have no trailing newline. `RunError` and `Crash` are platform-independent;
the Unix `runner` module also exports the same types.

When no valid manifest is available, `build_local_report(&ReportContext,
&ModuleId, &CheckOutput)` returns a local note in the same canonical JSON form
but omits `repository`. It uses the same finding projection and secret pass.
The title, description, fix results, and logs are empty, and the builder
imposes no separate byte limit. This form is never filed as a GitHub issue.

If all eight HMAC windows for a new path or ref are already in use, the builder
returns `alias_exhausted` without a report. It never reuses another value's alias
or emits the original value to resolve a collision.

## 9. Checking an adopter with the conformance kit

- [pinned] `cortexkit-doctor-conformance` runs a module program through an
  adapter. The adapter seeds scratch data and config homes and may name a
  fixable finding id and a `needs_stop` id. A `needs_stop` id needs a
  liveness signal: a pid file (decimal pid and LF) or an `flock(LOCK_EX)`
  lock file at a path relative to the data home with no `..`.
- [pinned] The kit never raises a real OS prompt. It runs no `doctor fix`
  unless the adapter declares that the module test build injects a
  non-prompting presence function. In the module's test build, only when
  `CORTEXKIT_DOCTOR_CONFORMANCE=1` is set, read the test-only switch
  `CORTEXKIT_DOCTOR_TEST_PRESENCE`. Inject a function returning `NoPresence`
  only when its value is `no_presence`. Leave this switch out of release
  builds. The runner's non-default `conformance` feature supplies the two
  values only to kit fixes, in place of the OS prompt environment. All kit
  fixes use a new session, with no controlling terminal. Do not enable this
  feature in the ordinary doctor caller.
- [pinned] The stand-in opens each liveness path component with `openat`,
  `O_NOFOLLOW`, and `O_NONBLOCK`. Every parent must be a directory; the final
  entry must be absent or a regular file. If not, fail with
  `unsafe_liveness_path` without writing anything.
- [pinned] Each rule gets newly seeded homes. If a fix needs the stand-in,
  start it before the before snapshot and stop it only after the after
  snapshot. Compare both home trees without following symlinks. The relative
  paths, entry types, symlink target strings, regular-file lengths, and
  regular-file SHA-256 hashes must match. Never open other entry types
  such as FIFOs, sockets, and devices; fail with `unsupported_entry`.
- [pinned] `output.valid` checks that the check output decodes.
  `check.read_only` checks that the check changes neither home. Neither rule
  uses the stand-in. `fix.stale_digest` and `fix.no_terminal` use the fixable
  id; `fix.module_running` uses the `needs_stop` id. A named id that is
  missing from the baseline check or has class `none` fails with
  `bad_adapter_input`, as does a `needs_stop` id without `needs_stop: true`.
- [pinned] `fix.stale_digest` expects exactly `refused`/`plan_changed`.
  `fix.no_terminal` expects exactly `refused`/`no_terminal`.
  `fix.module_running` expects exactly `refused`/`module_running`, which
  distinguishes preflight from a confirmation attempt made first. With
  the stand-in active, the check must report the `needs_stop` finding
  with the same digest, or the rule is `skipped` as `setup_not_observable`.
  `fix.no_change_on_refusal` compares snapshots for every fix run no matter
  its result, and is `skipped` when no fix runs.
- [pinned] `manifest.valid` fails for any manifest run failure except
  `invalid_output` with reason `templates_digest`. `manifest.digest`
  fails on that reason and is skipped if `manifest.valid` failed. Absent
  inputs or declarations and failed prerequisites yield `skipped`, not
  a pass. Fix rules are skipped after a failed `output.valid`. Report
  each rule by name, never with the child's output text.

Shared fixtures belong under `test-vectors/doctor-v1/` at the repository
root, with a `SHA256SUMS` file. They cover manifests, raw log tails, fixed
report contexts, expected reports, and redaction canaries.
