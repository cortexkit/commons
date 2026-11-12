//! Checksums used across the contract: of a part, of an entry, and of a view.
//!
//! Every checksum is the lowercase hex SHA-256 of a value's RFC 8785 (JCS)
//! serialization, computed by llm-runner's shared `content_digest`, so this
//! contract and llm-runner agree byte for byte and no second implementation
//! can drift. Absent optional fields are left out of every contract type, so
//! they never enter a checksum. Floats occur (a tool's `input` is any JSON),
//! and JCS fixes their form.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use cortexkit_role_llm_runner::content_digest::content_digest;

/// A checksum as entries and parts carry it: on the wire, the lowercase hex
/// SHA-256 string; in memory, its 32 bytes.
///
/// Decoding stays as lenient as a plain string: a value that is not 64
/// lowercase hex digits is kept verbatim ([`Digest::is_canonical`] is then
/// false). Every form serializes back to exactly the string it was decoded
/// from. A digest never equals one of the other form, and digests order as
/// their wire strings do.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Digest(Form);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Form {
    /// 64 lowercase hex digits on the wire.
    Sha256([u8; 32]),
    /// Any other string, kept as received.
    Verbatim(Box<str>),
}

const HEX: &[u8; 16] = b"0123456789abcdef";

impl Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Form::Sha256(bytes))
    }

    /// The 32 bytes, when the digest is in the canonical form.
    pub fn as_bytes(&self) -> Option<&[u8; 32]> {
        match &self.0 {
            Form::Sha256(bytes) => Some(bytes),
            Form::Verbatim(_) => None,
        }
    }

    /// Whether the digest is 64 lowercase hex digits, held as 32 bytes,
    /// rather than a string kept verbatim. Cheap: no formatting.
    pub fn is_canonical(&self) -> bool {
        matches!(self.0, Form::Sha256(_))
    }

    /// Call `f` with the wire string, formatted on the stack: no allocation.
    pub fn with_str<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        match &self.0 {
            Form::Sha256(bytes) => {
                let mut hex = [0u8; 64];
                for (pair, byte) in hex.as_chunks_mut::<2>().0.iter_mut().zip(bytes) {
                    pair[0] = HEX[usize::from(byte >> 4)];
                    pair[1] = HEX[usize::from(byte & 0xf)];
                }
                f(std::str::from_utf8(&hex).expect("hex digits are ASCII"))
            }
            Form::Verbatim(text) => f(text),
        }
    }

    /// Write the wire string into `out` without allocating.
    pub fn write_to<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        self.with_str(|text| out.write_str(text))
    }

    fn sha256(text: &str) -> Option<[u8; 32]> {
        fn nibble(digit: u8) -> Option<u8> {
            match digit {
                b'0'..=b'9' => Some(digit - b'0'),
                b'a'..=b'f' => Some(digit - b'a' + 10),
                _ => None,
            }
        }
        let text = text.as_bytes();
        if text.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (byte, pair) in bytes.iter_mut().zip(text.as_chunks::<2>().0) {
            *byte = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Some(bytes)
    }
}

impl From<&str> for Digest {
    fn from(text: &str) -> Self {
        match Self::sha256(text) {
            Some(bytes) => Self(Form::Sha256(bytes)),
            None => Self(Form::Verbatim(text.into())),
        }
    }
}

impl From<String> for Digest {
    fn from(text: String) -> Self {
        match Self::sha256(&text) {
            Some(bytes) => Self(Form::Sha256(bytes)),
            None => Self(Form::Verbatim(text.into_boxed_str())),
        }
    }
}

impl From<&String> for Digest {
    fn from(text: &String) -> Self {
        Self::from(text.as_str())
    }
}

impl From<&Digest> for Digest {
    fn from(digest: &Digest) -> Self {
        digest.clone()
    }
}

impl From<Digest> for String {
    fn from(digest: Digest) -> Self {
        digest.to_string()
    }
}

impl PartialEq<str> for Digest {
    fn eq(&self, other: &str) -> bool {
        match &self.0 {
            // A hex digest differs from any other string in its wire form.
            Form::Sha256(bytes) => Self::sha256(other).as_ref() == Some(bytes),
            Form::Verbatim(text) => **text == *other,
        }
    }
}

impl PartialEq<&str> for Digest {
    fn eq(&self, other: &&str) -> bool {
        *self == **other
    }
}

