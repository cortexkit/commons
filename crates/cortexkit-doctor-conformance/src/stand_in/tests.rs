use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;

use super::*;

#[test]
fn kit_core_stand_in_rejects_symlinks_and_fifo_without_writing() {
    let data = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let marker = other.path().join("marker");
    fs::write(&marker, "untouched").unwrap();
    let regular = data.path().join("regular");
    fs::write(&regular, "untouched").unwrap();
    symlink(other.path(), data.path().join("via-dir")).unwrap();
    symlink(&marker, data.path().join("via-file")).unwrap();
    let fifo = CString::new(data.path().join("fifo").as_os_str().as_bytes()).unwrap();
    // SAFETY: fifo is NUL-terminated and remains alive for the call.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);

    for path in [
        "via-dir/marker",
        "via-file",
        "fifo",
        "../marker",
        "regular/",
        "regular/.",
    ] {
        for liveness in [
            Liveness::PidFile(path.into()),
            Liveness::FlockFile(path.into()),
        ] {
            assert_eq!(
                StandIn::start(data.path(), &liveness).unwrap_err(),
                "unsafe_liveness_path"
            );
        }
    }
    assert_eq!(fs::read(&marker).unwrap(), b"untouched");
    assert_eq!(fs::read(&regular).unwrap(), b"untouched");
}

#[test]
fn kit_core_stand_in_publishes_pid_and_holds_exclusive_lock() {
    let data = tempfile::tempdir().unwrap();
    fs::create_dir(data.path().join("run")).unwrap();
    let pid = data.path().join("run/doctor.pid");
    fs::write(&pid, "old marker longer than a pid").unwrap();
    let stand_in =
        StandIn::start(data.path(), &Liveness::PidFile("run/doctor.pid".into())).unwrap();
    assert_eq!(
        fs::read_to_string(&pid).unwrap(),
        format!("{}\n", std::process::id())
    );
    drop(stand_in);
    assert!(!pid.exists());

    let stand_in =
        StandIn::start(data.path(), &Liveness::FlockFile("run/doctor.lock".into())).unwrap();
    let lock = File::open(data.path().join("run/doctor.lock")).unwrap();
    assert_eq!(
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive),
        Err(Errno::WOULDBLOCK)
    );
    drop(stand_in);
    rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap();
}
