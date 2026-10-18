//! Content-addressed executable copies and production-name guards.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static CACHE_PRUNER: CachePruner = CachePruner::new();

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u128,
    len: u64,
    modified_before_epoch: bool,
    modified_nanos: u128,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ContentDigestKey {
    identity: FileIdentity,
    dev_name: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PublishedVerificationKey {
    identity: FileIdentity,
    dev_name: String,
    digest: String,
}

/// How a source build becomes the bytes published at its content address.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Publication {
    /// The published file is a byte-for-byte copy of the source.
    Copy,
    /// macOS Mach-O only: the copy has its debug map removed with `strip -S`
    /// and is then ad hoc re-signed. A debug build's debug map (N_OSO stab
    /// entries) names every object file by absolute path in the build folder;
    /// symbolicating the running copy, for example for a panic backtrace,
    /// opens those files, and when the build folder is on a removable volume
    /// macOS shows a privacy prompt that stalls the test. The symbol table
    /// stays, so backtraces still show function names.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    StrippedMachO,
}

impl Publication {
    /// Bytes hashed ahead of the published name when computing the content
    /// address. A copy hashes nothing extra, so its addresses are the ones
    /// earlier releases used and their copies stay valid. A stripped
    /// publication hashes a format tag: a Mach-O source therefore gets a new
    /// address, and an unstripped copy published by 0.1.8 or earlier is never
    /// reused as if it were stripped. Published names always start with
    /// `ckdev-` and the tag does not, so the two hash inputs cannot coincide.
    fn address_tag(self) -> &'static [u8] {
        match self {
            Self::Copy => b"",
            Self::StrippedMachO => b"macos-strip-debug-map-adhoc-sign/1\0",
        }
    }
}

fn content_digest_memo() -> &'static Mutex<HashMap<ContentDigestKey, (String, Publication)>> {
    static MEMO: OnceLock<Mutex<HashMap<ContentDigestKey, (String, Publication)>>> =
        OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(HashMap::new()))
}

fn published_verification_memo() -> &'static Mutex<HashSet<PublishedVerificationKey>> {
    static MEMO: OnceLock<Mutex<HashSet<PublishedVerificationKey>>> = OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(HashSet::new()))
}

#[cfg(test)]
thread_local! {
    static HASH_COUNTS: std::cell::Cell<(usize, usize, usize, usize)> = const {
        std::cell::Cell::new((0, 0, 0, 0))
    };
}

#[cfg(test)]
fn record_hash_attempt() {
    HASH_COUNTS.with(|counts| {
        let (total, built, published, staged) = counts.get();
        counts.set((total + 1, built, published, staged));
    });
}

#[cfg(test)]
fn record_built_hash() {
    HASH_COUNTS.with(|counts| {
        let (total, built, published, staged) = counts.get();
        counts.set((total, built + 1, published, staged));
    });
}

#[cfg(test)]
fn record_published_hash() {
    HASH_COUNTS.with(|counts| {
        let (total, built, published, staged) = counts.get();
        counts.set((total, built, published + 1, staged));
    });
}

#[cfg(test)]
fn record_staged_hash() {
    HASH_COUNTS.with(|counts| {
        let (total, built, published, staged) = counts.get();
        counts.set((total, built, published, staged + 1));
    });
}

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
/// A binary already named `ckdev-*` is copied as well. Unix copies use the selected
/// temporary base: worker task directories are writable in the sandbox, while macOS
/// can delay launches of fresh copies from `/var/folders`.
/// A child writes a private staging copy, then an atomic rename publishes it.
/// This avoids hard-link execution failures and writable descriptors inherited
/// by concurrently spawned children. Published files are read-only, verified
/// against their digest on reuse. New copies opportunistically prune unused
/// builds after three days and limit the cache to 10 GiB, keeping recent uses.
/// On macOS a Mach-O copy has its debug map stripped and is re-signed before
/// it is published, so symbolicating it never opens object files in the
/// build folder; panics with the tool's error if that fails.
/// Use [`checked_output`] to report failed child exits, including signal names.
pub fn ckdev_binary(built: impl AsRef<Path>) -> PathBuf {
    ckdev_binary_at(built.as_ref(), &publish_root())
}

/// Where published `ckdev-` binaries live. macOS can delay launches of fresh
/// copies from `/var/folders`, so valid worker temp roots take precedence over it.
fn publish_root() -> PathBuf {
    #[cfg(unix)]
    {
        crate::private_temp_base(std::env::var_os("TMPDIR").as_deref()).join(format!(
            "cortexkit-ckdev-{}",
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
    // A binary already named `ckdev-*` is copied too, never run from where it
    // was built. Its build folder can sit on an external volume. There, macOS
    // shows a privacy prompt before a test daemon starts a module that runs
    // under its own privacy identity, and the test waits on that prompt.
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

/// A file digest: the first 32 hex digits of SHA-256 over the published name,
/// a NUL, and the file's bytes. A copied publication's file has the digest of
/// its source, which is its content address.
fn content_digest(path: &Path, dev_name: &str) -> io::Result<String> {
    tagged_digest(path, b"", dev_name)
}

/// The content address of a source build and how it is published: the
/// digest of the source, preceded by the publication's format tag (see
/// [`Publication::address_tag`]).
fn source_address(path: &Path, dev_name: &str) -> io::Result<(String, Publication)> {
    let publication = publication_for(path)?;
    let digest = tagged_digest(path, publication.address_tag(), dev_name)?;
    Ok((digest, publication))
}

/// On macOS a Mach-O file (thin or universal) is stripped when published;
/// anything else, such as a shell script, and every file on other platforms
/// is copied as it is.
fn publication_for(path: &Path) -> io::Result<Publication> {
    #[cfg(target_os = "macos")]
    {
        use std::io::Read;
        let mut magic = [0u8; 4];
        let mut file = fs::File::open(path)?;
        let mut filled = 0;
        while filled < magic.len() {
            match file.read(&mut magic[filled..])? {
                0 => return Ok(Publication::Copy),
                read => filled += read,
            }
        }
        // MH_MAGIC and MH_MAGIC_64 in either byte order, then FAT_MAGIC and
        // FAT_MAGIC_64, which universal binaries store big-endian.
        let mach_o = matches!(
            u32::from_be_bytes(magic),
            0xfeed_face | 0xfeed_facf | 0xcefa_edfe | 0xcffa_edfe | 0xcafe_babe | 0xcafe_babf
        );
        Ok(if mach_o {
            Publication::StrippedMachO
        } else {
            Publication::Copy
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Ok(Publication::Copy)
    }
}

fn tagged_digest(path: &Path, tag: &[u8], dev_name: &str) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    #[cfg(test)]
    record_hash_attempt();
    let mut hasher = Sha256::new();
    hasher.update(tag);
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

fn file_identity(path: &Path) -> io::Result<FileIdentity> {
    let metadata = fs::metadata(path)?;
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), u128::from(metadata.ino()))
    };
    #[cfg(windows)]
    let (device, inode) = match file_id::get_file_id(path)? {
        file_id::FileId::Inode {
            device_id,
            inode_number,
        } => (device_id, u128::from(inode_number)),
        file_id::FileId::LowRes {
            volume_serial_number,
            file_index,
        } => (u64::from(volume_serial_number), u128::from(file_index)),
        file_id::FileId::HighRes {
            volume_serial_number,
            file_id,
        } => (volume_serial_number, file_id),
    };
    #[cfg(not(any(unix, windows)))]
    let (device, inode) = return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file identity is unavailable on this platform",
    ));
    let modified = metadata.modified()?;
    let (modified_before_epoch, modified_nanos) = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => (false, duration.as_nanos()),
        Err(error) => (true, error.duration().as_nanos()),
    };
    Ok(FileIdentity {
        device,
        inode,
        len: metadata.len(),
        modified_before_epoch,
        modified_nanos,
    })
}

