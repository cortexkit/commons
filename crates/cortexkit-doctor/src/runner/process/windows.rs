#[path = "windows/spawn.rs"]
mod spawn;

use std::io::{self, PipeReader, Read};
use std::os::windows::io::AsRawHandle;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

use super::{Cancel, Crash, Limits, RunError};
use crate::{DecodeError, MAX_OUTPUT_BYTES};
use spawn::Child;

pub(super) fn run(
    command: &mut Command,
    try_console: bool,
    subcommand: &str,
    timeout: Duration,
    limits: Limits,
    cancel: Option<&Cancel>,
) -> Result<Vec<u8>, RunError> {
    if cancel.is_some_and(Cancel::is_cancelled) {
        return Err(RunError::Cancelled);
    }
    let start = Instant::now();
    let (child, pipe) = Child::spawn(command, try_console).map_err(launch_failed)?;
    let result = collect(&child, pipe, start, timeout, limits.exit_grace, cancel);
    if matches!(result, Err(RunError::TimedOut | RunError::Cancelled)) {
        child.interrupt(limits.term_to_kill);
    }
    child.terminate();

    let ReadEnd {
        bytes,
        read_failed,
        code,
    } = result?;
    if code != 0 {
        if bytes.is_empty() && matches!((subcommand, code), ("manifest", _) | ("check", 2)) {
            Err(RunError::NoDoctor)
        } else {
            Err(RunError::Crashed(Crash::ExitCode(code as i32)))
        }
    } else if read_failed {
        Err(RunError::InvalidOutput(DecodeError::NotJson))
    } else {
        Ok(bytes)
    }
}

fn launch_failed(error: io::Error) -> RunError {
    RunError::LaunchFailed(error.kind())
}

struct ReadEnd {
    bytes: Vec<u8>,
    read_failed: bool,
    code: u32,
}

fn collect(
    child: &Child,
    pipe: PipeReader,
    start: Instant,
    timeout: Duration,
    exit_grace: Duration,
    cancel: Option<&Cancel>,
) -> Result<ReadEnd, RunError> {
    let mut pipe = Some(pipe);
    let mut bytes = Vec::new();
    let mut read_failed = false;
    let mut buffer = [0; 16 * 1024];
    let mut exit = None;
    loop {
        if exit.is_none() {
            let cancelled = cancel.is_some_and(Cancel::is_cancelled);
            if let Some(code) = child.exit_code().map_err(launch_failed)? {
                exit = Some((Instant::now(), code));
            } else if cancelled {
                return Err(RunError::Cancelled);
            }
        }
        let remaining = if let Some((ended, code)) = exit {
            let left = exit_grace.saturating_sub(ended.elapsed());
            if pipe.is_none() || left.is_zero() {
                return Ok(ReadEnd {
                    bytes,
                    read_failed,
                    code,
                });
            }
            left
        } else {
            let left = timeout.saturating_sub(start.elapsed());
            if left.is_zero() {
                return Err(RunError::TimedOut);
            }
            left
        };

        // Poll in short intervals so a writer keeping stdout open without
        // writing cannot hide either a direct child exit or cancellation.
        let sleep = remaining.min(Duration::from_millis(5));
        let Some(reader) = pipe.as_mut() else {
            thread::sleep(sleep);
            continue;
        };
        match available(reader) {
            Ok(Some(0)) => thread::sleep(sleep),
            Ok(Some(available)) => {
                // This is the only reader. A read bounded by bytes already
                // queued cannot block, including when the writer closes.
                let cap = available
                    .min(buffer.len())
                    .min(MAX_OUTPUT_BYTES + 1 - bytes.len());
                match reader.read(&mut buffer[..cap]) {
                    Ok(0) => pipe = None,
                    Ok(n) if bytes.len() + n > MAX_OUTPUT_BYTES => {
                        return Err(RunError::InvalidOutput(DecodeError::TooLarge));
                    }
                    Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                    Err(_) => {
                        pipe = None;
                        read_failed = true;
                    }
                }
            }
            Ok(None) => pipe = None,
            Err(_) => {
                pipe = None;
                read_failed = true;
            }
        }
    }
}

fn available(pipe: &PipeReader) -> io::Result<Option<usize>> {
    let mut bytes = 0;
    // SAFETY: pipe stays open; bytes is writable and optional pointers are null.
    let ok = unsafe {
        PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut bytes,
            std::ptr::null_mut(),
        )
    };
    if ok != 0 {
        return Ok(Some(bytes as usize));
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error().map(|code| code as u32) {
        Some(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED) => Ok(None),
        _ => Err(error),
    }
}
