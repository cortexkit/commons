# Changelog

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
