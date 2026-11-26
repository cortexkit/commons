use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{finding, fix, runner, FixResult, Fixture, Limits, RunError};

#[test]
fn runner_helper_foreground_pty() {
    match std::env::var(PTY_CHILD).as_deref() {
        Ok("foreground") => return exercise_foreground(),
        Ok("background") => return exercise_background(),
        _ => {}
    }

    let (mut master, slave) = pty();
    let fd = slave.as_raw_fd();
    let mut command = pty_child("foreground");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: the slave is live through spawn. The hook uses only
    // async-signal-safe calls to create a session and acquire its tty.
    unsafe {
        command.pre_exec(move || {
            #[cfg(target_vendor = "apple")]
            let request = libc::c_ulong::from(libc::TIOCSCTTY);
            #[cfg(not(target_vendor = "apple"))]
            let request = libc::TIOCSCTTY;
            if libc::setsid() == -1
                || libc::ioctl(fd, request, 0) == -1
                || libc::tcsetpgrp(fd, libc::getpgrp()) == -1
            {
                return Err(io::Error::last_os_error());
            }
            let mut empty = std::mem::zeroed();
            libc::sigemptyset(&mut empty);
            let rc = libc::pthread_sigmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut());
            if rc != 0 {
                return Err(io::Error::from_raw_os_error(rc));
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let prompt = read_prompt(&mut master);
    if prompt {
        master.write_all(b"fixture-module\n").unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            // Closing the master also wakes a session leader stalled in tty
            // teardown, where killing it alone may not unblock the parent.
            drop(master);
            drop(slave);
            let _ = child.kill();
            break;
        }
        // Drain the typed line's echo. Darwin waits for terminal output to
        // drain before the session leader can exit.
        let mut fd = libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: fd points to one initialized pollfd and master is open.
        if unsafe { libc::poll(&mut fd, 1, 25) } > 0 && fd.revents & libc::POLLIN != 0 {
            let _ = master.read(&mut [0; 512]);
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success() && prompt,
        "prompt seen: {prompt}; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const PTY_CHILD: &str = "CK_DOCTOR_RUNNER_PTY_CHILD";

fn pty_child(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "terminal::runner_helper_foreground_pty",
            "--nocapture",
        ])
        .env(PTY_CHILD, mode);
    command
}

fn exercise_foreground() {
    let tty = File::open("/dev/tty").unwrap();
    // SAFETY: both are pure getters for the process's group and session.
    let (group, session) = unsafe { (libc::getpgrp(), libc::getsid(0)) };
    let assert_reclaimed = || {
        // SAFETY: the tty descriptor is live for the assertion.
        assert_eq!(unsafe { libc::tcgetpgrp(tty.as_raw_fd()) }, group);
    };
    assert_reclaimed();
    let fixture = Fixture::new();
    let probe = fixture.program("probe");
    let terminal = fixture.program("terminal");
    let mut limits = Limits::default();
    limits.fix_allowance = Duration::from_millis(100);
    limits.term_to_kill = Duration::from_millis(50);
    let options = fixture.options().with_limits(limits);

    let finding = finding(&probe, &options);
    for start in [fixture.start(), {
        runner::manifest(&probe, &options).unwrap();
        fixture.start()
    }] {
        assert!(!start.tty, "check/manifest inherited the controlling tty");
        assert_eq!(start.pid, start.sid);
        assert_eq!(start.pid, start.pgid);
    }
    assert_reclaimed();

    let output = fix(&terminal, &finding, &options).unwrap();
    assert_eq!(output.result(), FixResult::Fixed);
    let start = fixture.start();
    assert!(start.tty);
    assert!(!start.blocked, "child inherited a blocked signal");
    assert_eq!(start.pid, start.pgid);
    assert_eq!(start.sid, session);
    assert_reclaimed();

    assert_eq!(
        fix(&fixture.program("fix-hang"), &finding, &options),
        Err(RunError::TimedOut)
    );
    assert!(fixture.start().tty);
    assert_reclaimed();

    for detached in [
        options.clone().with_force_setsid(true),
        #[cfg(feature = "conformance")]
        options
            .clone()
            .with_conformance_fix_marker()
            .with_force_setsid(false),
    ] {
        let output = fix(&terminal, &finding, &detached).unwrap();
        assert_eq!(output.result(), FixResult::Refused);
        assert_eq!(output.code(), Some("no_terminal"));
        let start = fixture.start();
        assert!(!start.tty);
        assert_eq!(start.pid, start.sid);
        assert_reclaimed();
    }

    let bad = fixture.data.join("missing-interpreter");
    fs::write(&bad, "#!/no-such-fixture-interpreter\n").unwrap();
    fs::set_permissions(&bad, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        fix(&bad, &finding, &options),
        Err(RunError::LaunchFailed(std::io::ErrorKind::NotFound))
    );
    assert_reclaimed();

    let background = pty_child("background").process_group(0).output().unwrap();
    assert!(
        background.status.success(),
        "{}",
        String::from_utf8_lossy(&background.stderr)
    );
    assert_reclaimed();
}

fn exercise_background() {
    let tty = File::open("/dev/tty").unwrap();
    // SAFETY: both getter calls are valid while tty owns the descriptor.
    unsafe { assert_ne!(libc::tcgetpgrp(tty.as_raw_fd()), libc::getpgrp()) };
    let fixture = Fixture::new();
    let mut limits = Limits::default();
    limits.fix_allowance = Duration::from_millis(100);
    limits.term_to_kill = Duration::from_millis(50);
    let options = fixture.options().with_limits(limits);
    let finding = finding(&fixture.program("probe"), &options);
    let output = fix(&fixture.program("terminal"), &finding, &options).unwrap();
    assert_eq!(output.result(), FixResult::Refused);
    assert_eq!(output.code(), Some("no_terminal"));
    let start = fixture.start();
    assert!(!start.tty);
    assert_eq!(start.sid, start.pid);
}

fn pty() -> (File, File) {
    // SAFETY: openpty writes two fresh descriptors, we transfer ownership once.
    unsafe {
        let (mut master, mut slave) = (-1, -1);
        assert_eq!(
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            0
        );
        let files = (File::from_raw_fd(master), File::from_raw_fd(slave));
        for fd in [master, slave] {
            assert_eq!(libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC), 0);
        }
        files
    }
}

fn read_prompt(master: &mut File) -> bool {
    let phrase = b"Type fixture-module to confirm: ";
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bytes = Vec::new();
    let mut buf = [0; 512];
    let mut fd = libc::pollfd {
        fd: master.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        // SAFETY: fd points to an initialized pollfd and master is live.
        match unsafe { libc::poll(&mut fd, 1, remaining.as_millis() as i32 + 1) } {
            -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
            n if n <= 0 => break,
            _ => {}
        }
        match master.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
        }
        if bytes.windows(phrase.len()).any(|window| window == phrase) {
            return true;
        }
    }
    false
}
