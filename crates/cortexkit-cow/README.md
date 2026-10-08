# cortexkit-cow

Cross-platform copy-on-write workspace isolation and change capture, published
independently from [cortexkit/commons](https://github.com/cortexkit/commons) under
MIT. Originally derived from oh-my-pi's `pi-iso` isolation platform abstraction
layer.

## Backends

`backend(kind)` and `default_backend()` return process-wide, stateless backends.
`start(lower, merged)` creates a writable clone and `stop(merged)` removes it.
Drive these blocking operations from a blocking thread when using an async
runtime. `diff(lower, merged)` captures changes asynchronously, using git for
git trees and a metadata-short-circuited tree walk otherwise.

| Backend | Requirements |
| --- | --- |
| `Apfs` | macOS, a volume supporting `clonefile` |
| `Btrfs` | Linux, btrfs CLI and a source subvolume |
| `Zfs` | Unix, ZFS CLI and a dataset mountpoint |
| `LinuxReflink` | Linux, a filesystem supporting `FICLONE` (e.g. XFS with reflink) |
| `WindowsBlockClone` | Windows, a volume supporting block cloning |
| `Projfs` | Windows Projected File System |
| `Overlayfs` | Retired; always unavailable, never mounts or deletes paths |
| `Rcopy` | git worktree for git sources; explicit recursive copy otherwise |

Native clone backends **fail closed**: unsupported cloning returns
`IsoError::Unavailable`, never a byte-for-byte copy. `resolve(preferred)` returns
host-available candidates in preference order, but the caller decides whether
to retry another backend after a path-specific unavailable error. Only the
explicit `Rcopy` backend performs recursive copies.

## Child process privacy on macOS

Before using git or ZFS operations, configure a trampoline once per process.
The executable must be the caller's own binary and must handle subc-os's hidden
mode **before starting threads or an async runtime**:

```rust,no_run
fn main() -> std::io::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    subc_os::privacy_identity::trampoline_main(&args);
    cortexkit_cow::configure_spawn_trampoline(std::env::current_exe()?)?;
    // Now start the runtime and use cortexkit_cow::backend(...).
    Ok(())
}
```

An embedding binary needs its own `subc-os = "0.1.9"` dependency to call
`trampoline_main`. Configuration probes the trampoline with a five-second
deadline. Invalid configuration is not stored; reconfiguration after success
returns `AlreadyExists`. Every child uses `DisclaimedCommand` and confirms exec
with a five-second deadline, killing and reaping a child on confirmation
failure. Async diff performs blocking confirmation off the runtime worker.

On macOS, a spawn-using operation without configuration returns an error; it
never silently inherits the parent's privacy identity. Native clones and
non-git `Rcopy` do not spawn and need no configuration. On Linux and Windows,
configuration is a no-op and children run directly through `DisclaimedCommand`.

## Validation

```sh
cargo fmt --all --check
cargo clippy -p cortexkit-cow --all-targets --locked -- -D warnings
cargo test --locked -p cortexkit-cow
```

Tests use `tempfile`, respecting `TMPDIR`. To require native cloning rather than
allow a reported unavailable filesystem, set
`CORTEXKIT_COW_REQUIRE_NATIVE_CLONE=1` and use an APFS/XFS+reflink temp directory.
The Linux cross-device test uses `/dev/shm` as its unsupported destination.
The `test-support` feature only enables the dedicated privacy trampoline fixture
binary; consumers do not need it. macOS configuration tests run in separate
integration executables so the process-wide configuration cannot leak into the
unconfigured test.

## Public API

The public API: `BackendKind` (all eight variants;
`as_str`, `from_str`, `native`), `ProbeResult` (`available`, `reason` fields;
`available`, `unavailable` constructors), `IsoError` (`Unavailable`, `Other`;
`unavailable`, `other`, `is_unavailable`, `message`), `IsoResult<T>`,
`IsolationBackend` (`kind`, `probe`, `start`, `stop`, `diff`), `Diff` (`files`,
`is_empty`), `FileChange` (`path`, `op`, `diff`), `ChangeKind` (`Added`,
`Modified`, `Removed`), `decode_git_quoted_path`, `default_backend`, `backend`,
`backend_kind`, `auto_order`, `Resolution` (`kind`, `candidates`, `fell_back`,
`reason`), `resolve`, and
`configure_spawn_trampoline(path: impl Into<PathBuf>) -> std::io::Result<()>`.
