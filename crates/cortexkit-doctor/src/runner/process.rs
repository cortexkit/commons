use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{ChildStdout, Command};
use std::thread;
use std::time::{Duration, Instant};

use super::{Crash, Limits, RunError};
use crate::{DecodeError, MAX_OUTPUT_BYTES};

pub(super) fn run(
    command: &mut Command,
    try_foreground: bool,
    is_manifest: bool,
    timeout: Duration,
    limits: Limits,
) -> Result<Vec<u8>, RunError> {
    let foreground = try_foreground.then(Foreground::open).flatten();
    let tty = foreground.as_ref().map(|guard| guard.tty.as_raw_fd());
    // SAFETY: the hook uses only async-signal-safe operations and keeps the
    // inherited descriptor open until exec. The guard restores the parent's
    // foreground group even if the hook succeeded but exec failed.
    unsafe { command.pre_exec(move || start_group(tty)) };
    let start = Instant::now();
    let mut child = command.spawn().map_err(|_| RunError::LaunchFailed)?;
    let pgid = child.id() as libc::pid_t;
    let end = collect(child.stdout.take(), pgid, start, timeout, limits.exit_grace)?;

    // Keep the child waitable until after the last group signal. If waitid
    // failed (for example because another waiter reaped it), do not signal:
    // the numeric ID could already belong to a different process group.
    if matches!(end, ReadEnd::TimedOut) {
        signal_group(pgid, libc::SIGTERM);
        thread::sleep(limits.term_to_kill);
    }
    signal_group(pgid, libc::SIGKILL);
    let status = child.wait();

    // A waitid-observed status is immutable. Reaping it after cleanup cannot
    // turn a successful exit into a signal crash, or a timeout into a crash.
    let ReadEnd::Exited { bytes, read_failed } = end else {
        return match end {
            ReadEnd::TooLarge => Err(RunError::InvalidOutput(DecodeError::TooLarge)),
            ReadEnd::TimedOut => Err(RunError::TimedOut),
            ReadEnd::Exited { .. } => unreachable!(),
        };
    };
    let status = status.map_err(|_| RunError::LaunchFailed)?;
    if let Some(signal) = status.signal() {
        Err(RunError::Crashed(Crash::Signal(signal)))
    } else if !status.success() {
        if is_manifest && bytes.is_empty() {
            Err(RunError::NoDoctor)
        } else {
            Err(RunError::Crashed(Crash::ExitCode(
                status.code().expect("exited without a signal"),
            )))
        }
    } else if read_failed {
        Err(RunError::InvalidOutput(DecodeError::NotJson))
    } else {
        Ok(bytes)
    }
}

enum ReadEnd {
    Exited { bytes: Vec<u8>, read_failed: bool },
    TooLarge,
    TimedOut,
}

fn collect(
    mut stdout: Option<ChildStdout>,
    pid: libc::pid_t,
    start: Instant,
    timeout: Duration,
    exit_grace: Duration,
) -> Result<ReadEnd, RunError> {
    let mut exit = None;
    let mut bytes = Vec::new();
    let mut read_failed = false;
    let mut buffer = [0; 16 * 1024];
    loop {
        if exit.is_none() && exited(pid).map_err(|_| RunError::LaunchFailed)? {
            exit = Some(Instant::now());
        }
        let wait = if let Some(exit) = exit {
            let left = exit_grace.saturating_sub(exit.elapsed());
            if stdout.is_none() || left.is_zero() {
                return Ok(ReadEnd::Exited { bytes, read_failed });
            }
            left
        } else {
            let left = timeout.saturating_sub(start.elapsed());
            if left.is_zero() {
                return Ok(ReadEnd::TimedOut);
            }
            // A grandchild can keep stdout open after the direct child exits.
            // Sample waitid even when no bytes or EOF wake poll.
            left.min(Duration::from_millis(10))
        };

        let mut fd = libc::pollfd {
            fd: stdout.as_ref().map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = wait.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as i32;
        // SAFETY: fd is one initialized pollfd; stdout owns its fd while open.
        match unsafe { libc::poll(&mut fd, 1, ms) } {
            0 => continue,
            -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
            -1 => {
                stdout = None;
                read_failed = true;
                continue;
            }
            _ => {}
        }
        if exit.is_none() && start.elapsed() >= timeout {
            // Check for exit before deciding whether the deadline ends reads.
            continue;
        }
        if exit.is_some_and(|exit| exit.elapsed() >= exit_grace) {
            continue;
        }

        let Some(pipe) = stdout.as_mut() else {
            continue;
        };
        let cap = buffer.len().min(MAX_OUTPUT_BYTES + 1 - bytes.len());
        match pipe.read(&mut buffer[..cap]) {
            Ok(0) => stdout = None,
            Ok(n) if bytes.len() + n > MAX_OUTPUT_BYTES => return Ok(ReadEnd::TooLarge),
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                stdout = None;
                read_failed = true;
            }
        }
    }
}

fn exited(pid: libc::pid_t) -> io::Result<bool> {
    // SAFETY: POSIX allows a zeroed siginfo_t for WNOHANG. The initialized
    // buffer remains valid across each call, and pid names our direct child.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    retry_eintr(|| {
        // SAFETY: waitid writes to info and leaves the direct child waitable.
        unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        }
    })?;
    // An empty notification has code zero. WEXITED requests only exit events.
    Ok(matches!(
        info.si_code,
        libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED
    ))
}

fn signal_group(pgid: libc::pid_t, signal: libc::c_int) {
    debug_assert!(pgid > 0);
    // SAFETY: negative pgid selects the managed group; the positive ID is
    // still reserved by the unreaped direct child. An empty group needs no signal.
    unsafe { libc::kill(-pgid, signal) };
}

fn start_group(tty: Option<RawFd>) -> io::Result<()> {
    if let Some(fd) = tty {
        // SAFETY: zero IDs select this child and its own PID for its new group.
        retry_eintr(|| unsafe { libc::setpgid(0, 0) })?;
        set_foreground(fd, current_group())
    } else {
        // SAFETY: setsid takes no arguments and affects only the calling child.
        retry_eintr(|| unsafe { libc::setsid() })
    }
}

struct Foreground {
    tty: File,
    group: libc::pid_t,
}

impl Foreground {
    fn open() -> Option<Self> {
        let tty = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
            .open("/dev/tty")
            .ok()?;
        let group = current_group();
        // SAFETY: tty owns a live descriptor.
        let foreground = unsafe { libc::tcgetpgrp(tty.as_raw_fd()) };
        (foreground == group).then(|| Self { tty, group })
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        let _ = set_foreground(self.tty.as_raw_fd(), self.group);
    }
}

fn current_group() -> libc::pid_t {
    // SAFETY: getpgrp takes no arguments and returns the caller's group.
    unsafe { libc::getpgrp() }
}

fn set_foreground(fd: RawFd, group: libc::pid_t) -> io::Result<()> {
    // SAFETY: Both sigsets have writable storage. All calls here are
    // async-signal-safe so the same operation can run between fork and exec.
    unsafe {
        let mut blocked = std::mem::zeroed();
        let mut original = std::mem::zeroed();
        libc::sigemptyset(&mut blocked);
        libc::sigaddset(&mut blocked, libc::SIGTTOU);
        let rc = libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut original);
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }

        let result = retry_eintr(|| libc::tcsetpgrp(fd, group));
        let rc = libc::pthread_sigmask(libc::SIG_SETMASK, &original, std::ptr::null_mut());
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
        result
    }
}

fn retry_eintr(mut call: impl FnMut() -> libc::c_int) -> io::Result<()> {
    loop {
        if call() != -1 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}
