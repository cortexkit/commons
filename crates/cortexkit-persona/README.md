# cortexkit-persona

The shared engine a module uses to load, merge and render the system text
("persona") of its own roles: `PERSONA.md` bundles in an ordered tier ladder
with trust classes, operator approvals for repository tiers, typed extension
keys, MiniJinja templates in a sealed environment, LF-normalised digests, and
a serialisable resolution report. [SPEC.md](SPEC.md) states every rule
(persona/v1).

```toml
[dependencies]
cortexkit-persona = "0.1.0"
```

## Use

```rust
use cortexkit_persona::*;

// The module's compiled-in base bundles.
let base = EmbeddedTier::new()
    .with_file("scribe", "PERSONA.md", include_str!("../roles/scribe/PERSONA.md"))?
    .with_file("scribe", "fragments/rules.md", include_str!("../roles/scribe/fragments/rules.md"))?;

// base, user and workspace under the platform config directory, and the
// project tier, which applies only when approved.
let tiers = DefaultLadder::new("demo", base)?
    .workspace("ws-1")?
    .project(project_root)
    .tiers();

let mut engine = PersonaEngine::new("demo")?;
engine.register_extension(Extension::<Limits>::new("limits"))?;

let approvals: ApprovalSet = stored_approvals(); // the module's own operator state
let key = ProjectKey::new(project_identity)?;   // opaque, never a working-directory path
let role = engine.load(LoadRequest::new("scribe", &tiers, &approvals).project_key(&key))?;

let supplied = FragmentSources::new().with("mc/memory-profile@1", profile_text)?;
let out = role.render(&context, &supplied)?;
// out.bytes(), out.digest, and out.report (serialisable, for diagnostic views)
```

At `load`, a `repository` layer that is not approved, or whose bytes changed
since its approval, is skipped whole; `role.tiers()` still lists it with
`approval` set to `unapproved` or `changed`, its `relative_path` and its
`layer_digest`. Record an approval from that `load` result, before rendering:
take `role.tiers()[i]` for the repository tier, read its `layer_digest`, and
store it keyed by module, role, project key and `relative_path`. A render's
report lists the same tiers, so it shows a repository layer that applied or
one that was skipped, but you never need a render to approve one.

## Guarantees

- Rendering touches no filesystem, environment or clock; the same loaded role,
  context and supplied fragments give byte-identical output and report.
- Qualified fragment ids (`owner/name@major`) resolve only from sources the
  module supplies; no bundle file at any tier can provide one.
- MiniJinja is pinned at `=2.12.0`. Any change to a golden render's bytes or
  digests (`test-vectors/`, checked by `SHA256SUMS`) is a breaking release.
