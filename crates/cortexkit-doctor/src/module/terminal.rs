use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
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

pub(super) fn open() -> Option<Box<dyn Terminal>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
        .open("/dev/tty")
        .ok()?;
    Some(Box::new(Tty(file)))
}

struct Tty(File);

impl Terminal for Tty {
    fn write_prompt(&mut self, text: &str) -> io::Result<()> {
        self.0.write_all(text.as_bytes())
    }

    fn read_line(&mut self, deadline: Instant) -> io::Result<TerminalRead> {
        // Valid input fits a 64-byte module id and CRLF. Never buffer a long line.
        let mut line = [0; 66];
        let mut len = 0;
        let mut fd = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        while len < line.len() {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(TerminalRead::TimedOut);
            };
            let ms = (remaining.as_millis() + 1).min(libc::c_int::MAX as u128) as libc::c_int;
            // SAFETY: fd is initialized, points to one pollfd and the File stays open.
            match unsafe { libc::poll(&mut fd, 1, ms) } {
                -1 => {
                    let err = io::Error::last_os_error();
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(err);
                }
                0 => continue,
                _ => {}
            }
            if Instant::now() >= deadline {
                return Ok(TerminalRead::TimedOut);
            }
            match self.0.read(&mut line[len..]) {
                Ok(0) => return Ok(TerminalRead::Eof),
                Ok(n) => {
                    if let Some(lf) = line[len..len + n].iter().position(|&b| b == b'\n') {
                        return Ok(TerminalRead::Line(line[..len + lf + 1].to_vec()));
                    }
                    len += n;
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(TerminalRead::Line(line.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    use super::*;

    #[test]
    fn module_helper_terminal_poll_reads_one_line_without_waiting_for_eof() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut terminal = Tty(File::from(OwnedFd::from(reader)));
        writer.write_all(b"fixture-module\r\nextra\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            terminal.read_line(deadline).unwrap(),
            TerminalRead::Line(b"fixture-module\r\n".to_vec()),
        );
    }

    #[test]
    fn module_helper_terminal_poll_obeys_deadline_on_partial_input() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut terminal = Tty(File::from(OwnedFd::from(reader)));
        writer.write_all(b"fixture-module").unwrap();
        let deadline = Instant::now() + Duration::from_millis(15);
        assert_eq!(
            terminal.read_line(deadline).unwrap(),
            TerminalRead::TimedOut
        );
        drop(writer);
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(terminal.read_line(deadline).unwrap(), TerminalRead::Eof);
    }
}
