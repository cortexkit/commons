//! Public account-limit state (`route.account_status`), gated by
//! `account_status`. Unknown accounts answer `unknown`, not a refusal.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::Refusal;
use crate::members::Members;

open_set! {
    /// Account-local limit state; unfamiliar strings survive decoding.
    pub enum LimitState {
        Available => "available",
        Limited => "limited",
        Unknown => "unknown",
    }
}

/// An account's public identity, not a request for credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountStatusRequest {
    pub provider: String,
    pub account_id: String,
}

impl AccountStatusRequest {
    pub fn new(provider: impl Into<String>, account_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            account_id: account_id.into(),
        }
    }
}

/// Limit state and, only for `limited`, a known reset time in Unix ms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountStatusAnswer {
    pub provider: String,
    pub account_id: String,
    pub limit_state: LimitState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<u64>,
}

impl AccountStatusAnswer {
    pub fn new(
        provider: impl Into<String>,
        account_id: impl Into<String>,
        limit_state: LimitState,
    ) -> Self {
        Self {
            provider: provider.into(),
            account_id: account_id.into(),
            limit_state,
            reset_at_ms: None,
        }
    }
}

/// Decode public account ids after discovery has declared `account_status`.
pub fn parse_account_status_request(params: &Value) -> Result<AccountStatusRequest, Refusal> {
    let members = Members::of(params)?;
    Ok(AccountStatusRequest::new(
        members.text("provider", None)?,
        members.text("account_id", None)?,
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccountStatusProblem {
    Provider,
    AccountId,
    ResetAtMs,
}

impl std::fmt::Display for AccountStatusProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Provider => "provider does not echo the request",
            Self::AccountId => "account_id does not echo the request",
            Self::ResetAtMs => "reset_at_ms is only allowed for limited accounts",
        })
    }
}

/// Check public identity echoes and reset-time applicability. A missing reset
/// on `limited` means its time is unknown. An unfamiliar state is retained,
/// but cannot carry a reset: the contract permits resets only on `limited`.
pub fn check_account_status(
    request: &AccountStatusRequest,
    answer: &AccountStatusAnswer,
) -> Result<(), AccountStatusProblem> {
    if answer.provider != request.provider {
        return Err(AccountStatusProblem::Provider);
    }
    if answer.account_id != request.account_id {
        return Err(AccountStatusProblem::AccountId);
    }
    if answer.reset_at_ms.is_some() && answer.limit_state != LimitState::Limited {
        return Err(AccountStatusProblem::ResetAtMs);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_status_checks_each_identity_echo() {
        let request = AccountStatusRequest::new("p", "account");
        for (provider, account_id, problem) in [
            ("other", "account", AccountStatusProblem::Provider),
            ("p", "other", AccountStatusProblem::AccountId),
        ] {
            let answer = AccountStatusAnswer::new(provider, account_id, LimitState::Unknown);
            assert_eq!(check_account_status(&request, &answer), Err(problem));
        }
    }

    #[test]
    fn account_status_reset_is_optional_and_only_for_limited() {
        let request = AccountStatusRequest::new("p", "account");
        for state in [
            LimitState::Available,
            LimitState::Limited,
            LimitState::Unknown,
            LimitState::parse("future"),
        ] {
            let mut answer = AccountStatusAnswer::new("p", "account", state.clone());
            assert_eq!(check_account_status(&request, &answer), Ok(()));
            for reset in [0, u64::MAX] {
                answer.reset_at_ms = Some(reset);
                let expected = if state == LimitState::Limited {
                    Ok(())
                } else {
                    Err(AccountStatusProblem::ResetAtMs)
                };
                assert_eq!(check_account_status(&request, &answer), expected);
            }
        }
    }

    #[test]
    fn account_status_parser_names_invalid_members() {
        use serde_json::json;
        for (body, field) in [
            (json!({"account_id": "account"}), "provider"),
            (
                json!({"provider": null, "account_id": "account"}),
                "provider",
            ),
            (json!({"provider": "", "account_id": "account"}), "provider"),
            (json!({"provider": 1, "account_id": "account"}), "provider"),
            (json!({"provider": "p"}), "account_id"),
            (json!({"provider": "p", "account_id": null}), "account_id"),
            (json!({"provider": "p", "account_id": ""}), "account_id"),
            (json!({"provider": "p", "account_id": 1}), "account_id"),
        ] {
            assert_eq!(
                parse_account_status_request(&body).unwrap_err().detail["field"],
                field
            );
        }
        assert_eq!(
            parse_account_status_request(&json!({"provider": "p", "account_id": "account"}))
                .unwrap(),
            AccountStatusRequest::new("p", "account")
        );
    }

    #[test]
    fn account_status_unknown_strings_round_trip_and_absent_reset_is_omitted() {
        let wire = r#"{"provider":"p","account_id":"account","limit_state":"future"}"#;
        let answer: AccountStatusAnswer = serde_json::from_str(wire).unwrap();
        assert_eq!(answer.limit_state, LimitState::Other("future".into()));
        assert_eq!(serde_json::to_string(&answer).unwrap(), wire);
        assert_eq!(
            serde_json::to_string(&AccountStatusRequest::new("p", "account")).unwrap(),
            r#"{"provider":"p","account_id":"account"}"#
        );
    }
}
