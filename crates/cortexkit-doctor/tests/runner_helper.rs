#![cfg(all(unix, feature = "runner-test-fixture"))]

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use cortexkit_doctor::runner::{self, Cancel, Crash, Limits, Options, RunError};
use cortexkit_doctor::{
    DecodeError, Finding, FixOutput, FixResult, LogSource, Manifest, Mode, ModuleId, Template,
    MAX_OUTPUT_BYTES,
};

#[path = "support/runner_terminal.rs"]
mod terminal;

struct Fixture {
    _scratch: tempfile::TempDir,
    data: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let data = scratch.path().join("data");
        let config = scratch.path().join("config");
        fs::create_dir(&data).unwrap();
        fs::create_dir(&config).unwrap();
        Self {
            _scratch: scratch,
            data,
            config,
        }
    }

    fn program(&self, mode: &str) -> PathBuf {
        let source = env!("CARGO_BIN_EXE_cortexkit-doctor-runner-fixture");
        let target = self._scratch.path().join(mode);
        // A concurrent fork can inherit an open copy's write descriptor and
        // cause exec to fail with "Text file busy". Keep the mode in argv[0]
        // without opening the executable for writing.
        if !target.exists() {
            symlink(source, &target).unwrap();
        }
        target
    }

    fn options(&self) -> Options {
        Options::new()
            .with_data_home(&self.data)
            .with_config_home(&self.config)
    }

    fn start(&self) -> Start {
        let line = fs::read_to_string(self.data.join("start")).unwrap();
        let parts: Vec<_> = line.split_whitespace().collect();
        Start {
            pid: parts[0].parse().unwrap(),
            pgid: parts[1].parse().unwrap(),
            sid: parts[2].parse().unwrap(),
            tty: parts[3].parse().unwrap(),
            blocked: parts[4].parse().unwrap(),
        }
    }
}

#[derive(Debug)]
struct Start {
    pid: libc::pid_t,
    pgid: libc::pid_t,
    sid: libc::pid_t,
    tty: bool,
    blocked: bool,
}

fn fix(program: &Path, finding: &Finding, options: &Options) -> Result<FixOutput, RunError> {
    let plan = finding.fix().plan().unwrap();
    runner::fix(
        program,
        finding.id(),
        plan.digest(),
        plan.timeout_ms(),
        Mode::Quick,
        options,
    )
}

fn finding(program: &Path, options: &Options) -> Finding {
    runner::check(program, Mode::Quick, options)
        .unwrap()
        .findings()[0]
        .clone()
}

#[test]
fn runner_helper_exit_and_output_precedence() {
    let fixture = Fixture::new();
    let mut limits = Limits::default();
    limits.check = Duration::from_secs(6);
    let options = fixture.options().with_limits(limits);

    let start = Instant::now();
    assert_eq!(
        finding(&fixture.program("probe"), &options).id().as_str(),
        "history.backlog"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "waited for grace after EOF"
    );
    for (mode, expected) in [
        ("abort", RunError::Crashed(Crash::Signal(6))),
        ("exit3-text", RunError::Crashed(Crash::ExitCode(3))),
        (
            "extra-output",
            RunError::InvalidOutput(DecodeError::ExtraOutput),
        ),
        ("oversized", RunError::InvalidOutput(DecodeError::TooLarge)),
    ] {
        let start = Instant::now();
        assert_eq!(
            runner::check(fixture.program(mode), Mode::Quick, &options),
            Err(expected)
        );
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{mode} waited for the deadline"
        );
    }
    assert_eq!(
        runner::manifest(fixture.program("exit3"), &options),
        Err(RunError::NoDoctor)
    );
    assert_eq!(
        runner::manifest(fixture.program("exit3-text"), &options),
        Err(RunError::Crashed(Crash::ExitCode(3)))
    );
    assert_eq!(
        runner::manifest(fixture.program("abort"), &options),
        Err(RunError::Crashed(Crash::Signal(6)))
    );
}

