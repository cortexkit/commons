//! Content-addressed executable copies and production-name guards.

use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Executable names a test must never run a process under.
///
/// macOS (Activity Monitor, `ps -o comm`) and other process listings show a
/// process by its executable's file name. Names that start with `ck-`, plus the
/// `ck` CLI itself, belong to the production binaries installed in the
/// CortexKit bin directory, so a test daemon running as `ck-subc` looks exactly
/// like a second production daemon. Test processes run under `ckdev-<name>`
/// instead; [`ckdev_binary`] publishes a built binary under such a name.
pub fn is_production_executable_name(file_name: &OsStr) -> bool {
    let Some(name) = file_name.to_str() else {
        // Production names are ASCII; a non-UTF-8 name cannot be one.
        return false;
    };
    let stem = strip_exe_suffix(name).0.to_ascii_lowercase();
    stem == "ck" || stem.starts_with("ck-")
}

/// Panics when `program` would run under a production executable name (see
/// [`is_production_executable_name`]). Call it before spawning any CortexKit
/// binary from a test; [`dev_command`] and [`ckdev_binary`] already do.
///
/// Cargo's own test harness for the `ck` bin target is not refused: cargo names
/// it `target/<profile>/deps/ck-<16 hex digits>`, and re-running that harness
/// (a test that starts its own executable) is cargo's naming, not a copy of a
/// production binary.
pub fn refuse_production_executable(program: &Path) {
    let name = program.file_name().unwrap_or_default();
    assert!(
        !is_production_executable_name(name) || is_cargo_test_harness(program),
        "refusing to run a test process under the production executable name {:?} ({}): \
         `ck-*` and `ck` are reserved for installed binaries; run it through \
         cortexkit_test_support::ckdev_binary so it shows as ckdev-*",
        name,
        program.display()
    );
}

/// Whether `program` is a test harness cargo built for a bin target: a file
/// in a `deps` directory named `<target>-<16 lowercase hex digits>`.
fn is_cargo_test_harness(program: &Path) -> bool {
    let in_deps = program
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|dir| dir == "deps");
    let Some(name) = program.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let stem = strip_exe_suffix(name).0;
    let hash = stem.rsplit_once('-').map(|(_, hash)| hash).unwrap_or("");
    in_deps
        && hash.len() == 16
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Validate a caller-supplied exact `(test name, executable stem)` exemption.
/// The calling test thread must also match; exemptions cannot be borrowed.
/// Returns `program` when the test named `test` is the one test allowed to
/// place an executable with `program`'s production name (the exemption
/// table in this crate); panics for any other test or name. A
/// trailing `.exe` is ignored, so the exemption holds on Windows too.
///
/// When the calling thread carries a test name (libtest names each test's
/// thread after it), that name must be `test` as well, so another test cannot
/// borrow the exemption by passing the exempt test's name.
pub fn exempt_production_executable<'a>(
    test: &str,
    program: &'a Path,
    exemptions: &[(&str, &str)],
) -> &'a Path {
    let file_name = program.file_name().and_then(OsStr::to_str).unwrap_or("");
    let stem = strip_exe_suffix(file_name).0;
    let caller = std::thread::current().name().map(str::to_string);
    let caller_matches = match caller.as_deref() {
        None | Some("main") => true,
        Some(thread) => thread == test || thread.ends_with(&format!("::{test}")),
    };
    assert!(
        caller_matches
            && exemptions
                .iter()
                .any(|&(exempt_test, exempt_name)| exempt_test == test && exempt_name == stem),
        "no production-name exemption for {file_name:?} in test {test:?} (running on \
         thread {caller:?}); test processes run as ckdev-* through \
         cortexkit_test_support::ckdev_binary"
    );
    program
}

/// A `Command` for `program` that refuses (panics) when `program` has a
/// production executable name. Use it for every test spawn of a CortexKit
/// binary, with a path made by [`ckdev_binary`].
pub fn dev_command(program: impl AsRef<Path>) -> Command {
    let program = program.as_ref();
    refuse_production_executable(program);
    Command::new(program)
}

