use std::{fmt, io::ErrorKind};

use crate::DecodeError;

/// How the doctor program terminated unsuccessfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crash {
    ExitCode(i32),
    Signal(i32),
}

/// Failure of a doctor invocation. No variant includes text from the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunError {
    /// I/O error from spawning or waiting for the program.
    LaunchFailed(ErrorKind),
    HomeRelative,
    Crashed(Crash),
    TimedOut,
    InvalidOutput(DecodeError),
    NoDoctor,
}

impl RunError {
    /// Stable name of the failure, without the I/O kind, crash details or decode stage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LaunchFailed(_) => "launch_failed",
            Self::HomeRelative => "home_relative",
            Self::Crashed(_) => "crashed",
            Self::TimedOut => "timed_out",
            Self::InvalidOutput(_) => "invalid_output",
            Self::NoDoctor => "no_doctor",
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for RunError {}