#[test]
fn runner_helper_check_no_doctor_requires_empty_usage_error() {
    let fixture = Fixture::new();
    let options = fixture.options();
    let finding = finding(&fixture.program("probe"), &options);
    for (mode, code, check_error) in [
        ("exit2", 2, RunError::NoDoctor),
        ("exit101", 101, RunError::Crashed(Crash::ExitCode(101))),
    ] {
        let program = fixture.program(mode);
        assert_eq!(
            runner::check(&program, Mode::Quick, &options),
            Err(check_error),
            "{mode}"
        );
        assert_eq!(
            runner::manifest(&program, &options),
            Err(RunError::NoDoctor)
        );
        assert_eq!(
            fix(&program, &finding, &options),
            Err(RunError::Crashed(Crash::ExitCode(code)))
        );
    }
    assert_eq!(
        runner::check(fixture.program("exit2-text"), Mode::Quick, &options),
        Err(RunError::Crashed(Crash::ExitCode(2)))
    );
}

#[test]
fn runner_helper_full_check_does_not_retry_older_helper() {
    let fixture = Fixture::new();
    let program = fixture.program("quick-only");
    let options = fixture.options();
    assert!(runner::check(&program, Mode::Quick, &options).is_ok());
    assert_eq!(
        runner::check(&program, Mode::Full, &options),
        Err(RunError::NoDoctor)
    );
}

#[test]
fn runner_helper_full_check_uses_full_deadline() {
    let fixture = Fixture::new();
    let program = fixture.program("full-only");
    let mut limits = Limits::default();
    limits.check = Duration::from_secs(6);
    limits.full_check = Duration::from_millis(100);
    limits.term_to_kill = Duration::ZERO;
    let options = fixture.options().with_limits(limits);
    assert_eq!(
        runner::check(&program, Mode::Full, &options),
        Err(RunError::TimedOut)
    );
    limits.full_check = Duration::from_secs(6);
    let options = fixture.options().with_limits(limits);
    assert!(runner::check(&program, Mode::Quick, &options)
        .unwrap()
        .findings()
        .is_empty());
    let full = runner::check(&program, Mode::Full, &options).unwrap();
    assert_eq!(full.findings()[0].id().as_str(), "history.backlog");
}

#[test]
fn runner_helper_full_fix_uses_full_flag_and_allowance() {
    let fixture = Fixture::new();
    let program = fixture.program("full-only");
    let mut limits = Limits::default();
    limits.fix_allowance = Duration::ZERO;
    limits.term_to_kill = Duration::ZERO;
    let options = fixture
        .options()
        .with_force_setsid(true)
        .with_limits(limits);
    let full = runner::check(&program, Mode::Full, &options).unwrap();
    let finding = &full.findings()[0];
    let plan = finding.fix().plan().unwrap();
    let fix = runner::fix(
        &program,
        finding.id(),
        plan.digest(),
        plan.timeout_ms(),
        Mode::Full,
        &options,
    )
    .unwrap();
    assert_eq!(fix.result(), FixResult::Fixed);
}

#[test]
fn runner_helper_launch_error_preserves_io_kind() {
    let fixture = Fixture::new();
    let options = fixture.options();
    let missing = fixture.data.join("missing-program");
    let non_executable = fixture.data.join("non-executable");
    fs::write(&non_executable, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o600)).unwrap();

    for (path, kind) in [
        (missing, ErrorKind::NotFound),
        (non_executable, ErrorKind::PermissionDenied),
    ] {
        let error = runner::check(path, Mode::Quick, &options).unwrap_err();
        assert_eq!(error, RunError::LaunchFailed(kind));
        assert_eq!(error.as_str(), "launch_failed");
    }
}

#[test]
fn runner_helper_canonical_manifest_at_byte_limit() {
    let fixture = Fixture::new();
    let manifest = |extra: usize| {
        let mut templates: Vec<_> = (0..1024)
            .map(|i| {
                let len = 1 + extra.saturating_sub(i * 1023).min(1023);
                Template::new(format!("fixture::{i:04}"), "x".repeat(len), vec![]).unwrap()
            })
            .collect();
        templates.sort_by(|a, b| a.id().cmp(b.id()));
        Manifest::new(
            ModuleId::new("fixture-module").unwrap(),
            "https://github.com/example/diagnostics",
            vec![LogSource::new("*.log").unwrap()],
            templates,
        )
        .unwrap()
    };
    let base = manifest(0).to_canonical_bytes().unwrap().len();
    let exact = manifest(MAX_OUTPUT_BYTES - base);
    let mut bytes = exact.to_canonical_bytes().unwrap();
    assert_eq!(bytes.len(), MAX_OUTPUT_BYTES);
    assert_eq!(bytes.last(), Some(&b'\n'));
    let path = fixture.data.join("manifest.json");
    let program = fixture.program("from-file");
    let options = fixture.options();
    fs::write(&path, &bytes).unwrap();
    assert_eq!(runner::manifest(&program, &options), Ok(exact));
    bytes.push(b' ');
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        runner::manifest(&program, &options),
        Err(RunError::InvalidOutput(DecodeError::TooLarge))
    );
}

