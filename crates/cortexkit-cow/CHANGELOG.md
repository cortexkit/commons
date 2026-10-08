# Changelog

## 0.1.0 — First commons release

- First published release: copy-on-write workspace isolation (APFS, btrfs, ZFS,
  Linux reflink, Windows block clone, ProjFS, recursive copy) and change capture.
- Use published `subc-os` `DisclaimedCommand` for synchronous backend commands
  and asynchronous git diffs. Add `configure_spawn_trampoline` for process-wide
  macOS configuration; unconfigured macOS child launches now fail closed.
- Confirm child exec and kill/reap on confirmation failure, without blocking
  async runtime workers during confirmation.
- Tests use `tempfile`, and cover the trampoline rule and native clones.
- Deny unsafe code by default, with scoped native syscall/ProjFS exceptions
  and safety explanations.
