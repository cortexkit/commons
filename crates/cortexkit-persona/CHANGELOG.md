# Changelog

## 0.2.1

- `RenderLimits` configures nonzero recursion and fuel budgets with
  `with_recursion` and `with_fuel`, read back through `recursion` and `fuel`.
  Pass it to `LoadedRole::render_with_limits` to choose budgets for one call;
  a call's limits cannot affect later renders of the same loaded role.
- `LoadedRole::render` and `RenderContext` are unchanged. Default limits remain
  `RECURSION_LIMIT` (100 recursion-depth units) and `FUEL_LIMIT` (1,000,000
  instructions).
  Limits decide only whether a render completes and never change digests,
  approval keys, a completed render's bytes or its resolution report. Every
  existing golden vector is unchanged.

## 0.2.0

- Caller-declared roots: `RenderContext::with_root(name, value)` declares an
  extra root that every template rendered with that context can read, next to
  the built-in roots. A name must be a template identifier and must not shadow
  a built-in root or repeat another declaration; anything else is refused by
  name before rendering. An undeclared root is still a strict undefined error.
- `Value` and `TextValue`: the typed values a caller supplies (text, integer,
  boolean, list, map, and text with named fields). A text value prints as its
  exact text while `contract.submit_tool` reads one of its fields, so a
  template can use one value both ways.
- **Output is no longer LF-normalised.** Template files and supplied fragments
  are still normalised when they are loaded, so a CRLF checkout renders the
  same bytes as an LF one. The rendered output is now exactly what the
  templates produced, so text a caller supplies keeps its CR and CRLF bytes.
  A render that prints such a value can differ in bytes from 0.1.0; renders
  whose context holds no CR are unchanged, and every golden vector is
  unchanged.
- `Rendered.text` is documented as the produced bytes rather than normalised
  text.

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
