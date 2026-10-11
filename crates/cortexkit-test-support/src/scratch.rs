use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};
pub use subc_os::process_identity::Liveness;
use subc_os::process_identity::{liveness, process_start, ProcessStart};

/// Shared executable directory exempt from the scratch age sweep.
pub const STABLE_BIN_DIR: &str = "stable-bin";

/// Per-process scratch root, keyed by PID and the kernel's versioned start identity.
/// Unsupported platforms use an unknown identity, which cleanup always preserves.
pub fn scratch_root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let pid = std::process::id();
        let start = process_start(pid)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".into());
        let path = shared_scratch_root().join(format!("k{pid}-{start}"));
        std::fs::create_dir_all(&path).expect("create process scratch root");
        real_directory(&path);
        path
    })
    .clone()
}

/// Shared root under the system temp directory. Initialization sweeps stale fixtures.
pub fn shared_scratch_root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join("cortexkit-tests");
        std::fs::create_dir_all(&root).expect("create test scratch root");
        real_directory(&root);
        sweep(&root);
        root
    })
    .clone()
}

fn real_directory(path: &Path) {
    let metadata = std::fs::symlink_metadata(path).expect("inspect scratch directory");
    assert!(
        metadata.is_dir() && !metadata.is_symlink(),
        "scratch root must be a real directory: {}",
        path.display()
    );
}

fn process_root_liveness(name: &str) -> Liveness {
    let Some((pid, start)) = name.strip_prefix('k').and_then(|n| n.split_once('-')) else {
        return Liveness::Unknown;
    };
    let (Ok(pid), Some(start)) = (pid.parse(), ProcessStart::parse(start)) else {
        return Liveness::Unknown;
    };
    liveness(pid, Some(&start))
}

