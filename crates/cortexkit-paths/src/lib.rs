//! Shared path canonicalization primitives for CortexKit tooling.
//!
//! This crate deliberately owns only the dependency-light project-root identity
//! primitive: resolving an existing filesystem path into a canonical path-backed
//! [`ProjectRootId`]. It does not perform workspace discovery, Git inspection,
//! transport serialization, or operation-target fallback handling.

#![forbid(unsafe_code)]

use std::{
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

/// Stable canonical identity for a project root.
///
/// A `ProjectRootId` is represented by the canonical filesystem path of an
/// existing project root. Construction uses [`std::fs::canonicalize`], so the
/// stored path is absolute, has `.`/`..`/trailing separators collapsed, and has
/// symlinks resolved.
///
/// Git worktrees are first-class roots: this crate does not ask Git for a
/// repository common-dir and does not collapse linked worktrees back to their
/// main checkout. Because a linked worktree has its own checkout directory, the
/// canonical worktree path is a distinct id from the canonical main-checkout
/// path while alternate spellings of either path still converge.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProjectRootId(PathBuf);

impl ProjectRootId {
    /// Resolve an existing filesystem path into a canonical project-root id.
    ///
    /// Non-existent paths are rejected with [`IdentityError::NonExistentPath`]
    /// instead of being logically normalized. That policy avoids silently
    /// aliasing roots whose future meaning could change when missing path
    /// components or symlinks are later created.
    ///
    /// That rejection is load-bearing for callers who use it to DETECT a root
    /// that has gone away, so this constructor keeps it. Callers that must still
    /// address a vanished root -- ending or inspecting work that was admitted
    /// while the root existed -- use [`Self::from_path_allowing_missing`], which
    /// preserves the aliasing guarantee by a narrower means.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let requested_path = path.as_ref().to_path_buf();
        match fs::canonicalize(path.as_ref()) {
            Ok(canonical_path) => Ok(Self(platform_project_root_path(canonical_path))),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Err(IdentityError::NonExistentPath {
                    path: requested_path,
                })
            }
            Err(source) => Err(IdentityError::CanonicalizePath {
                path: requested_path,
                source,
            }),
        }
    }

    /// Resolve a path into a project-root id even when the path no longer exists.
    ///
    /// Resolves the longest prefix that still exists and re-appends the rest,
    /// following any symlink encountered on the missing tail. This is the
    /// behaviour of POSIX `realpath` on a non-existent path; [`fs::canonicalize`]
    /// is the outlier in refusing partial resolution, so this matches a
    /// documented reference rather than inventing a rule.
    ///
    /// WHY NOT LEXICAL NORMALIZATION: consumers key durable state on the
    /// resolved string. On macOS every temporary directory is reached through a
    /// symlink, so a lexically-normalized path is a DIFFERENT string from the id
    /// minted while the root existed -- the caller would address an empty
    /// lineage and receive a confident "no such thing" rather than an error.
    /// That is one caller, one spelling, and two ids across time.
    ///
    /// WHAT THIS DOES NOT PROMISE: if a missing component later reappears as a
    /// symlink pointing elsewhere, the id moves. [`Self::from_path`] does not
    /// prevent that either -- it declines to answer while the component is
    /// missing and then resolves through the new link exactly as this does, so
    /// the hazard is shared rather than introduced here. The aliasing guarantee
    /// the strict constructor exists for is preserved by refusing to create NEW
    /// durable state under an id resolved this way; callers admit only
    /// operations that read or end something already recorded.
    pub fn from_path_allowing_missing(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let resolved = resolve_allowing_missing(path.as_ref(), 0)?;
        Ok(Self(platform_project_root_path(resolved)))
    }

    /// Borrow the canonical path backing this identity.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Consume the identity and return its canonical path representation.
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

