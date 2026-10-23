//! Reading one request member at a time, so a validator can refuse the
//! exact member that is wrong instead of a whole-body decode error.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::errors::Refusal;

/// A request's members, with the refusal a bad member gets.
pub(crate) struct Members<'a> {
    pub(crate) object: &'a Map<String, Value>,
    /// Builds the refusal for a bad member, given its name and a reason.
    pub(crate) refuse: fn(String, String) -> Refusal,
}

impl<'a> Members<'a> {
    pub(crate) fn of(params: &'a Value) -> Result<Self, Refusal> {
        match params {
            Value::Object(object) => Ok(Self {
                object,
                refuse: |field, message| Refusal::invalid_request(field, message),
            }),
            _ => Err(Refusal::invalid_request(
                "params",
                "the request params must be an object",
            )),
        }
    }

    pub(crate) fn refuse(&self, field: &str, message: impl Into<String>) -> Refusal {
        (self.refuse)(field.to_owned(), message.into())
    }

    /// The member, decoded; `None` when it is absent or `null`.
    pub(crate) fn optional<T: DeserializeOwned>(&self, field: &str) -> Result<Option<T>, Refusal> {
        match self.object.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => serde_json::from_value(value.clone())
                .map(Some)
                .map_err(|e| self.refuse(field, format!("{field} is invalid: {e}"))),
        }
    }

    /// The member, decoded; refused when it is absent or `null`.
    pub(crate) fn required<T: DeserializeOwned>(&self, field: &str) -> Result<T, Refusal> {
        self.optional(field)?
            .ok_or_else(|| self.refuse(field, format!("{field} is required")))
    }

    /// A required string that must not be empty and may be at most `max`
    /// characters long.
    pub(crate) fn text(&self, field: &str, max: Option<usize>) -> Result<String, Refusal> {
        let text: String = self.required(field)?;
        self.check_text(field, &text, max)?;
        Ok(text)
    }

    /// An optional string that, when present, must not be empty and may be
    /// at most `max` characters long.
    pub(crate) fn optional_text(
        &self,
        field: &str,
        max: Option<usize>,
    ) -> Result<Option<String>, Refusal> {
        let text: Option<String> = self.optional(field)?;
        if let Some(text) = &text {
            self.check_text(field, text, max)?;
        }
        Ok(text)
    }

    fn check_text(&self, field: &str, text: &str, max: Option<usize>) -> Result<(), Refusal> {
        if text.is_empty() {
            return Err(self.refuse(field, format!("{field} must not be empty")));
        }
        if let Some(max) = max {
            if text.chars().count() > max {
                return Err(self.refuse(field, format!("{field} is longer than {max} characters")));
            }
        }
        Ok(())
    }
}
