//! Rendering: check every template, render the entry in a sealed environment,
//! and report what was used.

use std::collections::{BTreeMap, BTreeSet};

use minijinja::ErrorKind;
use serde::Serialize;
use serde_json::json;

use crate::{
    digest::normalize_newlines,
    report::{FragmentOrigin, FragmentReport, ResolutionReport},
    template::{analyse, environment, Analysis},
    Digest, FragmentId, LoadedRole, PersonaError, RenderContext, RenderFailure, RenderLimits,
    SourceRef,
};

/// The report and source-revision format tag.
pub const FORMAT: &str = "persona/v1";

/// Fragment sources a module supplies at render time, keyed by qualified id
/// (`owner/name@major`). A supplied fragment carries the module's trust, and
/// the module guarantees it is byte-stable for whatever it freezes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FragmentSources {
    sources: BTreeMap<String, String>,
}

impl FragmentSources {
    /// No supplied fragments.
    pub fn new() -> Self {
        Self::default()
    }

    /// Supply the text of qualified fragment `id`. The text is LF-normalised,
    /// like a bundle file: a supplied fragment stands in for a file the module
    /// ships, so its line endings follow the same rule.
    pub fn insert(&mut self, id: &str, text: impl Into<String>) -> Result<(), PersonaError> {
        let parsed = FragmentId::parse(id)?;
        if !parsed.is_qualified() {
            return Err(PersonaError::SuppliedFragmentNotQualified {
                fragment: id.to_string(),
            });
        }
        let text = text.into();
        self.sources
            .insert(id.to_string(), normalize_newlines(&text).into_owned());
        Ok(())
    }

    /// Builder form of [`Self::insert`].
    pub fn with(mut self, id: &str, text: impl Into<String>) -> Result<Self, PersonaError> {
        self.insert(id, text)?;
        Ok(self)
    }
}

/// The result of a render.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rendered {
    /// The rendered text, exactly the bytes MiniJinja produced. Every line
    /// ending a template file had was already normalised to LF when the role
    /// was loaded; text a context value contributes is passed through
    /// verbatim.
    pub text: String,
    /// SHA-256 of [`Self::text`]'s bytes.
    pub digest: Digest,
    /// What the engine used.
    pub report: ResolutionReport,
}

impl Rendered {
    /// The rendered bytes.
    pub fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }
}

struct Prepared {
    /// The source handed to MiniJinja (an inheriting body gains an `extends`
    /// tag on its first line, so line numbers are unchanged).
    source: String,
    /// The text as written, for error snippets.
    text: String,
    origin: SourceRef,
    analysis: Analysis,
    parent: Option<String>,
}

impl LoadedRole {
    /// Render the role. Touches no filesystem, environment or clock: every
    /// input is the loaded role, `context` and `supplied`.
    /// Uses [`RenderLimits::default`]; see [`Self::render_with_limits`] to
    /// choose execution budgets for one render.
    pub fn render<M: Serialize>(
        &self,
        context: &RenderContext<M>,
        supplied: &FragmentSources,
    ) -> Result<Rendered, PersonaError> {
        self.render_with_limits(context, supplied, RenderLimits::default())
    }

    /// Render the role with the given execution budgets for this call only.
    ///
    /// Like [`Self::render`], touches no filesystem, environment or clock.
    /// Limits decide only whether rendering completes: they do not change
    /// digests, approvals or a completed render's bytes.
    pub fn render_with_limits<M: Serialize>(
        &self,
        context: &RenderContext<M>,
        supplied: &FragmentSources,
        limits: RenderLimits,
    ) -> Result<Rendered, PersonaError> {
        context.validate()?;
        let declared: BTreeSet<String> = self
            .manifest
            .fragments
            .iter()
            .map(|f| f.to_string())
            .collect();

        let mut templates: BTreeMap<String, Prepared> = BTreeMap::new();
        let mut fragments: Vec<(String, FragmentOrigin, Digest)> = Vec::new();
        for id in &self.manifest.fragments {
            let (text, origin, from) = if id.is_qualified() {
                // Qualified ids resolve only from module-supplied sources;
                // no bundle file at any tier can provide one.
                let text = supplied.sources.get(id.as_str()).ok_or_else(|| {
                    PersonaError::MissingFragment {
                        role: self.role.clone(),
                        fragment: id.to_string(),
                    }
                })?;
                (
                    text.clone(),
                    SourceRef::Supplied {
                        fragment: id.to_string(),
                    },
                    FragmentOrigin::Supplied,
                )
            } else {
                let b = &self.bundle_fragments[id.as_str()];
                (
                    b.text.clone(),
                    b.origin.clone(),
                    FragmentOrigin::Bundle {
                        tier: b.tier.clone(),
                    },
                )
            };
            let analysis = analyse(&text, &origin, &declared, false)?;
            fragments.push((id.to_string(), from, Digest::of(text.as_bytes())));
            templates.insert(
                id.to_string(),
                Prepared {
                    source: text.clone(),
                    text,
                    origin,
                    analysis,
                    parent: None,
                },
            );
        }
        for body in &self.bodies {
            let analysis = analyse(&body.text, &body.origin, &declared, body.parent.is_some())?;
            let source = match &body.parent {
                Some(parent) => format!("{{% extends \"{parent}\" %}}{}", body.text),
                None => body.text.clone(),
            };
            templates.insert(
                body.name.clone(),
                Prepared {
                    source,
                    text: body.text.clone(),
                    origin: body.origin.clone(),
                    analysis,
                    parent: body.parent.clone(),
                },
            );
        }

        // The include closure: everything the entry can reach through
        // inheritance and references, whether or not a branch is taken.
        let mut included = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut stack = vec![self.entry.clone()];
        while let Some(name) = stack.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let t = &templates[&name];
            stack.extend(t.parent.iter().cloned());
            for r in &t.analysis.references {
                included.insert(r.clone());
                stack.push(r.clone());
            }
        }

