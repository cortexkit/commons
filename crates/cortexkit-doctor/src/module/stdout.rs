use std::fs::{File, OpenOptions};
use std::io;

#[cfg(unix)]
pub(super) fn take() -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    fn retry_fd(mut call: impl FnMut() -> libc::c_int) -> io::Result<libc::c_int> {
        loop {
            let fd = call();
            if fd >= 0 {
                return Ok(fd);
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
    }

    // SAFETY: fcntl creates a new owned descriptor without changing fd 1.
    let saved = retry_fd(|| unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD_CLOEXEC, 3) })?;
    // SAFETY: saved came from F_DUPFD_CLOEXEC and has no other owner.
    let saved = unsafe { File::from_raw_fd(saved) };
    let null = OpenOptions::new().write(true).open("/dev/null")?;
    // SAFETY: null stays open; dup2 atomically replaces process stdout.
    retry_fd(|| unsafe { libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO) })?;
    Ok(saved)
}

#[cfg(windows)]
pub(super) fn take() -> io::Result<File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle};
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS};
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    extern "C" {
        fn _dup2(source: libc::c_int, target: libc::c_int) -> libc::c_int;
    }

    let mut saved = null_mut();
    // SAFETY: both uses of the pseudo-handle refer to this process. The
    // standard handle stays valid during early CLI dispatch; saved is writable.
    let ok = unsafe {
        let process = GetCurrentProcess();
        DuplicateHandle(
            process,
            GetStdHandle(STD_OUTPUT_HANDLE),
            process,
            &mut saved,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: DuplicateHandle returned a new non-inheritable owned handle.
    let saved = unsafe { File::from_raw_handle(saved) };
    let null = OpenOptions::new().write(true).open(r"\\.\NUL")?;

    // SetStdHandle redirects Rust and Win32 writes; the C runtime also keeps
    // its own descriptor 1, which buffered C output can use even at exit.
    let name = [b'N' as u16, b'U' as u16, b'L' as u16, 0];
    // SAFETY: name is a terminated UTF-16 device name and no mode is needed.
    let fd = unsafe { libc::wopen(name.as_ptr(), libc::O_WRONLY | libc::O_NOINHERIT) };
    if fd < 0 {
        return Err(io::Error::other("open the CRT null descriptor"));
    }
    if fd != 1 {
        // SAFETY: the CRT just opened fd, and descriptor 1 is CRT stdout.
        let result = unsafe { _dup2(fd, 1) };
        // SAFETY: dup2 does not close the original descriptor and we own it.
        unsafe { libc::close(fd) };
        if result < 0 {
            return Err(io::Error::other("redirect CRT stdout"));
        }
    }
    // SAFETY: null owns a live handle, which must stay open for future writes.
    if unsafe { SetStdHandle(STD_OUTPUT_HANDLE, null.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _ = null.into_raw_handle();
    Ok(saved)
}
