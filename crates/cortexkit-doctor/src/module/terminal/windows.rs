use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::ptr::null;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Instant;

use windows_sys::Win32::System::Console::{ReadConsoleW, WriteConsoleW};
use windows_sys::Win32::System::IO::CancelSynchronousIo;

use super::{Terminal, TerminalRead};

pub(in crate::module) fn open() -> Option<Box<dyn Terminal>> {
    let input = OpenOptions::new().read(true).open(r"\\.\CONIN$").ok()?;
    let output = OpenOptions::new().write(true).open(r"\\.\CONOUT$").ok()?;
    Some(Box::new(Console { input, output }))
}

struct Console {
    input: File,
    output: File,
}

impl Terminal for Console {
    fn write_prompt(&mut self, text: &str) -> io::Result<()> {
        let text: Vec<_> = text.replace('\n', "\r\n").encode_utf16().collect();
        let mut remaining = text.as_slice();
        while !remaining.is_empty() {
            let count = remaining.len().min(u32::MAX as usize) as u32;
            let mut written = 0;
            // SAFETY: output is live and remaining contains count UTF-16 units.
            let ok = unsafe {
                WriteConsoleW(
                    self.output.as_raw_handle(),
                    remaining.as_ptr().cast(),
                    count,
                    &mut written,
                    null(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            remaining = &remaining[written as usize..];
        }
        Ok(())
    }

    fn read_line(&mut self, deadline: Instant) -> io::Result<TerminalRead> {
        let input = self.input.try_clone()?;
        let (send, receive) = mpsc::sync_channel(1);
        // A readable console can still have only a partial line. ReadConsoleW
        // preserves the console's line editing, but must not block the deadline.
        let reader = thread::Builder::new().spawn(move || {
            let _ = send.send(read_line(&input, deadline));
        })?;
        let result = receive.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        match result {
            Ok(result) if Instant::now() < deadline => result,
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::other("console reader stopped")),
            _ => {
                // SAFETY: the join handle owns the reader thread handle;
                // cancellation is best effort and never delays process exit.
                unsafe { CancelSynchronousIo(reader.as_raw_handle()) };
                Ok(TerminalRead::TimedOut)
            }
        }
    }
}

fn read_line(input: &File, deadline: Instant) -> io::Result<TerminalRead> {
    // Valid input is at most a 64-character ASCII module id and CRLF.
    let mut line = [0u16; 66];
    let mut len = 0;
    while len < line.len() && Instant::now() < deadline {
        let tail = &mut line[len..];
        let mut read = 0;
        // SAFETY: input is live and tail and read are writable for this call.
        let ok = unsafe {
            ReadConsoleW(
                input.as_raw_handle(),
                tail.as_mut_ptr().cast(),
                tail.len() as u32,
                &mut read,
                null(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if read == 0 {
            return Ok(TerminalRead::Eof);
        }
        if let Some(lf) = tail[..read as usize].iter().position(|&unit| unit == 10) {
            return Ok(TerminalRead::Line(
                String::from_utf16_lossy(&line[..len + lf + 1]).into_bytes(),
            ));
        }
        len += read as usize;
    }
    if len == line.len() {
        Ok(TerminalRead::Line(
            String::from_utf16_lossy(&line).into_bytes(),
        ))
    } else {
        Ok(TerminalRead::TimedOut)
    }
}