impl AsRef<Path> for ProjectRootId {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl fmt::Display for ProjectRootId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl From<ProjectRootId> for PathBuf {
    fn from(value: ProjectRootId) -> Self {
        value.into_path_buf()
    }
}

impl TryFrom<&Path> for ProjectRootId {
    type Error = IdentityError;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        Self::from_path(value)
    }
}

impl TryFrom<PathBuf> for ProjectRootId {
    type Error = IdentityError;

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        Self::from_path(value)
    }
}

impl TryFrom<&str> for ProjectRootId {
    type Error = IdentityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_path(Path::new(value))
    }
}

impl TryFrom<String> for ProjectRootId {
    type Error = IdentityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_path(PathBuf::from(value))
    }
}

/// Typed identity-resolution failures.
#[derive(Debug)]
pub enum IdentityError {
    /// The requested project root does not exist, or a path component cannot be
    /// resolved through an existing symlink chain.
    NonExistentPath { path: PathBuf },
    /// The OS rejected canonicalization for a reason other than non-existence.
    CanonicalizePath { path: PathBuf, source: io::Error },
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonExistentPath { path } => {
                write!(f, "project root does not exist: {}", path.display())
            }
            Self::CanonicalizePath { path, source } => {
                write!(
                    f,
                    "failed to canonicalize project root {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for IdentityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NonExistentPath { .. } => None,
            Self::CanonicalizePath { source, .. } => Some(source),
        }
    }
}

#[cfg(not(windows))]
fn platform_project_root_path(canonical_path: PathBuf) -> PathBuf {
    canonical_path
}

/// The kernel's own ceiling on symlink hops is typically 40 (`ELOOP`); matching
/// it means a chain this code refuses is one the OS would refuse too.
const MAX_SYMLINK_HOPS: u32 = 40;

/// Resolve the longest existing prefix of `path` and re-append the missing tail.
///
/// Recurses on the parent rather than looping so that following a symlink on the
/// missing tail re-enters the same resolution from the link's target.
fn resolve_allowing_missing(path: &Path, hops: u32) -> Result<PathBuf, IdentityError> {
    match fs::canonicalize(path) {
        Ok(canonical_path) => return Ok(canonical_path),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(IdentityError::CanonicalizePath {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    // `..` and `.` tails have no file name to re-append, so there is no honest
    // way to reconstruct them once the path has stopped existing. Refuse rather
    // than return a path that differs from what the caller named.
    let (Some(parent), Some(tail)) = (path.parent(), path.file_name()) else {
        return Err(IdentityError::NonExistentPath {
            path: path.to_path_buf(),
        });
    };
    // `Path::parent` yields an empty path for a bare relative name; that is the
    // current directory, not the absence of a parent.
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };

    let resolved_parent = resolve_allowing_missing(parent, hops)?;
    let candidate = resolved_parent.join(tail);

    // A DANGLING symlink reads as absent to `canonicalize` and to `Path::exists`,
    // because both follow links. `symlink_metadata` is the only predicate that
    // sees the link itself, and following it is what `realpath` does -- keeping
    // the link's own name instead would move the id the moment someone repairs
    // the link, so ordinary maintenance would silently strand whatever was
    // admitted under it.
    match fs::symlink_metadata(&candidate) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if hops >= MAX_SYMLINK_HOPS {
                return Err(IdentityError::CanonicalizePath {
                    path: path.to_path_buf(),
                    source: io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("symbolic link chain exceeded {MAX_SYMLINK_HOPS} hops"),
                    ),
                });
            }
            let target =
                fs::read_link(&candidate).map_err(|source| IdentityError::CanonicalizePath {
                    path: candidate.clone(),
                    source,
                })?;
            let target = if target.is_absolute() {
                target
            } else {
                resolved_parent.join(target)
            };
            resolve_allowing_missing(&target, hops.saturating_add(1))
        }
        _ => Ok(candidate),
    }
}

#[cfg(windows)]
fn platform_project_root_path(canonical_path: PathBuf) -> PathBuf {
    windows_non_verbatim_path(canonical_path)
}

