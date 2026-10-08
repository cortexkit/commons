//! Shared executable cache for real-sibling integration tests.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run git for sibling identity");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}",
        repo.display()
    );
    output.stdout
}

/// Return a cache key derived from the checkout's HEAD and source bytes, not
/// its directory location. Staged changes, unstaged changes, and untracked
/// non-ignored files contribute, so each local edit invalidates a cached build.
pub fn sibling_revision(repo: &Path) -> String {
    let head = String::from_utf8(git(repo, &["rev-parse", "HEAD"])).unwrap();
    let mut contents = git(repo, &["diff", "HEAD", "--binary", "--no-ext-diff"]);
    for name in git(repo, &["ls-files", "--others", "--exclude-standard", "-z"])
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let name = std::str::from_utf8(name).expect("UTF-8 sibling file name");
        let bytes = std::fs::read(repo.join(name)).expect("read untracked sibling file");
        contents.extend_from_slice(&(name.len() as u64).to_le_bytes());
        contents.extend_from_slice(name.as_bytes());
        contents.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        contents.extend(bytes);
    }
    if contents.is_empty() {
        return head.trim().to_owned();
    }
    format!("{}-dirty-{}", head.trim(), hash_contents(&contents))
}

fn hash_contents(contents: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("hash sibling changes");
    child.stdin.take().unwrap().write_all(contents).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Return a user-owned directory outside the source checkouts for compiled
/// sibling builds. Downloading or unpacking pinned source trees is the caller's
/// responsibility; this cache stores build outputs, not those source trees.
pub fn sibling_cache_root() -> PathBuf {
    PathBuf::from(
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .expect("user home for sibling build cache"),
    )
    .join(".cache/cortexkit-tests/sibling-target")
}

/// Resolve an immutable executable, building only on a cache miss. `revision`
/// may be an explicit pin rather than the live checkout's HEAD. The builder
/// receives a staging Cargo target directory and must produce `debug/<bin>`.
/// A sibling-wide OS lock covers publication and pruning, including across
/// processes; dropping the file releases it even when a builder panics.
pub fn cached_sibling_binary(
    sibling: &str,
    revision: &str,
    bin: &str,
    builder: impl FnOnce(&Path),
) -> PathBuf {
    cached_sibling_binary_with_target(sibling, revision, bin, builder).executable
}

/// Paths to the cached Cargo target directory and the executable used by tests.
/// A `ck-*` artifact is copied to a separate `ckdev-*` publication before running,
/// so its execution path is not the path where Cargo wrote the build output.
pub struct CachedSiblingBinary {
    pub executable: PathBuf,
    pub target_dir: PathBuf,
}

/// Resolve both paths so a caller can track or restrict where Cargo writes build
/// files separately from the executable path used to start test processes.
pub fn cached_sibling_binary_with_target(
    sibling: &str,
    revision: &str,
    bin: &str,
    builder: impl FnOnce(&Path),
) -> CachedSiblingBinary {
    resolve_with_target(&sibling_cache_root(), sibling, revision, bin, builder)
}

#[cfg(test)]
fn resolve(
    root: &Path,
    sibling: &str,
    revision: &str,
    bin: &str,
    builder: impl FnOnce(&Path),
) -> PathBuf {
    resolve_with_target(root, sibling, revision, bin, builder).executable
}

fn resolve_with_target(
    root: &Path,
    sibling: &str,
    revision: &str,
    bin: &str,
    builder: impl FnOnce(&Path),
) -> CachedSiblingBinary {
    for component in [sibling, revision, bin] {
        assert!(
            !component.is_empty()
                && component
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid sibling cache component"
        );
    }
    let root = root.join(sibling);
    std::fs::create_dir_all(&root).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("build.lock"))
        .unwrap();
    lock.lock().expect("lock shared sibling build");
    let destination = root.join(revision).join(bin);
    let executable = destination
        .join("debug")
        .join(format!("{bin}{}", std::env::consts::EXE_SUFFIX));
    if !executable.is_file() {
        eprintln!("sibling cache: miss, building {sibling}/{revision}/{bin}");
        let staging = root.join("building");
        if staging.exists() {
            std::fs::remove_dir_all(&staging).unwrap();
        }
        std::fs::create_dir_all(&staging).unwrap();
        builder(&staging);
        let built = staging
            .join("debug")
            .join(format!("{bin}{}", std::env::consts::EXE_SUFFIX));
        assert!(
            built.is_file(),
            "builder did not produce {}",
            built.display()
        );
        super::sign_test_binary(&built, None);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::rename(&staging, &destination).expect("publish sibling executable atomically");
        super::warm_exec_fenced(&executable);
    } else {
        eprintln!("sibling cache: hit {}", executable.display());
    }
    // Refresh recency on reuse too, so pruning keeps the actively used pins.
    std::fs::write(destination.parent().unwrap().join("last-used"), []).unwrap();
    let mut revisions: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir()) && entry.file_name() != "building"
        })
        .map(|entry| {
            (
                std::fs::metadata(entry.path().join("last-used"))
                    .and_then(|m| m.modified())
                    .ok(),
                entry.path(),
            )
        })
        .collect();
    revisions.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, old) in revisions
        .into_iter()
        .filter(|(_, path)| path != destination.parent().unwrap())
        .skip(3)
    {
        std::fs::remove_dir_all(old).unwrap();
    }
    // Stage while the sibling lock is still held, so another revision's prune
    // cannot remove this build before its execution copy has been published.
    CachedSiblingBinary {
        target_dir: destination,
        executable: super::stage_test_binary(&executable),
    }
}