/// Execute a prepared command, returning an error with the exit code or signal
/// number and name on failure. Plain `Command::output` does not reject failed exits.
pub fn checked_output(command: &mut Command) -> io::Result<Output> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{} failed: {}; stderr: {}",
            command.get_program().to_string_lossy(),
            describe_exit_status(output.status),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output)
}

/// Human-readable child failure, including Unix signal kills rather than `None`.
pub fn describe_exit_status(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            let name = match rustix::process::Signal::from_named_raw(signal) {
                Some(rustix::process::Signal::KILL) => "SIGKILL".to_string(),
                Some(rustix::process::Signal::TERM) => "SIGTERM".to_string(),
                Some(rustix::process::Signal::ABORT) => "SIGABRT".to_string(),
                Some(rustix::process::Signal::SEGV) => "SIGSEGV".to_string(),
                Some(s) => format!("{s:?}"),
                None => "unknown signal".to_string(),
            };
            return format!("signal {signal} ({name})");
        }
    }
    format!("exit code {:?}", status.code())
}

/// Compatibility staging: `ck-*` artifacts become `ckdev-*`; other names pass through.
pub fn stage_test_binary(source: &Path) -> PathBuf {
    if source
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|s| s.starts_with("ck-"))
    {
        ckdev_binary(source)
    } else {
        source.to_owned()
    }
}

/// The `ckdev-` name for a built binary's file name: `ck-subc` becomes
/// `ckdev-subc`, `ck` becomes `ckdev-ck`, any other name `n` becomes
/// `ckdev-n`, and a name that already starts with `ckdev-` is returned as it
/// is. A trailing `.exe` stays at the end, so Windows still runs the result.
pub fn ckdev_file_name(file_name: &str) -> String {
    let (stem, exe) = strip_exe_suffix(file_name);
    if stem.starts_with("ckdev-") {
        return file_name.to_string();
    }
    let base = match stem.strip_prefix("ck-") {
        Some(rest) => rest,
        None => stem,
    };
    format!("ckdev-{base}{exe}")
}

fn strip_exe_suffix(name: &str) -> (&str, &str) {
    let len = name.len();
    if len > 4 && name.is_char_boundary(len - 4) && name[len - 4..].eq_ignore_ascii_case(".exe") {
        (&name[..len - 4], &name[len - 4..])
    } else {
        (name, "")
    }
}

/// Publish a built executable under a content-addressed `ckdev-*` name.
/// Already-dev-named paths pass through. Unix copies live under `/tmp`, never
/// `$TMPDIR`, to avoid per-user temporary-directory execution assessment delays.
/// A child writes a private staging copy, then an atomic rename publishes it.
/// This avoids hard-link execution failures and writable descriptors inherited
/// by concurrently spawned children. Published files are read-only, verified
/// against their digest on reuse, and pruned after three days.
/// Use [`checked_output`] to report failed child exits, including signal names.
pub fn ckdev_binary(built: impl AsRef<Path>) -> PathBuf {
    ckdev_binary_at(built.as_ref(), &publish_root())
}

/// Where published `ckdev-` binaries live. `/tmp`, not `$TMPDIR`: see
/// [`ckdev_binary`].
fn publish_root() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from(format!(
            "/tmp/cortexkit-ckdev-{}",
            rustix::process::getuid().as_raw()
        ))
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("cortexkit-ckdev")
    }
}

fn ckdev_binary_at(built: &Path, root: &Path) -> PathBuf {
    let file_name = built
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_else(|| panic!("built binary has no UTF-8 file name: {}", built.display()));
    let dev_name = ckdev_file_name(file_name);
    #[cfg(windows)]
    let dev_name = if dev_name.to_ascii_lowercase().ends_with(".exe") {
        dev_name
    } else {
        format!("{dev_name}.exe")
    };
    if dev_name == file_name {
        return built.to_path_buf();
    }
    let placed = publish(built, &dev_name, root).unwrap_or_else(|error| {
        panic!(
            "could not publish {} as {dev_name} under {}: {error}",
            built.display(),
            root.display()
        )
    });
    refuse_production_executable(&placed);
    placed
}

