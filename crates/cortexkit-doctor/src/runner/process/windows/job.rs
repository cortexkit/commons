use std::io::{self, Write};
use std::mem::{offset_of, size_of, size_of_val};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null_mut;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    IsProcessInJob, JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
    QueryInformationJobObject, TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_BASIC_PROCESS_ID_LIST,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

const CLEANUP_LIMIT: Duration = Duration::from_secs(5);

pub(super) fn terminate(job: OwnedHandle, leader: OwnedHandle, leader_pid: u32) -> io::Result<()> {
    let deadline = Instant::now() + CLEANUP_LIMIT;
    let mut members = vec![leader];
    let captured = capture_members(&job, leader_pid, &mut members, deadline);

    // SAFETY: this is the runner's own job, not the caller's enclosing job.
    if unsafe { TerminateJobObject(job.as_raw_handle(), 1) } == 0 {
        let error = io::Error::last_os_error();
        let active = active_processes(&job).ok();
        // Last-handle close requests termination too, but it leaves no handle
        // to query. Wait on the handles we have and report the unverified job.
        drop(job);
        let _ = drain(None, &mut members, deadline);
        return failed("terminate", error, active, members.len());
    }

    let mut active = None;
    let waited = drain(Some((&job, &mut active)), &mut members, deadline);
    if let Err(error) = waited {
        failed("wait", error, active, members.len())
    } else if let Err(error) = captured {
        failed("capture members", error, active, members.len())
    } else {
        Ok(())
    }
}

fn failed(
    stage: &str,
    error: io::Error,
    active: Option<u32>,
    pending_handles: usize,
) -> io::Result<()> {
    let count = active.map_or_else(|| "unknown".to_owned(), |n| n.to_string());
    let _ = writeln!(
        io::stderr().lock(),
        "doctor: Windows job cleanup could not {stage}: {error} \
         (last active process count: {count}; unsignaled process handles: {pending_handles})",
    );
    Err(error)
}

fn capture_members(
    job: &OwnedHandle,
    leader_pid: u32,
    members: &mut Vec<OwnedHandle>,
    deadline: Instant,
) -> io::Result<()> {
    for pid in member_ids(job, deadline)? {
        if pid == leader_pid {
            continue;
        }
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        // SAFETY: this only opens a handle for querying and waiting. The PID
        // may have been recycled, so membership must be checked on that handle.
        let raw = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        if raw.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                continue; // The listed process is already gone.
            }
            return Err(error);
        }
        // SAFETY: OpenProcess returned a new owned handle.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut in_job = 0;
        // SAFETY: process and job stay open and in_job is writable.
        if unsafe { IsProcessInJob(process.as_raw_handle(), job.as_raw_handle(), &mut in_job) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if in_job != 0 {
            members.push(process);
        }
    }
    Ok(())
}

fn member_ids(job: &OwnedHandle, deadline: Instant) -> io::Result<Vec<u32>> {
    const HEADER: usize =
        offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList) / size_of::<usize>();
    let mut capacity = 16;
    loop {
        let words = HEADER + capacity;
        let bytes = words
            .checked_mul(size_of::<usize>())
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(io::ErrorKind::InvalidData)?;
        let mut buffer = Vec::<usize>::new();
        buffer
            .try_reserve_exact(words)
            .map_err(|_| io::ErrorKind::OutOfMemory)?;
        buffer.resize(words, 0);
        // SAFETY: job stays open and buffer is aligned for the native header
        // followed by capacity pointer-sized IDs.
        let ok = unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast(),
                bytes,
                null_mut(),
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) {
                return Err(error);
            }
        } else {
            // SAFETY: the initialized buffer is aligned and has room for the
            // native header, including its first array element.
            let list = unsafe { &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
            let listed = list.NumberOfProcessIdsInList as usize;
            if listed > capacity {
                return Err(io::ErrorKind::InvalidData.into());
            }
            if list.NumberOfProcessIdsInList == list.NumberOfAssignedProcesses {
                return buffer[HEADER..HEADER + listed]
                    .iter()
                    .map(|&pid| u32::try_from(pid).map_err(|_| io::ErrorKind::InvalidData.into()))
                    .collect();
            }
        }
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        capacity *= 2;
    }
}

fn drain(
    mut job: Option<(&OwnedHandle, &mut Option<u32>)>,
    members: &mut Vec<OwnedHandle>,
    deadline: Instant,
) -> io::Result<()> {
    loop {
        let mut empty = true;
        if let Some((job, last_active)) = &mut job {
            let active = active_processes(job)?;
            **last_active = Some(active);
            empty = active == 0;
        }
        let mut index = 0;
        while index < members.len() {
            // SAFETY: members owns the process handle throughout the wait.
            match unsafe { WaitForSingleObject(members[index].as_raw_handle(), 0) } {
                WAIT_OBJECT_0 => drop(members.swap_remove(index)),
                WAIT_TIMEOUT => index += 1,
                WAIT_FAILED => return Err(io::Error::last_os_error()),
                _ => return Err(io::Error::other("unexpected process wait result")),
            }
        }
        // The count can reach zero before a terminating process handle is
        // signaled. The count also covers members added after the snapshot.
        if empty && members.is_empty() {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        thread::sleep(remaining.min(Duration::from_millis(1)));
    }
}

fn active_processes(job: &OwnedHandle) -> io::Result<u32> {
    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    // SAFETY: job stays open and accounting has the requested class's layout.
    if unsafe {
        QueryInformationJobObject(
            job.as_raw_handle(),
            JobObjectBasicAccountingInformation,
            (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            size_of_val(&accounting) as u32,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(accounting.ActiveProcesses)
}
