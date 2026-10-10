# Changelog

## 0.1.3

- Add `durable_replace` and `sync_dir` helpers for durable local filesystem updates on Unix and Windows.

## 0.1.2

- Add an opt-in, Windows-only `test-support` module for native ACL observations,
  assertions and disposable broad-ACL fixtures. The feature is off by default.
- Windows files and directories are now owner-only, using protected DACLs that
  grant only the current process user full control. New directories pass these
  permissions on to their children; reparse points are never adjusted.
- Narrowing existing Windows directories also replaces broad inherited ACLs on
  existing descendants, while preserving protected child DACLs.
