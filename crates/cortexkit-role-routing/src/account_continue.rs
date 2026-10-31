//! Opt-in `route.account_for` continuation of a run paused on an account limit.
//! The legacy account request and reply remain unchanged; dispatch this mode
//! only after checking `role.describe.capabilities`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    account::{ADMISSION_ID_MAX_CHARS, CONVERSATION_KEY_MAX_CHARS},
    capabilities,
    describe::RoleDescribe,
    errors::Refusal,
    members::Members,
};

/// The exhausted public account and its original limit evidence. Field types
/// match `observe::AccountObserveRequest`: public ids and windows are strings,
/// and observation/reset times are unsigned Unix milliseconds. `retry_after_s`
/// is an optional unsigned duration in seconds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExhaustedAccount {
    pub credential_id: String,
    /// The public account id (`served_account_id` in an observation).
    pub account: String,
    /// Only `five_hour` or `seven_day` is valid in a continue request.
    pub limit_window: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_s: Option<u64>,
    pub observed_at_ms: u64,
}

impl ExhaustedAccount {
    pub fn new(
        credential_id: impl Into<String>,
        account: impl Into<String>,
        limit_window: impl Into<String>,
        observed_at_ms: u64,
    ) -> Self {
        Self {
            credential_id: credential_id.into(),
            account: account.into(),
            limit_window: limit_window.into(),
            pool: None,
            resets_at_ms: None,
            retry_after_s: None,
            observed_at_ms,
        }
    }

    pub fn with_pool(mut self, pool: impl Into<String>) -> Self {
        self.pool = Some(pool.into());
        self
    }

    pub fn with_resets_at_ms(mut self, resets_at_ms: u64) -> Self {
        self.resets_at_ms = Some(resets_at_ms);
        self
    }

    pub fn with_retry_after_s(mut self, retry_after_s: u64) -> Self {
        self.retry_after_s = Some(retry_after_s);
        self
    }
}

/// A fresh `selection_id` asks for a fresh eligibility check. Transport retries
/// keep the id; asking again after a hold uses a new one.
/// Provider, auth method, model and owner scope come only from the previous
/// decision, never from new caller-supplied selection hints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContinueRequest {
    pub mode: String,
    pub run_id: String,
    pub admission_id: String,
    pub conversation_key: String,
    pub previous_decision_id: String,
    pub exhausted: ExhaustedAccount,
    pub selection_id: String,
}

impl ContinueRequest {
    pub fn new(
        run_id: impl Into<String>,
        admission_id: impl Into<String>,
        conversation_key: impl Into<String>,
        previous_decision_id: impl Into<String>,
        exhausted: ExhaustedAccount,
        selection_id: impl Into<String>,
    ) -> Self {
        Self {
            mode: "continue".into(),
            run_id: run_id.into(),
            admission_id: admission_id.into(),
            conversation_key: conversation_key.into(),
            previous_decision_id: previous_decision_id.into(),
            exhausted,
            selection_id: selection_id.into(),
        }
    }

    /// Whether two parsed requests ask the same question. A router checks this
    /// before replaying the answer cached under a runner's `selection_id`;
    /// a changed question is `invalid_request` naming `selection_id` and must
    /// not replace the first answer.
    pub fn same_question(&self, other: &Self) -> bool {
        self.run_id == other.run_id
            && self.admission_id == other.admission_id
            && self.conversation_key == other.conversation_key
            && self.previous_decision_id == other.previous_decision_id
            && self.exhausted == other.exhausted
    }
}

/// One reservation, linked to the previous decision for pricing and task records.
/// The first `route.report` under the offered decision marks it used; otherwise
/// the router releases it at `valid_until_ms`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContinueOffer {
    pub credential_id: String,
    pub account: String,
    pub decision_id: String,
    pub valid_until_ms: u64,
}

