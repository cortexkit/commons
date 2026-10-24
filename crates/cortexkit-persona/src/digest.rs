//! Newline normalisation and the three digests of SPEC.md section 6.

use std::{borrow::Cow, collections::BTreeMap, fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

/// A SHA-256 digest, written `sha256:` followed by 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// SHA-256 of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Digest(Sha256::digest(bytes).into())
    }

    /// The raw 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The 64 lowercase hex digits, without the `sha256:` prefix.
    pub fn hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for b in self.0 {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
        out
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.hex())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The text is not `sha256:` followed by 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a digest is `sha256:` followed by 64 lowercase hex digits")]
pub struct DigestParseError;

impl FromStr for Digest {
    type Err = DigestParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.strip_prefix("sha256:").ok_or(DigestParseError)?;
        if hex.len() != 64 {
            return Err(DigestParseError);
        }
        let nibble = |c: u8| match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            _ => Err(DigestParseError),
        };
        let mut out = [0u8; 32];
        for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
            out[i] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Ok(Digest(out))
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Rewrite CRLF and lone CR as LF. Every source file is normalised on load and
/// the rendered output before hashing, so a Windows checkout that converted
/// line endings gives the same bytes and digests as macOS and Linux.
pub fn normalize_newlines(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// The layer digest: SHA-256 over the layer's normalised files in byte order
/// of their logical paths, each framed as
/// `path NUL decimal-length NUL content` so no two file sets share an input.
pub(crate) fn layer_digest(files: &BTreeMap<String, String>) -> Digest {
    let mut hasher = Sha256::new();
    for (path, content) in files {
        hasher.update(path.as_bytes());
        hasher.update([0u8]);
        hasher.update(content.len().to_string().as_bytes());
        hasher.update([0u8]);
        hasher.update(content.as_bytes());
    }
    Digest(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_lone_cr_become_lf() {
        assert_eq!(normalize_newlines("a\r\nb\rc\n\r\n"), "a\nb\nc\n\n");
        assert!(matches!(normalize_newlines("plain\n"), Cow::Borrowed(_)));
    }

    #[test]
    fn digest_round_trips_through_text() {
        let d = Digest::of(b"abc");
        assert_eq!(
            d.to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(d.to_string().parse::<Digest>().unwrap(), d);
        assert!("sha256:BA78".parse::<Digest>().is_err());
    }

    #[test]
    fn layer_framing_separates_path_and_content() {
        let a = BTreeMap::from([("ab".to_string(), "c".to_string())]);
        let b = BTreeMap::from([("a".to_string(), "bc".to_string())]);
        assert_ne!(layer_digest(&a), layer_digest(&b));
    }
}
