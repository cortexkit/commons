use std::io;
use std::time::Instant;

/// One bounded read from the confirmation terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TerminalRead {
    /// Include the line ending so the helper can reject unterminated input.
    Line(Vec<u8>),
    Eof,
    TimedOut,
}

/// Terminal seam for testing confirmation without a controlling terminal.
pub trait Terminal {
    fn write_prompt(&mut self, text: &str) -> io::Result<()>;
    fn read_line(&mut self, deadline: Instant) -> io::Result<TerminalRead>;
}

#[cfg(unix)]
#[path = "terminal/unix.rs"]
mod native;
#[cfg(windows)]
#[path = "terminal/windows.rs"]
mod native;

pub(super) use native::open;
