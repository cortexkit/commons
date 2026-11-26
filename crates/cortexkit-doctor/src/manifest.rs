use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    evidence::valid_name,
    is_digest,
    secrets::secret_free,
    wire::{canonical_digest, frame, is_lower_hex, require, shape, string_enum, Object},
    DecodeError, EvidenceKind, ModuleId, MAX_OUTPUT_BYTES,
};

const SCHEMA: &str = "ck.doctor-manifest/v1";
const LOG_DIR: &str = "data";
const LOG_FORMAT: &str = "cortexkit-log/v1";

/// Derive a log field's allow-listed kind, or return `None` for an invalid or
/// unlisted name.
pub fn field_kind(name: &str) -> Option<EvidenceKind> {
    use EvidenceKind::*;

    if !valid_name(name) {
        return None;
    }
    if name.starts_with("is_") || name.starts_with("has_") {
        return Some(Bool);
    }
    if name.starts_with("n_") {
        return Some(Count);
    }
    Some(match name {
        "ok" => Bool,
        "elapsed_ms" | "duration_ms" | "timeout_ms" => DurationMs,
        "bytes" => Bytes,
        "count" => Count,
        "ratio" => Ratio,
        "code" | "reason" | "state" | "cause" | "outcome" | "level" | "kind" => Code,
        "version" => Version,
        "path" => Path,
        "session" | "agent" | "project" | "scope_ref" => Ref,
        _ => {
            return [
                ("_at_ms", TimestampMs),
                ("_ms", DurationMs),
                ("_bytes", Bytes),
                ("_count", Count),
                ("_ratio", Ratio),
                ("_code", Code),
                ("_reason", Code),
                ("_state", Code),
                ("_version", Version),
                ("_path", Path),
                ("_dir", Path),
                ("_file", Path),
                ("_id", Ref),
                ("_ref", Ref),
            ]
            .into_iter()
            .find_map(|(suffix, kind)| name.ends_with(suffix).then_some(kind));
        }
    })
}

/// Compute the first 12 lowercase hex digits of SHA-256 over `target\0text`.
pub fn template_id(target: &str, text: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(target);
    digest.update([0]);
    digest.update(text);
    format!("{:x}", digest.finalize())[..12].to_owned()
}

/// Compute the digest over the RFC 8785 encoding of the template array as given.
pub fn templates_digest(templates: &[Template]) -> Result<String, DecodeError> {
    canonical_digest(templates)
}

/// A relative glob for log files in the module's data directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct LogSource {
    dir: &'static str,
    glob: String,
    format: &'static str,
}

impl LogSource {
    pub fn new(glob: impl Into<String>) -> Result<Self, DecodeError> {
        let glob = glob.into();
        require(
            (1..=256).contains(&glob.len())
                && !glob.contains("**")
                && glob.split('/').all(|segment| {
                    !matches!(segment, "" | "." | "..")
                        && segment
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._*-".contains(&b))
                }),
        )?;
        Ok(Self {
            dir: LOG_DIR,
            glob,
            format: LOG_FORMAT,
        })
    }

    pub fn dir(&self) -> &str {
        self.dir
    }
    pub fn glob(&self) -> &str {
        &self.glob
    }
    pub fn format(&self) -> &str {
        self.format
    }
}

/// A field whose kind derives from its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct TemplateField {
    name: String,
    kind: EvidenceKind,
}

impl TemplateField {
    pub fn new(name: impl Into<String>) -> Result<Self, DecodeError> {
        let name = name.into();
        let kind = field_kind(&name).ok_or(DecodeError::Shape)?;
        Ok(Self { name, kind })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn kind(&self) -> EvidenceKind {
        self.kind
    }
}

/// A log message and its sorted allow-list of fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Template {
    id: String,
    target: String,
    text: String,
    fields: Vec<TemplateField>,
}

impl Template {
    pub fn new(
        target: impl Into<String>,
        text: impl Into<String>,
        fields: Vec<TemplateField>,
    ) -> Result<Self, DecodeError> {
        let (target, text) = (target.into(), text.into());
        validate_template(&target, &text, &fields)?;
        Ok(Self {
            id: template_id(&target, &text),
            target,
            text,
            fields,
        })
    }

    /// Test the manifest's target text bounds.
    pub fn valid_target(target: &str) -> bool {
        valid_plain(target, 256)
    }

    /// Test the manifest's message text bounds. The generator also skips
    /// messages starting with `[` because they are ambiguous in the log format.
    pub fn valid_text(text: &str) -> bool {
        valid_plain(text, 1024)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn target(&self) -> &str {
        &self.target
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn fields(&self) -> &[TemplateField] {
        &self.fields
    }
}

fn valid_plain(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.len()) && !text.chars().any(char::is_control)
}

fn validate_template(
    target: &str,
    text: &str,
    fields: &[TemplateField],
) -> Result<(), DecodeError> {
    require(
        Template::valid_target(target)
            && Template::valid_text(text)
            && fields
                .windows(2)
                .all(|pair| pair[0].name() < pair[1].name()),
    )
}

/// A complete `doctor manifest` document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Manifest {
    schema: &'static str,
    module_id: ModuleId,
    repository: String,
    logs: Vec<LogSource>,
    templates: Vec<Template>,
    templates_digest: String,
}

