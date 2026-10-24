//! The typed errors. Every variant names the field, fragment, template or file
//! at fault, so a refusal can be acted on without reading the engine's source.

use std::{fmt, io, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Where a piece of persona text came from.
///
/// `File` names the tier and the file's path relative to that tier's root
/// (for example `cortexkit/demo/roles/writer/PERSONA.md` in tier `user`), which
/// is stable across machines. `Supplied` is a qualified fragment the module
/// handed to [`crate::LoadedRole::render`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum SourceRef {
    /// A file in a tier.
    File {
        /// The tier's name.
        tier: String,
        /// Path relative to the tier root, `/`-separated.
        path: String,
    },
    /// A fragment source supplied by the module at render time.
    Supplied {
        /// The qualified fragment id.
        fragment: String,
    },
}

impl fmt::Display for SourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceRef::File { tier, path } => write!(f, "{path} (tier `{tier}`)"),
            SourceRef::Supplied { fragment } => write!(f, "supplied fragment `{fragment}`"),
        }
    }
}

/// Why a render stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RenderFailure {
    /// A value the template read is not in the context (strict undefined).
    UndefinedValue,
    /// The template ran out of fuel (too many instructions).
    OutOfFuel,
    /// Includes or nesting went deeper than the recursion limit.
    RecursionLimit,
    /// Any other template runtime error, such as a type error in a filter.
    Other,
}

impl fmt::Display for RenderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RenderFailure::UndefinedValue => "undefined value",
            RenderFailure::OutOfFuel => "out of fuel",
            RenderFailure::RecursionLimit => "recursion limit exceeded",
            RenderFailure::Other => "template error",
        })
    }
}

