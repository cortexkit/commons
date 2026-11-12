//! Validation shared by Setup and Conditions. Decoding keeps missing epochs so
//! providers can request Setup; encoding must never produce a stamp alone.

use serde::{de::Error, Deserialize, Deserializer};

pub(crate) fn validate_epoch(epoch: &str) -> Result<(), String> {
    if (1..=64).contains(&epoch.len()) && epoch.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        Ok(())
    } else {
        Err("statement_epoch must contain 1 to 64 visible ASCII characters (0x21..0x7e)".into())
    }
}

pub(crate) fn optional_epoch<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let epoch = Option::<String>::deserialize(deserializer)?;
    if let Some(epoch) = &epoch {
        validate_epoch(epoch).map_err(D::Error::custom)?;
    }
    Ok(epoch)
}

pub(crate) fn validate_encoding<E: serde::ser::Error>(
    epoch: Option<&str>,
    seq: Option<u64>,
) -> Result<(), E> {
    if let Some(epoch) = epoch {
        validate_epoch(epoch).map_err(E::custom)?;
    } else if seq.is_some() {
        return Err(E::custom("statement_seq requires statement_epoch"));
    }
    Ok(())
}

// Validate before writing and preserve declaration order. The exhaustive
// destructuring makes an omitted field a compile error when either type grows.
macro_rules! serialize_statement {
    ($value:ident, $serializer:ident, $name:literal,
     [$($kind:ident $field:ident),* $(,)?]) => {{
        use serde::ser::SerializeStruct;
        $crate::statement_order::validate_encoding(
            $value.statement_epoch.as_deref(), $value.statement_seq,
        )?;
        let Self { $($field),* } = $value;
        let len = 0usize $(+ $crate::statement_order::serialize_statement!(@len $kind $field))*;
        let mut state = $serializer.serialize_struct($name, len)?;
        $($crate::statement_order::serialize_statement!(@field state, $kind $field);)*
        state.end()
    }};
    (@len required $field:ident) => { 1usize };
    (@len optional $field:ident) => { usize::from($field.is_some()) };
    (@field $state:ident, required $field:ident) => {
        $state.serialize_field(stringify!($field), $field)?;
    };
    (@field $state:ident, optional $field:ident) => {
        if let Some(value) = $field {
            $state.serialize_field(stringify!($field), value)?;
        }
    };
}

pub(crate) use serialize_statement;
