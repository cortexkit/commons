# Changelog

## 0.1.8

- Memoize built-binary digests and successful published-copy verification within
  each process, keyed by device, inode, length, and full-nanosecond modification
  time. Replaced or otherwise changed files are hashed again.

## 0.1.7

- The Unix `ckdev_binary` cache and macOS `ScratchDir` root use `$TMPDIR` when it
  is non-empty, absolute, and canonicalizes outside `/private/var/folders`; all
  other values fall back to `/tmp`. Canonicalization catches the `/var/folders`
  alias, allowing worker task temp roots without running new macOS executables
  from its assessed user-temp tree. Windows behavior is unchanged.

## 0.1.6

- Track `ckdev_binary` cache use with sidecar timestamps instead of relying on
  access times. A new copy opportunistically prunes entries unused for three
  days, then least-recently-used entries over a 10 GiB budget. The returned
  entry and uses within 30 minutes are protected. Pruning is best-effort,
  coordinated across processes, and limited to once per process and per hour;
  locked executables on Windows are skipped when removal fails.
- Explicitly unlock cache lock descriptors before closing them, so children
  inheriting descriptors during process creation cannot extend a finished
  operation's lock. A busy lock does not consume the process's pruning run;
  the next new copy can attempt pruning again.

## 0.1.5

- `ckdev_binary` copies a binary that is already named `ckdev-*` too, instead of returning its build path. A bin target with that name was run straight from its build folder. When that folder is on an external volume, macOS shows a privacy prompt before a test daemon starts it as a module under its own privacy identity, and the test waits on that prompt.

## 0.1.4

- On macOS, `ScratchDir` now lives under `/tmp/cortexkit-tests-<uid>/` (mode
  0700, owned by the current user) instead of the per-user `$TMPDIR`. Tests that
  copy a freshly built executable into a scratch folder and run it could stall
  there for 30 seconds or more while macOS assessed the new binary; under `/tmp`
  it starts at once. A pre-existing root that isn't a real directory owned by
  this user with no group or other access is refused by name. Linux and Windows
  keep their previous root.

## 0.1.3

- Add `process_liveness(pid)`, which returns alive, dead or unknown (Linux
  zombies count as dead), and `process_alive(pid)` for test assertions, which
  panics instead of guessing when liveness can't be determined. Use
  `wait_until_gone` to wait for a process to exit.

## 0.1.2

- The daemon watcher writes the daemon's PID file atomically (a temporary
  file, then a rename). A reader polling for the file could otherwise see it
  empty for a moment and fail to parse it.

## 0.1.1

- Require `subc-os` 0.1.10 or any later 0.1.x instead of exactly 0.1.10, so a
  consumer that pins its own compatible `subc-os` can resolve this crate.

## 0.1.0

- Add portable scratch fixtures with kernel-identity-based cleanup and preserved
  panic/keep evidence.
- Add content-addressed executable copies, guards refusing production `ck`/`ck-*`
  names, checked exit reporting, and a reusable Rust test-source spawn guard.
- Add fenced daemon lifecycle, shared sibling build caches, stable executables,
  configurable macOS signing, and the privileged-test opt-in gate.
- Let callers select extra scratch-root variables and sibling dependency checkouts,
  and report the Unix daemon watcher's `python3` requirement in spawn errors.