/// Every refusal the engine produces.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PersonaError {
    /// An identifier (module, role, tier, fragment, block, workspace, path
    /// segment, extension key) does not follow its grammar.
    #[error("invalid {what} `{value}`: {reason}")]
    InvalidId {
        /// Which kind of identifier.
        what: &'static str,
        /// The value as given.
        value: String,
        /// The rule it breaks.
        reason: &'static str,
    },
    /// The module passed an empty tier list.
    #[error("the tier list is empty")]
    NoTiers,
    /// Two tiers share a name, so reports and `extends` could not tell them apart.
    #[error("tier name `{tier}` appears more than once in the tier list")]
    DuplicateTier {
        /// The repeated name.
        tier: String,
    },
    /// An extension key collides with a core field or was registered twice.
    #[error("extension key `{key}` cannot be registered: {reason}")]
    ExtensionConflict {
        /// The key.
        key: String,
        /// Why.
        reason: &'static str,
    },
    /// The platform config directory could not be resolved to an absolute path.
    #[error("the config directory resolved to `{resolved}`, which is not absolute; set XDG_CONFIG_HOME to an absolute path (or HOME; APPDATA on Windows)")]
    ConfigHomeNotAbsolute {
        /// What the resolver returned.
        resolved: String,
    },
    /// A tier file or directory could not be read.
    #[error("cannot read {path}: {error}", path = path.display())]
    Io {
        /// The path on disk.
        path: PathBuf,
        /// The underlying error.
        #[source]
        error: io::Error,
    },
    /// A file is larger than the engine accepts.
    #[error("{file}: larger than {limit} bytes")]
    FileTooLarge {
        /// The file.
        file: SourceRef,
        /// The limit in bytes.
        limit: u64,
    },
    /// A file is not valid UTF-8.
    #[error("{file}: not valid UTF-8")]
    NotUtf8 {
        /// The file.
        file: SourceRef,
    },
    /// A layer file is not a regular file (a directory, a device, ...).
    #[error("{file}: not a regular file")]
    NotRegularFile {
        /// The file.
        file: SourceRef,
    },
    /// A path component below a repository tier's root is a symbolic link
    /// (on Windows also a junction), which could pull files from outside the
    /// repository into the persona.
    #[error("{component}: is a symbolic link or junction; no component below a repository tier's root may be a link")]
    LinkInRepositoryTier {
        /// The first linked component, as a path relative to the tier root.
        component: SourceRef,
    },
    /// A repository-tier file resolves to a location outside the tier root.
    #[error("{file}: resolves outside the tier root")]
    OutsideTierRoot {
        /// The file.
        file: SourceRef,
    },
    /// `PERSONA.md` does not start with a `---` front matter line.
    #[error("{file}: must start with a `---` front matter line")]
    MissingFrontMatter {
        /// The file.
        file: SourceRef,
    },
    /// The front matter has no closing `---` line.
    #[error("{file}: front matter has no closing `---` line")]
    UnterminatedFrontMatter {
        /// The file.
        file: SourceRef,
    },
    /// The front matter is not valid YAML, or not a mapping with string keys.
    #[error("{file}: front matter: {message}")]
    FrontMatter {
        /// The file.
        file: SourceRef,
        /// The parser's message.
        message: String,
    },
    /// The front matter repeats a mapping key, at any depth. A reader of the
    /// file would see the first value while a parser may apply the last.
    #[error("{file}: front matter key `{key}` appears more than once")]
    DuplicateKey {
        /// The file.
        file: SourceRef,
        /// The key's path, such as `version`, `limits.max_words` or
        /// `flows[0].package`.
        key: String,
    },
    /// A top-level key that is neither a core field nor a registered extension.
    #[error("{file}: unknown field `{field}`")]
    UnknownField {
        /// The file.
        file: SourceRef,
        /// The key.
        field: String,
    },
    /// A field has the wrong type or breaks its rule.
    #[error("{file}: field `{field}`: {reason}")]
    InvalidField {
        /// The file.
        file: SourceRef,
        /// The field.
        field: String,
        /// The rule it breaks.
        reason: String,
    },
    /// The first applied layer omits a field that layer must supply.
    #[error("{file}: field `{field}` is required in the first applied layer")]
    MissingField {
        /// The first applied layer's `PERSONA.md`.
        file: SourceRef,
        /// The field.
        field: String,
    },
    /// An extension's merge rule refused a layer's value.
    #[error("{file}: field `{field}` could not be merged: {reason}")]
    ExtensionMerge {
        /// The layer being merged.
        file: SourceRef,
        /// The extension key.
        field: String,
        /// The merge rule's message.
        reason: String,
    },
    /// No tier supplied an applicable layer for the role.
    #[error("role `{role}`: no tier has an applicable PERSONA.md")]
    NoLayers {
        /// The role.
        role: String,
    },
    /// Every applied layer has an empty body, so there is nothing to render.
    #[error("role `{role}`: every applied layer has an empty body")]
    NoBody {
        /// The role.
        role: String,
    },
    /// A layer's `extends` parent cannot be used.
    #[error("{file}: extends role `{role}` at tier `{tier}`: {reason}")]
    ExtendsUnavailable {
        /// The declaring layer.
        file: SourceRef,
        /// The parent's role.
        role: String,
        /// The parent's tier.
        tier: String,
        /// Why.
        reason: &'static str,
    },
    /// A declared fragment has no source: no bundle file for a bare id, or no
    /// supplied source for a qualified id.
    #[error("role `{role}`: fragment `{fragment}` is declared but has no source")]
    MissingFragment {
        /// The role.
        role: String,
        /// The fragment id.
        fragment: String,
    },
    /// A supplied fragment source used a bare id; only qualified ids are supplied.
    #[error("supplied fragment `{fragment}` must use a qualified id (owner/name@major)")]
    SuppliedFragmentNotQualified {
        /// The id as given.
        fragment: String,
    },
    /// A template does not parse.
    #[error("{origin}: line {line}: template syntax: {message}")]
    TemplateSyntax {
        /// The template's source.
        origin: SourceRef,
        /// The line, 1-based (0 when the parser gave none).
        line: usize,
        /// The parser's message.
        message: String,
    },
    /// An `include`, `extends` or `import` whose target is not a literal string.
    #[error("{origin}: line {line}: `{statement}` must name a literal fragment id")]
    NonLiteralReference {
        /// The template's source.
        origin: SourceRef,
        /// The line, 1-based.
        line: usize,
        /// The statement kind.
        statement: &'static str,
    },
    /// An `include`, `extends` or `import` of a fragment the role does not declare.
    #[error("{origin}: line {line}: `{statement}` of `{target}`, which is not in the role's declared fragments")]
    UndeclaredReference {
        /// The template's source.
        origin: SourceRef,
        /// The line, 1-based.
        line: usize,
        /// The statement kind.
        statement: &'static str,
        /// The target as written.
        target: String,
    },
    /// `include ... ignore missing`, which would render a missing fragment as empty.
    #[error(
        "{origin}: line {line}: `ignore missing` is refused; a missing fragment must fail by name"
    )]
    IgnoreMissing {
        /// The template's source.
        origin: SourceRef,
        /// The line, 1-based.
        line: usize,
    },
    /// A body that inherits from a lower body has output outside `{% block %}`,
    /// which template inheritance would silently drop.
    #[error("{origin}: line {line}: this body extends a lower body, so everything outside `{{% block %}}` would be dropped; put the text in a block")]
    ContentOutsideBlocks {
        /// The template's source.
        origin: SourceRef,
        /// The line, 1-based.
        line: usize,
    },
    /// A body that already inherits from a lower body also has its own `extends`.
    #[error("{origin}: line {line}: this body already extends a lower body and cannot also use `extends`")]
    ExtendsInInheritingBody {
        /// The template's source.
        origin: SourceRef,
        /// The line, 1-based.
        line: usize,
    },
    /// A `disabled_blocks` id names no block in the entry template's inheritance chain.
    #[error("role `{role}`: disabled block `{block}` is not a block of the entry template or its parents")]
    UnknownDisabledBlock {
        /// The role.
        role: String,
        /// The block id.
        block: String,
    },
    /// The render context breaks a rule.
    #[error("render context field `{field}`: {reason}")]
    InvalidContext {
        /// The dotted field path, for example `demand.iq`.
        field: String,
        /// The rule it breaks.
        reason: String,
    },
    /// The template failed while rendering.
    #[error("{}line {}: {failure}: {detail}{}", origin.as_ref().map(|o| format!("{o}: ")).unwrap_or_default(), line.map(|l| l.to_string()).unwrap_or_else(|| "?".into()), snippet.as_ref().map(|s| format!(" (at `{s}`)")).unwrap_or_default())]
    Render {
        /// The template's source, when the engine reported one.
        origin: Option<SourceRef>,
        /// The line, 1-based, when known.
        line: Option<u32>,
        /// What kind of failure.
        failure: RenderFailure,
        /// The engine's message.
        detail: String,
        /// The trimmed source line, which names the value or statement at fault.
        snippet: Option<String>,
    },
}
