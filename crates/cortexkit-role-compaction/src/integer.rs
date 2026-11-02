//! Serde deserialization helpers for W1.2's ceiling on the contract's own
//! integer fields (0..2^53-1), so JavaScript readers never round them. They
//! check decoding only; constructors and serialization do not run them.

use serde::{de::Error, Deserialize, Deserializer};

const MAX: u64 = 9_007_199_254_740_991;

fn checked<E: Error>(value: u64) -> Result<u64, E> {
    if value > MAX {
        Err(E::custom("integer exceeds 9007199254740991"))
    } else {
        Ok(value)
    }
}

pub(crate) fn required<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    checked(u64::deserialize(deserializer)?)
}

pub(crate) fn optional<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(deserializer)?
        .map(checked)
        .transpose()
}

/// An optional read limit (W5.4): when present, at least 1 and within W1.2.
pub(crate) fn positive<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    let value = optional(deserializer)?;
    if value == Some(0) {
        return Err(D::Error::custom("limit must be at least 1"));
    }
    Ok(value)
}

pub(crate) fn list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u64>, D::Error> {
    #[derive(Deserialize)]
    struct Integer(#[serde(deserialize_with = "required")] u64);

    Vec::<Integer>::deserialize(deserializer)
        .map(|values| values.into_iter().map(|n| n.0).collect())
}