impl Manifest {
    pub fn new(
        module_id: ModuleId,
        repository: impl Into<String>,
        logs: Vec<LogSource>,
        templates: Vec<Template>,
    ) -> Result<Self, DecodeError> {
        let repository = repository.into();
        validate_metadata(&repository, &logs, &templates)?;
        let templates_digest = templates_digest(&templates)?;
        Ok(Self {
            schema: SCHEMA,
            module_id,
            repository,
            logs,
            templates,
            templates_digest,
        })
    }

    pub fn module_id(&self) -> &ModuleId {
        &self.module_id
    }
    pub fn repository(&self) -> &str {
        &self.repository
    }
    pub fn logs(&self) -> &[LogSource] {
        &self.logs
    }
    pub fn templates(&self) -> &[Template] {
        &self.templates
    }
    pub fn templates_digest(&self) -> &str {
        &self.templates_digest
    }

    /// Serialize the generator's exact file bytes: RFC 8785 JSON and one LF.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, DecodeError> {
        let mut bytes = serde_jcs::to_vec(self).map_err(|_| DecodeError::Shape)?;
        if bytes.len() >= MAX_OUTPUT_BYTES {
            return Err(DecodeError::TooLarge);
        }
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Validate framing, shape, template ids and the templates digest, in that order.
    pub fn decode(input: &[u8]) -> Result<Self, DecodeError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireManifest {
            schema: String,
            module_id: String,
            repository: String,
            logs: Vec<Object<WireLogSource>>,
            templates: Vec<Object<WireTemplate>>,
            templates_digest: String,
        }

        let Object(raw): Object<WireManifest> = shape(frame(input)?)?;
        require(raw.schema == SCHEMA && is_digest(&raw.templates_digest))?;
        let module_id = ModuleId::new(raw.module_id)?;
        let logs = raw
            .logs
            .into_iter()
            .map(|Object(log)| log.validate())
            .collect::<Result<Vec<_>, _>>()?;
        let templates = raw
            .templates
            .into_iter()
            .map(|Object(template)| template.validate())
            .collect::<Result<Vec<_>, _>>()?;
        validate_metadata(&raw.repository, &logs, &templates)?;
        for template in &templates {
            if template.id != template_id(&template.target, &template.text) {
                return Err(DecodeError::TemplateId);
            }
        }
        if raw.templates_digest != templates_digest(&templates)? {
            return Err(DecodeError::TemplatesDigest);
        }
        Ok(Self {
            schema: SCHEMA,
            module_id,
            repository: raw.repository,
            logs,
            templates,
            templates_digest: raw.templates_digest,
        })
    }
}

fn validate_metadata(
    repository: &str,
    logs: &[LogSource],
    templates: &[Template],
) -> Result<(), DecodeError> {
    require(
        valid_repository(repository)
            && (1..=8).contains(&logs.len())
            && templates.windows(2).all(|pair| pair[0].id() < pair[1].id()),
    )
}

fn valid_repository(url: &str) -> bool {
    let Some((owner, repo)) = url
        .strip_prefix("https://github.com/")
        .and_then(|path| path.split_once('/'))
    else {
        return false;
    };
    (1..=39).contains(&owner.len())
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && (1..=100).contains(&repo.len())
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !matches!(repo, "." | "..")
        && !repo.ends_with(".git")
        && secret_free(owner)
        && secret_free(repo)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLogSource {
    dir: String,
    glob: String,
    format: String,
}

impl WireLogSource {
    fn validate(self) -> Result<LogSource, DecodeError> {
        require(self.dir == LOG_DIR && self.format == LOG_FORMAT)?;
        LogSource::new(self.glob)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTemplate {
    id: String,
    target: String,
    text: String,
    fields: Vec<Object<WireTemplateField>>,
}

impl WireTemplate {
    fn validate(self) -> Result<Template, DecodeError> {
        require(is_lower_hex(&self.id, 12))?;
        let fields = self
            .fields
            .into_iter()
            .map(|Object(field)| field.validate())
            .collect::<Result<Vec<_>, _>>()?;
        validate_template(&self.target, &self.text, &fields)?;
        Ok(Template {
            id: self.id,
            target: self.target,
            text: self.text,
            fields,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTemplateField {
    name: String,
    #[serde(deserialize_with = "string_enum")]
    kind: EvidenceKind,
}

impl WireTemplateField {
    fn validate(self) -> Result<TemplateField, DecodeError> {
        let field = TemplateField::new(self.name)?;
        require(field.kind() == self.kind)?;
        Ok(field)
    }
}
