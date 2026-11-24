use serde::{Deserialize, Serialize};

/// A different model served the step after a refusal. The enclosing step's
/// counters include both refused attempts and the answer.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Fallback {
    /// `client` for a host retry, or `provider` for server-side fallback.
    pub by: String,
    pub requested_model: String,
    /// The provider's category id, never refusal text.
    pub category: String,
    /// `refused_here` for this step's refusal, or `round` for continued recovery.
    pub cause: String,
    /// Whether the client retry presented the provider's fallback credit token.
    /// This records only presence, never the token itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credited: Option<bool>,
    /// Required for `refused_here`, in attempt order. Absent for `round`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused_attempts: Option<Vec<RefusedAttempt>>,
    /// Recovery round in force after this step, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<FallbackRound>,
}

impl Fallback {
    /// Set the required members. Set `refused_attempts` for `refused_here`, then
    /// validate the containing step with [`super::StepFinished::check`].
    pub fn new(
        by: impl Into<String>,
        requested_model: impl Into<String>,
        category: impl Into<String>,
        cause: impl Into<String>,
    ) -> Self {
        Self {
            by: by.into(),
            requested_model: requested_model.into(),
            category: category.into(),
            cause: cause.into(),
            credited: None,
            refused_attempts: None,
            round: None,
        }
    }

    pub fn with_credited(mut self, credited: bool) -> Self {
        self.credited = Some(credited);
        self
    }

    pub fn with_refused_attempts(mut self, refused_attempts: Vec<RefusedAttempt>) -> Self {
        self.refused_attempts = Some(refused_attempts);
        self
    }

    pub fn with_round(mut self, round: FallbackRound) -> Self {
        self.round = Some(round);
        self
    }

    pub(super) fn check(&self, served_model: &str) -> Result<(), &'static str> {
        if !valid_by(&self.by) {
            return Err("fallback.by");
        }
        if self.requested_model.is_empty() || self.requested_model == served_model {
            return Err("fallback.requested_model");
        }
        if !valid_category(&self.category) {
            return Err("fallback.category");
        }
        if !valid_cause(&self.cause) {
            return Err("fallback.cause");
        }
        if self.credited.is_some() && self.by != "client" {
            return Err("fallback.credited");
        }
        match (self.cause.as_str(), &self.refused_attempts) {
            ("refused_here", Some(attempts)) if (1..=8).contains(&attempts.len()) => {
                if attempts.iter().any(|attempt| attempt.model.is_empty()) {
                    return Err("fallback.refused_attempts[].model");
                }
            }
            ("round", None) => {}
            _ => return Err("fallback.refused_attempts"),
        }
        if self
            .round
            .as_ref()
            .is_some_and(|round| round.steps_left > round.steps)
        {
            return Err("fallback.round.steps_left");
        }
        Ok(())
    }
}

pub(crate) fn valid_by(value: &str) -> bool {
    matches!(value, "client" | "provider")
}

pub(crate) fn valid_category(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

pub(crate) fn valid_cause(value: &str) -> bool {
    matches!(value, "refused_here" | "round")
}

/// Token counters for one refused attempt, using the enclosing step's units.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RefusedAttempt {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_5m_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

impl RefusedAttempt {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            input_tokens: None,
            cached_input_tokens: None,
            cache_write_tokens: None,
            cache_write_5m_tokens: None,
            cache_write_1h_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
        }
    }

    pub fn with_input_tokens(mut self, value: u64) -> Self {
        self.input_tokens = Some(value);
        self
    }

    pub fn with_cached_input_tokens(mut self, value: u64) -> Self {
        self.cached_input_tokens = Some(value);
        self
    }

    pub fn with_cache_write_tokens(mut self, value: u64) -> Self {
        self.cache_write_tokens = Some(value);
        self
    }

    pub fn with_cache_write_5m_tokens(mut self, value: u64) -> Self {
        self.cache_write_5m_tokens = Some(value);
        self
    }

    pub fn with_cache_write_1h_tokens(mut self, value: u64) -> Self {
        self.cache_write_1h_tokens = Some(value);
        self
    }

    pub fn with_output_tokens(mut self, value: u64) -> Self {
        self.output_tokens = Some(value);
        self
    }

    pub fn with_reasoning_tokens(mut self, value: u64) -> Self {
        self.reasoning_tokens = Some(value);
        self
    }
}

/// The number of steps in a recovery round and the number remaining.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct FallbackRound {
    pub steps: u32,
    pub steps_left: u32,
}

impl FallbackRound {
    pub fn new(steps: u32, steps_left: u32) -> Self {
        Self { steps, steps_left }
    }
}