/// The content address: the first 32 hex digits of SHA-256 over the
/// published name, a NUL, and the file's bytes.
fn content_digest(path: &Path, dev_name: &str) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(dev_name.as_bytes());
    hasher.update([0u8]);
    let mut file = fs::File::open(path)?;
    io::copy(&mut file, &mut hasher)?;
    let digest: String = hasher.finalize()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(digest)
}

fn publish(built: &Path, dev_name: &str, root: &Path) -> io::Result<PathBuf> {
    let digest = content_digest(built, dev_name)?;
    prepare_root(root)?;
    let dir = root.join(&digest);
    let placed = dir.join(dev_name);
    if dir.exists() {
        verify_published(&placed, dev_name, &digest)?;
        return Ok(placed);
    }
    prune_stale(root);
    let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let staging = root.join(format!(".staging-{}-{nonce}", std::process::id()));
    fs::create_dir(&staging)?;
    let staged = staging.join(dev_name);
    let result = (|| {
        copy_executable(built, &staged)?;
        // Permissions are set by path (chmod), so no writable descriptor is
        // held; on Unix the published file is read-only and executable.
        fs::set_permissions(&staged, published_permissions(built)?)?;
        if content_digest(&staged, dev_name)? != digest {
            return Err(io::Error::other(format!(
                "{} changed while it was being published",
                built.display()
            )));
        }
        seal_dir(&staging)?;
        match fs::rename(&staging, &dir) {
            Ok(()) => Ok(()),
            // Another process published the same build first; its file is
            // checked below like any reused one.
            Err(_) if dir.exists() => Ok(()),
            Err(error) => Err(error),
        }
    })();
    if staging.exists() {
        remove_published(&staging);
    }
    result?;
    verify_published(&placed, dev_name, &digest)?;
    Ok(placed)
}

/// Re-hashes a published file before trusting it: a partial or altered file
/// is refused, never run.
fn verify_published(placed: &Path, dev_name: &str, digest: &str) -> io::Result<()> {
    let directory = fs::symlink_metadata(placed.parent().expect("publication directory"))?;
    let file = fs::symlink_metadata(placed)?;
    if !directory.is_dir() || directory.is_symlink() || !file.is_file() || file.is_symlink() {
        return Err(io::Error::other(
            "publication must be a real directory and regular file",
        ));
    }
    let found = content_digest(placed, dev_name)?;
    if found == digest {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "published {} does not match its content address {digest} (found {found}); \
             remove {} to republish it",
            placed.display(),
            placed.parent().unwrap_or(placed).display()
        )))
    }
}

