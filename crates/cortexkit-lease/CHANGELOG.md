# Changelog

## 0.1.4

- Add `LeaseHandle::raise_epoch(at_least)`: raises a held exclusive lease's
  epoch to at least `at_least`, persists and syncs it through the held lock,
  and returns the resulting epoch. It never lowers an epoch, works through a
  shared `Arc<dyn LeaseHandle>`, and refuses on a shared (reader) handle. For a
  writer whose fence token is behind a token already stamped in the data it
  guards, such as a database restored without its lease files.
- Additive: the new trait method has a default that returns `LeaseError::Io`
  with `io::ErrorKind::Unsupported`, so existing `LeaseHandle` implementors
  still compile and callers can tell that the raise did not happen.

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
