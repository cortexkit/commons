//! Shared executable cache for real-sibling integration tests.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A read-only Git command on one sibling checkout.
///
/// A remote test runner can hand a test a source snapshot that another OS user
/// owns, and plain Git refuses to work there ("dubious ownership"). So each
/// command trusts exactly this checkout, by its canonical path, for this
/// invocation only: never a wildcard and never a saved config change.
///
/// Trusting the checkout lets Git read its config, including drivers that run
/// programs. That trust is no wider than the build that follows: the helper
/// compiles this checkout, which runs its build scripts anyway, so pass only
/// checkouts you would build. The command also turns off `core.fsmonitor`, so
/// no file-monitor program or daemon starts in the snapshot, and never writes
/// the checkout's index.
fn git_command(repo: &Path) -> Command {
    let repo = git_trust_path(repo);
    assert!(
        !repo.to_string_lossy().contains('*'),
        "sibling checkout must be an exact path: {}",
        repo.display()
    );
    let mut command = Command::new("git");
    command
        .arg("-c")
        .arg(format!("safe.directory={}", repo.display()))
        .args(["-c", "core.fsmonitor=false", "--no-optional-locks", "-C"])
        .arg(&repo);
    command
}

/// The checkout's canonical path in the form Git compares `safe.directory`
/// with. On Windows, a drive-letter path loses the `\\?\` prefix that
/// `canonicalize` adds; other verbatim paths, such as network shares, are kept
/// as they are.
fn git_trust_path(repo: &Path) -> PathBuf {
    let canonical = repo
        .canonicalize()
        .unwrap_or_else(|error| panic!("resolve sibling checkout {}: {error}", repo.display()));
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if rest.as_bytes().get(1) == Some(&b':') {
                return PathBuf::from(rest);
            }
        }
    }
    canonical
}

fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = git_command(repo)
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
    // The key hashes the checkout's own bytes, so neither an external diff
    // program nor a text conversion may stand in for them.
    let mut contents = git(
        repo,
        &["diff", "HEAD", "--binary", "--no-ext-diff", "--no-textconv"],
    );
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

/// Names the shared root of the sibling build cache. Set it to the same
/// absolute path for every test when each test gets a fresh home directory,
/// as on a remote runner, so they share one cache.
pub const SIBLING_CACHE_ROOT_ENV: &str = "CORTEXKIT_TEST_SIBLING_CACHE_ROOT";

/// Return the root of the compiled sibling build cache, which holds one
/// directory per sibling. Downloading or unpacking pinned source trees is the
/// caller's responsibility; this cache stores build outputs, not those source
/// trees.
///
/// By default the root is `.cache/cortexkit-tests/sibling-target` under the
/// user's home directory. [`SIBLING_CACHE_ROOT_ENV`], when set, replaces it and
/// must be an absolute path; only absoluteness is checked, so choose a
/// user-owned directory outside the source checkouts.
pub fn sibling_cache_root() -> PathBuf {
    cache_root_from(
        std::env::var_os(SIBLING_CACHE_ROOT_ENV),
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")),
    )
}

