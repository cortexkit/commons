#[cfg(unix)]
use std::path::PathBuf;

/// Immutable content-addressed fake executables shared across processes.
/// Do not modify or delete the returned directory. Each complete file is copied
/// by a child and atomically renamed, so concurrent readers never see a partial write.
#[cfg(unix)]
pub fn stable_executable_dir(files: &[(&str, &[u8])]) -> PathBuf {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut hash = Sha256::new();
    for (name, bytes) in files {
        assert!(
            !name.is_empty()
                && !name.contains('/')
                && !name.contains('\\')
                && !name.starts_with('.'),
            "stable executable name must be plain"
        );
        for piece in [name.as_bytes(), *bytes] {
            hash.update((piece.len() as u64).to_le_bytes());
            hash.update(piece);
        }
    }
    let key = format!("{:x}", hash.finalize());
    let dir = crate::shared_scratch_root()
        .join(crate::STABLE_BIN_DIR)
        .join(key);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in files {
        let path = dir.join(name);
        let current = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.permissions().mode() & 0o777 == 0o755)
            .and_then(|_| std::fs::read(&path).ok());
        if current.as_deref() == Some(*bytes) {
            continue;
        }
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let input = dir.join(format!(".input-{}-{nonce}", std::process::id()));
        let staging = dir.join(format!(".stage-{}-{nonce}", std::process::id()));
        std::fs::write(&input, bytes).unwrap();
        crate::checked_output(std::process::Command::new("cp").arg(&input).arg(&staging)).unwrap();
        std::fs::remove_file(input).unwrap();
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(staging, &path).unwrap();
    }
    dir
}
/// Single-file form of [`stable_executable_dir`].
#[cfg(unix)]
pub fn stable_executable(name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    stable_executable_dir(&[(name, contents.as_ref())]).join(name)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn stable_executable_from_eight_threads_leaves_one_file_with_the_right_bytes() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let body = format!(
            "#!/bin/sh\n# {}-{:?}\nexit 0\n",
            std::process::id(),
            std::time::SystemTime::now()
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let barrier = barrier.clone();
                let body = body.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    stable_executable("probe", body)
                })
            })
            .collect();
        let paths: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(paths.iter().all(|p| p == &paths[0]));
        let path = &paths[0];
        assert!(path.starts_with(crate::shared_scratch_root().join(crate::STABLE_BIN_DIR)));
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        assert_eq!(std::fs::read(path).unwrap(), body.as_bytes());
        let meta = std::fs::metadata(path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o755);
        assert_eq!(stable_executable("probe", body), *path);
        assert_eq!(std::fs::metadata(path).unwrap().ino(), meta.ino());
    }
}
