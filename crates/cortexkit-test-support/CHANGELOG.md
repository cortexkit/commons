# Changelog

## 0.1.0

- Add portable scratch fixtures with kernel-identity-based cleanup and preserved
  panic/keep evidence.
- Add content-addressed executable copies, guards refusing production `ck`/`ck-*`
  names, checked exit reporting, and a reusable Rust test-source spawn guard.
- Add fenced daemon lifecycle, shared sibling build caches, stable executables,
  configurable macOS signing, and the privileged-test opt-in gate.
- Let callers select extra scratch-root variables and sibling dependency checkouts,
  and report the Unix daemon watcher's `python3` requirement in spawn errors.