fn memoized_source_address(path: &Path, dev_name: &str) -> io::Result<(String, Publication)> {
    let identity = match file_identity(path) {
        Ok(identity) => identity,
        Err(_) => {
            #[cfg(test)]
            record_built_hash();
            return source_address(path, dev_name);
        }
    };
    let key = ContentDigestKey {
        identity,
        dev_name: dev_name.to_owned(),
    };
    let cached = {
        content_digest_memo()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .cloned()
    };
    if let Some(address) = cached {
        return Ok(address);
    }
    #[cfg(test)]
    record_built_hash();
    let address = source_address(path, dev_name)?;
    if matches!(file_identity(path), Ok(identity) if identity == key.identity) {
        content_digest_memo()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, address.clone());
    }
    Ok(address)
}

fn publish(built: &Path, dev_name: &str, root: &Path) -> io::Result<PathBuf> {
    publish_with_pruner(built, dev_name, root, |placed| {
        CACHE_PRUNER.maybe_prune(root, placed, SystemTime::now(), remove_published);
    })
}

fn publish_with_pruner(
    built: &Path,
    dev_name: &str,
    root: &Path,
    prune: impl FnOnce(&Path),
) -> io::Result<PathBuf> {
    let (digest, publication) = memoized_source_address(built, dev_name)?;
    prepare_root(root, 0o022)?;
    // Shared locks cover lookup, publication and the use marker. An exclusive
    // prune cannot delete an entry between verifying it and recording its use.
    let cache_lock = CacheLock::shared(root);
    if let Err(error) = &cache_lock {
        log_cache_error(root, error);
    }
    let dir = root.join(&digest);
    let placed = dir.join(dev_name);
    if dir.exists() {
        verify_published(&placed, dev_name, &digest, publication)?;
        record_use(root, &digest, SystemTime::now());
        return Ok(placed);
    }
    let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let staging = root.join(format!(".staging-{}-{nonce}", std::process::id()));
    fs::create_dir(&staging)?;
    let staged = staging.join(dev_name);
    let result = (|| {
        copy_executable(built, &staged)?;
        #[cfg(test)]
        record_staged_hash();
        if source_address(&staged, dev_name)? != (digest.clone(), publication) {
            return Err(io::Error::other(format!(
                "{} changed while it was being published",
                built.display()
            )));
        }
        // A failure here leaves the staging directory unpublished; it is
        // removed below and nothing is renamed into place, so a copy that
        // still carries its debug map is never handed out.
        if publication == Publication::StrippedMachO {
            strip_debug_map(&staged)?;
        }
        // Permissions are set by path (chmod), so no writable descriptor is
        // held; on Unix the published file is read-only and executable.
        fs::set_permissions(&staged, published_permissions(built)?)?;
        if publication == Publication::StrippedMachO {
            write_publication_record(&staging, &content_digest(&staged, dev_name)?)?;
        }
        seal_dir(&staging)?;
        match fs::rename(&staging, &dir) {
            Ok(()) => Ok(true),
            // Another process published the same build first; its file is
            // checked below like any reused one.
            Err(_) if dir.exists() => Ok(false),
            Err(error) => Err(error),
        }
    })();
    if staging.exists() {
        if let Err(error) = remove_published(&staging) {
            log_cache_error(&staging, &error);
        }
    }
    let created = result?;
    verify_published(&placed, dev_name, &digest, publication)?;
    record_use(root, &digest, SystemTime::now());
    let can_prune = cache_lock.is_ok();
    drop(cache_lock);
    if created && can_prune {
        prune(&placed);
    }
    Ok(placed)
}

/// The file in a stripped publication's sealed directory that records the
/// digest of the published executable. Published names start with `ckdev-`,
/// so it never collides with the executable.
const PUBLICATION_RECORD: &str = ".published-digest";

/// Records the stripped executable's digest next to it, inside the staging
/// directory, before the directory is sealed and renamed into place, so the
/// record and the executable appear together or not at all.
fn write_publication_record(staging: &Path, published_digest: &str) -> io::Result<()> {
    let record = staging.join(PUBLICATION_RECORD);
    fs::write(&record, format!("{published_digest}\n"))?;
    let mut permissions = fs::metadata(&record)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&record, permissions)
}

/// The digest a published file must have to be the correct publication of
/// the source at content address `digest`.
///
/// A copy must hash to the address itself, so its bytes are the source's.
/// A stripped copy cannot: `strip` and `codesign` change its bytes, and
/// recomputing them on every cache hit would cost a strip and a signature
/// per process. Instead the publisher hashes the stripped file and records
/// that digest in the same sealed directory before the atomic rename, and
/// reuse checks the file against the record. A partial, replaced or altered
/// executable is still refused. The source-to-copy binding now rests on the
/// publisher: a writer with this user's id who rewrites both the executable
/// and its record in the user-private cache is not detected.
fn expected_published_digest(
    placed: &Path,
    digest: &str,
    publication: Publication,
) -> io::Result<String> {
    match publication {
        Publication::Copy => Ok(digest.to_owned()),
        Publication::StrippedMachO => {
            let record = placed
                .parent()
                .expect("publication directory")
                .join(PUBLICATION_RECORD);
            let malformed = |detail: &dyn std::fmt::Display| {
                io::Error::other(format!(
                    "publication record {} is unusable ({detail}); remove {} to republish it",
                    record.display(),
                    placed.parent().unwrap_or(placed).display()
                ))
            };
            let metadata = fs::symlink_metadata(&record).map_err(|error| malformed(&error))?;
            if !metadata.is_file() || metadata.is_symlink() {
                return Err(malformed(&"not a regular file"));
            }
            let text = fs::read_to_string(&record).map_err(|error| malformed(&error))?;
            let recorded = text.trim_end_matches('\n');
            if recorded.len() == 32
                && recorded
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                Ok(recorded.to_owned())
            } else {
                Err(malformed(&"not a 32-digit lowercase hex digest"))
            }
        }
    }
}

fn published_mismatch(placed: &Path, digest: &str, expected: &str, found: &str) -> io::Error {
    io::Error::other(format!(
        "published {} does not match its content address {digest} \
         (expected {expected}, found {found}); remove {} to republish it",
        placed.display(),
        placed.parent().unwrap_or(placed).display()
    ))
}

