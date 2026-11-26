//! Closed request params and their structural bounds. The implementer checks
//! authority and item lifecycle using the authenticated route and stored state.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::errors::{Code, Refusal};
use crate::object::Object;

// Use the same fields for serialization and a map-only, closed reader.
macro_rules! closed_params {
    ($(#[$model_attr:meta])* $name:ident { $($(#[$field_attr:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        $(#[$model_attr])*
        pub struct $name {
            $($(#[$field_attr])* pub $field: $ty,)*
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Wire {
                    $($(#[$field_attr])* $field: $ty,)*
                }

                let Wire { $($field,)* } = Wire::deserialize(Object(deserializer))?;
                Ok(Self { $($field,)* })
            }
        }
    };
}

fn invalid(field: &str, reason: &str) -> Refusal {
    Refusal::new(Code::InvalidParams, format!("{field}: {reason}"))
}

fn validate_instance(instance: &str) -> Result<(), Refusal> {
    if !(1..=64).contains(&instance.chars().count())
        || !instance
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(invalid(
            "instance",
            "must be 1 to 64 ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

fn validate_key(field: &str, key: &str) -> Result<(), Refusal> {
    if !(1..=128).contains(&key.chars().count()) || !key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(invalid(
            field,
            "must be 1 to 128 printable ASCII characters without spaces",
        ));
    }
    Ok(())
}

fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

closed_params! {
    OpenParams {
        instance: String,
        key: String,
        label: String,
    }
}

impl OpenParams {
    pub fn validate(&self) -> Result<(), Refusal> {
        validate_instance(&self.instance)?;
        validate_key("key", &self.key)?;
        if !(1..=200).contains(&self.label.chars().count())
            || self
                .label
                .chars()
                .any(|c| c.is_control() || is_line_break(c))
        {
            return Err(invalid(
                "label",
                "must be 1 to 200 characters on one line without control characters",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Done,
    Failed,
}

closed_params! {
    FinishParams {
        instance: String,
        key: String,
        outcome: Outcome,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        more: Option<String>,
    }
}

impl FinishParams {
    pub fn validate(&self) -> Result<(), Refusal> {
        validate_instance(&self.instance)?;
        validate_key("key", &self.key)?;
        if self.summary.chars().count() > 2000 {
            return Err(invalid("summary", "must be at most 2000 characters"));
        }
        if self.more.as_ref().is_some_and(|more| {
            !(1..=200).contains(&more.chars().count()) || more.chars().any(is_line_break)
        }) {
            return Err(invalid("more", "must be 1 to 200 characters on one line"));
        }
        Ok(())
    }
}

closed_params! {
    ProgressParams {
        instance: String,
        key: String,
        text: String,
    }
}

impl ProgressParams {
    pub fn validate(&self) -> Result<(), Refusal> {
        validate_instance(&self.instance)?;
        validate_key("key", &self.key)?;
        if !(1..=2000).contains(&self.text.chars().count()) {
            return Err(invalid("text", "must be 1 to 2000 characters"));
        }
        Ok(())
    }
}

closed_params! {
    CancelParams {
        instance: String,
        key: String,
    }
}

impl CancelParams {
    pub fn validate(&self) -> Result<(), Refusal> {
        validate_instance(&self.instance)?;
        validate_key("key", &self.key)
    }
}

closed_params! {
    SyncParams {
        instance: String,
        open: Vec<String>,
    }
}

impl SyncParams {
    pub fn validate(&self) -> Result<(), Refusal> {
        validate_instance(&self.instance)?;
        if self.open.len() > 4096 {
            return Err(invalid("open", "must have at most 4096 keys"));
        }
        let mut seen = HashSet::with_capacity(self.open.len());
        for key in &self.open {
            validate_key("open", key)?;
            if !seen.insert(key) {
                return Err(invalid("open", "must not contain duplicate keys"));
            }
        }
        Ok(())
    }
}

closed_params! {
    #[derive(Default)]
    ListParams {}
}

impl ListParams {
    pub fn validate(&self) -> Result<(), Refusal> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_decoded_on_their_own_are_closed_objects() {
        assert!(serde_json::from_str::<ListParams>("{}").is_ok());
        assert!(serde_json::from_str::<ListParams>("[]").is_err());
        assert!(serde_json::from_str::<ListParams>(r#"{"extra":1}"#).is_err());
        assert!(serde_json::from_str::<CancelParams>(r#"["demo-1","work-1"]"#).is_err());
    }

    #[test]
    fn open_label_upper_bound_counts_unicode_scalars() {
        let mut params = OpenParams {
            instance: "demo-1".into(),
            key: "work-1".into(),
            label: "é".repeat(200),
        };
        params.validate().expect("200 scalars are allowed");
        params.label.push('é');
        let refusal = params.validate().expect_err("201 scalars must be refused");
        assert_eq!(refusal.code, Code::InvalidParams);
        assert!(refusal.message.starts_with("label:"));
    }
}
