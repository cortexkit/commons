use std::fs::OpenOptions;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;

use windows_sys::Win32::Foundation::{
    ERROR_INVALID_PARAMETER, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
use windows_sys::Win32::System::Console::{
    WriteConsoleInputW, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT, KEY_EVENT_RECORD,
    KEY_EVENT_RECORD_0,
};
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, WaitForSingleObject,
    CREATE_NEW_CONSOLE, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::*;

#[test]
fn runner_helper_windows_job_cleanup() {
    let mut limits = Limits::default();
    limits.check = Duration::from_millis(1_250);
    for mode in ["hold", "late-hold", "grace-large", "hang"] {
        let fixture = Fixture::new();
        let options = fixture.options().with_limits(limits);
        let program = fixture.program(mode);
        let start = Instant::now();
        let output = runner::check(program, Mode::Quick, &options);
        let elapsed = start.elapsed();
        assert!(!started(&fixture).1);
        assert_tree_exited(&fixture, mode);
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
            "hang" => {
                assert_eq!(output, Err(RunError::TimedOut));
                assert!(elapsed >= limits.check, "ended before the deadline");
                assert!(
                    elapsed < limits.check + limits.term_to_kill,
                    "detached job waited for console grace"
                );
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn runner_helper_windows_cancel_running_check_kills_job() {
    let fixture = Fixture::new();
    let mut limits = Limits::default();
    limits.full_check = Duration::from_secs(5);
    let options = fixture.options().with_limits(limits);
    let program = fixture.program("hang");
    let cancel = Cancel::new();
    let (output, elapsed) = std::thread::scope(|scope| {
        let run = scope.spawn(|| runner::check_cancellable(program, Mode::Full, &options, &cancel));
        wait_for("the running helper's readiness marker", || {
            fixture.data.join("ready").exists()
        });
        let start = Instant::now();
        cancel.cancel();
        (run.join().unwrap(), start.elapsed())
    });
    assert_eq!(output, Err(RunError::Cancelled));
    assert!(
        elapsed < limits.term_to_kill,
        "detached job waited for console grace"
    );
    assert_tree_exited(&fixture, "cancel-running");
}

#[test]
fn runner_helper_windows_cancel_after_exit_preserves_result() {
    let fixture = Fixture::new();
    let program = fixture.program("hold");
    let options = fixture.options();
    let cancel = Cancel::new();
    let output = std::thread::scope(|scope| {
        let run =
            scope.spawn(|| runner::check_cancellable(program, Mode::Quick, &options, &cancel));
        wait_for("the helper's grandchild", || {
            fs::read_to_string(fixture.data.join("grandchild"))
                .is_ok_and(|text| text.parse::<u32>().is_ok())
        });
        wait_for("the direct helper to exit", || exited(started(&fixture).0));
        cancel.cancel();
        run.join().unwrap()
    });
    assert_eq!(
        output.unwrap().findings()[0].id().as_str(),
        "history.backlog"
    );
    assert_tree_exited(&fixture, "cancel-after-exit");
}

#[test]
fn runner_helper_windows_ctrl_break_then_job_cleanup() {
    if !isolated_console("runner_helper_windows_ctrl_break_then_job_cleanup") {
        return;
    }
    let fixture = Fixture::new();
    let mut limits = Limits::default();
    limits.fix_allowance = Duration::ZERO;
    limits.term_to_kill = Duration::from_millis(700);
    let options = fixture.options().with_limits(limits);
    let finding = finding(&fixture.program("probe"), &options);
    assert!(
        !started(&fixture).1,
        "a check inherited the caller's console"
    );
    runner::manifest(fixture.program("probe"), &options).unwrap();
    assert!(
        !started(&fixture).1,
        "manifest inherited the caller's console"
    );
    let start = Instant::now();
    assert_eq!(
        fix(&fixture.program("fix-break"), &finding, &options),
        Err(RunError::TimedOut)
    );
    let timeout = limits.fix_timeout(finding.fix().plan().unwrap().timeout_ms(), Mode::Quick);
    assert!(
        start.elapsed() >= timeout + limits.term_to_kill,
        "shortened the console grace"
    );
    assert!(
        started(&fixture).1,
        "a fix failed to inherit the caller's console"
    );
    assert!(
        fixture.data.join("direct-exited").exists(),
        "CTRL_BREAK did not reach the direct child; {}",
        recorded_job(&fixture),
    );
    assert_tree_exited(&fixture, "fix-break");
}

#[test]
fn runner_helper_windows_console_confirms_one_complete_line() {
    if !isolated_console("runner_helper_windows_console_confirms_one_complete_line") {
        return;
    }
    let fixture = Fixture::new();
    let options = fixture.options();
    let program = fixture.program("terminal");
    let finding = finding(&program, &options);
    let no_console = fix(&program, &finding, &options.clone().with_force_setsid(true)).unwrap();
    assert_eq!(no_console.code(), Some("no_terminal"));
    enqueue_keys("fixture-module\rwrong\r");
    assert_eq!(
        fix(&program, &finding, &options).unwrap().result(),
        FixResult::Fixed
    );
    assert_eq!(
        fix(&program, &finding, &options).unwrap().code(),
        Some("not_confirmed")
    );
}

#[test]
fn runner_helper_windows_console_partial_input_times_out() {
    if !isolated_console("runner_helper_windows_console_partial_input_times_out") {
        return;
    }
    let fixture = Fixture::new();
    let options = fixture.options();
    let program = fixture.program("terminal-partial");
    let finding = finding(&program, &options);
    enqueue_keys("fixture-module");
    let start = Instant::now();
    assert_eq!(
        fix(&program, &finding, &options).unwrap().code(),
        Some("not_confirmed")
    );
    assert!(start.elapsed() >= Duration::from_millis(200));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "pending console read held the fix open"
    );
}

fn isolated_console(test: &str) -> bool {
    const CHILD: &str = "CK_DOCTOR_WINDOWS_TEST_CONSOLE";
    if std::env::var_os(CHILD).is_some() {
        assert!(OpenOptions::new().read(true).open(r"\\.\CONIN$").is_ok());
        return true;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("windows::{test}"), "--nocapture"])
        .creation_flags(CREATE_NEW_CONSOLE)
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "console test was not run"
    );
    false
}

fn enqueue_keys(text: &str) {
    let con = OpenOptions::new().write(true).open(r"\\.\CONIN$").unwrap();
    let keys: Vec<_> = text
        .encode_utf16()
        .map(|unit| INPUT_RECORD {
            EventType: KEY_EVENT as u16,
            Event: INPUT_RECORD_0 {
                KeyEvent: KEY_EVENT_RECORD {
                    bKeyDown: 1,
                    wRepeatCount: 1,
                    wVirtualKeyCode: if unit == 13 { 13 } else { 0 },
                    wVirtualScanCode: 0,
                    uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
                    dwControlKeyState: 0,
                },
            },
        })
        .collect();
    let mut written = 0;
    // SAFETY: con holds the isolated console and all key records stay open for the call.
    assert_ne!(
        unsafe {
            WriteConsoleInputW(
                con.as_raw_handle(),
                keys.as_ptr(),
                keys.len() as u32,
                &mut written,
            )
        },
        0
    );
    assert_eq!(written as usize, keys.len());
}

fn started(fixture: &Fixture) -> (u32, bool) {
    let start = fs::read_to_string(fixture.data.join("start")).unwrap();
    let (pid, console) = start.split_once(' ').unwrap();
    (pid.parse().unwrap(), console.parse().unwrap())
}

fn assert_tree_exited(fixture: &Fixture, case: &str) {
    let grandchild: u32 = fs::read_to_string(fixture.data.join("grandchild"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        exited(started(fixture).0),
        "{case}: leader survived job cleanup; {}",
        recorded_job(fixture)
    );
    if let Some(process) = open_process(grandchild, SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION)
    {
        assert!(
            signaled(&process),
            "{case}: grandchild {grandchild} survived job cleanup; before cleanup: {}; after cleanup: {}",
            recorded_job(fixture),
            describe_failed_wait(&process),
        );
    }
}

fn recorded_job(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.data.join("grandchild-job"))
        .unwrap_or_else(|error| format!("no fixture job report: {error}"))
}

fn exited(pid: u32) -> bool {
    open_process(pid, SYNCHRONIZE).as_ref().is_none_or(signaled)
}

fn open_process(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<OwnedHandle> {
    // SAFETY: only a query handle is requested; the test never signals this PID.
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(ERROR_INVALID_PARAMETER as i32),
            "OpenProcess({pid}, access={access:#x})"
        );
        return None;
    }
    // SAFETY: OpenProcess returned a new owned handle for the test to release.
    Some(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn signaled(handle: &OwnedHandle) -> bool {
    // SAFETY: the process handle stays open across the nonblocking wait.
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
        WAIT_OBJECT_0 => true,
        WAIT_TIMEOUT => false,
        found => panic!("wait on fixture process returned {found}"),
    }
}

fn describe_failed_wait(handle: &OwnedHandle) -> String {
    let raw = handle.as_raw_handle();
    let [mut created, mut exited, mut kernel, mut user] = [FILETIME::default(); 4];
    // SAFETY: handle stays open and all four output times are writable.
    let times = unsafe { GetProcessTimes(raw, &mut created, &mut exited, &mut kernel, &mut user) };
    let birth = if times == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    };
    let membership = in_job(raw);
    // SAFETY: the pseudo-handle identifies this test process and needs no close.
    let caller = unsafe { GetCurrentProcess() };
    let caller_membership = in_job(caller);
    let status = exit_status(raw);
    // The nonblocking assertion already failed. Waiting on the same handle
    // now tells us whether teardown was still in flight without changing the verdict.
    // SAFETY: handle stays open across the bounded wait.
    let later = unsafe { WaitForSingleObject(raw, 500) };
    format!(
        "test_pid={} test_in_any_job={caller_membership:?} observed_birth={birth:?} \
         observed_in_any_job={membership:?} immediate_status={status:?} wait_500ms={later:#x} later_status={:?}",
        std::process::id(), exit_status(raw),
    )
}

fn in_job(handle: HANDLE) -> std::io::Result<bool> {
    let mut member = 0;
    // SAFETY: callers keep the process handle live, NULL asks about any job,
    // and member is writable.
    if unsafe { IsProcessInJob(handle, std::ptr::null_mut(), &mut member) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(member != 0)
    }
}

fn exit_status(handle: HANDLE) -> std::io::Result<u32> {
    let mut status = 0;
    // SAFETY: callers keep the process handle live and status is writable.
    if unsafe { GetExitCodeProcess(handle, &mut status) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(status)
    }
}