#[cfg(windows)]
fn windows_non_verbatim_path(path: PathBuf) -> PathBuf {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };

    const SEPARATOR: u16 = b'\\' as u16;
    const DRIVE_SEPARATOR: u16 = b':' as u16;
    const LOWER_A: u16 = b'a' as u16;
    const LOWER_Z: u16 = b'z' as u16;
    const ASCII_CASE_DELTA: u16 = (b'a' - b'A') as u16;
    const VERBATIM_PREFIX: [u16; 4] = [SEPARATOR, SEPARATOR, b'?' as u16, SEPARATOR];
    const VERBATIM_UNC_PREFIX: [u16; 8] = [
        SEPARATOR,
        SEPARATOR,
        b'?' as u16,
        SEPARATOR,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        SEPARATOR,
    ];

    let encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    let mut normalized = if encoded.starts_with(&VERBATIM_UNC_PREFIX) {
        let mut non_verbatim = Vec::with_capacity(encoded.len() - VERBATIM_UNC_PREFIX.len() + 2);
        non_verbatim.extend_from_slice(&[SEPARATOR, SEPARATOR]);
        non_verbatim.extend_from_slice(&encoded[VERBATIM_UNC_PREFIX.len()..]);
        non_verbatim
    } else if encoded.starts_with(&VERBATIM_PREFIX) {
        encoded[VERBATIM_PREFIX.len()..].to_vec()
    } else {
        encoded
    };

    if normalized.len() >= 2
        && normalized[1] == DRIVE_SEPARATOR
        && (LOWER_A..=LOWER_Z).contains(&normalized[0])
    {
        normalized[0] -= ASCII_CASE_DELTA;
    }

    PathBuf::from(OsString::from_wide(&normalized))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    static NEXT_TEST_DIR: AtomicUsize = AtomicUsize::new(0);

    #[cfg(unix)]
    fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = format!(
                "cortexkit-paths-project-root-id-{label}-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time should not be before the Unix epoch")
                    .as_nanos(),
                NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir(&path).expect("create temporary project-root-id test directory");
            Self { path }
        }

        fn child(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// The property the whole constructor exists for: an id minted while the root
    /// existed must still be reachable after it is gone.
    ///
    /// Written as an EQUALITY against the strict constructor's output rather than
    /// against a hand-written expected string, because a literal would encode
    /// whatever this author believed canonicalization does. The equality fails if
    /// the fallback and the strict path ever disagree, which is the only thing
    /// consumers keying durable state on the result actually require.
    #[test]
    fn id_survives_the_root_being_deleted() {
        let temp = TestDir::new("vanished");
        let root = temp.child("project");
        fs::create_dir(&root).expect("create project root");

        let while_present = ProjectRootId::from_path(&root).expect("canonicalize live root");
        fs::remove_dir(&root).expect("remove project root");

        assert!(
            ProjectRootId::from_path(&root).is_err(),
            "the strict constructor must still refuse a vanished root, or callers that \
             use the refusal to DETECT a dead root would silently keep it"
        );
        assert_eq!(
            ProjectRootId::from_path_allowing_missing(&root).expect("resolve vanished root"),
            while_present,
            "a root deleted after admission must resolve to the id it was admitted \
             under, or the caller addresses an empty lineage and is told no such thing \
             exists rather than being given an error"
        );
    }

    /// macOS reaches every temp directory through a symlink, so this is the case
    /// that distinguishes resolving from lexical normalization on this host --
    /// and the one a lexical implementation silently gets wrong.
    #[test]
    fn missing_tail_resolves_through_a_symlinked_ancestor() {
        let temp = TestDir::new("symlinked-ancestor");
        let real = temp.child("real");
        let link = temp.child("link");
        fs::create_dir(&real).expect("create real directory");
        symlink_dir(&real, &link).expect("create ancestor symlink");

        let through_link = ProjectRootId::from_path_allowing_missing(link.join("gone"))
            .expect("resolve through symlinked ancestor");
        let through_real = ProjectRootId::from_path_allowing_missing(real.join("gone"))
            .expect("resolve through real ancestor");

        assert_eq!(
            through_link, through_real,
            "a missing tail must resolve through a live symlinked ancestor, or two \
             spellings of one location mint two different ids"
        );
        assert_ne!(
            through_link.as_path(),
            link.join("gone"),
            "non-vacuity: if this equals the input the implementation is normalizing \
             lexically and the test above would pass for the wrong reason"
        );
    }

    /// A dangling link reads as absent to both `canonicalize` and `Path::exists`,
    /// so the naive walk-up stops one component too high and keeps the link's own
    /// name. Following it is what `realpath` does, and it is the choice that
    /// SURVIVES REPAIR: if someone later creates the target, the strict
    /// constructor produces this same id, so ordinary maintenance cannot strand
    /// work admitted while the link dangled.
    #[test]
    fn dangling_link_resolves_to_its_target_and_survives_the_link_being_repaired() {
        let temp = TestDir::new("dangling");
        let target = temp.child("target");
        let link = temp.child("link");
        symlink_dir(&target, &link).expect("create dangling symlink");

        let while_dangling = ProjectRootId::from_path_allowing_missing(link.join("session"))
            .expect("resolve through dangling link");

        fs::create_dir(&target).expect("create link target");
        fs::create_dir(target.join("session")).expect("create session directory");
        let after_repair =
            ProjectRootId::from_path(link.join("session")).expect("canonicalize repaired path");

        assert_eq!(
            while_dangling, after_repair,
            "repairing a dangling link must not move the id, or an act of maintenance \
             silently strands whatever was admitted while it dangled"
        );
    }

    /// A link chain long enough to be an error must fail rather than recurse until
    /// the stack gives out. Asserting the ERROR VARIANT, not merely that it failed:
    /// a stack overflow is not a refusal.
    #[test]
    fn symlink_chain_beyond_the_hop_ceiling_is_refused() {
        let temp = TestDir::new("loop");
        let first = temp.child("a");
        let second = temp.child("b");
        symlink_dir(&second, &first).expect("create first link");
        symlink_dir(&first, &second).expect("create second link");

        let error = ProjectRootId::from_path_allowing_missing(first.join("gone"))
            .expect_err("a symlink cycle must be refused");
        assert!(
            matches!(error, IdentityError::CanonicalizePath { .. }),
            "a cycle is an unresolvable path, not a missing one: {error:?}"
        );
    }

    #[test]
    fn path_spellings_to_same_root_have_equal_project_root_ids() {
        let temp = TestDir::new("spellings");
        let root = temp.child("project");
        let nested = root.join("nested");
        fs::create_dir(&root).expect("create project root");
        fs::create_dir(&nested).expect("create nested directory");

        let trailing = PathBuf::from(format!("{}{}", root.display(), std::path::MAIN_SEPARATOR));
        let direct = ProjectRootId::from_path(&root).expect("canonicalize direct root");
        let with_trailing = ProjectRootId::from_path(trailing).expect("canonicalize trailing root");
        let with_dot = ProjectRootId::from_path(root.join(".")).expect("canonicalize dot root");
        let round_trip =
            ProjectRootId::from_path(nested.join("..")).expect("canonicalize round-trip root");

        assert_eq!(direct, with_trailing);
        assert_eq!(direct, with_dot);
        assert_eq!(direct, round_trip);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_project_root_has_same_id_as_target() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("symlink");
        let target = temp.child("target");
        let link = temp.child("link");
        fs::create_dir(&target).expect("create symlink target");
        symlink(&target, &link).expect("create symlink to project root");

        let target_id = ProjectRootId::from_path(&target).expect("canonicalize target");
        let link_id = ProjectRootId::from_path(&link).expect("canonicalize symlink");

        assert_eq!(target_id, link_id);
    }

    #[test]
    fn git_worktree_checkout_path_is_distinct_from_main_checkout_path() {
        let temp = TestDir::new("worktree");
        let main_checkout = temp.child("main-checkout");
        let linked_worktree = temp.child("linked-worktree");
        let main_gitdir = main_checkout.join(".git");
        let worktree_gitdir = main_gitdir.join("worktrees").join("linked-worktree");

        fs::create_dir(&main_checkout).expect("create main checkout");
        fs::create_dir(&linked_worktree).expect("create linked worktree checkout");
        fs::create_dir_all(&worktree_gitdir).expect("create simulated worktree gitdir");
        fs::write(
            linked_worktree.join(".git"),
            format!("gitdir: {}\n", worktree_gitdir.display()),
        )
        .expect("write simulated linked-worktree .git file");

        let main_id = ProjectRootId::from_path(&main_checkout).expect("canonicalize main checkout");
        let worktree_id =
            ProjectRootId::from_path(&linked_worktree).expect("canonicalize linked worktree");

        assert_ne!(main_id, worktree_id);
    }

    #[test]
    fn non_existent_project_root_returns_typed_error() {
        let temp = TestDir::new("missing");
        let missing_root = temp.child("missing-project");

        match ProjectRootId::from_path(&missing_root) {
            Err(IdentityError::NonExistentPath { path }) => assert_eq!(path, missing_root),
            Err(other) => panic!("expected NonExistentPath error, got {other}"),
            Ok(id) => panic!("expected missing project root to fail, got {id}"),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_var_symlink_resolves_to_private_var() {
        let id = ProjectRootId::from_path("/var").expect("canonicalize /var");

        assert_eq!(id.as_path(), std::path::Path::new("/private/var"));
    }

    #[test]
    fn realpath_preserves_stored_case_on_case_insensitive_filesystems() {
        let temp = TestDir::new("stored-case");
        let stored_case = temp.child("SUB");
        let alternate_case = temp.child("sub");
        fs::create_dir(&stored_case).expect("create stored-case project root");

        let stored_id =
            ProjectRootId::from_path(&stored_case).expect("canonicalize stored-case root");
        match ProjectRootId::from_path(&alternate_case) {
            Ok(alternate_id) => {
                assert_eq!(stored_id, alternate_id);
                assert!(alternate_id.as_path().ends_with("SUB"));
            }
            Err(IdentityError::NonExistentPath { path }) if path == alternate_case => {
                // This filesystem is case-sensitive; the seed vector is not applicable here.
            }
            Err(other) => {
                panic!("expected alternate case to canonicalize or be absent, got {other}")
            }
        }
    }

    #[test]
    fn project_root_id_is_hashable_as_hash_map_key() {
        let temp = TestDir::new("hashmap");
        let root = temp.child("project");
        let other_root = temp.child("other-project");
        fs::create_dir(&root).expect("create project root");
        fs::create_dir(&other_root).expect("create other project root");

        let id = ProjectRootId::from_path(&root).expect("canonicalize project root");
        let same_id =
            ProjectRootId::from_path(root.join(".")).expect("canonicalize equivalent root");
        let other_id = ProjectRootId::from_path(&other_root).expect("canonicalize different root");

        let mut entries = HashMap::new();
        entries.insert(id.clone(), "project state");

        assert_eq!(entries.get(&same_id), Some(&"project state"));
        assert_eq!(entries.get(&other_id), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_verbatim_prefix_is_stripped() {
        let path = windows_non_verbatim_path(PathBuf::from(r"\\?\C:\existing"));

        assert_eq!(path, PathBuf::from(r"C:\existing"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_unc_verbatim_prefix_is_stripped() {
        let path = windows_non_verbatim_path(PathBuf::from(r"\\?\UNC\server\share\existing"));

        assert_eq!(path, PathBuf::from(r"\\server\share\existing"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_lowercase_drive_letter_is_uppercased() {
        let path = windows_non_verbatim_path(PathBuf::from(r"c:\existing"));

        assert_eq!(path, PathBuf::from(r"C:\existing"));
    }
}
