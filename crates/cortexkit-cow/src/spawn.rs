//! One process-wide privacy trampoline for the stateless isolation backends.

#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    io,
    path::PathBuf,
    process::{Child, Output, Stdio},
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use subc_os::privacy_identity;
use subc_os::privacy_identity::DisclaimedCommand;

#[cfg(target_os = "macos")]
static TRAMPOLINE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

const CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);

/// Configure the process-wide executable used to disclaim child processes.
///
/// On macOS, call this once before any git, btrfs, or ZFS operation. The path
/// must name the caller's executable, which must call
/// [`subc_os::privacy_identity::trampoline_main`] at startup, before starting
/// threads or an async runtime. This function probes the executable before
/// storing it; an invalid trampoline leaves configuration unset. A second
/// successful configuration is rejected with [`io::ErrorKind::AlreadyExists`].
/// Without configuration, spawn-using operations return an error on macOS;
/// they never launch children with inherited privacy permissions. Native
/// filesystem clones and non-git recursive copies do not need a trampoline.
/// On other platforms the path is ignored and this function is a no-op.
pub fn configure_spawn_trampoline(path: impl Into<PathBuf>) -> io::Result<()> {
    let path = path.into();
    #[cfg(target_os = "macos")]
    {
        if TRAMPOLINE.get().is_some() {
            return Err(already_configured());
        }
        privacy_identity::probe(&path, Instant::now() + CONFIRM_TIMEOUT)
            .map_err(io::Error::other)?;
        TRAMPOLINE.set(path).map_err(|_| already_configured())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn already_configured() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "spawn trampoline already configured",
    )
}

pub(crate) fn command(program: impl Into<OsString>) -> io::Result<DisclaimedCommand> {
    #[cfg(target_os = "macos")]
    let trampoline = TRAMPOLINE
        .get()
        .cloned()
        .ok_or_else(|| io::Error::other("macOS child spawn requires configure_spawn_trampoline"))?;
    #[cfg(not(target_os = "macos"))]
    let trampoline = PathBuf::new();
    let program = program.into();
    #[cfg(target_os = "macos")]
    {
        // The trampoline reports exec refusal separately from the parent's
        // spawn errno. Preserve the backends' NotFound/unavailable classification
        // for missing CLIs instead of misclassifying them as privacy failures.
        // Darwin's exec search uses the system default when PATH is unset.
        let path =
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into());
        if !std::env::split_paths(&path).any(|dir| dir.join(&program).exists()) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} not on PATH", program.to_string_lossy()),
            ));
        }
    }
    Ok(DisclaimedCommand::new(trampoline, program))
}

pub(crate) fn spawn(builder: DisclaimedCommand) -> io::Result<Child> {
    let (mut command, confirmation) = builder.into_command()?;
    let deadline = Instant::now() + CONFIRM_TIMEOUT;
    let mut child = command.spawn()?;
    // The command owns the parent's acknowledgement writer. Release it before
    // confirming exec, otherwise the reader cannot observe EOF.
    drop(command);
    if let Err(error) = confirmation.confirm(deadline) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other(error));
    }
    Ok(child)
}

pub(crate) fn output(mut builder: DisclaimedCommand) -> io::Result<Output> {
    builder.stdout(Stdio::piped()).stderr(Stdio::piped());
    spawn(builder)?.wait_with_output()
}

pub(crate) async fn output_async(mut builder: DisclaimedCommand) -> io::Result<Output> {
    builder.stdout(Stdio::piped()).stderr(Stdio::piped());
    let (mut command, confirmation) = builder.into_tokio_command()?;
    let deadline = Instant::now() + CONFIRM_TIMEOUT;
    let mut child = command.spawn()?;
    drop(command);
    // Confirmation reads a blocking pipe. Never block the async runtime worker.
    let result = tokio::task::spawn_blocking(move || confirmation.confirm(deadline))
        .await
        .map_err(io::Error::other)
        .and_then(|result| result.map_err(io::Error::other));
    if let Err(error) = result {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(error);
    }
    child.wait_with_output().await
}
