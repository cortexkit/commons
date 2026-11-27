use std::ffi::{c_void, OsStr};
use std::fs::OpenOptions;
use std::io::{self, PipeReader};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    GenerateConsoleCtrlEvent, GetConsoleProcessList, CTRL_BREAK_EVENT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    DETACHED_PROCESS, EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

pub(super) struct Child {
    job: OwnedHandle,
    process: OwnedHandle,
    pid: u32,
    shares_console: bool,
}

impl Child {
    pub(super) fn spawn(command: &Command, try_console: bool) -> io::Result<(Self, PipeReader)> {
        let job = new_job()?;
        let (read, write) = io::pipe()?;
        let null_device = OpenOptions::new().read(true).write(true).open(r"\\.\NUL")?;
        let stdout = inheritable(&write)?;
        let null_device = inheritable(&null_device)?;
        let handles = [stdout.as_raw_handle(), null_device.as_raw_handle()];
        let attributes = Attributes::new(&handles)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[1];
        startup.StartupInfo.hStdOutput = handles[0];
        startup.StartupInfo.hStdError = handles[1];
        startup.lpAttributeList = attributes.as_ptr();

        let program = command.get_program();
        let mut line = command_line(program, command.get_args())?;
        let path = std::path::Path::new(program);
        let explicit_program = (path.is_absolute() && path.extension().is_some())
            .then(|| program.encode_wide().chain([0]).collect::<Vec<_>>());
        let environment = environment(command)?;
        let mut console_pid = 0;
        // SAFETY: the API writes at most one PID into the live buffer. Its
        // return value is nonzero even if there are more attached processes.
        let share_console =
            try_console && unsafe { GetConsoleProcessList(&mut console_pid, 1) } != 0;
        let mut flags = CREATE_SUSPENDED
            | CREATE_NEW_PROCESS_GROUP
            | CREATE_UNICODE_ENVIRONMENT
            | EXTENDED_STARTUPINFO_PRESENT;
        if !share_console {
            flags |= DETACHED_PROCESS;
        }
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: both buffers are terminated, line is writable, info holds
        // two output handles, and the startup data and inheritable handles live
        // across the call. When no explicit application name is supplied, a
        // quoted argv[0] lets Win32 resolve it without splitting path spaces.
        check(unsafe {
            CreateProcessW(
                explicit_program
                    .as_ref()
                    .map_or(null(), |name| name.as_ptr()),
                line.as_mut_ptr(),
                null(),
                null(),
                1,
                flags,
                environment.as_ptr().cast(),
                null(),
                &startup.StartupInfo,
                &mut info,
            )
        })?;
        // SAFETY: CreateProcessW succeeded and transfers both handle owners.
        let (process, main_thread) = unsafe {
            (
                OwnedHandle::from_raw_handle(info.hProcess),
                OwnedHandle::from_raw_handle(info.hThread),
            )
        };
        // SAFETY: the new job and suspended direct child have live handles. No
        // child code can execute or start a descendant before assignment.
        if let Err(error) =
            check(unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) })
        {
            // SAFETY: assignment failed, so the still-suspended process isn't
            // covered by job cleanup; terminate only this owned direct child.
            if unsafe { TerminateProcess(process.as_raw_handle(), 1) } != 0 {
                // SAFETY: the owned direct child was just asked to terminate.
                unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) };
            }
            return Err(error);
        }
        let child = Self {
            job,
            process,
            pid: info.dwProcessId,
            shares_console: share_console,
        };
        // SAFETY: the new main thread is still suspended and its handle is live.
        if unsafe { ResumeThread(main_thread.as_raw_handle()) } == u32::MAX {
            let error = io::Error::last_os_error();
            child.terminate()?;
            return Err(error);
        }
        Ok((child, read))
    }

    pub(super) fn exit_code(&self) -> io::Result<Option<u32>> {
        // SAFETY: process is a live waitable handle. Waiting, rather than
        // checking for STILL_ACTIVE, also handles that value as an exit code.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: process stays open and code has room for the DWORD.
                check(unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) })?;
                Ok(Some(code))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(super) fn interrupt(&self, grace: Duration) {
        if self.shares_console {
            // SAFETY: pid is nonzero and identifies the new process group.
            // The open process handle keeps that process ID reserved until cleanup.
            unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.pid) };
            thread::sleep(grace);
        }
    }

    pub(super) fn terminate(self) -> io::Result<()> {
        super::job::terminate(self.job, self.process, self.pid)
    }
}