#[test]
fn runner_helper_environment() {
    const HOME: &str = "CK_DOCTOR_RUNNER_ENV_CASE";
    const MARKER: &str = "CORTEXKIT_DOCTOR_CONFORMANCE";
    const SWITCH: &str = "CORTEXKIT_DOCTOR_TEST_PRESENCE";
    if std::env::var_os("CK_DOCTOR_RUNNER_RELATIVE_CASE").is_some() {
        assert_eq!(
            runner::check("unused", Mode::Quick, &Options::new()),
            Err(RunError::HomeRelative)
        );
        return;
    }
    if let Some(root) = std::env::var_os(HOME) {
        assert_eq!(std::env::var("OTHER").as_deref(), Ok("not passed"));
        let root = PathBuf::from(root);
        let data = root.join(".local/share");
        let config = root.join(".config");
        let program = root.join("probe");
        let options = Options::new();
        let expected = BTreeMap::from([
            (
                "XDG_CONFIG_HOME".to_owned(),
                config.to_str().unwrap().to_owned(),
            ),
            (
                "XDG_DATA_HOME".to_owned(),
                data.to_str().unwrap().to_owned(),
            ),
        ]);
        let env = || -> BTreeMap<String, String> {
            let vars: BTreeMap<String, String> =
                serde_json::from_slice(&fs::read(data.join("env")).unwrap()).unwrap();
            #[cfg(target_os = "macos")]
            let vars = {
                let mut vars = vars;
                // macOS adds this at process start; the runner never passes it.
                vars.remove("__CF_USER_TEXT_ENCODING");
                vars
            };
            vars
        };
        let finding = finding(&program, &options);
        assert_eq!(env(), expected);
        runner::manifest(&program, &options).unwrap();
        assert_eq!(env(), expected);
        assert_eq!(
            fix(&program, &finding, &options).unwrap().result(),
            FixResult::Fixed
        );
        let mut expected_fix = expected.clone();
        if cfg!(target_os = "linux") {
            expected_fix.extend(prompt_env().map(|(k, v)| (k.to_owned(), v.to_owned())));
        }
        assert_eq!(env(), expected_fix);

        #[cfg(feature = "conformance")]
        {
            let options = options
                .with_conformance_fix_marker()
                .with_force_setsid(false);
            runner::check(&program, Mode::Quick, &options).unwrap();
            assert_eq!(env(), expected);
            runner::manifest(&program, &options).unwrap();
            assert_eq!(env(), expected);
            fix(&program, &finding, &options).unwrap();
            let mut expected = expected;
            expected.insert(MARKER.into(), "1".into());
            expected.insert(SWITCH.into(), "no_presence".into());
            assert_eq!(env(), expected);
        }
        return;
    }

    let fixture = Fixture::new();
    let program = fixture.program("probe");
    let root = program.parent().unwrap();
    fs::create_dir_all(root.join(".local/share")).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "runner_helper_environment", "--nocapture"])
        .env_clear()
        .env(HOME, root)
        .env("HOME", root)
        .env("TERM", "fixture")
        .env("OTHER", "not passed")
        .env(MARKER, "not passed")
        .env(SWITCH, "not passed")
        .envs(prompt_env())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for options in [
        fixture.options().with_data_home("relative"),
        fixture.options().with_config_home("relative"),
    ] {
        assert_eq!(
            runner::check(&program, Mode::Quick, &options),
            Err(RunError::HomeRelative)
        );
        assert!(!fixture.data.join("start").exists());
    }
    for env in [
        Vec::new(),
        vec![("HOME", "/fixture"), ("XDG_CONFIG_HOME", "relative")],
    ] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "runner_helper_environment", "--nocapture"])
            .env_clear()
            .env("CK_DOCTOR_RUNNER_RELATIVE_CASE", "1")
            .envs(env)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn prompt_env() -> [(&'static str, &'static str); 4] {
    [
        ("DBUS_SESSION_BUS_ADDRESS", "fixture-address"),
        ("XDG_RUNTIME_DIR", "/fixture-runtime"),
        ("DISPLAY", ":99"),
        ("WAYLAND_DISPLAY", ""),
    ]
}

