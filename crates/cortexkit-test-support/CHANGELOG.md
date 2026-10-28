# Changelog

## 0.1.11

- Deleting a published `ckdev-*` entry no longer removes it in place. The entry is first renamed to a unique `.removing-*` name in the same cache root, then the renamed copy is deleted. Readers see either the complete entry or nothing; a deletion that fails partway can no longer leave a sealed entry with no record that wedges every later lookup. A failed deletion leaves the renamed copy for a later prune to retry, and prunes also retry leftover `.removing-*` directories. On Windows, when a running image keeps the rename from succeeding, the entry is left in place as before.
- A lookup that finds a broken entry (a missing or unusable publication record, or an executable that no longer matches its address) moves the entry aside and publishes afresh from the source build instead of refusing, so one bad entry cannot wedge the cache until someone deletes it by hand. The lookup reports each repair on stderr with the reason the entry failed verification, so a planted or damaged executable stays visible.

## 0.1.10

- Cache maintenance no longer reports ordinary contention. When several test
  threads or processes publish at once, the background prune finds the cache
  lock held (WouldBlock) and now skips quietly, as intended, instead of printing
  "try_lock failed: kind=WouldBlock".
- The remaining maintenance reports use `eprintln!`, which libtest captures per
  test, instead of writing to stderr directly. A direct write could land inside
  libtest's "test <name> ... ok" status line, and mutation runners that parse
  those lines then failed the baseline with "unrecognized test status".

## 0.1.9

- On macOS, `ckdev_binary` removes the debug map from a published Mach-O copy
  with `/usr/bin/strip -S`, then ad hoc re-signs it with `/usr/bin/codesign -f -s -`.
  A debug build's debug map (N_OSO stab entries) names every object file by
  absolute path in the build folder. Symbolicating the running copy, for
  example in a panic hook that captures a backtrace, opened those files, so
  macOS asked for access to the removable volume holding the build folder and
  the test stalled on the prompt. The symbol table stays, so backtraces keep
  function names. Re-signing is needed because stripping invalidates the
  linker's signature and Apple Silicon refuses unsigned arm64 binaries.
- If either tool fails, cannot start, or runs longer than two minutes,
  publication fails with an error naming the tool, the file and the tool's
  stderr. Nothing is published: an unstripped fallback would bring the prompts
  back without any signal.
- Stripped publications use new content addresses (the source digest with a
  format tag), so unstripped copies published by 0.1.8 and earlier are not
  reused; they age out through the existing pruning. Because a stripped copy's
  bytes differ from its source, its sealed directory also holds
  `.published-digest`, the stripped file's digest recorded before the atomic
  rename, and reuse checks the executable against that record. Scripts and
  other non-Mach-O files, and every file on Linux and Windows, are still
  published as exact copies at unchanged addresses.

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
