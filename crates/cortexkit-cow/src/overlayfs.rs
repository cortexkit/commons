//! Compatibility stub for the retired overlayfs backend.
//!
//! The path-only isolation API cannot prove ownership of a mount's auxiliary
//! directories. Overlayfs is therefore unavailable on every platform; callers
//! can fall back to a reflink backend without risking unrelated sibling paths.

use std::path::Path;

use async_trait::async_trait;

use crate::{BackendKind, IsoError, IsoResult, IsolationBackend, ProbeResult};

pub struct OverlayfsBackend;

pub fn backend() -> &'static dyn IsolationBackend {
    &OverlayfsBackend
}

#[async_trait]
impl IsolationBackend for OverlayfsBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Overlayfs
    }

    fn probe(&self) -> ProbeResult {
        ProbeResult::unavailable("overlayfs isolation has been retired")
    }

    fn start(&self, _lower: &Path, _merged: &Path) -> IsoResult<()> {
        Err(IsoError::unavailable(
            "overlayfs isolation has been retired",
        ))
    }

    fn stop(&self, _merged: &Path) -> IsoResult<()> {
        // A path is not an ownership-bearing mount handle. In particular, never
        // infer authority to unmount or delete generic upper/work siblings.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_overlayfs_preserves_unowned_siblings_on_start_and_stop() {
        let root = tempfile::tempdir().unwrap();
        let lower = root.path().join("lower");
        std::fs::create_dir(&lower).unwrap();
        for name in ["upper", "work", "merged"] {
            let dir = root.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("pinned"), name).unwrap();
        }
        let backend = crate::backend(BackendKind::Overlayfs);
        assert!(!backend.probe().available);
        assert!(backend
            .start(&lower, &root.path().join("merged"))
            .unwrap_err()
            .is_unavailable());
        backend.stop(&root.path().join("merged")).unwrap();
        for name in ["upper", "work", "merged"] {
            assert_eq!(
                std::fs::read(root.path().join(name).join("pinned")).unwrap(),
                name.as_bytes()
            );
        }
    }

    #[test]
    fn retired_overlayfs_resolution_preserves_live_backend_apis() {
        assert!(!crate::resolve(Some(BackendKind::Overlayfs))
            .candidates
            .contains(&BackendKind::Overlayfs));
        assert_eq!(crate::backend(BackendKind::Apfs).kind(), BackendKind::Apfs);
        assert_eq!(
            crate::backend(BackendKind::LinuxReflink).kind(),
            BackendKind::LinuxReflink
        );
        assert_eq!(
            crate::decode_git_quoted_path("\"hello\\tworld\""),
            Some("hello\tworld".into())
        );
    }
}
