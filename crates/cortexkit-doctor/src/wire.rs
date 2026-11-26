use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::{fmt, marker::PhantomData};

/// Maximum document size in bytes, including any trailing JSON whitespace.
pub const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// The first failing stage of doctor document validation, with no input text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DecodeError {
    TooLarge,
    NotJson,
    ExtraOutput,
    Shape,
    PlanDigest,
    TemplateId,
    TemplatesDigest,
}

impl DecodeError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::NotJson => "not_json",
            Self::ExtraOutput => "extra_output",
            Self::Shape => "shape",
            Self::PlanDigest => "plan_digest",
            Self::TemplateId => "template_id",
            Self::TemplatesDigest => "templates_digest",
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for DecodeError {}

/// Borrow the first JSON value without interpreting any object members or numbers.
pub(crate) fn frame(input: &[u8]) -> Result<&str, DecodeError> {
    if input.len() > MAX_OUTPUT_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let raw = <&RawValue>::deserialize(&mut deserializer).map_err(|_| DecodeError::NotJson)?;
    deserializer.end().map_err(|_| DecodeError::ExtraOutput)?;
    Ok(raw.get())
}

pub(crate) fn shape<'a, T: Deserialize<'a>>(raw: &'a str) -> Result<T, DecodeError> {
    serde_json::from_str(raw).map_err(|_| DecodeError::Shape)
}

// Derived struct deserializers also accept arrays. Wire structs must be objects.
pub(crate) struct Object<T>(pub T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> de::Visitor<'de> for ObjectVisitor<T> {
            type Value = Object<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }

            fn visit_map<M: de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                T::deserialize(de::value::MapAccessDeserializer::new(map)).map(Object)
            }
        }

        deserializer.deserialize_map(ObjectVisitor(PhantomData))
    }
}

pub(crate) fn present<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(de).map(Some)
}

// JSON's externally tagged enum format also accepts objects for unit variants.
pub(crate) fn string_enum<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(de::value::StringDeserializer::new(String::deserialize(de)?))
}

pub(crate) fn unsigned(raw: &RawValue, max: u64) -> Result<u64, DecodeError> {
    let token = raw.get();
    if !token.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DecodeError::Shape);
    }
    token
        .parse::<u64>()
        .ok()
        .filter(|&value| value <= max)
        .ok_or(DecodeError::Shape)
}

pub(crate) fn canonical_digest<T: Serialize + ?Sized>(data: &T) -> Result<String, DecodeError> {
    let bytes = serde_jcs::to_vec(data).map_err(|_| DecodeError::Shape)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub(crate) fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn require(ok: bool) -> Result<(), DecodeError> {
    ok.then_some(()).ok_or(DecodeError::Shape)
}