impl ContinueOffer {
    pub fn new(
        credential_id: impl Into<String>,
        account: impl Into<String>,
        decision_id: impl Into<String>,
        valid_until_ms: u64,
    ) -> Self {
        Self {
            credential_id: credential_id.into(),
            account: account.into(),
            decision_id: decision_id.into(),
            valid_until_ms,
        }
    }
}

/// No compatible account right now. An absent wake time is unknown, not now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContinueHold {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_ms: Option<u64>,
    pub routing_code: String,
    pub detail: Value,
}

impl ContinueHold {
    pub fn new(routing_code: impl Into<String>, detail: Value) -> Self {
        Self {
            until_ms: None,
            routing_code: routing_code.into(),
            detail,
        }
    }

    pub fn with_until_ms(mut self, until_ms: u64) -> Self {
        self.until_ms = Some(until_ms);
        self
    }
}

/// A terminal refusal to switch for this pause, not a transport failure. A
/// later pause of the same run may ask again with a new selection id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContinueRefusal {
    pub routing_code: String,
    pub detail: Value,
}

impl ContinueRefusal {
    pub fn new(routing_code: impl Into<String>, detail: Value) -> Self {
        Self {
            routing_code: routing_code.into(),
            detail,
        }
    }
}

/// Exactly one continuation outcome, tagged on the wire by `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContinueAnswer {
    Offer(ContinueOffer),
    Hold(ContinueHold),
    Refuse(ContinueRefusal),
}

impl ContinueAnswer {
    /// Only a hold with a known wake time is timed. Never substitute zero or
    /// the current time for an unknown reset.
    pub fn wait_until_ms(&self) -> Option<u64> {
        match self {
            Self::Hold(hold) => hold.until_ms,
            _ => None,
        }
    }
}

/// Decode the opt-in request. A non-declaring router refuses `mode` exactly
/// as the legacy parser refuses an unknown mode. Authorization and binding to
/// the runner's durable pause and currently used decision are the router's checks.
/// This parser does not refuse members it does not read. As with the legacy
/// parsers, refusing unread members under contract §2 is the router's own check.
pub fn parse_continue_request(
    params: &Value,
    describe: &RoleDescribe,
) -> Result<ContinueRequest, Refusal> {
    let members = Members::of(params)?;
    if !describe.serves(capabilities::ACCOUNT_CONTINUE) {
        return Err(members.refuse("mode", "continue is not a declared mode"));
    }
    let mode = members.text("mode", None)?;
    if mode != "continue" {
        return Err(members.refuse("mode", "expected continue"));
    }
    let exhausted_value: Value = members.required("exhausted")?;
    let exhausted = parse_exhausted(&exhausted_value).map_err(|mut refusal| {
        if let Some(field) = refusal.detail.get_mut("field") {
            *field = Value::String(format!("exhausted.{}", field.as_str().unwrap_or("params")));
        }
        refusal
    })?;
    Ok(ContinueRequest {
        mode,
        run_id: members.text("run_id", None)?,
        admission_id: members.text("admission_id", Some(ADMISSION_ID_MAX_CHARS))?,
        conversation_key: members.text("conversation_key", Some(CONVERSATION_KEY_MAX_CHARS))?,
        previous_decision_id: members.text("previous_decision_id", None)?,
        exhausted,
        selection_id: members.text("selection_id", Some(ADMISSION_ID_MAX_CHARS))?,
    })
}

fn parse_exhausted(value: &Value) -> Result<ExhaustedAccount, Refusal> {
    let members = Members::of(value)?;
    let limit_window = members.text("limit_window", None)?;
    if !matches!(limit_window.as_str(), "five_hour" | "seven_day") {
        return Err(members.refuse("limit_window", "expected five_hour or seven_day"));
    }
    Ok(ExhaustedAccount {
        credential_id: members.text("credential_id", None)?,
        account: members.text("account", None)?,
        limit_window,
        pool: members.pool()?,
        resets_at_ms: members.optional("resets_at_ms")?,
        retry_after_s: members.optional("retry_after_s")?,
        observed_at_ms: members.required("observed_at_ms")?,
    })
}