        // `disabled_blocks` empties blocks of the entry template's inheritance
        // chain: the entry body, each body it extends in turn, then each
        // fragment reached by following `{% extends %}` from the root body,
        // fragment to fragment. Blocks of a fragment reached only through
        // `include` or `import` render on their own and cannot be overridden,
        // so naming one, or any block outside the chain, is a mistake rather
        // than a no-op.
        let mut chain_blocks = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut current = Some(self.entry.clone());
        while let Some(name) = current {
            if !visited.insert(name.clone()) {
                break;
            }
            let t = &templates[&name];
            chain_blocks.extend(t.analysis.blocks.iter().cloned());
            current = t.parent.clone().or_else(|| t.analysis.extends.clone());
        }
        for block in &self.manifest.disabled_blocks {
            if !chain_blocks.contains(block) {
                return Err(PersonaError::UnknownDisabledBlock {
                    role: self.role.clone(),
                    block: block.clone(),
                });
            }
        }

        // A wrapper template that extends the entry and redefines each
        // disabled block as empty, so the empty definition is the one that
        // wins.
        let wrapper = (!self.manifest.disabled_blocks.is_empty()).then(|| {
            let mut wrapper = format!("{{% extends \"{}\" %}}", self.entry);
            for block in &self.manifest.disabled_blocks {
                wrapper.push_str(&format!("{{% block {block} %}}{{% endblock %}}"));
            }
            wrapper
        });
        // Each call owns its environment, so its budgets cannot affect another render.
        let mut env = environment(limits);
        for (name, t) in &templates {
            env.add_template(name, &t.source)
                .map_err(|e| PersonaError::TemplateSyntax {
                    origin: t.origin.clone(),
                    line: e.line().unwrap_or(0),
                    message: e.detail().unwrap_or("invalid syntax").to_string(),
                })?;
        }
        let entry = match &wrapper {
            Some(wrapper) => {
                env.add_template("@entry", wrapper)
                    .expect("block ids are checked identifiers");
                "@entry"
            }
            None => self.entry.as_str(),
        };
        let template = env.get_template(entry).expect("the entry was added");
        // Template sources were LF-normalised when the role was loaded, and the
        // output itself is not normalised: a CR in it can only come from a
        // context value, and rewriting it would change caller-supplied bytes.
        let text = template
            .render(context.to_value())
            .map_err(|e| render_error(&e, &templates))?;
        let digest = Digest::of(text.as_bytes());

        let layers: Vec<serde_json::Value> = self
            .layer_digests
            .iter()
            .map(|(tier, role, d)| json!({"tier": tier, "role": role, "digest": d.to_string()}))
            .collect();
        let closure: Vec<serde_json::Value> = fragments
            .iter()
            .filter(|(id, _, _)| included.contains(id))
            .map(|(id, from, d)| json!({"id": id, "origin": from, "digest": d.to_string()}))
            .collect();
        let revision_doc = json!({
            "format": FORMAT,
            "role": self.role,
            "layers": layers,
            "manifest": self.manifest.to_json(),
            "fragments": closure,
        });
        let canonical = serde_json_canonicalizer::to_vec(&revision_doc).map_err(|e| {
            PersonaError::InvalidField {
                file: self.bodies[0].origin.clone(),
                field: "manifest".into(),
                reason: format!("cannot be written as canonical JSON: {e}"),
            }
        })?;

        let report = ResolutionReport {
            format: FORMAT.to_string(),
            module: self.module.clone(),
            role: self.role.clone(),
            tiers: self.tiers.clone(),
            parents: self.parents.clone(),
            fragments: fragments
                .into_iter()
                .map(|(id, origin, digest)| FragmentReport {
                    included: included.contains(&id),
                    id,
                    origin,
                    digest,
                })
                .collect(),
            entry_tier: self.entry_tier.clone(),
            source_revision: Digest::of(&canonical),
            rendered_digest: digest,
        };
        Ok(Rendered {
            text,
            digest,
            report,
        })
    }
}

fn render_error(e: &minijinja::Error, templates: &BTreeMap<String, Prepared>) -> PersonaError {
    // An error inside an included template arrives wrapped in a "could not
    // render include" error; the innermost one names the real cause and file.
    let mut e = e;
    while let Some(inner) =
        std::error::Error::source(e).and_then(|s| s.downcast_ref::<minijinja::Error>())
    {
        e = inner;
    }
    let detail = e.detail().unwrap_or_default().to_string();
    let failure = match e.kind() {
        ErrorKind::UndefinedError => RenderFailure::UndefinedValue,
        ErrorKind::OutOfFuel => RenderFailure::OutOfFuel,
        _ if detail.contains("recursion limit") => RenderFailure::RecursionLimit,
        _ => RenderFailure::Other,
    };
    let template = e.name().and_then(|n| templates.get(n));
    let snippet = match (template, e.line()) {
        (Some(t), Some(line)) if line > 0 => {
            t.text.lines().nth(line - 1).map(|l| l.trim().to_string())
        }
        _ => None,
    };
    PersonaError::Render {
        origin: template.map(|t| t.origin.clone()),
        line: e.line().map(|l| u32::try_from(l).unwrap_or(u32::MAX)),
        failure,
        detail: if detail.is_empty() {
            failure.to_string()
        } else {
            detail
        },
        snippet,
    }
}