fn group_test_subprocess(test: &str) -> bool {
    const CHILD: &str = "CK_DOCTOR_RUNNER_GROUP_CASES";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return false;
    }

    // A CI container's PID 1 may not reap orphans. Act as the reaper in this
    // test subprocess only, so the ESRCH check is independent of PID 1.
    #[cfg(target_os = "linux")]
    {
        // SAFETY: this only changes orphan adoption by this test subprocess.
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
            0
        );
    }

    true
}

#[test]
fn runner_helper_group_cleanup() {
    if !group_test_subprocess("runner_helper_group_cleanup") {
        return;
    }

    let mut limits = Limits::default();
    limits.check = Duration::from_millis(1_250);
    for mode in [
        "hold",
        "late-hold",
        "grace-large",
        "hang",
        "term-grandchild",
    ] {
        let fixture = Fixture::new();
        let options = fixture.options().with_limits(limits);
        let program = fixture.program(mode);
        let start = Instant::now();
        let output = runner::check(program, Mode::Quick, &options);
        let elapsed = start.elapsed();
        let child = fixture.start();
        assert_eq!(child.pid, child.pgid);
        assert_eq!(child.pid, child.sid);
        let grandchild: libc::pid_t = fs::read_to_string(fixture.data.join("grandchild"))
            .unwrap()
            .parse()
            .unwrap();
        assert_ne!(child.pid, grandchild);
        wait_for_group_to_empty(child.pgid);
        match mode {
            "hold" | "late-hold" => {
                assert_eq!(
                    output.unwrap().findings()[0].id().as_str(),
                    "history.backlog"
                );
                let delay = if mode == "late-hold" {
                    Duration::from_millis(750)
                } else {
                    Duration::ZERO
                };
                assert!(
                    elapsed >= delay + limits.exit_grace,
                    "{mode} ended grace early"
                );
                assert!(
                    elapsed < delay + Duration::from_secs(3),
                    "{mode} exceeded grace"
                );
            }
            "grace-large" => {
                assert_eq!(output, Err(RunError::InvalidOutput(DecodeError::TooLarge)));
                assert!(
                    elapsed < limits.exit_grace,
                    "over-cap output did not end grace"
                );
            }
            "hang" | "term-grandchild" => {
                assert_eq!(output, Err(RunError::TimedOut), "{mode}");
                assert!(
                    elapsed >= limits.check + limits.term_to_kill,
                    "{mode} cut the SIGTERM interval short"
                );
                assert!(
                    elapsed < limits.check + Duration::from_secs(3),
                    "{mode} exceeded deadline + 3 s"
                );
                if mode == "term-grandchild" {
                    assert!(
                        fixture.data.join("direct-exited").exists(),
                        "direct child did not die before SIGKILL"
                    );
                }
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn runner_helper_cancel_running_check() {
    if group_test_subprocess("runner_helper_cancel_running_check") {
        cancel_running_check("term-grandchild");
    }
}

#[test]
fn runner_helper_cancel_kills_term_ignoring_check() {
    if group_test_subprocess("runner_helper_cancel_kills_term_ignoring_check") {
        cancel_running_check("hang");
    }
}

fn cancel_running_check(fixture_mode: &str) {
    let fixture = Fixture::new();
    let mut limits = Limits::default();
    limits.full_check = Duration::from_secs(5);
    limits.term_to_kill = Duration::from_millis(250);
    let options = fixture.options().with_limits(limits);
    let program = fixture.program(fixture_mode);
    let cancel = Cancel::new();
    let cancel_request = cancel.clone();
    let (output, elapsed) = std::thread::scope(|scope| {
        let run = scope.spawn(|| runner::check_cancellable(program, Mode::Full, &options, &cancel));
        wait_for("the helper to set its TERM disposition", || {
            fixture.data.join("ready").exists()
        });
        let start = Instant::now();
        cancel_request.cancel();
        let output = run.join().unwrap();
        (output, start.elapsed())
    });

    assert_reaped(&fixture.start());
    assert_eq!(output, Err(RunError::Cancelled));
    assert!(
        elapsed >= limits.term_to_kill,
        "skipped TERM grace: {elapsed:?}"
    );
    assert!(
        elapsed < limits.term_to_kill + Duration::from_secs(1),
        "cancellation took {elapsed:?}"
    );
    if fixture_mode == "term-grandchild" {
        assert!(
            fixture.data.join("direct-exited").exists(),
            "TERM did not reach the leader before the group was killed"
        );
    }
}

#[test]
fn runner_helper_cancel_before_start_never_spawns() {
    let fixture = Fixture::new();
    let cancel = Cancel::new();
    cancel.cancel();
    let output = runner::check_cancellable(
        fixture.program("probe"),
        Mode::Quick,
        &fixture.options(),
        &cancel,
    );
    assert!(!fixture.data.join("start").exists(), "spawned after cancel");
    assert_eq!(output, Err(RunError::Cancelled));
    assert_eq!(RunError::Cancelled.as_str(), "cancelled");
}

#[test]
fn runner_helper_cancel_after_exit_preserves_result() {
    if !group_test_subprocess("runner_helper_cancel_after_exit_preserves_result") {
        return;
    }
    let fixture = Fixture::new();
    let program = fixture.program("hold");
    let options = fixture.options();
    let cancel = Cancel::new();
    let output = std::thread::scope(|scope| {
        let run =
            scope.spawn(|| runner::check_cancellable(program, Mode::Quick, &options, &cancel));
        wait_for("the helper to fork a grandchild", || {
            fixture.data.join("grandchild").exists()
        });
        let pid = fixture.start().pid;
        // The grandchild holds stdout open past the direct child's exit.
        // Observe that exit without reaping the child or ending the read grace.
        wait_for("the helper to exit", || {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: info is writable and WNOWAIT leaves the child for the runner.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
                )
            };
            if result == -1 {
                let error = std::io::Error::last_os_error();
                assert_eq!(error.kind(), ErrorKind::Interrupted, "waitid: {error}");
            }
            result == 0 && info.si_code == libc::CLD_EXITED
        });
        cancel.cancel();
        run.join().unwrap()
    });
    assert_reaped(&fixture.start());
    assert_eq!(
        output.unwrap().findings()[0].id().as_str(),
        "history.backlog"
    );
}

fn wait_for(description: &str, mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn assert_reaped(child: &Start) {
    assert_eq!(child.pid, child.pgid);
    // SAFETY: signal zero only probes the recorded direct child's PID.
    assert_eq!(unsafe { libc::kill(child.pid, 0) }, -1, "leader survived");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    wait_for_group_to_empty(child.pgid);
}

fn wait_for_group_to_empty(pgid: libc::pid_t) {
    let start = Instant::now();
    loop {
        #[cfg(target_os = "linux")]
        // SAFETY: only reap this fixture group's adopted orphans, without blocking.
        while unsafe { libc::waitpid(-pgid, std::ptr::null_mut(), libc::WNOHANG) } > 0 {}

        // SAFETY: signal 0 only checks whether the process group exists.
        if unsafe { libc::kill(-pgid, 0) } == -1 {
            let err = std::io::Error::last_os_error().raw_os_error();
            if err == Some(libc::ESRCH) {
                return;
            }
            // On macOS a group with only zombies can report EPERM until the
            // OS reaps the last member. Still insist on ESRCH within the bound.
            assert!(
                cfg!(target_os = "macos") && err == Some(libc::EPERM),
                "kill(-{pgid}, 0): {err:?}"
            );
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "group {pgid} survived cleanup"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "real 9 s check and 119 s injected approval exercise the default limits"]
fn runner_helper_slow_default_fix() {
    let fixture = Fixture::new();
    let program = fixture.program("slow");
    let options = fixture.options().with_force_setsid(true);
    let finding = finding(&program, &options);
    let start = Instant::now();
    assert_eq!(
        fix(&program, &finding, &options).unwrap().result(),
        FixResult::Fixed
    );
    assert!(start.elapsed() >= Duration::from_secs(129));
}
