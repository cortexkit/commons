//! Typed wire contract and caller-side checks for `media-provider/v1`.
//!
//! See `CONTRACT.md` for provider obligations. Checks validate wire values, not
//! authentication, persistence or the execution of an engine.

macro_rules! wire_struct {
    ($(#[$meta:meta])* $name:ident { $($field:ident: $ty:ty),* $(,)? } $(optional { $($opt:ident: $oty:ty => $with:ident),* $(,)? })?) => {
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        #[non_exhaustive]
        $(#[$meta])*
        pub struct $name {
            $(pub $field: $ty,)*
            $($(#[serde(default, skip_serializing_if = "Option::is_none")]
                pub $opt: Option<$oty>,)*)?
        }
        impl $name {
            pub fn new($($field: $ty),*) -> Self {
                Self { $($field,)* $($($opt: None,)*)? }
            }
            $($(pub fn $with(mut self, value: $oty) -> Self {
                self.$opt = Some(value);
                self
            })*)?
        }
    };
}

macro_rules! open_enum {
    ($name:ident { $($variant:ident => $value:literal),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum $name { $($variant,)* Unknown(String) }
        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                let value = value.into();
                match value.as_str() { $($value => Self::$variant,)* _ => Self::Unknown(value) }
            }
            pub fn as_str(&self) -> &str {
                match self { $(Self::$variant => $value,)* Self::Unknown(value) => value }
            }
        }
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                <String as serde::Deserialize>::deserialize(deserializer).map(Self::new)
            }
        }
    };
}

pub mod bias;
pub mod describe;
pub mod errors;
pub mod handoff;
pub mod jobs;
pub mod objects;
pub mod realtime;
pub mod streams;
pub mod usage;
#[cfg(any(test, feature = "test-vectors"))]
pub mod vectors;
pub mod wire;

pub use errors::{CheckError, CheckResult};
pub use handoff::{hand_off_message, hand_off_send_id};

pub const ROLE: &str = "media-provider/v1";
pub const CHUNK_MAX: u64 = 2_097_152;
pub const MESSAGE_MAX: usize = 3_145_728;
pub const USAGE_RECORD_MAX: usize = 262_144;

pub(crate) fn encoded_len(value: &impl serde::Serialize) -> Result<usize, CheckError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| CheckError::new("invalid_params").with_field("params"))
}

pub fn check_caller_id(value: &str, field: &str) -> CheckResult {
    if !(1..=128).contains(&value.chars().count()) {
        return Err(CheckError::new("invalid_params").with_field(field));
    }
    Ok(())
}
