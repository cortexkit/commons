use serde::{ser::SerializeStruct, Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;
use std::collections::HashSet;

use crate::{
    secrets::secret_free,
    wire::{float, require, shape, string_enum, unsigned, Object},
    DecodeError,
};

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Value kinds shared by check/fix evidence and allow-listed log fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EvidenceKind {
    Count,
    Bytes,
    DurationMs,
    Ratio,
    Bool,
    Code,
    Version,
    TimestampMs,
    Path,
    Ref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PathCategory {
    Data,
    Config,
    Project,
    Temp,
    Other,
}

/// Typed input to [`Evidence::new`]. The constructor enforces the wire bounds.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum EvidenceValue {
    Count(u64),
    Bytes(u64),
    DurationMs(f64),
    Ratio(f64),
    Bool(bool),
    Code(String),
    Version(String),
    TimestampMs(u64),
    Path {
        category: PathCategory,
        path: String,
    },
    Ref(String),
}

impl EvidenceValue {
    pub fn kind(&self) -> EvidenceKind {
        match self {
            Self::Count(_) => EvidenceKind::Count,
            Self::Bytes(_) => EvidenceKind::Bytes,
            Self::DurationMs(_) => EvidenceKind::DurationMs,
            Self::Ratio(_) => EvidenceKind::Ratio,
            Self::Bool(_) => EvidenceKind::Bool,
            Self::Code(_) => EvidenceKind::Code,
            Self::Version(_) => EvidenceKind::Version,
            Self::TimestampMs(_) => EvidenceKind::TimestampMs,
            Self::Path { .. } => EvidenceKind::Path,
            Self::Ref(_) => EvidenceKind::Ref,
        }
    }

    fn validate(&self) -> Result<(), DecodeError> {
        require(match self {
            Self::Count(n) | Self::Bytes(n) | Self::TimestampMs(n) => *n <= MAX_SAFE_INTEGER,
            Self::DurationMs(n) => n.is_finite() && *n >= 0.0,
            Self::Ratio(n) => n.is_finite(),
            Self::Bool(_) => true,
            Self::Code(s) => valid_code(s),
            Self::Version(s) => {
                (1..=64).contains(&s.len())
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
            }
            Self::Path { path, .. } => (1..=4096).contains(&path.len()),
            Self::Ref(s) => (1..=256).contains(&s.len()),
        })
    }

    fn from_wire(kind: EvidenceKind, raw: &RawValue) -> Result<Self, DecodeError> {
        Ok(match kind {
            EvidenceKind::Count => Self::Count(unsigned(raw, MAX_SAFE_INTEGER)?),
            EvidenceKind::Bytes => Self::Bytes(unsigned(raw, MAX_SAFE_INTEGER)?),
            EvidenceKind::TimestampMs => Self::TimestampMs(unsigned(raw, MAX_SAFE_INTEGER)?),
            EvidenceKind::DurationMs => Self::DurationMs(float(raw)?),
            EvidenceKind::Ratio => Self::Ratio(float(raw)?),
            EvidenceKind::Bool => Self::Bool(shape(raw.get())?),
            EvidenceKind::Code => Self::Code(shape(raw.get())?),
            EvidenceKind::Version => Self::Version(shape(raw.get())?),
            EvidenceKind::Ref => Self::Ref(shape(raw.get())?),
            EvidenceKind::Path => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct PathValue {
                    #[serde(deserialize_with = "string_enum")]
                    category: PathCategory,
                    path: String,
                }
                let Object(PathValue { category, path }) = shape(raw.get())?;
                Self::Path { category, path }
            }
        })
    }

    pub(crate) fn from_log(kind: EvidenceKind, text: &str, category: PathCategory) -> Option<Self> {
        let value = match kind {
            EvidenceKind::Code => Self::Code(text.to_owned()),
            EvidenceKind::Version => Self::Version(text.to_owned()),
            EvidenceKind::Ref => Self::Ref(text.to_owned()),
            EvidenceKind::Path => Self::Path {
                category,
                path: text.to_owned(),
            },
            _ => {
                let raw: &RawValue = shape(text).ok()?;
                // A field holds one token, not a whitespace-framed document.
                if raw.get() != text {
                    return None;
                }
                Self::from_wire(kind, raw).ok()?
            }
        };
        value.validate().ok()?;
        Some(value)
    }
}

/// A named, validated finding or fix-result value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Evidence {
    name: String,
    value: EvidenceValue,
}

impl Evidence {
    pub fn new(name: impl Into<String>, value: EvidenceValue) -> Result<Self, DecodeError> {
        let name = name.into();
        require(valid_name(&name))?;
        value.validate()?;
        Ok(Self { name, value })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> EvidenceKind {
        self.value.kind()
    }

    pub fn value(&self) -> &EvidenceValue {
        &self.value
    }
}

impl Serialize for Evidence {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut evidence = serializer.serialize_struct("Evidence", 3)?;
        evidence.serialize_field("name", &self.name)?;
        evidence.serialize_field("kind", &self.kind())?;
        evidence.serialize_field("value", &self.value)?;
        evidence.end()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireEvidence {
    name: String,
    #[serde(deserialize_with = "string_enum")]
    kind: EvidenceKind,
    value: Box<RawValue>,
}

pub(crate) fn decode_evidence(
    raw: Vec<Object<WireEvidence>>,
) -> Result<Vec<Evidence>, DecodeError> {
    require(raw.len() <= 16)?;
    raw.into_iter()
        .map(|Object(raw)| Evidence::new(raw.name, EvidenceValue::from_wire(raw.kind, &raw.value)?))
        .collect()
}

pub(crate) fn validate_evidence(evidence: &[Evidence]) -> Result<(), DecodeError> {
    let mut names = HashSet::with_capacity(evidence.len().min(16));
    require(evidence.len() <= 16 && evidence.iter().all(|item| names.insert(item.name())))
}

pub(crate) fn valid_name(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && secret_free(s)
}

pub(crate) fn valid_code(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(is_id_byte)
}

pub(crate) fn is_id_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b)
}
