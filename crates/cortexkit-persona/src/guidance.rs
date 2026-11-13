//! Named guidance templates rendered without persona tiers or approvals.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{normalize_newlines, Digest, PersonaError, RenderLimits, SourceRef, MAX_FILE_BYTES};

mod check;
mod shape;

pub use shape::FactsShape;

/// An immutable, caller-owned collection of guidance templates.
///
/// Sources are LF-normalized at construction. Includes, imports and inheritance
/// use literal ids within this set only; there is no filesystem loader.
#[derive(Clone, Debug)]
pub struct GuidanceSet {
    templates: BTreeMap<String, String>,
}

/// The output and source identities of a guidance render.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rendered {
    /// Rendered bytes as UTF-8 text.
    pub text: String,
    /// SHA-256 of the output bytes.
    pub output_digest: Digest,
    /// SHA-256 of every LF-normalized source in the set, in id order.
    /// Includes unused templates, so this is independent of render branches.
    pub template_digests: BTreeMap<String, Digest>,
}

impl GuidanceSet {
    /// Collect and validate named sources, typically supplied with `include_str!`.
    ///
    /// Ids use the existing bare-id grammar: 1–64 lowercase ASCII letters,
    /// digits or hyphens, starting and ending with a letter or digit. Duplicate
    /// ids, sources larger than [`MAX_FILE_BYTES`], invalid syntax, dynamic
    /// template references, missing references and `ignore missing` are refused.
    pub fn new<I, N, T>(templates: I) -> Result<Self, PersonaError>
    where
        I: IntoIterator<Item = (N, T)>,
        N: Into<String>,
        T: Into<String>,
    {
        let mut sources = BTreeMap::new();
        for (id, text) in templates {
            let id = id.into();
            crate::ids::check_bare("guidance template id", &id)?;
            let text = text.into();
            if text.len() as u64 > MAX_FILE_BYTES {
                return Err(PersonaError::FileTooLarge {
                    file: origin(&id),
                    limit: MAX_FILE_BYTES,
                });
            }
            if sources
                .insert(id.clone(), normalize_newlines(&text).into_owned())
                .is_some()
            {
                return Err(error(&id, "duplicate template id"));
            }
        }
        let declared = sources.keys().cloned().collect();
        for (id, text) in &sources {
            crate::template::analyse(text, &origin(id), &declared, false)?;
        }
        Ok(Self { templates: sources })
    }

    /// SHA-256 of the compact JSON object mapping ids to LF-normalized texts.
    ///
    /// Keys are sorted lexicographically by UTF-8 bytes. JSON framing makes id
    /// and source boundaries unambiguous. Every template participates, even if
    /// no entry currently references it.
    pub fn digest(&self) -> Digest {
        Digest::of(&serde_json::to_vec(&self.templates).expect("string map serializes"))
    }

    /// Check every source and branch against an object-shaped facts declaration.
    ///
    /// Uses a custom walk of MiniJinja's AST, not its root-name analysis. Dotted
    /// and literal-string item paths, scalar-list indexing/loops and simple
    /// `set`/`with` aliases are checked, including accesses guarded by `is defined`.
    /// Each template is checked independently against the facts roots; it cannot
    /// rely on locals established by its includer. Names first bound in a
    /// conditional are unavailable after it; existing aliases retain their
    /// shape only when both branches agree. Bindings must not change a declared
    /// facts root's shape, including when shadowing it in a loop.
    ///
    /// This is deliberately conservative, not a general Jinja type checker:
    /// dynamic object keys, attribute-indirection filters, unmodelled call
    /// results and attributes on scalar loop items are refused rather than
    /// silently unchecked. Filter/call results are treated as opaque scalars;
    /// their member paths and iteration cannot be proven. This does not validate
    /// runtime facts against the schema or require every declared field at render
    /// time. A template may supply its own default with `is defined`.
    pub fn check(&self, shape: &FactsShape) -> Result<(), PersonaError> {
        if !matches!(&shape.0, shape::Shape::Object(_)) {
            return Err(error("<set>", "facts shape must describe an object"));
        }
        for (id, text) in &self.templates {
            check::check(id, text, shape)?;
        }
        Ok(())
    }

    /// Render one entry against a JSON object whose top-level keys are roots.
    ///
    /// Shares the persona renderer's sealed, strict-undefined, non-escaping
    /// environment and per-call fuel/recursion limits. Caller values are inserted
    /// verbatim, never parsed as templates or newline-normalized. No clock,
    /// environment, filesystem, session state or implicit facts are consulted.
    pub fn render(
        &self,
        entry: &str,
        facts: &Value,
        limits: RenderLimits,
    ) -> Result<Rendered, PersonaError> {
        if !facts.is_object() {
            return Err(error(entry, "facts must be one JSON object"));
        }
        let mut env = crate::template::environment(limits);
        for (id, text) in &self.templates {
            env.add_template(id, text)
                .map_err(|e| error(id, e.to_string()))?;
        }
        let text = env
            .get_template(entry)
            .and_then(|template| template.render(facts))
            .map_err(|e| error(entry, e.to_string()))?;
        Ok(Rendered {
            output_digest: Digest::of(text.as_bytes()),
            text,
            template_digests: self
                .templates
                .iter()
                .map(|(id, text)| (id.clone(), Digest::of(text.as_bytes())))
                .collect(),
        })
    }
}

fn origin(id: &str) -> SourceRef {
    SourceRef::Guidance {
        template: id.into(),
    }
}

fn error(id: &str, message: impl Into<String>) -> PersonaError {
    PersonaError::Guidance {
        template: id.into(),
        message: message.into(),
    }
}