/// Creates the publish root, and refuses one this user does not own or that
/// others can write: `/tmp` is shared, and a planted file there would be run.
fn prepare_root(root: &Path) -> io::Result<()> {
    if !root.exists() {
        fs::create_dir_all(root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
    }
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.is_symlink() {
        return Err(io::Error::other("publish root must be a real directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = rustix::process::getuid().as_raw();
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::other(format!(
                "{} must be a directory owned by uid {uid} and writable by no one else",
                root.display()
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn published_permissions(built: &Path) -> io::Result<fs::Permissions> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(built)?.permissions().mode();
    Ok(fs::Permissions::from_mode(mode & 0o555))
}

#[cfg(not(unix))]
fn published_permissions(built: &Path) -> io::Result<fs::Permissions> {
    let mut permissions = fs::metadata(built)?.permissions();
    permissions.set_readonly(true);
    Ok(permissions)
}

/// Makes a published directory read-only, so nothing can be renamed into it
/// or removed from it.
fn seal_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Removes a staging or published directory, restoring the write permission
/// sealing took away.
fn remove_published(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    #[cfg(windows)]
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                let mut permissions = metadata.permissions();
                // This branch clears a Windows file attribute, not Unix mode bits.
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                let _ = fs::set_permissions(entry.path(), permissions);
            }
        }
    }
    let _ = fs::remove_dir_all(dir);
}

/// Published builds are kept this long after publishing; older ones are
/// removed when a new build is published. A process still running one keeps
/// its open image, and asking again republishes it.
const PUBLISHED_RETENTION: std::time::Duration = std::time::Duration::from_secs(3 * 24 * 60 * 60);

fn prune_stale(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.is_dir() || metadata.is_symlink() {
            continue;
        }
        let stale = Ok::<_, io::Error>(metadata)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > PUBLISHED_RETENTION);
        if stale {
            remove_published(&entry.path());
        }
    }
}

/// Copies through a `cp` child on Unix: a writable descriptor held by this
/// multi-threaded test process would be inherited by a child another thread
/// forks at that moment, and executing the copy while that child still holds
/// it fails with "text file busy".
fn copy_executable(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let status = Command::new("cp").arg(src).arg(dst).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "cp failed: {}",
                describe_exit_status(status)
            )))
        }
    }
    #[cfg(not(unix))]
    {
        fs::copy(src, dst).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_child_fixture() {
        #[cfg(unix)]
        if std::env::var("CORTEXKIT_STAGED_FIXTURE").as_deref() == Ok("kill") {
            rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::KILL)
                .unwrap();
            panic!("SIGKILL did not terminate child");
        }
    }

    #[cfg(unix)]
    fn built_fixture(scratch: &crate::ScratchDir) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let built = scratch.join(format!("ck-stage-{}-{nonce}", std::process::id()));
        copy_executable(&std::env::current_exe().unwrap(), &built).unwrap();
        built
    }
    #[cfg(unix)]
    fn fixture_command(placed: &Path) -> Command {
        let mut command = dev_command(placed);
        command.args([
            "--exact",
            "binaries::tests::staged_child_fixture",
            "--nocapture",
        ]);
        command
    }
    #[cfg(unix)]
    #[test]
    fn staging_lives_under_tmp_and_other_names_pass_through() {
        let scratch = crate::ScratchDir::new("publish-tmp");
        let built = built_fixture(&scratch);
        let placed = stage_test_binary(&built);
        assert!(
            placed.starts_with("/tmp"),
            "published at {}",
            placed.display()
        );
        assert!(placed
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("ckdev-"));
        assert_eq!(
            stage_test_binary(Path::new("/nonexistent/other")),
            Path::new("/nonexistent/other")
        );
        checked_output(&mut fixture_command(&placed)).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn sigkill_failure_names_signal_nine() {
        let scratch = crate::ScratchDir::new("signal-status");
        let placed = ckdev_binary(built_fixture(&scratch));
        let error =
            checked_output(fixture_command(&placed).env("CORTEXKIT_STAGED_FIXTURE", "kill"))
                .unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains("signal 9") && text.contains("SIGKILL"),
            "{text}"
        );
        assert!(!text.contains("exit code None"), "{text}");
    }
    #[cfg(unix)]
    #[test]
    fn concurrent_staging_publishes_one_copy_and_every_spawn_succeeds() {
        let scratch = crate::ScratchDir::new("staging-concurrency");
        let built = built_fixture(&scratch);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let built = built.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let placed = ckdev_binary(built);
                    let output = checked_output(&mut fixture_command(&placed)).unwrap();
                    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
                    placed
                })
            })
            .collect();
        let paths: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(paths.iter().all(|p| p == &paths[0]));
        assert_eq!(fs::read_dir(paths[0].parent().unwrap()).unwrap().count(), 1);
        use std::os::unix::fs::MetadataExt;
        let inode = fs::metadata(&paths[0]).unwrap().ino();
        assert_eq!(ckdev_binary(&built), paths[0]);
        assert_eq!(fs::metadata(&paths[0]).unwrap().ino(), inode);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn published_file_has_no_write_descriptor_in_test_process() {
        use std::os::unix::fs::MetadataExt;
        let scratch = crate::ScratchDir::new("publish-descriptors");
        let placed = ckdev_binary(built_fixture(&scratch));
        let published = fs::metadata(&placed).unwrap();
        for entry in fs::read_dir("/proc/self/fd").unwrap().flatten() {
            let Ok(metadata) = fs::metadata(entry.path()) else {
                continue;
            };
            if metadata.dev() != published.dev() || metadata.ino() != published.ino() {
                continue;
            }
            let info =
                fs::read_to_string(Path::new("/proc/self/fdinfo").join(entry.file_name())).unwrap();
            let flags = info
                .lines()
                .find_map(|line| line.strip_prefix("flags:\t"))
                .unwrap();
            let flags = u32::from_str_radix(flags.trim(), 8).unwrap();
            assert_eq!(
                flags & 3,
                0,
                "write descriptor for published executable: {info}"
            );
        }
        checked_output(&mut fixture_command(&placed)).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn three_day_prune_keeps_recent_and_symlink_entries() {
        let root = PublishRoot::new("publish-prune");
        let old = root.path().join("old");
        let recent = root.path().join("recent");
        for path in [&old, &recent] {
            fs::create_dir(path).unwrap();
            fs::write(path.join("payload"), "keep").unwrap();
        }
        fs::File::open(&old)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                std::time::SystemTime::now()
                    - PUBLISHED_RETENTION
                    - std::time::Duration::from_secs(10),
            ))
            .unwrap();
        std::os::unix::fs::symlink(&old, root.path().join("link")).unwrap();
        prune_stale(&root.path());
        assert!(!old.exists());
        assert!(recent.join("payload").exists());
        assert!(fs::symlink_metadata(root.path().join("link"))
            .unwrap()
            .is_symlink());
    }

    #[test]
    fn ckdev_names_drop_the_production_prefix_and_keep_exe() {
        assert_eq!(ckdev_file_name("ck-subc"), "ckdev-subc");
        assert_eq!(ckdev_file_name("ck-subc-mcp"), "ckdev-subc-mcp");
        assert_eq!(ckdev_file_name("ck-bus.exe"), "ckdev-bus.exe");
        assert_eq!(ckdev_file_name("ck"), "ckdev-ck");
        assert_eq!(ckdev_file_name("ck.exe"), "ckdev-ck.exe");
        assert_eq!(ckdev_file_name("ck-under-test"), "ckdev-under-test");
        assert_eq!(ckdev_file_name("fake-aft-stub"), "ckdev-fake-aft-stub");
        assert_eq!(ckdev_file_name("ckdev-subc"), "ckdev-subc");
        assert_eq!(ckdev_file_name("ckdev-subc.exe"), "ckdev-subc.exe");
    }

    #[test]
    fn production_names_are_recognised() {
        for name in ["ck-subc", "ck-bus.exe", "CK-SUBC.EXE", "ck", "ck.exe"] {
            assert!(
                is_production_executable_name(OsStr::new(name)),
                "{name} is a production executable name"
            );
        }
        for name in [
            "ckdev-subc",
            "ckdev-ck.exe",
            "cksum",
            "fake-aft-stub",
            "subc",
        ] {
            assert!(
                !is_production_executable_name(OsStr::new(name)),
                "{name} is not a production executable name"
            );
        }
    }

    /// The guard itself: the spawn helper refuses a `ck-*` path before any
    /// process starts.
    #[test]
    #[should_panic(
        expected = "refusing to run a test process under the production executable name"
    )]
    fn dev_command_refuses_a_production_named_binary() {
        let _ = dev_command(Path::new("/nonexistent/target/debug/ck-subc"));
    }

    #[test]
    fn cargo_test_harness_for_the_ck_bin_is_not_refused() {
        let _ = dev_command(Path::new("/w/target/debug/deps/ck-0123456789abcdef"));
        let _ = dev_command(Path::new("/w/target/debug/deps/ck-0123456789abcdef.exe"));
        for refused in [
            "/w/target/debug/ck-0123456789abcdef",
            "/w/target/debug/deps/ck-subc",
            "/w/target/debug/deps/ck-0123456789ABCDEF",
            "/w/target/debug/deps/ck-0123456789abcde",
        ] {
            assert!(
                std::panic::catch_unwind(|| dev_command(Path::new(refused))).is_err(),
                "{refused} must be refused"
            );
        }
    }

    const EXEMPT_TEST: &str = "executable_discovery";
    const EXEMPTIONS: &[(&str, &str)] = &[(EXEMPT_TEST, "ck-twin"), (EXEMPT_TEST, "ck-twin-two")];

    /// The exemption admits exactly the two twin copies, and only on the
    /// thread of the one test it names.
    #[test]
    fn the_exemption_admits_only_the_named_test_and_its_two_copies() {
        let outcomes = std::thread::Builder::new()
            .name(EXEMPT_TEST.to_string())
            .spawn(|| {
                [
                    "ck-twin",
                    "ck-twin-two",
                    "ck-twin.exe",
                    "ck-twin-three",
                    "ck-subc",
                    "ck",
                ]
                .map(|name| {
                    let path = Path::new("/fixture/bin").join(name);
                    std::panic::catch_unwind(|| {
                        exempt_production_executable(EXEMPT_TEST, &path, EXEMPTIONS);
                    })
                    .is_ok()
                })
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(outcomes, [true, true, true, false, false, false]);
        // Another test cannot borrow the exemption by naming the exempt test:
        // this thread carries this test's own name.
        assert!(std::panic::catch_unwind(|| {
            exempt_production_executable(
                EXEMPT_TEST,
                Path::new("/fixture/bin/ck-twin"),
                EXEMPTIONS,
            );
        })
        .is_err());
    }

    /// A publish root inside a test temp dir. Published directories are
    /// sealed read-only, so the guard restores write permission before the
    /// temp dir removes the tree.
    #[cfg(unix)]
    struct PublishRoot {
        temp: crate::ScratchDir,
    }

    #[cfg(unix)]
    impl PublishRoot {
        fn new(label: &str) -> Self {
            let temp = crate::ScratchDir::new(label);
            fs::create_dir_all(temp.join("root")).unwrap();
            Self { temp }
        }

        fn path(&self) -> PathBuf {
            self.temp.join("root")
        }
    }

    #[cfg(unix)]
    impl Drop for PublishRoot {
        fn drop(&mut self) {
            if let Ok(entries) = fs::read_dir(self.path()) {
                for entry in entries.flatten() {
                    remove_published(&entry.path());
                }
            }
        }
    }

    /// The guard wired through the helper: binaries built as `ck-subc` and `ck`
    /// are published under `ckdev-*` names and the spawn helper runs them. If
    /// the helper handed back the built path, `dev_command` would refuse it
    /// here.
    #[cfg(unix)]
    #[test]
    fn a_placed_production_binary_spawns_under_its_ckdev_name() {
        let build = crate::ScratchDir::new("ckdev-guard-build");
        let root = PublishRoot::new("ckdev-guard-root");
        for (name, published) in [("ck-subc", "ckdev-subc"), ("ck", "ckdev-ck")] {
            let built = build.join(name);
            write_script(&built, &format!("#!/bin/sh\necho {name}\n"));
            let placed = ckdev_binary_at(&built, &root.path());
            let output = dev_command(&placed).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{name}\n"));
            assert_eq!(placed.file_name().unwrap(), published);
            assert_eq!(
                placed.parent().and_then(Path::parent),
                Some(root.path().as_path())
            );
        }
    }

    /// Two requests for the same build, even from different build paths, get
    /// the same published file; a changed binary gets a new one.
    #[cfg(unix)]
    #[test]
    fn the_same_build_shares_one_published_path_and_a_changed_build_gets_another() {
        let first = crate::ScratchDir::new("ckdev-address-first");
        let second = crate::ScratchDir::new("ckdev-address-second");
        let root = PublishRoot::new("ckdev-address-root");
        let (a, b) = (first.join("ck-subc"), second.join("ck-subc"));
        write_script(&a, "#!/bin/sh\necho one\n");
        write_script(&b, "#!/bin/sh\necho one\n");
        let placed = ckdev_binary_at(&a, &root.path());
        assert_eq!(ckdev_binary_at(&a, &root.path()), placed);
        assert_eq!(ckdev_binary_at(&b, &root.path()), placed);

        let changed = crate::ScratchDir::new("ckdev-address-changed");
        let c = changed.join("ck-subc");
        write_script(&c, "#!/bin/sh\necho two\n");
        let republished = ckdev_binary_at(&c, &root.path());
        assert_ne!(republished, placed);
        assert_eq!(republished.file_name(), placed.file_name());
        let output = dev_command(&republished).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "two\n");
        // The first build is still published, unchanged.
        let output = dev_command(&placed).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "one\n");
    }

    /// The published file is a read-only copy in a read-only directory: its
    /// own inode, the same bytes, executable, and no staging left behind.
    #[cfg(unix)]
    #[test]
    fn a_published_binary_is_a_sealed_copy() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let build = crate::ScratchDir::new("ckdev-copy-build");
        let root = PublishRoot::new("ckdev-copy-root");
        let built = build.join("ck-subc");
        write_script(&built, "#!/bin/sh\necho copied\n");
        let placed = ckdev_binary_at(&built, &root.path());
        let (source, copy) = (
            fs::metadata(&built).unwrap(),
            fs::metadata(&placed).unwrap(),
        );
        assert_ne!(
            (source.dev(), source.ino()),
            (copy.dev(), copy.ino()),
            "a published binary must not share the built binary's inode"
        );
        assert_eq!(fs::read(&built).unwrap(), fs::read(&placed).unwrap());
        assert_eq!(copy.permissions().mode() & 0o777, 0o555);
        let dir = placed.parent().unwrap();
        assert_eq!(
            fs::metadata(dir).unwrap().permissions().mode() & 0o777,
            0o555
        );
        assert!(
            rustix::process::getuid().is_root()
                || fs::OpenOptions::new().write(true).open(&placed).is_err(),
            "a published binary must not be writable"
        );
        let entries: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, [dir.file_name().unwrap()]);
    }

    /// A published file that no longer matches its address is refused, not
    /// run, and so is a publish root other users can write.
    #[cfg(unix)]
    #[test]
    fn altered_publications_and_shared_roots_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let build = crate::ScratchDir::new("ckdev-refuse-build");
        let root = PublishRoot::new("ckdev-refuse-root");
        let built = build.join("ck-bus");
        write_script(&built, "#!/bin/sh\necho genuine\n");
        let placed = ckdev_binary_at(&built, &root.path());
        let dir = placed.parent().unwrap().to_path_buf();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&placed, fs::Permissions::from_mode(0o755)).unwrap();
        let altered = build.join("altered");
        write_script(&altered, "#!/bin/sh\necho planted\n");
        fs::remove_file(&placed).unwrap();
        copy_executable(&altered, &placed).unwrap();
        let error = publish(&built, "ckdev-bus", &root.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match its content address"),
            "{error}"
        );

        let open = PublishRoot::new("ckdev-open-root");
        fs::set_permissions(open.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let error = publish(&built, "ckdev-bus", &open.path()).unwrap_err();
        assert!(
            error.to_string().contains("writable by no one else"),
            "{error}"
        );
    }

    #[test]
    fn an_already_ckdev_binary_is_returned_unchanged() {
        let name = format!("/nonexistent/ckdev-subc{}", std::env::consts::EXE_SUFFIX);
        let built = Path::new(&name);
        assert_eq!(ckdev_binary(built), built);
    }

    /// Writes through a staging file and a `cp` child, so this multi-threaded
    /// test process never holds a writable descriptor to the file it runs. On
    /// Linux, a child forked by another test thread inherits any open write
    /// descriptor until it execs, and while one exists, executing the file fails
    /// with ETXTBSY ("text file busy").
    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let staging = path.with_extension("staging");
        fs::write(&staging, body).unwrap();
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o755)).unwrap();
        copy_executable(&staging, path).unwrap();
        fs::remove_file(&staging).unwrap();
    }
}