/// Re-hashes a published file before trusting it: a partial or altered file
/// is refused, never run.
fn verify_published(
    placed: &Path,
    dev_name: &str,
    digest: &str,
    publication: Publication,
) -> io::Result<()> {
    let directory = fs::symlink_metadata(placed.parent().expect("publication directory"))?;
    let file = fs::symlink_metadata(placed)?;
    if !directory.is_dir() || directory.is_symlink() || !file.is_file() || file.is_symlink() {
        return Err(io::Error::other(
            "publication must be a real directory and regular file",
        ));
    }
    let key = PublishedVerificationKey {
        identity: match file_identity(placed) {
            Ok(identity) => identity,
            Err(_) => {
                return verify_published_bytes(placed, dev_name, digest, publication);
            }
        },
        dev_name: dev_name.to_owned(),
        digest: digest.to_owned(),
    };
    let verified = {
        published_verification_memo()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&key)
    };
    if verified {
        return Ok(());
    }
    #[cfg(test)]
    record_published_hash();
    let expected = expected_published_digest(placed, digest, publication)?;
    let found = content_digest(placed, dev_name)?;
    let identity_unchanged = match file_identity(placed) {
        Ok(identity) if identity == key.identity => true,
        Ok(_) => {
            return Err(io::Error::other(format!(
                "published {} changed while it was being verified",
                placed.display()
            )));
        }
        Err(_) => false,
    };
    if found == expected {
        if identity_unchanged {
            published_verification_memo()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(key);
        }
        Ok(())
    } else {
        Err(published_mismatch(placed, digest, &expected, &found))
    }
}

fn verify_published_bytes(
    placed: &Path,
    dev_name: &str,
    digest: &str,
    publication: Publication,
) -> io::Result<()> {
    #[cfg(test)]
    record_published_hash();
    let expected = expected_published_digest(placed, digest, publication)?;
    let found = content_digest(placed, dev_name)?;
    if found == expected {
        Ok(())
    } else {
        Err(published_mismatch(placed, digest, &expected, &found))
    }
}

/// Creates and verifies a real root. On Unix, `forbidden_access` is the mask
/// of group and other permission bits that must be absent.
fn prepare_root(root: &Path, forbidden_access: u32) -> io::Result<()> {
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
        return Err(io::Error::other("root must be a real directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = rustix::process::getuid().as_raw();
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & forbidden_access != 0 {
            return Err(io::Error::other(format!(
                "{} must be a directory owned by uid {uid} and writable by no one else, with no group or other permission bits in mask {forbidden_access:#o}",
                root.display()
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = forbidden_access;
    Ok(())
}

/// Creates a private root and refuses one that group or other users can access.
/// `/tmp` is shared, so every entry in this directory must belong to this user.
/// Only the macOS scratch root uses it outside tests.
#[cfg(any(target_os = "macos", all(unix, test)))]
pub(crate) fn prepare_private_root(root: &Path) -> io::Result<()> {
    prepare_root(root, 0o077)
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
fn remove_published(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let mut permissions = entry.metadata()?.permissions();
            // This branch clears a Windows file attribute, not Unix mode bits.
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(entry.path(), permissions)?;
        }
    }
    let result = fs::remove_dir_all(dir);
    if result.is_err() && dir.exists() {
        if let Err(error) = seal_dir(dir) {
            log_cache_error(dir, &error);
        }
        #[cfg(windows)]
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    let mut permissions = metadata.permissions();
                    permissions.set_readonly(true);
                    if let Err(error) = fs::set_permissions(entry.path(), permissions) {
                        log_cache_error(&entry.path(), &error);
                    }
                }
            }
        }
    }
    result
}

// Three days keeps builds warm across ordinary test sessions without retaining
// every historical build in the shared per-user cache.
const PUBLISHED_RETENTION: Duration = Duration::from_secs(3 * 24 * 60 * 60);
// Half an hour protects paths handed to other test processes before they spawn.
const RECENT_USE_GRACE: Duration = Duration::from_secs(30 * 60);
// An hourly scan bounds filesystem work when many repositories run tests.
const PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);
// Ten GiB leaves room for large test executables without letting the shared
// cache consume tens of GiB and crowd out builds on the same disk.
const CACHE_SIZE_LIMIT: u64 = 10 * 1024 * 1024 * 1024;
const LAST_PRUNE: &str = ".last-prune";

fn cache_lock(root: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".cache-lock"))
}

struct CacheLock {
    file: fs::File,
    root: PathBuf,
}

impl CacheLock {
    fn shared(root: &Path) -> io::Result<Self> {
        let file = cache_lock(root)?;
        file.lock_shared()?;
        Ok(Self {
            file,
            root: root.to_owned(),
        })
    }

    fn try_exclusive(root: &Path) -> io::Result<Self> {
        let file = cache_lock(root)?;
        // Contention (WouldBlock) is the normal case when several test threads
        // publish at once; the caller decides whether it is worth reporting.
        file.try_lock().map_err(io::Error::from)?;
        Ok(Self {
            file,
            root: root.to_owned(),
        })
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        // A concurrent fork can inherit this open file description until exec.
        // Closing only our descriptor would leave its flock held by the child;
        // explicitly unlocking releases it for every inherited descriptor.
        if let Err(error) = self.file.unlock() {
            log_cache_error(&self.root, &error);
        }
    }
}

// eprintln! goes through libtest's output capture, so a report from a test
// thread stays with that test's captured output. Writing to io::stderr()
// directly bypasses the capture and can land in the middle of libtest's
// "test <name> ... ok" status line, which test-output parsers then misread.
fn log_cache_error(path: &Path, error: &dyn std::fmt::Display) {
    #[cfg(test)]
    LOGGED.with(|logged| logged.set(logged.get() + 1));
    eprintln!("ckdev cache maintenance at {}: {error}", path.display());
}

#[cfg(test)]
thread_local! {
    /// Reports made on this thread, so a test can prove that ordinary
    /// contention prints nothing.
    static LOGGED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn touch_marker(path: &Path, now: SystemTime) -> io::Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?
        .set_modified(now)
}

fn use_marker(root: &Path, digest: &str) -> PathBuf {
    root.join(format!("{digest}.last-use"))
}

fn record_use(root: &Path, digest: &str, now: SystemTime) {
    // Sidecars stay in the writable root; neither the sealed directory nor the
    // executable needs to change, and filesystems need not maintain atime.
    let marker = use_marker(root, digest);
    if let Err(error) = touch_marker(&marker, now) {
        log_cache_error(&marker, &error);
    }
}

struct CachePruner {
    completed: Mutex<bool>,
}

impl CachePruner {
    const fn new() -> Self {
        Self {
            completed: Mutex::new(false),
        }
    }

