//! `route.account_for`: which account a runner uses for a model call on a
//! provider whose accounts the routing provider allocates.
//!
//! Only runner modules call it. The caller comes from the route, never the
//! body. A request is idempotent on `admission_id`: the same id and body
//! returns the first answer, and a different body is `admission_conflict`.
//! Bodies are compared after [`AccountForRequest::normalized`], so a member
//! left out and the same member sent with its default value are the same
//! body.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{errors::Refusal, members::Members};

/// The longest `conversation_key`, in characters.
pub const CONVERSATION_KEY_MAX_CHARS: usize = 256;
/// The longest `admission_id`, in characters.
pub const ADMISSION_ID_MAX_CHARS: usize = 128;
/// The default `hold_wait_max_s`.
pub const DEFAULT_HOLD_WAIT_MAX_S: u64 = 900;

open_set! {
    /// What the runner asks for.
    pub enum AccountMode {
        /// A first admission for the conversation (the default).
        Admit => "admit",
        /// Extend the current pin.
        Renew => "renew",
        /// A different account after the current one failed.
        Reassign => "reassign",
    }
}

open_set! {
    /// Why the provider chose the account, for logs. A reader never acts on
    /// it.
    pub enum AccountReasonCode {
        /// The conversation's account affinity kept its account.
        Affinity => "affinity",
        /// The request's `decision_id` decided it.
        Decision => "decision",
        /// The request's `requested_account` was honoured.
        RequestedAccount => "requested_account",
        /// The account with the most headroom.
        Headroom => "headroom",
        /// The current pin was renewed.
        Renewed => "renewed",
        /// A different account replaced one that failed.
        Reassigned => "reassigned",
    }
}

/// The pin a `renew` or `reassign` expects to replace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedPin {
    pub id: String,
    pub generation: u64,
}

/// One account the runner can use now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateAccount {
    pub account_id: String,
    pub credential_id: String,
}

/// The accounts the runner can use now. When present, the provider picks
/// only from this list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidates {
    pub view_digest: String,
    pub accounts: Vec<CandidateAccount>,
}

/// A hint from an explicit pin; `account_id` wins when both are present.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestedAccount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

/// The `route.account_for` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountForRequest {
    /// The call about to be made.
    pub provider: String,
    pub model: String,
    /// The model the conversation's account affinity is bound to; defaults
    /// to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affinity_model: Option<String>,
    /// The runner's stable conversation id. One key stays on one account
    /// while that account is healthy.
    pub conversation_key: String,
    /// The idempotency key.
    pub admission_id: String,
    /// Defaults to `admit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<AccountMode>,
    /// Required for `renew` and `reassign`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_pin: Option<ExpectedPin>,
    /// Defaults to 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_input_bytes: Option<u64>,
    /// Defaults to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_input_estimated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Candidates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_account: Option<RequestedAccount>,
    /// The longest the runner waits out a short hold instead of being
    /// refused; defaults to [`DEFAULT_HOLD_WAIT_MAX_S`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_wait_max_s: Option<u64>,
    /// A `route.select` decision to honour instead of deciding twice.
    /// Honoured only from the runner module that decision named, and only
    /// for that decision's provider and model; otherwise `unknown_decision`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
}

impl AccountForRequest {
    /// An `admit` request with nothing optional.
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        conversation_key: impl Into<String>,
        admission_id: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            affinity_model: None,
            conversation_key: conversation_key.into(),
            admission_id: admission_id.into(),
            mode: None,
            expected_pin: None,
            initial_input_bytes: None,
            initial_input_estimated: None,
            candidates: None,
            requested_account: None,
            hold_wait_max_s: None,
            decision_id: None,
        }
    }

    /// The mode, with its default applied.
    pub fn mode(&self) -> AccountMode {
        self.mode.clone().unwrap_or(AccountMode::Admit)
    }

    /// The request with every default applied, for comparing bodies that
    /// share an `admission_id`.
    pub fn normalized(&self) -> Self {
        let mut normalized = self.clone();
        normalized
            .affinity_model
            .get_or_insert_with(|| self.model.clone());
        normalized.mode = Some(self.mode());
        normalized.initial_input_bytes.get_or_insert(0);
        normalized.initial_input_estimated.get_or_insert(false);
        normalized
            .hold_wait_max_s
            .get_or_insert(DEFAULT_HOLD_WAIT_MAX_S);
        normalized
    }

    /// Whether `other` is the same body as this one, defaults applied.
    pub fn same_body(&self, other: &Self) -> bool {
        self.normalized() == other.normalized()
    }
}

/// The admission the answer belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admission {
    /// The request's `admission_id`.
    pub id: String,
    /// The admission's state, as the provider names it.
    pub state: String,
}

/// The account to use, named by public id only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub provider: String,
    pub account_id: String,
    pub credential_id: String,
    /// How the account signs in, as the provider names it.
    pub method: String,
}

