# Changelog

## 0.1.0

- First release: persona/v1 as stated in SPEC.md. Loads a role from an
  ordered tier ladder with `operator` and `repository` trust classes, decides
  repository layers from a module-supplied approval set keyed by module, role,
  opaque project key, tier-relative path and layer digest, merges layers
  (replace, `disabled_blocks` union, registered extension rules), and renders
  with MiniJinja 2.12.0 in a sealed environment against a typed context.
- `render` returns the LF-normalised bytes, their SHA-256 digest and a
  serialisable resolution report with each tier's state and layer digest, the
  composed fragment list with each fragment's origin and digest, the entry
  tier, and the source revision.
- The default ladder resolves the config directory with
  `cortexkit-store-types::resolve_config_home`: a non-empty
  `XDG_CONFIG_HOME`; on Windows `APPDATA`, then `USERPROFILE\AppData\Roaming`;
  then `HOME/.config`; otherwise a bare `.config`, which is refused as
  relative (as is a relative `XDG_CONFIG_HOME`). A change to that crate's
  lookup changes this crate's default ladder.
- Front matter that repeats a mapping key at any depth is refused with
  `DuplicateKey`, naming the key's path.
- Golden vectors ship in `test-vectors/` with `SHA256SUMS` and
  `generate.py`, the independent Python implementation of the digest and
  report rules that writes them.
- Repository tiers read no file through a link: every path component below
  the tier root (module, roles and role directories, `fragments/`, each file)
  must not be a symbolic link or Windows junction, and every file must resolve
  inside the root (`LinkInRepositoryTier`, `OutsideTierRoot`). The root itself
  may be reached through a link.
