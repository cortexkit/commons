# persona/v1

A persona is the system text a module renders for one of its roles. Each
module renders its own roles with the `cortexkit-persona` library; there is no
persona service and no central role inventory. The library loads tiers, merges
layers, renders, and reports what it did. Storing approvals, and how an
operator confirms one, stay with each module.

This document is normative. "Refused" means the library returns a typed error
that identifies what failed (the field, fragment, template or file at fault
wherever there is one); it never falls back to a default or renders a partial
result.

## 1. The file: `PERSONA.md`

A role's layer in one tier is a directory named after the role id. It holds
`PERSONA.md` and, optionally, fragment files `fragments/<bare-id>.md`. A layer
is present when its `PERSONA.md` exists. A path component that is absent, or
a plain file where a directory belongs, makes the layer absent; in a
repository tier the first link found is refused instead (section 4). Nothing
else in the directory is part of the layer: other files are not read and do
not enter the layer digest. Each file is at most 1,048,576 bytes and must be
UTF-8; the limit is checked on the bytes as read, before line endings are
normalised.

`PERSONA.md` starts with a line `---`, then closed YAML front matter, then a
line `---`, then the MiniJinja body. Empty front matter is allowed. Unknown
top-level keys are refused by name unless the module registered the key as an
extension. A mapping key that appears twice, at any depth of the front matter
(a top-level field, a key inside an extension's value, a key of a list item),
is refused, naming the key's path such as `limits.max_words` or
`flows[0].package`: a reader of the file would see the first value while a
parser may apply the last.

**Core fields:**

| Field | Rule |
|---|---|
| `name` | equals the role id (the layer's directory name) |
| `version` | positive integer |
| `display_name` | non-blank string, at most 128 UTF-8 bytes |
| `extends` | optional `{role, tier}`: an explicit lower-layer parent of this layer's body (section 5) |
| `fragments` | optional list of fragment ids the templates may reference (section 3), no duplicates |
| `disabled_blocks` | optional list of block ids, no duplicates; each becomes an empty override of that block of the entry template (section 5) |

Identifier grammars:

- role id: lowercase ASCII letters, digits, `.`, `_` and `-`, starting with a
  letter or digit, at most 128 bytes;
- module id, tier name, bare fragment id: lowercase ASCII letters, digits and
  `-`, starting and ending with a letter or digit, at most 64 bytes;
- qualified fragment id: `owner/name@major`, where `owner` and `name` are bare
  ids and `major` is a decimal integer of at most nine digits without leading
  zeros;
- block id: lowercase ASCII letters, digits and `_`, not starting with a digit,
  at most 64 bytes (block names must be template identifiers).

**Extensions.** A module registers named top-level keys, each with its own
typed schema and merge rule, and may require a key in the first applied layer.
An extension key is lowercase letters, digits and `_`, starts with a letter,
is at most 64 bytes, and cannot be a core field. A module whose roles are
hireable agents might register keys such as `lifecycle`, `routing`, `tools`,
`model_recommendations`, `skills`, `identity`, `activation`, `requires`,
`flows` and `flows_remove`; a role carries none of them unless its module
registers them. Kept out of the file entirely: which component routes the
role and with what demand, the tools the module actually offers, and the
module's output schema.

## 2. The render context

Typed and closed; unknown keys are refused:

- `agent`: `name?`, `agent_id?`, `project?`, `workspace?`, `role`.
- `model`: `provider`, `id`, `family?`, `variant`.
- `harness`: `id`, `capabilities {name: bool}`.
- `tools`, `mcp`, `modules`: grant maps `{id: bool}`; a name that is not in
  the map reads as `false`.
- `demand`: `{iq, eq}`, each 0-100.
- `env`: `os` (`darwin`, `linux`, `windows`, `unknown`), `timezone` (an IANA
  name; checked only for its characters: ASCII letters, digits, `/`, `_`,
  `-`, `+`), `locale` (a BCP 47 tag; checked only for its shape: `-`-separated
  ASCII alphanumeric subtags of 1-8 characters).
- `module`: one value the module types itself (for example its config,
  output contract and limits).

An optional field that is absent reads as `none`, not as undefined. The
engine adds no time, counter or other ambient value to the context, so the
same inputs render the same bytes; a module that puts a time in its own
`module` value gives up that stability itself.

## 3. Templates and fragments

- MiniJinja at exactly `=2.12.0`, default features off, with `builtins`,
  `fuel`, `multi_template`, `serde` and `unstable_machinery`. Undefined values
  are strict errors. One render may execute at most 1,000,000 instructions
  (fuel) and has a recursion budget of 100. There is no loader and no custom
  filter, test, function or global; auto-escaping is off. Whitespace settings
  are MiniJinja's defaults: a template's single trailing newline is dropped,
  `trim_blocks` and `lstrip_blocks` are off.
- `include`, `extends`, `import` and `from ... import` take a literal string
  that is one of the role's declared `fragments`; anything else (a variable,
  an expression, a list) is refused. `ignore missing` is refused.
- A **bare id** (`header`) is the bundle file `fragments/header.md`. A
  **qualified id** `owner/name@major` (for example `mc/memory-profile@1`) is
  supplied by the module at render time as a fragment source. A missing
  fragment of either kind is refused by name, never rendered as empty: a bare
  one when the role loads, a qualified one when it renders.
- **Qualified ids resolve only from module-supplied sources.** No bundle file
  can provide one at any tier, so a repository-tier persona cannot shadow a
  module's fragment. A supplied fragment carries the module's trust, not the
  including layer's.
- The module guarantees that a supplied fragment is byte-stable for whatever
  it freezes. The report lists each supplied fragment's digest the same way it
  lists bundle fragments.
- Rendering touches no filesystem, environment or clock. Loading reads the
  tier files first; a loaded role renders even if its files are gone.

## 4. Tiers and trust

A module gives the library its ordered tier list, least to most specific.
Tier names are unique in the list. A role's layer in a tier lives at
`<root>/<roles_path>/<role>/`; its **tier-relative path** is
`<roles_path>/<role>` (for a tier compiled into the module, `roles/<role>`).
Each tier has a trust class:

- `operator`: applies as found. The default ladder uses it for the embedded
  base and for the user and workspace tiers (the operator's own files); a
  module may give it to any other folder it trusts, such as an installed
  catalog (signature checks belong to whatever installs the package, never to
  this library). Files may be symbolic links.
- `repository`: applies only when the operator has approved that layer's
  digest for that project. Its files must be regular files, and no path
  component below the tier root may be a link (a symbolic link, or on Windows
  a junction): the module, roles and role directories, `fragments/` and each
  file are all checked, and every file must resolve inside the tier root. The
  link and regular-file rules apply to tiers on disk; an embedded tier holds
  no paths and is never checked. The root itself may be reached through a
  link; it is resolved once. A link is
  refused by name, naming the first linked component.

The default ladder:

1. `base`: the module's embedded bundles (`operator`);
2. `user`: `<config>/cortexkit/<module>/roles/<role>/` (`operator`);
3. `workspace`: `<config>/cortexkit/<module>/workspaces/<ws>/roles/<role>/`
   (`operator`), when the module names a workspace;
4. `project`: `<project>/.cortexkit/<module>/roles/<role>/` (`repository`),
   when the module names a project root.

`<config>` is the platform config directory: a non-empty `XDG_CONFIG_HOME`;
on Windows, then `APPDATA`, then `USERPROFILE\AppData\Roaming`; then
`HOME/.config`. When none of those is set the lookup answers `.config`.
Empty variables count as unset, and a result that is not absolute is refused,
so an unset `HOME` (or a relative `XDG_CONFIG_HOME`) is an error rather than
a path taken relative to the current directory. A module may pass its own
longer ladder instead, with a trust class for each tier.

**Project-tier approval.** A present `repository` layer is in one of three
states:

- `approved`: the approval set holds an entry for
  `(module, role, project_key, tier-relative path, layer digest)`;
- `changed`: it holds an entry for the same module, role, project key and
  path, but with another digest;
- `unapproved`: anything else, including when the module gave no project key.

A layer that is `changed` or `unapproved` is skipped whole and never parsed,
so nothing in it applies; the next lower tier stands. The report names it with
its path, state and digest.

The module records an approval only after the operator confirms in person on
that computer, and keeps it as local operator state, never synced, where an
agent's write paths cannot reach it. `project_key` is an opaque project
identity of 1 to 512 bytes that the module supplies, never a working
directory, so a moved checkout keeps its approvals and a different repository
cloned into the same path does not inherit them.

## 5. Merging

Only applied layers take part (present, and `operator` or `approved`).

- The first applied layer must carry `name`, `version`, `display_name` and
  every extension its module registered as required. Later layers may be
  partial.
- A present field replaces the inherited one, except `disabled_blocks`, whose
  lists union. Each extension key merges by its registered rule: either
  "replace", or a rule the module supplies that sees, for every applied layer
  in order, the inherited value, the layer's value if any, and the layer's
  other extension values.
- `fragments`: the last present list wins. Each bare fragment's file comes
  from the most specific applied layer that has it; same-named fragments never
  concatenate.
- **Bodies.** A body is empty when it is only whitespace. Each non-empty body
  extends a parent: the body of the layer its `extends` names, or else the
  previous non-empty body. The first non-empty body without `extends` has no
  parent. The last non-empty body is the entry template.
  - `extends: {role, tier}` must name a tier lower than the declaring layer's.
    For the same role, that tier's layer must be applied and have a non-empty
    body. For another role, that role's layer in that tier is loaded under the
    same approval rule; only its body is used, as a template with no parent of
    its own, and it may not declare `extends` itself. A layer with `extends`
    must have a non-empty body.
  - A body that has a parent inherits from it (MiniJinja template
    inheritance). Its top level may hold only `{% block %}` overrides,
    whitespace, comments, `set` and `do`. Template inheritance renders only
    the blocks of a child, so text, expressions and every statement that
    prints or runs another template (`include`, `import`, `from ... import`,
    `if`, `for`, `with`, `filter`, `autoescape`, `set ... endset`) would be
    silently dropped or run outside any block there; they are refused, as is
    an `extends` of its own. A body without a parent may `extends` a declared
    fragment, and a fragment may `extends` another declared fragment.
- `disabled_blocks` empties each listed block of the entry template's
  inheritance chain: the entry body, each body it extends in turn (its
  `extends` parent or the previous non-empty body) down to the root body, then
  each fragment reached by following `{% extends %}` from the root body,
  fragment to fragment. A block anywhere in those templates counts, nested
  blocks included. A fragment reached only through `include` or `import` is
  not in the chain: it renders on its own and its blocks cannot be
  overridden. A listed block that is not in the chain is refused.

## 6. Bytes and digests

- Every source file and supplied fragment is normalised to LF (CRLF and lone
  CR become LF) when it is loaded, and the rendered output is normalised the
  same way. A Windows checkout that converted line endings therefore gives the
  same bytes and digests as macOS and Linux.
- Digests are SHA-256, written `sha256:` and 64 lowercase hex digits.
- **Layer digest:** SHA-256 over the layer's normalised files in byte order of
  their logical paths (`PERSONA.md`, `fragments/<id>.md`), each file framed as
  its logical path in UTF-8, a NUL byte, its byte length in ASCII decimal, a
  NUL byte, then its content.
- **Fragment digest:** SHA-256 of the fragment's normalised text, for bundle
  and supplied fragments alike.
- **Source revision:** SHA-256 of the RFC 8785 canonical JSON of
  `{"format": "persona/v1", "role", "layers", "manifest", "fragments"}` where
  - `layers` lists `{"tier", "role", "digest"}` for each applied layer in
    ladder order, then for each other-role `extends` parent;
  - `manifest` is the composed manifest: `name`, `version`, `display_name`,
    `fragments` (declared order), `disabled_blocks` (sorted), and each
    composed extension under its key;
  - `fragments` lists `{"id", "origin", "digest"}`, in declared order, for the
    fragments in the entry template's include closure only. The closure is
    everything the entry reaches through inheritance and `include`, `import`
    and `extends`, whether or not a branch is taken at render time. `origin`
    is `{"kind": "bundle", "tier"}` or `{"kind": "supplied"}`.
- **Rendered digest:** SHA-256 of the rendered bytes. The same sources,
  context and supplied fragments give identical bytes and digests, so a
  consumer can freeze the result per session.
- Golden renders with their digests ship with the crate in `test-vectors/`. A
  release that changes any golden's bytes, digests or expected report, a
  MiniJinja upgrade included, is a breaking release.

## 7. The resolution report

A deterministic, serialisable report:

- `format` (`persona/v1`), `module`, `role`;
- `tiers`, in ladder order: `name`, `trust`, `root` (as the module gave it, or
  `null` for an embedded tier), `relative_path`, `present`, `approval`
  (`not_required`, `approved`, `unapproved`, `changed`), `layer_digest`
  (`null` when absent) and `used`;
- `parents`: each other-role `extends` parent with its `role`, `tier`,
  `relative_path`, `approval` and `layer_digest`;
- `fragments`: the composed fragment list in declared order, each with `id`,
  `origin` (the tier it came from, or `supplied`), `digest` and `included`
  (whether it is in the include closure);
- `entry_tier`: the tier whose body is the entry template;
- `source_revision` and `rendered_digest`.

## 8. Tests

Golden vectors (role sources, contexts, expected bytes with digests and full
reports), plus tests that each fail by name when their rule is removed:

1. an unknown top-level key is refused, unless registered as an extension;
2. strict undefined: a missing context value is an error, not empty text;
3. an `include`, `extends`, `import` or `from ... import` whose target is not
   in the declared fragments is refused, as are a non-literal target and
   `ignore missing`;
4. a missing qualified fragment is refused by name;
5. an unapproved project layer is skipped whole and reported `unapproved`; an
   approved one whose bytes change is reported `changed` and skipped;
6. CRLF and LF sources give the same digests and bytes;
7. the same inputs render byte-identical twice, and the report is identical;
8. `disabled_blocks` unions across layers; other fields replace;
9. a repeated front matter key is refused by name, at any depth.
