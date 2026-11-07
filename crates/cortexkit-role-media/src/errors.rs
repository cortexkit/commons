use std::collections::BTreeMap;

use crate::encoded_len;

pub type CheckResult = Result<(), CheckError>;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CheckError {
    pub code: String,
    pub field: Option<String>,
}

impl CheckError {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            field: None,
        }
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code)?;
        if let Some(field) = &self.field {
            write!(f, ": {field}")?;
        }
        Ok(())
    }
}
impl std::error::Error for CheckError {}

open_enum!(RefusalClass { Transient => "transient", Permanent => "permanent" });

impl RefusalClass {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient)
    }
}

wire_struct!(Failure { code: String, class: RefusalClass, message: String } optional {
    retry_after_ms: u64 => with_retry_after_ms,
    detail: BTreeMap<String, String> => with_detail,
});

/// The ERROR-frame detail keeps classification separate from code-specific facts.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RefusalDetail {
    pub class: RefusalClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(flatten)]
    pub members: BTreeMap<String, serde_json::Value>,
}
impl RefusalDetail {
    pub fn new(class: RefusalClass) -> Self {
        Self {
            class,
            retry_after_ms: None,
            members: BTreeMap::new(),
        }
    }
    pub fn with_retry_after_ms(mut self, value: u64) -> Self {
        self.retry_after_ms = Some(value);
        self
    }
    pub fn with_members(mut self, value: BTreeMap<String, serde_json::Value>) -> Self {
        self.members = value;
        self
    }
}

wire_struct!(Refusal {
    code: String,
    message: String,
    detail: RefusalDetail
});

pub fn check_failure(failure: &Failure) -> CheckResult {
    if failure.message.chars().count() > 1_024 {
        return Err(CheckError::new("invalid_failure").with_field("message"));
    }
    if let Some(detail) = &failure.detail {
        for (key, value) in detail {
            if matches!(key.as_str(), "class" | "retry_after_ms")
                || key.chars().count() > 256
                || value.chars().count() > 256
            {
                return Err(CheckError::new("invalid_failure").with_field(format!("detail.{key}")));
            }
        }
    }
    if failure.code == "routing_refused"
        && failure
            .detail
            .as_ref()
            .and_then(|d| d.get("routing_code"))
            .is_none()
    {
        return Err(CheckError::new("invalid_failure").with_field("detail.routing_code"));
    }
    // Serialization is also checked here so callers do not forward invalid JSON values.
    encoded_len(failure)?;
    Ok(())
}

pub fn truncate_failure_message(message: &str) -> String {
    message.chars().take(1_024).collect()
}

pub fn routing_refusal_class(code: &str) -> RefusalClass {
    if matches!(
        code,
        "account_hold" | "account_pool_exhausted" | "account_selection_unavailable"
    ) {
        RefusalClass::Transient
    } else {
        RefusalClass::Permanent
    }
}
