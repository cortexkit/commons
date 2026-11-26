# Changelog

## 0.1.0

- Add a Unix adapter for isolated check and manifest conformance rules.
- Compare both scratch homes by path, type, symlink target and file content.
- Check stale-digest, running-module and no-terminal refusals on declared
  non-prompting test builds, and compare both homes across every attempted fix.