impl PartialEq<String> for Digest {
    fn eq(&self, other: &String) -> bool {
        *self == **other
    }
}

impl PartialEq<Digest> for str {
    fn eq(&self, other: &Digest) -> bool {
        *other == *self
    }
}

impl PartialEq<Digest> for &str {
    fn eq(&self, other: &Digest) -> bool {
        *other == **self
    }
}

impl PartialEq<Digest> for String {
    fn eq(&self, other: &Digest) -> bool {
        *other == **self
    }
}

impl PartialOrd for Digest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Digest {
    /// As the wire strings order. Bytes order as their lowercase hex does,
    /// so two canonical digests never need formatting.
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (Form::Sha256(left), Form::Sha256(right)) => left.cmp(right),
            _ => self.with_str(|left| other.with_str(|right| left.cmp(right))),
        }
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with_str(|text| f.write_str(text))
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with_str(|text| f.debug_tuple("Digest").field(&text).finish())
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.with_str(|text| serializer.serialize_str(text))
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Digest;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string")
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Digest, E> {
                Ok(Digest::from(text))
            }

            fn visit_string<E: serde::de::Error>(self, text: String) -> Result<Digest, E> {
                Ok(Digest::from(text))
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

/// [`content_digest`] of `value`, as a [`Digest`].
pub(crate) fn digest_of(value: &serde_json::Value) -> Digest {
    Digest::from(content_digest(value))
}

/// SHA-256 of the value's whole serialized form, through the shared
/// `content_digest`. It picks no fields: use [`crate::part::body_digest`] for
/// a part, [`crate::read::entry_digest`] for an entry and
/// [`crate::view::View::seal`] for a view.
pub fn checksum<T: Serialize>(value: &T) -> Digest {
    let value = serde_json::to_value(value).expect("contract types always serialize");
    digest_of(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn same_value_same_checksum_whatever_the_key_order() {
        let left: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let right: Value = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        assert_eq!(checksum(&left), checksum(&right));
    }

    #[test]
    fn a_digest_keeps_its_wire_string_and_holds_hex_as_bytes() {
        let hex = checksum(&json!({"a": 1})).to_string();
        let digest = Digest::from(hex.as_str());
        assert!(digest.is_canonical());
        assert_eq!(digest.to_string(), hex);
        assert_eq!(digest, hex);
        assert_eq!(
            serde_json::to_string(&digest).unwrap(),
            serde_json::to_string(&hex).unwrap()
        );
        assert_eq!(
            serde_json::from_str::<Digest>(&format!("{hex:?}")).unwrap(),
            digest
        );

        // Anything else is kept as received, uppercase hex included, and is
        // never equal to a canonical digest.
        for other in ["invented-checksum", "", &hex[..63], &hex.to_uppercase()] {
            let kept = Digest::from(other);
            assert!(!kept.is_canonical());
            assert_eq!(kept.to_string(), other);
            assert_eq!(kept, other);
            assert_ne!(kept, digest);
            let wire = serde_json::to_value(&kept).unwrap();
            assert_eq!(wire, json!(other));
            assert_eq!(serde_json::from_value::<Digest>(wire).unwrap(), kept);
        }
    }

    #[test]
    fn digests_order_as_their_wire_strings() {
        let mut strings = vec![
            "0".repeat(64),
            "f".repeat(64),
            format!("{}1", "a".repeat(63)),
            format!("{}0", "a".repeat(63)),
            "a".to_owned(),
            "b".to_owned(),
            "A".repeat(64),
            "g".repeat(64),
            format!("{}0", "a".repeat(62)),
            String::new(),
        ];
        let mut digests: Vec<Digest> = strings.iter().map(Digest::from).collect();
        strings.sort();
        digests.sort();
        let ordered: Vec<String> = digests.iter().map(Digest::to_string).collect();
        assert_eq!(ordered, strings);
    }

    #[test]
    fn floats_hash_in_their_jcs_form() {
        // JCS writes 1e21 and 1.0e21 the same way; a hand-written form that
        // kept serde_json's text would not.
        let written: Value = serde_json::from_str(r#"{"x":1e21}"#).unwrap();
        let other: Value = serde_json::from_str(r#"{"x":1.0e21}"#).unwrap();
        assert_eq!(checksum(&written), checksum(&other));
        assert_ne!(checksum(&json!({"x": 1.5})), checksum(&json!({"x": 1.25})));
    }
}
