use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use sha2::{Digest, Sha256};

pub type Headers = BTreeMap<String, String>;
pub type MessageId = String;

/// A SHA-256 content digest in its canonical lowercase hexadecimal wire form.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for ContentDigest {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64 {
            return Err("SHA-256 digest must contain exactly 64 hexadecimal characters");
        }
        if value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
        {
            return Err("SHA-256 digest must use lowercase hexadecimal");
        }

        let mut digest = [0_u8; 32];
        for (index, byte) in digest.iter_mut().enumerate() {
            let offset = index * 2;
            let pair = &value[offset..offset + 2];
            *byte = u8::from_str_radix(pair, 16).expect("hexadecimal was checked above");
        }
        Ok(Self(digest))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub subject: String,
    pub id: MessageId,
    pub digest: ContentDigest,
    pub headers: Headers,
}

impl Message {
    pub fn wire_size(&self) -> usize {
        self.subject.len()
            + self.id.len()
            + 64
            + self
                .headers
                .iter()
                .map(|(name, value)| name.len() + value.len())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_wire_form_is_lowercase_sha256() {
        let digest = ContentDigest::of_bytes(b"record");
        let encoded = digest.to_string();

        assert_eq!(encoded.len(), 64);
        assert_eq!(encoded.parse::<ContentDigest>(), Ok(digest));
        assert!(encoded.to_uppercase().parse::<ContentDigest>().is_err());
    }
}