/// The conversation's pin to the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountPin {
    pub id: String,
    pub generation: u64,
    pub affinity_expires_at_ms: u64,
    pub admission_valid_until_ms: u64,
    /// The runner renews before this.
    pub lease_valid_until_ms: u64,
    pub mode: AccountMode,
}

/// Why the provider chose the account, for logs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReason {
    pub code: AccountReasonCode,
    /// The decision behind the allocation: the request's `decision_id` when
    /// it carried one, otherwise the provider's own id for this choice.
    pub decision_id: String,
    pub policy_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_fetched_at_ms: Option<u64>,
    pub quota_age_band: String,
}

/// The `route.account_for` answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountForAnswer {
    pub admission: Admission,
    pub account: Account,
    pub pin: AccountPin,
    pub reason: AccountReason,
}

/// Validate and decode `route.account_for` params. A bad member is
/// `invalid_request` naming it: `renew` and `reassign` without
/// `expected_pin` name `expected_pin`, and a mode this crate does not name
/// names `mode`.
pub fn parse_account_for_request(params: &Value) -> Result<AccountForRequest, Refusal> {
    let members = Members::of(params)?;
    let request = AccountForRequest {
        provider: members.text("provider", None)?,
        model: members.text("model", None)?,
        affinity_model: members.optional_text("affinity_model", None)?,
        conversation_key: members.text("conversation_key", Some(CONVERSATION_KEY_MAX_CHARS))?,
        admission_id: members.text("admission_id", Some(ADMISSION_ID_MAX_CHARS))?,
        mode: members.optional("mode")?,
        expected_pin: members.optional("expected_pin")?,
        initial_input_bytes: members.optional("initial_input_bytes")?,
        initial_input_estimated: members.optional("initial_input_estimated")?,
        candidates: members.optional("candidates")?,
        requested_account: members.optional("requested_account")?,
        hold_wait_max_s: members.optional("hold_wait_max_s")?,
        decision_id: members.optional_text("decision_id", None)?,
    };
    match request.mode() {
        AccountMode::Admit => {}
        AccountMode::Renew | AccountMode::Reassign => {
            if request.expected_pin.is_none() {
                return Err(
                    members.refuse("expected_pin", "renew and reassign require expected_pin")
                );
            }
        }
        other => {
            return Err(members.refuse("mode", format!("{other} is not a mode")));
        }
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal() -> Value {
        json!({
            "provider": "example-subscription",
            "model": "model-c",
            "conversation_key": "conversation-1",
            "admission_id": "admission-1"
        })
    }

    #[test]
    fn defaults_make_the_same_body() {
        let bare = parse_account_for_request(&minimal()).unwrap();
        let mut explicit = minimal();
        explicit["mode"] = json!("admit");
        explicit["initial_input_bytes"] = json!(0);
        explicit["hold_wait_max_s"] = json!(900);
        explicit["affinity_model"] = json!("model-c");
        explicit["decision_id"] = Value::Null;
        let explicit = parse_account_for_request(&explicit).unwrap();
        assert!(bare.same_body(&explicit));
        let mut other = minimal();
        other["conversation_key"] = json!("conversation-2");
        assert!(!bare.same_body(&parse_account_for_request(&other).unwrap()));
    }

    #[test]
    fn renew_needs_expected_pin_and_unknown_modes_are_refused() {
        let mut renew = minimal();
        renew["mode"] = json!("renew");
        let refusal = parse_account_for_request(&renew).unwrap_err();
        assert_eq!(refusal.detail["field"], "expected_pin");
        renew["expected_pin"] = json!({"id": "pin-1", "generation": 1});
        assert!(parse_account_for_request(&renew).is_ok());
        let mut unknown = minimal();
        unknown["mode"] = json!("borrow");
        assert_eq!(
            parse_account_for_request(&unknown).unwrap_err().detail["field"],
            "mode"
        );
    }

    #[test]
    fn bounds_are_refused_by_name() {
        let mut long = minimal();
        long["conversation_key"] = json!("k".repeat(257));
        assert_eq!(
            parse_account_for_request(&long).unwrap_err().detail["field"],
            "conversation_key"
        );
        let mut long = minimal();
        long["admission_id"] = json!("a".repeat(129));
        assert_eq!(
            parse_account_for_request(&long).unwrap_err().detail["field"],
            "admission_id"
        );
        let mut empty = minimal();
        empty["model"] = json!("");
        assert_eq!(
            parse_account_for_request(&empty).unwrap_err().detail["field"],
            "model"
        );
    }

    #[test]
    fn an_unknown_reason_code_decodes() {
        let reason: AccountReason = serde_json::from_value(json!({
            "code": "warm_spare",
            "decision_id": "d",
            "policy_version": "1",
            "quota_fetched_at_ms": null,
            "quota_age_band": "fresh"
        }))
        .unwrap();
        assert_eq!(reason.code, AccountReasonCode::Other("warm_spare".into()));
        assert_eq!(reason.quota_fetched_at_ms, None);
    }
}