    fn maybe_prune(
        &self,
        root: &Path,
        protected: &Path,
        now: SystemTime,
        remove: impl FnMut(&Path) -> io::Result<()>,
    ) {
        let mut completed = match self.completed.try_lock() {
            Ok(completed) => completed,
            Err(std::sync::TryLockError::WouldBlock) => return,
            Err(error) => {
                log_cache_error(root, &error);
                return;
            }
        };
        if *completed {
            return;
        }
        let result = (|| -> io::Result<bool> {
            // A busy cache is skipped without consuming the process's prune,
            // so the next new copy can try again without delaying this caller.
            let _lock = CacheLock::try_exclusive(root)?;
            let marker = root.join(LAST_PRUNE);
            match fs::metadata(&marker) {
                Ok(metadata) => {
                    if now.duration_since(metadata.modified()?).unwrap_or_default()
                        <= PRUNE_INTERVAL
                    {
                        return Ok(false);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            // Record the attempt while locked, even if individual removals fail.
            touch_marker(&marker, now)?;
            prune_entries(root, protected, now, CACHE_SIZE_LIMIT, remove);
            Ok(true)
        })();
        match result {
            Ok(pruned) => *completed = pruned,
            // Another process holds the cache for its own prune or publish:
            // skip quietly and let a later copy try again.
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => log_cache_error(root, &error),
        }
    }
}

struct CacheEntry {
    path: PathBuf,
    marker: PathBuf,
    last_use: SystemTime,
    bytes: u64,
}

fn cache_entry(root: &Path, entry: fs::DirEntry) -> io::Result<Option<CacheEntry>> {
    let name = entry.file_name();
    let Some(digest) = name.to_str().filter(|name| {
        name.len() == 32
            && name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) else {
        // Never collect staging directories, locks, markers or unrelated files.
        return Ok(None);
    };
    let path = entry.path();
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.is_symlink() {
        return Ok(None);
    }
    let marker = use_marker(root, digest);
    let last_use = match fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => metadata.modified()?,
        Ok(_) => return Err(io::Error::other("last-use marker must be a regular file")),
        // Existing caches predate use markers. Their directory mtime records
        // publication and is the only available age until the next cache hit.
        Err(error) if error.kind() == io::ErrorKind::NotFound => metadata.modified()?,
        Err(error) => return Err(error),
    };
    let mut bytes = 0u64;
    for file in fs::read_dir(&path)? {
        let metadata = fs::symlink_metadata(file?.path())?;
        if metadata.is_file() {
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    Ok(Some(CacheEntry {
        path,
        marker,
        last_use,
        bytes,
    }))
}

fn prune_entries(
    root: &Path,
    protected: &Path,
    now: SystemTime,
    size_limit: u64,
    mut remove: impl FnMut(&Path) -> io::Result<()>,
) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            log_cache_error(root, &error);
            return;
        }
    };
    let mut candidates = Vec::new();
    for entry in entries {
        match entry.and_then(|entry| cache_entry(root, entry)) {
            Ok(Some(entry)) => candidates.push(entry),
            Ok(None) => {}
            Err(error) => log_cache_error(root, &error),
        }
    }
    candidates.sort_by(|a, b| {
        a.last_use
            .cmp(&b.last_use)
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut total = candidates
        .iter()
        .fold(0u64, |total, entry| total.saturating_add(entry.bytes));
    for entry in candidates {
        let age = now.duration_since(entry.last_use).unwrap_or_default();
        if protected.parent() == Some(entry.path.as_path()) || age <= RECENT_USE_GRACE {
            continue;
        }
        if age <= PUBLISHED_RETENTION && total <= size_limit {
            continue;
        }
        match remove(&entry.path) {
            Ok(()) => {
                total = total.saturating_sub(entry.bytes);
                if let Err(error) = fs::remove_file(&entry.marker) {
                    if error.kind() != io::ErrorKind::NotFound {
                        log_cache_error(&entry.marker, &error);
                    }
                }
            }
            // In particular, Windows refuses deletion of an executing image.
            // Leave it in the cache and continue with the next candidate.
            Err(error) => log_cache_error(&entry.path, &error),
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

/// The tools that strip and re-sign a staged Mach-O copy, and how long each
/// may run. Absolute paths: a test's `PATH` never chooses which program
/// rewrites an executable.
#[cfg(target_os = "macos")]
#[derive(Clone)]
struct StripTools {
    strip: PathBuf,
    codesign: PathBuf,
    timeout: Duration,
}

#[cfg(target_os = "macos")]
impl StripTools {
    fn system() -> Self {
        Self {
            strip: PathBuf::from("/usr/bin/strip"),
            codesign: PathBuf::from("/usr/bin/codesign"),
            // Large debug builds take seconds; a hung tool must not hang the
            // test that asked for the binary.
            timeout: Duration::from_secs(120),
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
thread_local! {
    /// Replaces [`StripTools::system`] for publications made on this test
    /// thread, so tests can force a tool to fail.
    static STRIP_TOOLS_OVERRIDE: std::cell::RefCell<Option<StripTools>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Removes a staged Mach-O copy's debug map, then re-signs it.
///
/// `strip -S` removes debug symbols and stabs, including the N_OSO entries
/// that point at object files in the build folder, and keeps the symbol
/// table. Stripping invalidates the linker's ad hoc signature, and Apple
/// Silicon refuses to run an unsigned arm64 binary, so `codesign -f -s -`
/// signs the result again. Both run as child processes for the same reason
/// as [`copy_executable`]: this process never opens the executable for
/// writing.
#[cfg(target_os = "macos")]
fn strip_debug_map(staged: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    #[cfg(test)]
    let tools = STRIP_TOOLS_OVERRIDE
        .with_borrow(Option::clone)
        .unwrap_or_else(StripTools::system);
    #[cfg(not(test))]
    let tools = StripTools::system();
    // `cp` keeps the source's mode; both tools rewrite the file.
    fs::set_permissions(staged, fs::Permissions::from_mode(0o700))?;
    run_tool(&tools.strip, &["-S"], staged, tools.timeout)?;
    run_tool(&tools.codesign, &["-f", "-s", "-"], staged, tools.timeout)
}

#[cfg(not(target_os = "macos"))]
fn strip_debug_map(staged: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("stripping {} is only done on macOS", staged.display()),
    ))
}

/// Runs `tool args target` with stdin from null, waits at most `timeout`,
/// and turns any failure into an error naming the tool, the target and the
/// tool's stderr.
#[cfg(target_os = "macos")]
fn run_tool(tool: &Path, args: &[&str], target: &Path, timeout: Duration) -> io::Result<()> {
    use std::io::Read;
    use std::process::Stdio;
    let failed = |detail: String| {
        io::Error::other(format!(
            "{} {} {} failed: {detail}",
            tool.display(),
            args.join(" "),
            target.display()
        ))
    };
    let mut child = Command::new(tool)
        .args(args)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| failed(format!("could not start it: {error}; stderr: (none)")))?;
    // A reader thread drains stderr so a chatty tool cannot block on a full
    // pipe while this thread waits for it to exit.
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        let _ = sender.send(bytes);
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failed(format!("could not wait for it: {error}")));
            }
        }
    };
    // A descendant the tool left running can keep stderr open; do not wait
    // for it indefinitely.
    let stderr = receiver
        .recv_timeout(Duration::from_secs(1))
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .unwrap_or_else(|_| "(unavailable)".to_owned());
    match status {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(failed(format!(
            "{}; stderr: {stderr}",
            describe_exit_status(status)
        ))),
        None => Err(failed(format!(
            "timed out after {:?}; stderr: {stderr}",
            timeout
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_root_refuses_group_or_other_access() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = crate::ScratchDir::new("private-root-mode");
        let root = scratch.join("root");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();

        let error = prepare_private_root(&root).unwrap_err();
        assert!(
            error.to_string().contains("group or other permission bits"),
            "{error}"
        );
    }

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
    fn staging_uses_private_temp_base_and_other_names_pass_through() {
        let scratch = crate::ScratchDir::new("publish-tmp");
        let built = built_fixture(&scratch);
        let placed = stage_test_binary(&built);
        assert!(
            placed.starts_with(crate::private_temp_base(
                std::env::var_os("TMPDIR").as_deref()
            )),
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
    fn selected_private_root_is_created_0700_and_rejects_group_readability() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmpdir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("private-root-{}-{nonce}", std::process::id()));
        fs::create_dir(&tmpdir).unwrap();
        let base = crate::private_temp_base(Some(tmpdir.as_os_str()));
        assert_eq!(base, tmpdir);
        let root = base.join("cortexkit-tests-private-root");

        prepare_private_root(&root).unwrap();
        let metadata = fs::metadata(&root).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o700);
        assert_eq!(metadata.uid(), rustix::process::getuid().as_raw());

        fs::remove_dir(&root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o744)).unwrap();
        let error = prepare_private_root(&root).unwrap_err();
        assert!(
            error.to_string().contains("group or other permission bits"),
            "{error}"
        );

        fs::remove_dir_all(tmpdir).unwrap();
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
        // One executable per publication. On macOS this fixture is a Mach-O
        // test harness, which is stripped and published with a record of
        // the stripped file's digest beside it.
        let mut entries: Vec<_> = fs::read_dir(paths[0].parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        entries.sort();
        let mut expected = vec![paths[0].file_name().unwrap().to_owned()];
        if cfg!(target_os = "macos") {
            expected.push(PUBLICATION_RECORD.into());
        }
        expected.sort();
        assert_eq!(entries, expected);
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
    fn three_day_prune_keeps_recent_returned_and_symlink_entries() {
        let root = PublishRoot::new("publish-prune");
        let now = SystemTime::now();
        let old_time = now - PUBLISHED_RETENTION - Duration::from_secs(10);
        let old = cache_fixture(&root.path(), 1, old_time, Some(old_time), 4);
        let recent = cache_fixture(&root.path(), 2, now, Some(now), 4);
        let returned = cache_fixture(&root.path(), 3, old_time, Some(old_time), 4);
        let staging = root.path().join(".staging-live");
        fs::create_dir(&staging).unwrap();
        std::os::unix::fs::symlink(
            old.parent().unwrap(),
            root.path().join(format!("{:032x}", 4)),
        )
        .unwrap();
        prune_entries(
            &root.path(),
            &returned,
            now,
            CACHE_SIZE_LIMIT,
            remove_published,
        );
        assert!(!old.exists());
        assert!(recent.exists());
        assert!(returned.exists());
        assert!(staging.exists());
        assert!(
            fs::symlink_metadata(root.path().join(format!("{:032x}", 4)))
                .unwrap()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    fn cache_fixture(
        root: &Path,
        id: u32,
        published: SystemTime,
        last_use: Option<SystemTime>,
        bytes: usize,
    ) -> PathBuf {
        let digest = format!("{id:032x}");
        let dir = root.join(&digest);
        fs::create_dir(&dir).unwrap();
        let binary = dir.join("ckdev-fixture");
        fs::write(&binary, vec![0; bytes]).unwrap();
        fs::set_permissions(&binary, published_permissions(&binary).unwrap()).unwrap();
        fs::File::open(&dir)
            .unwrap()
            .set_modified(published)
            .unwrap();
        seal_dir(&dir).unwrap();
        if let Some(last_use) = last_use {
            touch_marker(&use_marker(root, &digest), last_use).unwrap();
        }
        binary
    }

    #[cfg(unix)]
    #[test]
    fn prune_uses_last_use_marker_not_publication_time() {
        let root = PublishRoot::new("prune-last-use");
        let now = SystemTime::now();
        let old = now - PUBLISHED_RETENTION - Duration::from_secs(1);
        let used = cache_fixture(&root.path(), 1, old, Some(now), 4);
        let unused = cache_fixture(&root.path(), 2, old, Some(old), 4);
        prune_entries(
            &root.path(),
            Path::new(""),
            now,
            CACHE_SIZE_LIMIT,
            remove_published,
        );
        assert!(
            used.exists(),
            "a recent use must keep an old publication alive"
        );
        assert!(!unused.exists(), "unused old entries must be collected");
    }

    #[cfg(unix)]
    #[test]
    fn size_cap_removes_least_recently_used_first() {
        let root = PublishRoot::new("prune-size-order");
        let now = SystemTime::now();
        let oldest = cache_fixture(
            &root.path(),
            1,
            now,
            Some(now - Duration::from_secs(3 * 3600)),
            4,
        );
        let middle = cache_fixture(
            &root.path(),
            2,
            now,
            Some(now - Duration::from_secs(2 * 3600)),
            4,
        );
        let newest = cache_fixture(
            &root.path(),
            3,
            now,
            Some(now - Duration::from_secs(3600)),
            4,
        );
        prune_entries(&root.path(), Path::new(""), now, 8, remove_published);
        assert!(!oldest.exists(), "the oldest use should be evicted first");
        assert!(middle.exists());
        assert!(newest.exists());
    }

    #[cfg(unix)]
    #[test]
    fn size_cap_keeps_recent_and_returned_entries_even_over_budget() {
        let root = PublishRoot::new("prune-size-grace");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(3600);
        let returned = cache_fixture(&root.path(), 1, old, Some(old), 4);
        let recent = cache_fixture(&root.path(), 2, old, Some(now - RECENT_USE_GRACE), 4);
        let unused = cache_fixture(&root.path(), 3, old, Some(old), 4);
        prune_entries(&root.path(), &returned, now, 1, remove_published);
        assert!(returned.exists());
        assert!(recent.exists(), "the grace interval includes its boundary");
        assert!(!unused.exists());
    }

    #[cfg(unix)]
    #[test]
    fn pruning_is_hourly_and_at_most_once_per_process() {
        let root = PublishRoot::new("prune-interval");
        let now = SystemTime::now();
        let old_time = now - PUBLISHED_RETENTION - Duration::from_secs(1);
        let first = cache_fixture(&root.path(), 1, old_time, Some(old_time), 4);
        let pruner = CachePruner::new();
        pruner.maybe_prune(&root.path(), Path::new(""), now, remove_published);
        assert!(!first.exists());
        let next = cache_fixture(&root.path(), 2, old_time, Some(old_time), 4);
        let waiting = CachePruner::new();
        waiting.maybe_prune(
            &root.path(),
            Path::new(""),
            now + PRUNE_INTERVAL,
            remove_published,
        );
        assert!(next.exists(), "another process cannot prune within an hour");
        let later = now + PRUNE_INTERVAL + Duration::from_secs(1);
        pruner.maybe_prune(&root.path(), Path::new(""), later, remove_published);
        assert!(next.exists(), "one process cannot prune twice");
        waiting.maybe_prune(&root.path(), Path::new(""), later, remove_published);
        assert!(
            !next.exists(),
            "an hour-gated process may retry after the hour"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_only_legacy_entry_is_actually_deleted() {
        use std::os::unix::fs::PermissionsExt;
        let root = PublishRoot::new("prune-sealed-legacy");
        let now = SystemTime::now();
        let old = cache_fixture(
            &root.path(),
            1,
            now - PUBLISHED_RETENTION - Duration::from_secs(1),
            None,
            4,
        );
        let dir = old.parent().unwrap();
        assert_eq!(
            fs::metadata(dir).unwrap().permissions().mode() & 0o777,
            0o555
        );
        assert!(fs::metadata(&old).unwrap().permissions().readonly());
        prune_entries(
            &root.path(),
            Path::new(""),
            now,
            CACHE_SIZE_LIMIT,
            remove_published,
        );
        assert!(!dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn failed_deletion_does_not_fail_publication_or_stop_pruning() {
        let root = PublishRoot::new("prune-failure");
        let build = crate::ScratchDir::new("prune-failure-build");
        let built = build.join("ck-fixture");
        write_script(&built, "#!/bin/sh\necho retained\n");
        let now = SystemTime::now();
        let old_time = now - PUBLISHED_RETENTION - Duration::from_secs(1);
        let failed = cache_fixture(&root.path(), 1, old_time, Some(old_time), 4);
        let removed = cache_fixture(&root.path(), 2, old_time, Some(old_time), 4);
        let mut attempts = 0;
        let placed = publish_with_pruner(&built, "ckdev-fixture", &root.path(), |placed| {
            CachePruner::new().maybe_prune(&root.path(), placed, now, |dir| {
                attempts += 1;
                if Some(dir) == failed.parent() {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected deletion failure",
                    ))
                } else {
                    remove_published(dir)
                }
            });
        })
        .unwrap();
        assert_eq!(attempts, 2);
        assert!(failed.exists());
        assert!(!removed.exists());
        assert!(placed.exists());
        checked_output(&mut dev_command(&placed)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn publication_records_use_on_copy_and_hit_but_only_copy_prunes() {
        let root = PublishRoot::new("prune-publish-use");
        let build = crate::ScratchDir::new("prune-publish-use-build");
        let built = build.join("ck-fixture");
        write_script(&built, "#!/bin/sh\necho used\n");
        let mut prunes = 0;
        let placed =
            publish_with_pruner(&built, "ckdev-fixture", &root.path(), |_| prunes += 1).unwrap();
        assert_eq!(prunes, 1);
        let digest = placed
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let marker = use_marker(&root.path(), digest);
        assert!(marker.is_file());
        let old_time = SystemTime::now() - PUBLISHED_RETENTION - Duration::from_secs(1);
        touch_marker(&marker, old_time).unwrap();
        let before = SystemTime::now();
        let reused = publish_with_pruner(&built, "ckdev-fixture", &root.path(), |_| {
            panic!("cache hits must not prune")
        })
        .unwrap();
        assert_eq!(reused, placed);
        assert!(fs::metadata(marker).unwrap().modified().unwrap() >= before);
        assert!(fs::metadata(placed).unwrap().permissions().readonly());
    }

    #[cfg(unix)]
    #[test]
    fn prune_lock_skips_busy_publishers_and_other_pruners() {
        let root = PublishRoot::new("prune-lock");
        let now = SystemTime::now();
        let old_time = now - PUBLISHED_RETENTION - Duration::from_secs(1);
        let old = cache_fixture(&root.path(), 1, old_time, Some(old_time), 4);
        let lock = CacheLock::shared(&root.path()).unwrap();
        let pruner = CachePruner::new();
        pruner.maybe_prune(&root.path(), Path::new(""), now, remove_published);
        assert!(old.exists());
        assert!(!root.path().join(LAST_PRUNE).exists());
        lock.file.unlock().unwrap();
        lock.file.lock().unwrap();
        pruner.maybe_prune(&root.path(), Path::new(""), now, remove_published);
        assert!(old.exists());
        assert!(!root.path().join(LAST_PRUNE).exists());
        drop(lock);
        pruner.maybe_prune(&root.path(), Path::new(""), now, remove_published);
        assert!(!old.exists());
    }

    #[cfg(unix)]
    #[test]
    fn dropping_cache_lock_unlocks_inherited_file_description() {
        let root = PublishRoot::new("prune-inherited-lock");
        let shared = CacheLock::shared(&root.path()).unwrap();
        // A duplicated descriptor shares the same open file description and
        // flock as a descriptor inherited by a child between fork and exec.
        let inherited = shared.file.try_clone().unwrap();
        drop(shared);
        let exclusive = CacheLock::try_exclusive(&root.path());
        drop(inherited);
        assert!(
            exclusive.is_ok(),
            "inherited descriptor retained the lock: {exclusive_error:?}",
            exclusive_error = exclusive.as_ref().err()
        );
        let exclusive = exclusive.unwrap();
        let inherited = exclusive.file.try_clone().unwrap();
        drop(exclusive);
        let released = CacheLock::try_exclusive(&root.path());
        drop(inherited);
        assert!(
            released.is_ok(),
            "inherited prune descriptor retained its lock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_prune_that_finds_the_cache_busy_is_skipped_without_a_report() {
        let root = PublishRoot::new("prune-busy-quiet");
        let held = CacheLock::try_exclusive(&root.path()).unwrap();
        let contended = CacheLock::try_exclusive(&root.path());
        assert_eq!(
            contended.as_ref().err().map(io::Error::kind),
            Some(io::ErrorKind::WouldBlock)
        );
        let before = LOGGED.with(std::cell::Cell::get);
        let pruner = CachePruner::new();
        let protected = root.path().join("protected");
        let mut removed = 0;
        pruner.maybe_prune(&root.path(), &protected, SystemTime::now(), |_| {
            removed += 1;
            Ok(())
        });
        assert_eq!(
            LOGGED.with(std::cell::Cell::get),
            before,
            "busy cache was reported"
        );
        assert_eq!(removed, 0);
        assert!(
            !*pruner.completed.lock().unwrap(),
            "a skipped prune must not count as done"
        );
        drop(held);
    }

    #[cfg(unix)]
    #[test]
    fn failed_use_marker_update_does_not_fail_a_cache_hit() {
        let root = PublishRoot::new("prune-marker-failure");
        let build = crate::ScratchDir::new("prune-marker-failure-build");
        let built = build.join("ck-fixture");
        write_script(&built, "#!/bin/sh\necho used\n");
        let placed = publish_with_pruner(&built, "ckdev-fixture", &root.path(), |_| {}).unwrap();
        let digest = placed
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let marker = use_marker(&root.path(), digest);
        fs::remove_file(&marker).unwrap();
        fs::create_dir(&marker).unwrap();
        let reused = publish_with_pruner(&built, "ckdev-fixture", &root.path(), |_| {
            panic!("cache hits must not prune")
        })
        .unwrap();
        assert_eq!(reused, placed);
        assert_eq!(fs::read(&built).unwrap(), fs::read(&placed).unwrap());
        assert!(fs::metadata(placed).unwrap().permissions().readonly());
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
                    let _ = remove_published(&entry.path());
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

    #[cfg(unix)]
    fn reset_hash_counts() {
        HASH_COUNTS.with(|counts| counts.set((0, 0, 0, 0)));
    }

    #[cfg(unix)]
    fn hash_counts() -> (usize, usize, usize, usize) {
        HASH_COUNTS.with(std::cell::Cell::get)
    }

    #[cfg(unix)]
    fn set_modified(path: &Path, modified: SystemTime) {
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn repeated_calls_hash_the_same_build_and_publication_once() {
        let build = crate::ScratchDir::new("ckdev-memo-count-build");
        let root = PublishRoot::new("ckdev-memo-count-root");
        let built = build.join("ck-subc");
        write_script(&built, "#!/bin/sh\necho memo-count\n");
        set_modified(
            &built,
            UNIX_EPOCH + Duration::from_secs(1_800_000_000) + Duration::from_nanos(101),
        );
        reset_hash_counts();

        let first = ckdev_binary_at(&built, &root.path());
        for _ in 0..7 {
            assert_eq!(ckdev_binary_at(&built, &root.path()), first);
        }

        assert_eq!(hash_counts(), (3, 1, 1, 1));
    }

    #[cfg(unix)]
    #[test]
    fn rebuilding_a_file_with_a_new_mtime_rehashes_it() {
        let build = crate::ScratchDir::new("ckdev-memo-rebuild");
        let built = build.join("ck-subc");
        write_script(&built, "#!/bin/sh\necho rebuilt\n");
        let initial_time =
            UNIX_EPOCH + Duration::from_secs(1_800_000_010) + Duration::from_nanos(203);
        set_modified(&built, initial_time);
        reset_hash_counts();

        let first = memoized_source_address(&built, "ckdev-subc").unwrap();
        let first_identity = file_identity(&built).unwrap();
        set_modified(&built, initial_time + Duration::from_secs(1));
        let second_identity = file_identity(&built).unwrap();
        assert_ne!(first_identity, second_identity);

        let second = memoized_source_address(&built, "ckdev-subc").unwrap();
        assert_eq!(first, second);
        assert_eq!(hash_counts(), (2, 2, 0, 0));
    }

    #[cfg(unix)]
    #[test]
    fn nanosecond_mtime_changes_file_identity() {
        let build = crate::ScratchDir::new("ckdev-memo-nanos");
        let built = build.join("ck-subc");
        write_script(&built, "#!/bin/sh\necho nanoseconds\n");
        let modified =
            UNIX_EPOCH + Duration::from_secs(1_800_000_020) + Duration::from_nanos(123_456_789);
        set_modified(&built, modified);
        let first = file_identity(&built).unwrap();

        set_modified(&built, modified + Duration::from_nanos(1));
        let second = file_identity(&built).unwrap();

        assert_ne!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn changed_publication_identity_is_reverified_and_corruption_refused() {
        use std::os::unix::fs::PermissionsExt;

        let build = crate::ScratchDir::new("ckdev-memo-published-build");
        let root = PublishRoot::new("ckdev-memo-published-root");
        let built = build.join("ck-bus");
        write_script(&built, "#!/bin/sh\necho genuine\n");
        set_modified(
            &built,
            UNIX_EPOCH + Duration::from_secs(1_800_000_030) + Duration::from_nanos(307),
        );
        let placed = ckdev_binary_at(&built, &root.path());
        let old_identity = file_identity(&placed).unwrap();

        let directory = placed.parent().unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&placed, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(&placed, "#!/bin/sh\necho planted\n").unwrap();
        set_modified(
            &placed,
            UNIX_EPOCH + Duration::from_secs(1_900_000_000) + Duration::from_nanos(409),
        );
        assert_eq!(
            fs::metadata(&placed).unwrap().len(),
            fs::metadata(&built).unwrap().len()
        );
        assert_ne!(old_identity, file_identity(&placed).unwrap());

        reset_hash_counts();
        let error = publish(&built, "ckdev-bus", &root.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match its content address"),
            "{error}"
        );
        assert_eq!(hash_counts(), (1, 0, 1, 0));
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
        assert!(entries.iter().any(|name| name == dir.file_name().unwrap()));
        assert!(entries
            .iter()
            .all(|name| !name.to_string_lossy().starts_with(".staging-")));
        assert!(use_marker(&root.path(), dir.file_name().unwrap().to_str().unwrap()).is_file());
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

    /// A bin target already named `ckdev-*` is still copied out of its build
    /// folder, under the same name.
    #[cfg(unix)]
    #[test]
    fn an_already_ckdev_binary_is_copied_out_of_its_build_folder() {
        let build = crate::ScratchDir::new("ckdev-named-build");
        let root = PublishRoot::new("ckdev-named-root");
        let built = build.join("ckdev-fixture");
        write_script(&built, "#!/bin/sh\necho fixture\n");
        let placed = ckdev_binary_at(&built, &root.path());
        assert_ne!(placed, built);
        assert_eq!(placed.file_name().unwrap(), "ckdev-fixture");
        assert_eq!(
            placed.parent().and_then(Path::parent),
            Some(root.path().as_path())
        );
        let output = dev_command(&placed).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "fixture\n");
    }

    /// Measures the common repeated-launch path with the test harness as the
    /// built binary. Run explicitly with `--ignored --nocapture` when timing it.
    #[cfg(unix)]
    #[test]
    #[ignore = "manual repeated-launch timing test"]
    fn repeated_ckdev_launches_of_one_binary() {
        use std::time::Instant;

        const LAUNCHES: usize = 20;
        let build = crate::ScratchDir::new("ckdev-repeat-build");
        let root = PublishRoot::new("ckdev-repeat-root");
        let built = built_fixture(&build);
        let started = Instant::now();
        for _ in 0..LAUNCHES {
            let placed = ckdev_binary_at(&built, &root.path());
            checked_output(&mut fixture_command(&placed)).unwrap();
        }
        eprintln!(
            "{LAUNCHES} repeated ckdev launches: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }

    /// Runs a system tool on `path` and returns its stdout, failing the test
    /// when the tool fails.
    #[cfg(target_os = "macos")]
    fn tool_output(tool: &str, args: &[&str], path: &Path) -> String {
        let output = Command::new(tool)
            .args(args)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{tool} {args:?} {} failed: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// The number of N_OSO stab entries, each naming an object file the
    /// debug map points at.
    #[cfg(target_os = "macos")]
    fn debug_map_entries(path: &Path) -> usize {
        tool_output("/usr/bin/nm", &["-ap"], path)
            .lines()
            .filter(|line| line.split_whitespace().any(|word| word == "OSO"))
            .count()
    }

    #[cfg(target_os = "macos")]
    fn dev_name_of(built: &Path) -> String {
        ckdev_file_name(built.file_name().unwrap().to_str().unwrap())
    }

    /// A debug build published on macOS loses its debug map, so nothing that
    /// symbolicates the running copy opens object files in the build folder.
    /// It keeps its function symbols and a valid signature, and still runs.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_published_mach_o_has_no_debug_map_but_keeps_symbols_signature_and_behaviour() {
        let build = crate::ScratchDir::new("ckdev-strip-build");
        let root = PublishRoot::new("ckdev-strip-root");
        // This crate's own debug test harness, which links a debug map.
        let built = built_fixture(&build);
        let function = "staged_child_fixture";
        assert!(
            debug_map_entries(&built) > 0,
            "the source must carry a debug map, or the check below proves nothing"
        );
        assert!(tool_output("/usr/bin/nm", &[], &built).contains(function));

        let placed = ckdev_binary_at(&built, &root.path());

        assert_eq!(debug_map_entries(&placed), 0, "{}", placed.display());
        assert!(
            tool_output("/usr/bin/nm", &[], &placed).contains(function),
            "the symbol table must survive stripping"
        );
        tool_output("/usr/bin/codesign", &["--verify", "--strict"], &placed);
        let output = checked_output(&mut fixture_command(&placed)).unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        let record = fs::read_to_string(placed.parent().unwrap().join(PUBLICATION_RECORD)).unwrap();
        assert_eq!(
            record.trim_end(),
            content_digest(&placed, &dev_name_of(&built)).unwrap()
        );
        // A second request reuses the stripped publication.
        assert_eq!(ckdev_binary_at(&built, &root.path()), placed);
    }

    /// A copy published by 0.1.8 or earlier sits at the untagged digest of
    /// the unstripped bytes. It is never reused; the build is published again
    /// at its stripped address.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_unstripped_copy_at_the_old_address_is_not_reused() {
        let build = crate::ScratchDir::new("ckdev-old-format-build");
        let root = PublishRoot::new("ckdev-old-format-root");
        let built = built_fixture(&build);
        let dev_name = dev_name_of(&built);
        let old_dir = root.path().join(content_digest(&built, &dev_name).unwrap());
        fs::create_dir(&old_dir).unwrap();
        copy_executable(&built, &old_dir.join(&dev_name)).unwrap();
        seal_dir(&old_dir).unwrap();

        let placed = ckdev_binary_at(&built, &root.path());

        assert_ne!(placed.parent().unwrap(), old_dir);
        assert_eq!(debug_map_entries(&placed), 0);
    }

    /// Replaces the strip and codesign tools for publications on this test
    /// thread until dropped.
    #[cfg(target_os = "macos")]
    struct ToolOverride;

    #[cfg(target_os = "macos")]
    impl ToolOverride {
        fn set(tools: StripTools) -> Self {
            STRIP_TOOLS_OVERRIDE.with_borrow_mut(|slot| *slot = Some(tools));
            Self
        }
    }

    #[cfg(target_os = "macos")]
    impl Drop for ToolOverride {
        fn drop(&mut self) {
            STRIP_TOOLS_OVERRIDE.with_borrow_mut(|slot| *slot = None);
        }
    }

    /// After a failed publication the root holds only its lock file: no
    /// publication directory, no staging directory and no use marker.
    #[cfg(target_os = "macos")]
    fn assert_nothing_published(root: &Path, built: &Path) {
        let dev_name = dev_name_of(built);
        let (address, publication) = source_address(built, &dev_name).unwrap();
        assert_eq!(publication, Publication::StrippedMachO);
        assert!(!root.join(&address).join(&dev_name).exists());
        let entries: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            entries.iter().all(|name| name == ".cache-lock"),
            "left behind: {entries:?}"
        );
    }

    /// A missing or failing strip or codesign fails publication with an
    /// error naming the tool, the file and the tool's stderr, and nothing is
    /// published: an unstripped copy would bring the privacy prompts back.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_failed_strip_or_codesign_publishes_nothing() {
        let build = crate::ScratchDir::new("ckdev-strip-failure-build");
        let built = built_fixture(&build);
        let dev_name = dev_name_of(&built);
        let failing = build.join("failing-tool");
        write_script(&failing, "#!/bin/sh\necho \"refused $1\" >&2\nexit 3\n");
        let failing_text = failing.display().to_string();
        let system = StripTools::system();
        let cases = [
            (
                StripTools {
                    strip: PathBuf::from("/nonexistent/strip"),
                    ..system.clone()
                },
                "/nonexistent/strip -S",
                "could not start it",
            ),
            (
                StripTools {
                    strip: failing.clone(),
                    ..system.clone()
                },
                failing_text.as_str(),
                "exit code Some(3); stderr: refused -S",
            ),
            (
                StripTools {
                    codesign: failing.clone(),
                    ..system.clone()
                },
                failing_text.as_str(),
                "exit code Some(3); stderr: refused -f",
            ),
        ];
        for (tools, tool, detail) in cases {
            let root = PublishRoot::new("ckdev-strip-failure-root");
            let _override = ToolOverride::set(tools);
            let error = publish_with_pruner(&built, &dev_name, &root.path(), |_| {
                panic!("a failed publication must not prune")
            })
            .unwrap_err();
            let text = error.to_string();
            assert!(
                text.contains(tool) && text.contains(detail) && text.contains(".staging-"),
                "{text}"
            );
            assert_nothing_published(&root.path(), &built);
        }
    }

    /// A tool that never exits is killed after its timeout and the
    /// publication fails instead of hanging the test.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_hung_strip_times_out_and_publishes_nothing() {
        let build = crate::ScratchDir::new("ckdev-strip-hang-build");
        let root = PublishRoot::new("ckdev-strip-hang-root");
        let built = built_fixture(&build);
        let hung = build.join("hung-tool");
        write_script(&hung, "#!/bin/sh\nexec /bin/sleep 30\n");
        let _override = ToolOverride::set(StripTools {
            strip: hung,
            timeout: Duration::from_millis(300),
            ..StripTools::system()
        });
        let started = std::time::Instant::now();
        let error =
            publish_with_pruner(&built, &dev_name_of(&built), &root.path(), |_| {}).unwrap_err();
        assert!(error.to_string().contains("timed out after"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_nothing_published(&root.path(), &built);
    }

    /// Reuse still refuses a stripped publication whose executable no longer
    /// matches the digest recorded when it was published, or whose record is
    /// gone.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_altered_stripped_publication_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let build = crate::ScratchDir::new("ckdev-strip-altered-build");
        let built = built_fixture(&build);
        let dev_name = dev_name_of(&built);

        let root = PublishRoot::new("ckdev-strip-altered-root");
        let placed = ckdev_binary_at(&built, &root.path());
        let dir = placed.parent().unwrap();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(&placed).unwrap();
        // The unstripped build swapped in for its stripped publication.
        copy_executable(&built, &placed).unwrap();
        let error = publish(&built, &dev_name, &root.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match its content address"),
            "{error}"
        );

        let root = PublishRoot::new("ckdev-strip-unrecorded-root");
        let placed = ckdev_binary_at(&built, &root.path());
        let dir = placed.parent().unwrap();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(dir.join(PUBLICATION_RECORD)).unwrap();
        // Replace the executable with an identical copy so the verification
        // memo, keyed by file identity, does not answer for it.
        let same = build.join("same");
        copy_executable(&placed, &same).unwrap();
        fs::remove_file(&placed).unwrap();
        copy_executable(&same, &placed).unwrap();
        let error = publish(&built, &dev_name, &root.path()).unwrap_err();
        assert!(error.to_string().contains("publication record"), "{error}");
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
