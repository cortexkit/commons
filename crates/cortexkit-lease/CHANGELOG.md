# Changelog

## 0.1.6

- On Windows, `verify_private` now requires a protected DACL and returns
  `NotPrivate::Unprotected` when permissions can be inherited from the parent.
  A caller found that an unprotected, user-only DACL was accepted even though a
  later parent permission change could propagate broader access into the entry.
  Entries created or protected by `create_private_dir`, `create_private_file`,
  `protect_file`, or `protect_dir_only` already have protected DACLs and are
  unaffected. Unix behavior is unchanged because mode bits are not inherited.

## 0.1.5

- Add `verify_private` and `NotPrivate` to check ownership and owner-only
  permissions without following a symlink or reparse point. Refusals identify
  the offending SID (with well-known account aliases where available) or Unix
  principal, without potentially slow domain lookups.
- Add `reset_to_inherited` so a backup tool can restore normal Windows DACL
  inheritance, optionally recursively, without traversing reparse points or
  changing ownership. On Unix it checks existence but leaves modes unchanged:
  Unix has no permission inheritance, and the caller applies captured mode bits.
- Add `create_private_file` to create a new read/write file with owner-only
  security from the instant it exists, refusing existing entries.
- Add `protect_dir_only` to tighten an existing directory without changing
  existing children's ACLs or Unix modes. Unlike `create_private_dir`, the
  Windows operation does not propagate ACL changes into existing contents.
- When protecting Windows files and directories, assign the current token user's
  SID as owner only if ownership differs, including with an elevated token whose
  default owner may be Administrators. Entries already owned by the user need
  only READ_CONTROL and WRITE_DAC, as before. New files and directories receive
  the user's SID explicitly in their creation descriptor.
- Keep existing-directory tightening failures in `create_private_dir` diagnostic
  only, as in 0.1.4; callers needing proof use `verify_private`. `protect_file` and
  `protect_dir_only` return protection and ownership-setting failures.

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