/// Return a cache key combining an archived source revision with caller-supplied
/// dependency names and full commit SHAs. Changing any dependency pin produces
/// a new key; fetching or unpacking those source revisions is up to the caller.
pub fn pinned_sibling_build_revision(revision: &str, dependencies: &[(&str, &str)]) -> String {
    let mut key = revision.to_owned();
    for (name, sha) in dependencies {
        assert_eq!(sha.len(), 40, "dependency pin must be a full commit SHA");
        assert!(sha.bytes().all(|b| b.is_ascii_hexdigit()));
        key.push_str(&format!("-{name}-{sha}"));
    }
    format!("{revision}-pins-{}", hash_contents(key.as_bytes()))
}

/// Include the source revisions of caller-named adjacent path-dependency checkouts
/// in the build cache key. Missing checkouts and `repo` itself are skipped.
/// Names must be plain directory names; order is significant in the hash.
pub fn sibling_build_revision(
    repo: &Path,
    revision: &str,
    dependency_checkouts: &[&str],
) -> String {
    let mut key = revision.to_owned();
    for name in dependency_checkouts {
        crate::fence::plain_name(name);
        let dependency = repo.parent().unwrap().join(name);
        if dependency.join(".git").exists()
            && dependency.canonicalize().ok() != repo.canonicalize().ok()
        {
            key.push('-');
            key.push_str(&sibling_revision(&dependency));
        }
    }
    format!("{revision}-deps-{}", hash_contents(key.as_bytes()))
}