fn sweep(root: &Path) {
    let cutoff = SystemTime::now().checked_sub(Duration::from_secs(24 * 60 * 60));
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name() == STABLE_BIN_DIR {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            // Unknown names and old encodings are not proof that an owner died.
            // Only an affirmative kernel liveness result permits process-root removal.
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('k') || name.starts_with('p') {
                if process_root_liveness(&name) == Liveness::Dead {
                    let _ = std::fs::remove_dir_all(&path);
                }
                continue;
            }
        }
        if cutoff.is_some_and(|cutoff| metadata.modified().is_ok_and(|mtime| mtime < cutoff)) {
            if metadata.is_dir() {
                if descendants_are_old(&path, cutoff.unwrap()) {
                    let _ = std::fs::remove_dir_all(&path);
                }
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn descendants_are_old(path: &Path, cutoff: SystemTime) -> bool {
    let Ok(mut entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.all(|entry| {
        let Ok(entry) = entry else { return false };
        let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
            return false;
        };
        !metadata.is_symlink()
            && metadata.modified().is_ok_and(|mtime| mtime < cutoff)
            && (!metadata.is_dir() || descendants_are_old(&entry.path(), cutoff))
    })
}

/// Wait for a PID to be positively known dead. Unknown never counts as gone.
/// On unsupported platforms the result is false after the timeout.
pub fn wait_until_gone(pid: i32, timeout: Duration) -> bool {
    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    let started = process_start(pid);
    let deadline = Instant::now() + timeout;
    loop {
        if liveness(pid, started.as_ref()) == Liveness::Dead {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
}

/// Return the tri-state liveness of `pid`: alive, dead, or unknown. Linux zombies
/// are dead. Use [`process_alive`] for assertions and [`wait_until_gone`] when waiting.
pub fn process_liveness(pid: i32) -> Liveness {
    let Ok(pid) = u32::try_from(pid) else {
        return Liveness::Unknown;
    };
    let started = process_start(pid);
    liveness(pid, started.as_ref())
}

/// Return whether `pid` is positively alive: true for alive and false for dead.
/// Panics with the PID and reason when liveness is unknown, including on
/// unsupported platforms. Linux zombies are dead. Use this for assertions and
/// [`wait_until_gone`] when waiting.
pub fn process_alive(pid: i32) -> bool {
    match process_liveness(pid) {
        Liveness::Alive => true,
        Liveness::Dead => false,
        Liveness::Unknown => panic!(
            "could not determine liveness for pid {pid}: the operating system returned an unknown state (unsupported platform or unavailable process identity)"
        ),
    }
}

/// Owned scratch fixture. Drop removes it unless unwinding or explicitly kept.
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
    kept: bool,
}

impl ScratchDir {
    pub fn new(prefix: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        assert!(!prefix.is_empty() && !prefix.contains('/') && !prefix.contains('\\'));
        loop {
            let path = scratch_root().join(format!(
                "{prefix}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self { path, kept: false },
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create scratch directory {}: {e}", path.display()),
            }
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Preserve this fixture and print its path for investigation.
    pub fn keep(mut self) -> PathBuf {
        self.kept = true;
        self.path.clone()
    }
}
impl AsRef<Path> for ScratchDir {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}
impl std::ops::Deref for ScratchDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        self.path()
    }
}
impl Drop for ScratchDir {
    fn drop(&mut self) {
        if self.kept || std::thread::panicking() {
            eprintln!("ScratchDir preserved: {}", self.path.display());
        } else if let Err(e) = std::fs::remove_dir_all(&self.path) {
            eprintln!("remove scratch directory {}: {e}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn old(path: &Path) {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Windows requires backup semantics to open a directory for timestamps.
            const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            options
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
        }
        options
            .open(path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::now() - Duration::from_secs(48 * 3600)),
            )
            .unwrap();
    }
    #[test]
    fn success_drop_removes_the_tree() {
        let dir = ScratchDir::new("normal");
        let path = dir.path().to_owned();
        assert!(path.exists());
        drop(dir);
        assert!(!path.exists());
    }
    #[test]
    fn keep_preserves_without_panic() {
        let path = ScratchDir::new("keep").keep();
        assert!(path.exists());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn panic_preserves_the_tree() {
        let mut path = None;
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let dir = ScratchDir::new("panic");
            path = Some(dir.path().to_owned());
            panic!("intentional panic");
        }))
        .is_err());
        let path = path.unwrap();
        assert!(path.exists());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn unknown_encoding_root_is_kept() {
        let dir = ScratchDir::new("sweep-unknown");
        for name in [
            "k0-future-v2-start",
            "p999999-00",
            "kgarbled",
            "p-unparseable",
        ] {
            let path = dir.join(name);
            std::fs::create_dir(&path).unwrap();
            old(&path);
            sweep(&dir);
            assert!(path.exists(), "unknown root {name} removed");
        }
    }
    #[cfg(unix)]
    #[test]
    fn reaped_child_root_is_removed() {
        let dir = ScratchDir::new("sweep-reaped");
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let start = process_start(child.id()).unwrap();
        let path = dir.join(format!("k{}-{start}", child.id()));
        std::fs::create_dir(&path).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        sweep(&dir);
        assert!(!path.exists());
    }
    #[cfg(unix)]
    #[test]
    fn live_pid_different_same_family_start_is_removed() {
        let dir = ScratchDir::new("sweep-reuse");
        let pid = std::process::id();
        let start = process_start(pid).unwrap();
        let other = if start.as_str().starts_with("linux-") {
            "linux-v1-0"
        } else {
            "macos-v1-0-0"
        };
        assert_ne!(start.as_str(), other);
        let live = dir.join(format!("k{pid}-{start}"));
        let reused = dir.join(format!("k{pid}-{other}"));
        for path in [&live, &reused] {
            std::fs::create_dir(path).unwrap();
        }
        sweep(&dir);
        assert!(live.exists());
        assert!(!reused.exists());
    }
    #[test]
    fn stable_bin_is_kept_and_old_files_are_removed() {
        let dir = ScratchDir::new("sweep-age");
        let stable = dir.join(STABLE_BIN_DIR);
        std::fs::create_dir(&stable).unwrap();
        std::fs::write(stable.join("probe"), "keep").unwrap();
        old(&stable);
        let stale = dir.join("old-file");
        std::fs::write(&stale, "old").unwrap();
        old(&stale);
        let young = dir.join("young-file");
        std::fs::write(&young, "young").unwrap();
        let old_dir = dir.join("old-dir");
        std::fs::create_dir(&old_dir).unwrap();
        old(&old_dir);
        let active = dir.join("old-parent");
        std::fs::create_dir(&active).unwrap();
        std::fs::write(active.join("young"), "keep").unwrap();
        old(&active);
        sweep(&dir);
        assert!(stable.join("probe").exists());
        assert!(!stale.exists());
        assert!(!old_dir.exists());
        assert!(active.join("young").exists());
        assert!(young.exists());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_is_kept() {
        let dir = ScratchDir::new("sweep-link");
        let target = ScratchDir::new("outside");
        std::os::unix::fs::symlink(target.path(), dir.join("k123-linux-v1-0")).unwrap();
        sweep(&dir);
        assert!(std::fs::symlink_metadata(dir.join("k123-linux-v1-0"))
            .unwrap()
            .is_symlink());
        assert!(target.exists());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn an_exited_unreaped_child_is_not_alive() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        let gone = wait_until_gone(pid, Duration::from_secs(5));
        let found =
            rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap())
                .is_ok();
        let liveness = process_liveness(pid);
        let alive = process_alive(pid);
        child.wait().unwrap();
        assert!(found);
        assert!(gone);
        assert_eq!(liveness, Liveness::Dead);
        assert!(!alive);
    }
    #[cfg(unix)]
    #[test]
    fn a_running_child_is_alive() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let liveness = process_liveness(pid);
        let alive = process_alive(pid);
        let gone = wait_until_gone(pid, Duration::from_millis(20));
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(liveness, Liveness::Alive);
        assert!(alive);
        assert!(!gone);
    }

    #[cfg(unix)]
    #[test]
    fn a_reaped_child_is_dead() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(process_liveness(pid), Liveness::Dead);
        assert!(!process_alive(pid));
    }

    #[test]
    #[should_panic(expected = "pid -1: the operating system returned an unknown state")]
    fn process_alive_panics_when_liveness_is_unknown() {
        process_alive(-1);
    }
}
