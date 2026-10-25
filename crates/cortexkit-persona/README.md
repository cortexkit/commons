# cortexkit-persona

The shared engine a module uses to load, merge and render the system text
("persona") of its own roles: `PERSONA.md` bundles in an ordered tier ladder
with trust classes, operator approvals for repository tiers, typed extension
keys, MiniJinja templates in a sealed environment, LF-normalised digests, and
a serialisable resolution report. [SPEC.md](SPEC.md) states every rule
(persona/v1).

```toml
[dependencies]
cortexkit-persona = "0.2.1"
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

The context carries the built-in roots (`agent`, `model`, `harness`, `tools`,
`mcp`, `modules`, `demand`, `env`, `module`). A module whose templates read
more than that declares the extra roots itself:

```rust
use cortexkit_persona::{TextValue, Value};

let context = context
    .with_root(
        "config",
        Value::map([("board", Value::map([("enabled", true.into())]))]),
    )
    .with_root("limits", Value::map([("sidekick_readers", 6i64.into())]))
    // Prints as its exact text; `contract.submit_tool` reads the field.
    .with_root(
        "contract",
        TextValue::new(contract_text).with_field("submit_tool", true),
    );
```

A declared root is visible to every template rendered with that context,
including an operator-approved project-level persona, so approvals stay tied
to a layer's exact content and a declared root never changes a layer's
digest. A name that shadows a built-in root or is declared twice is refused by
name before rendering, and a root the caller never declares is still a strict
undefined error.

At `load`, a `repository` layer that is not approved, or whose bytes changed
since its approval, is skipped whole; `role.tiers()` still lists it with
`approval` set to `unapproved` or `changed`, its `relative_path` and its
`layer_digest`. Record an approval from that `load` result, before rendering:
take `role.tiers()[i]` for the repository tier, read its `layer_digest`, and
store it keyed by module, role, project key and `relative_path`. A render's
report lists the same tiers, so it shows a repository layer that applied or
one that was skipped, but you never need a render to approve one.

## Per-render execution limits

`LoadedRole::render` keeps the default budgets: `FUEL_LIMIT` is 1,000,000
template instructions and `RECURSION_LIMIT` is 100 recursion-depth units. To
choose both budgets for a single render, pass `RenderLimits` to
`render_with_limits`:

```rust
use std::num::{NonZeroU64, NonZeroUsize};
use cortexkit_persona::RenderLimits;

let limits = RenderLimits::default()
    .with_recursion(NonZeroUsize::new(200).unwrap())
    .with_fuel(NonZeroU64::new(100_000).unwrap());
let out = role.render_with_limits(&context, &supplied, limits)?;
```

The nonzero budgets are available through `recursion()` and `fuel()`. They
apply only to that call, not to later renders of the loaded role. Limits are
not template roots or fields of `RenderContext` or `ResolutionReport`. They
decide only whether rendering completes: they never change layer or fragment
digests, approval keys, or a completed render's bytes, digest or report.

## Guarantees

- Rendering touches no filesystem, environment or clock; the same loaded role,
  context and supplied fragments give byte-identical output and report.
- Template files and supplied fragments are LF-normalised when loaded, so a
  CRLF checkout renders the same bytes as an LF one. The output itself is not
  rewritten: a value the caller supplies, including a text value, is rendered
  exactly as given.
- Qualified fragment ids (`owner/name@major`) resolve only from sources the
  module supplies; no bundle file at any tier can provide one.
- MiniJinja is pinned at `=2.12.0`. Any change to a golden render's bytes or
  digests (`test-vectors/`, checked by `SHA256SUMS`) is a breaking release.