fn cache_root_from(
    explicit: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(root) = explicit {
        let root = PathBuf::from(root);
        assert!(
            root.is_absolute(),
            "{SIBLING_CACHE_ROOT_ENV} must be an absolute path, got {}",
            root.display()
        );
        return root;
    }
    PathBuf::from(home.expect("user home for sibling build cache"))
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

    /// A committed one-file checkout in `scratch`, plus an empty global Git
    /// config, so the machine's own `safe.directory` entries can't mask the
    /// ownership check.
    fn snapshot_repo(scratch: &crate::ScratchDir) -> (PathBuf, PathBuf) {
        let repo = scratch.path().join("snapshot");
        std::fs::create_dir_all(&repo).unwrap();
        git_ok(&repo, &["init", "-q"]);
        std::fs::write(repo.join("file.txt"), b"one\n").unwrap();
        git_ok(&repo, &["add", "file.txt"]);
        git_ok(&repo, &["commit", "-q", "-m", "one"]);
        let global = scratch.path().join("empty-gitconfig");
        std::fs::write(&global, b"").unwrap();
        (repo, global)
    }

    /// Git's own test switch makes it treat the checkout as owned by another
    /// user, which is what a remote runner's source snapshot looks like.
    fn as_foreign_owner(command: &mut Command, global: &Path) -> std::process::Output {
        command
            .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
            .env("GIT_CONFIG_GLOBAL", global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
    }

    #[test]
    fn git_queries_a_checkout_another_user_owns() {
        let scratch = crate::ScratchDir::new("sibling-cache-foreign-owner");
        let (repo, global) = snapshot_repo(&scratch);

        let mut plain = Command::new("git");
        plain.current_dir(&repo).args(["rev-parse", "HEAD"]);
        let refused = as_foreign_owner(&mut plain, &global);
        assert!(
            !refused.status.success()
                && String::from_utf8_lossy(&refused.stderr).contains("dubious ownership"),
            "control: plain git must refuse the checkout, got {refused:?}"
        );

        let mut trusted = git_command(&repo);
        trusted.args(["rev-parse", "HEAD"]);
        let answered = as_foreign_owner(&mut trusted, &global);
        assert!(
            answered.status.success(),
            "trusted query refused: {}",
            String::from_utf8_lossy(&answered.stderr)
        );
    }

    #[test]
    fn git_trust_names_exactly_the_canonical_checkout() {
        let scratch = crate::ScratchDir::new("sibling-cache-trust-path");
        let (repo, _) = snapshot_repo(&scratch);
        let trust = git_trust_path(&repo.join("."));
        assert_eq!(trust, git_trust_path(&repo));
        let canonical = repo.canonicalize().unwrap();
        let canonical = canonical.to_string_lossy();
        if let Some(rest) = canonical.strip_prefix(r"\\?\") {
            if rest.as_bytes().get(1) == Some(&b':') {
                assert_eq!(
                    trust,
                    PathBuf::from(rest),
                    "drive paths drop the verbatim prefix"
                );
            }
        }
        let args: Vec<_> = git_command(&repo.join("."))
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&format!("safe.directory={}", trust.display())));
        assert!(!args.iter().any(|arg| arg.contains('*')));
    }

    #[cfg(unix)]
    #[test]
    fn trusted_checkout_fsmonitor_and_textconv_do_not_run() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = crate::ScratchDir::new("sibling-cache-config-programs");
        let (repo, _) = snapshot_repo(&scratch);
        let marker = scratch.path().join("ran");
        let program = scratch.path().join("program.sh");
        std::fs::write(
            &program,
            format!("#!/bin/sh\necho \"$*\" >> '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let program = program.to_str().unwrap();
        git_ok(&repo, &["config", "core.fsmonitor", program]);
        git_ok(&repo, &["config", "diff.probe.textconv", program]);
        std::fs::write(repo.join(".gitattributes"), b"*.txt diff=probe\n").unwrap();
        std::fs::write(repo.join("file.txt"), b"two\n").unwrap();

        assert!(sibling_revision(&repo).contains("-dirty-"));
        assert!(
            !marker.exists(),
            "the checkout's fsmonitor or textconv program ran: {}",
            std::fs::read_to_string(&marker).unwrap_or_default()
        );
    }

    #[test]
    fn cache_root_override_must_be_absolute_and_wins_over_home() {
        let home = std::env::temp_dir().join("home");
        let shared = std::env::temp_dir().join("shared-sibling-cache");
        assert_eq!(
            cache_root_from(Some(shared.clone().into()), Some(home.clone().into())),
            shared
        );
        assert_eq!(
            cache_root_from(None, Some(home.clone().into())),
            home.join(".cache/cortexkit-tests/sibling-target")
        );
        let relative = std::panic::catch_unwind(|| {
            cache_root_from(Some("relative/cache".into()), Some(home.into()))
        });
        assert!(relative.is_err(), "a relative override must be refused");
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