fn new_job() -> io::Result<OwnedHandle> {
    // SAFETY: null arguments request an unnamed, non-inheritable job.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the newly created handle has no other owner.
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: limits is initialized and has the layout of the requested class.
    check(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of_val(&limits) as u32,
        )
    })?;
    Ok(job)
}

fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn inheritable(source: &impl AsRawHandle) -> io::Result<OwnedHandle> {
    let mut copy = null_mut();
    // SAFETY: source is live in this process, and copy is writable.
    check(unsafe {
        let process = GetCurrentProcess();
        DuplicateHandle(
            process,
            source.as_raw_handle(),
            process,
            &mut copy,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    })?;
    // SAFETY: DuplicateHandle returned a new handle owned by the caller.
    Ok(unsafe { OwnedHandle::from_raw_handle(copy) })
}

struct Attributes<'a> {
    storage: Vec<usize>,
    _handles: &'a [HANDLE],
}

impl<'a> Attributes<'a> {
    fn new(handles: &'a [HANDLE]) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: a null list asks for the required allocation size.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        let ptr = storage.as_mut_ptr().cast();
        // SAFETY: storage is pointer-aligned and large enough for the list.
        check(unsafe { InitializeProcThreadAttributeList(ptr, 1, 0, &mut size) })?;
        let list = Self {
            storage,
            _handles: handles,
        };
        // SAFETY: list is initialized, and the handle array outlives it.
        check(unsafe {
            UpdateProcThreadAttribute(
                list.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast::<c_void>(),
                size_of_val(handles),
                null_mut(),
                null_mut(),
            )
        })?;
        Ok(list)
    }

    fn as_ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_ptr().cast_mut().cast()
    }
}

impl Drop for Attributes<'_> {
    fn drop(&mut self) {
        // SAFETY: the list was initialized and its allocation is still live.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

fn command_line<'a>(
    program: &OsStr,
    args: impl Iterator<Item = &'a OsStr>,
) -> io::Result<Vec<u16>> {
    const QUOTE: u16 = b'"' as u16;
    const SPACE: u16 = b' ' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let mut line = vec![QUOTE];
    // Windows treats argv[0] specially: quotes protect spaces in the path;
    // backslashes have no escape meaning in this first argument.
    for unit in program.encode_wide() {
        if matches!(unit, 0 | QUOTE) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        line.push(unit);
    }
    line.push(QUOTE);
    for arg in args {
        line.push(SPACE);
        line.push(QUOTE);
        // A quote after N backslashes needs 2N+1 backslashes; closing the
        // argument needs 2N. Other runs pass through unchanged.
        let mut backslashes = 0;
        for unit in arg.encode_wide() {
            match unit {
                0 => return Err(io::ErrorKind::InvalidInput.into()),
                BACKSLASH => backslashes += 1,
                _ => {
                    let count = if unit == QUOTE {
                        2 * backslashes + 1
                    } else {
                        backslashes
                    };
                    line.extend(std::iter::repeat_n(BACKSLASH, count));
                    line.push(unit);
                    backslashes = 0;
                }
            }
        }
        line.extend(std::iter::repeat_n(BACKSLASH, 2 * backslashes));
        line.push(QUOTE);
    }
    line.push(0);
    Ok(line)
}

fn environment(command: &Command) -> io::Result<Vec<u16>> {
    let mut entries: Vec<_> = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_ascii_uppercase(), key, value)))
        .collect();
    // Windows expects a case-insensitively sorted block of NUL-terminated
    // entries. Keys in the doctor allowlist are ASCII; retain their spelling.
    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut block = Vec::new();
    for (_, key, value) in entries {
        let entry = key
            .encode_wide()
            .chain([b'=' as u16])
            .chain(value.encode_wide());
        for unit in entry {
            if unit == 0 {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            block.push(unit);
        }
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}