/// Build a sibling using a cache key derived from its current source revision
/// and caller-selected adjacent dependency checkouts. The same dependency names
/// are checked again after compilation to reject sources changed during the build.
pub fn build_sibling_binary(
    repo: &Path,
    sibling: &str,
    bin: &str,
    dependency_checkouts: &[&str],
) -> PathBuf {
    let revision = sibling_build_revision(repo, &sibling_revision(repo), dependency_checkouts);
    cached_sibling_binary(sibling, &revision, bin, |target| {
        let status = super::sibling_cargo_build(repo, target, bin)
            .status()
            .expect("build sibling binary");
        assert!(
            status.success(),
            "building {bin} failed: {}",
            crate::describe_exit_status(status)
        );
        assert_eq!(
            sibling_build_revision(repo, &sibling_revision(repo), dependency_checkouts),
            revision,
            "sibling changed during build; retry with the new revision"
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    };

    fn fake_build(target: &Path) {
        std::fs::create_dir_all(target.join("debug")).unwrap();
        std::fs::write(
            target.join(format!("debug/probe{}", std::env::consts::EXE_SUFFIX)),
            b"fake executable",
        )
        .unwrap();
    }

    #[test]
    fn cached_binary_keeps_build_target_separate_from_ckdev_execution() {
        let scratch = crate::ScratchDir::new("sibling-cache-build-and-exec");
        let cache = scratch.path().join("cache");
        let built_name = format!("ck-probe{}", std::env::consts::EXE_SUFFIX);
        let first = resolve_with_target(&cache, "example", "revision", "ck-probe", |target| {
            std::fs::create_dir_all(target.join("debug")).unwrap();
            std::fs::copy(
                std::env::current_exe().unwrap(),
                target.join("debug").join(&built_name),
            )
            .unwrap();
        });
        let published_target = cache.join("example/revision/ck-probe");
        assert_eq!(first.target_dir, published_target);
        assert!(first.target_dir.join("debug").join(built_name).is_file());
        assert_eq!(
            first.executable.file_name().unwrap(),
            format!("ckdev-probe{}", std::env::consts::EXE_SUFFIX).as_str()
        );
        assert!(!first.executable.starts_with(&cache));
        let reused = resolve_with_target(&cache, "example", "revision", "ck-probe", |_| {
            panic!("cache hit must not build")
        });
        assert_eq!(reused.target_dir, first.target_dir);
        assert_eq!(reused.executable, first.executable);
    }

    fn git_ok(repo: &Path, args: &[&str]) {
        assert!(Command::new("git")
            .current_dir(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.org")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.org")
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn pinned_identity_includes_every_dependency() {
        let pin = "a".repeat(40);
        let next = "b".repeat(40);
        let baseline =
            pinned_sibling_build_revision(&pin, &[("library-a", &pin), ("library-b", &pin)]);
        for dependencies in [
            [("library-a", next.as_str()), ("library-b", pin.as_str())],
            [("library-a", pin.as_str()), ("library-b", next.as_str())],
        ] {
            assert_ne!(baseline, pinned_sibling_build_revision(&pin, &dependencies));
        }
        assert_ne!(
            baseline,
            pinned_sibling_build_revision(&next, &[("library-a", &pin), ("library-b", &pin)])
        );
        assert_eq!(
            baseline,
            pinned_sibling_build_revision(&pin, &[("library-a", &pin), ("library-b", &pin)])
        );
    }

    #[test]
    fn live_build_identity_includes_only_caller_selected_dependencies() {
        let scratch = crate::ScratchDir::new("sibling-dependency-selection");
        let repo = scratch.join("application");
        let selected = scratch.join("library-a");
        let excluded = scratch.join("library-b");
        for checkout in [&repo, &selected, &excluded] {
            std::fs::create_dir(checkout).unwrap();
            git_ok(checkout, &["init"]);
            std::fs::write(checkout.join("source"), "initial").unwrap();
            git_ok(checkout, &["add", "source"]);
            git_ok(checkout, &["commit", "-m", "initial"]);
        }
        let revision = sibling_revision(&repo);
        let empty = sibling_build_revision(&repo, &revision, &[]);
        let baseline = sibling_build_revision(&repo, &revision, &["library-a"]);
        assert_ne!(baseline, empty);
        assert_eq!(
            baseline,
            sibling_build_revision(&repo, &revision, &["library-a", "missing", "application"])
        );
        std::fs::write(excluded.join("source"), "excluded edit").unwrap();
        assert_eq!(
            baseline,
            sibling_build_revision(&repo, &revision, &["library-a"])
        );
        assert_ne!(
            baseline,
            sibling_build_revision(&repo, &revision, &["library-a", "library-b"])
        );
        std::fs::write(selected.join("source"), "selected edit").unwrap();
        let edited = sibling_build_revision(&repo, &revision, &["library-a"]);
        assert_ne!(baseline, edited);
        assert_eq!(empty, sibling_build_revision(&repo, &revision, &[]));
        std::fs::write(selected.join("untracked"), "new dependency source").unwrap();
        assert_ne!(
            edited,
            sibling_build_revision(&repo, &revision, &["library-a"])
        );
    }

    #[test]
    fn same_revision_from_different_checkout_roots_reuses_path_and_changes_get_new_paths() {
        let scratch = crate::ScratchDir::new("sibling-cache-identity");
        let first = scratch.path().join("first");
        let second = scratch.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        git_ok(&first, &["init"]);
        std::fs::write(first.join("source"), "one").unwrap();
        git_ok(&first, &["add", "source"]);
        git_ok(&first, &["commit", "-m", "first"]);
        git_ok(
            scratch.path(),
            &["clone", first.to_str().unwrap(), second.to_str().unwrap()],
        );
        let cache = scratch.path().join("cache");
        let a = resolve(
            &cache,
            "example",
            &sibling_revision(&first),
            "probe",
            fake_build,
        );
        let b = resolve(
            &cache,
            "example",
            &sibling_revision(&second),
            "probe",
            |_| panic!("cache hit must not build"),
        );
        assert_eq!(a, b);
        std::fs::write(second.join("source"), "two").unwrap();
        let dirty = resolve(
            &cache,
            "example",
            &sibling_revision(&second),
            "probe",
            fake_build,
        );
        assert_ne!(a, dirty);
        std::fs::write(second.join("source"), "three").unwrap();
        assert_ne!(
            dirty,
            resolve(
                &cache,
                "example",
                &sibling_revision(&second),
                "probe",
                fake_build
            )
        );
        git_ok(&second, &["add", "source"]);
        git_ok(&second, &["commit", "-m", "second"]);
        assert_ne!(
            a,
            resolve(
                &cache,
                "example",
                &sibling_revision(&second),
                "probe",
                fake_build
            )
        );
        let clean_revision = sibling_revision(&second);
        std::fs::write(second.join("untracked"), "new source").unwrap();
        assert_ne!(clean_revision, sibling_revision(&second));
        let dirty_revision = sibling_revision(&second);
        std::fs::write(second.join("untracked"), "edited source").unwrap();
        assert_ne!(dirty_revision, sibling_revision(&second));
    }

    #[test]
    fn concurrent_resolution_builds_once() {
        let scratch = crate::ScratchDir::new("sibling-cache-concurrent");
        let root = scratch.path().to_owned();
        let count = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let root = root.clone();
                let count = count.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    resolve(&root, "example", "revision", "probe", |target| {
                        count.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(30));
                        fake_build(target);
                    })
                })
            })
            .collect();
        let paths: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(paths.iter().all(|path| path == &paths[0]));
    }

    #[test]
    fn cache_prunes_to_four_revisions() {
        let scratch = crate::ScratchDir::new("sibling-cache-prune");
        for revision in 0..6 {
            resolve(
                scratch.path(),
                "example",
                &format!("rev-{revision}"),
                "probe",
                fake_build,
            );
        }
        let count = std::fs::read_dir(scratch.path().join("example"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().unwrap().is_dir())
            .count();
        assert_eq!(count, 4);
    }
}
