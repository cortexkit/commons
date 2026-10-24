//! Refusal codes, the refusal body, and each code's detail.
//!
//! A refusal is an `ERROR` frame whose body is `{code, message, detail}`
//! ([`Refusal`]). `detail` is always an object; every refusal field other
//! than `code` and `message` lives inside it. Codes are an open set
//! ([`RefusalCode`]): a code this crate does not name decodes as
//! [`RefusalCode::Other`], and a reader treats it as a refusal that is not
//! timed, whatever its detail carries ([`Refusal::wait_until_ms`]).
//!
//! A timed refusal names when its cause ends: `reset_at_ms` on
//! `pin_unavailable`, `no_route` and `account_pool_exhausted`, and
//! `until_ms` on `account_hold`. A consumer waits until then and asks again
//! rather than failing its work. A refusal without that time is not timed.

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

open_set! {
    /// A refusal code. The contract's refusal table lists which op answers
    /// each one.
    pub enum RefusalCode {
        /// A request member is missing, of the wrong type, or out of its
        /// bounds; `detail.field` names it. Any op.
        InvalidRequest => "invalid_request",
        /// `route.select`: a requirement is unknown or invalid;
        /// `detail.field` names it. Never ignored.
        InvalidRequirements => "invalid_requirements",
        /// `route.select`: the consumer id is unknown in configuration and
        /// accepted declarations; `detail.consumer` names it.
        UnknownConsumer => "unknown_consumer",
        /// `route.select`: neither the attested caller nor its stamped scope
        /// owns the declared consumer, or ownership conflicts. Terminal;
        /// `detail.consumer` names it.
        ConsumerNotOwned => "consumer_not_owned",
        /// `route.select`: the pin cannot be served now;
        /// `detail {pin, reason, reset_at_ms?}`.
        PinUnavailable => "pin_unavailable",
        /// `route.select`: nothing meets the request;
        /// `detail {reason, reset_at_ms?}`.
        NoRoute => "no_route",
        /// `route.report` or `route.account_for`: no decision with that id is
        /// visible to this caller. Answered the same way whether or not the
        /// decision exists.
        UnknownDecision => "unknown_decision",
        /// `route.report`: a report with a different body was already
        /// recorded for the decision.
        ReportConflict => "report_conflict",
        /// The runner ops: the caller is not a runner module, or may not use
        /// the account it named.
        AccountNotAuthorized => "account_not_authorized",
        /// `route.account_for`: the provider needs a login or credential the
        /// runner does not have.
        AuthRequired => "auth_required",
        /// `route.account_for`: the account is held for longer than the
        /// runner waits; timed by `detail.until_ms`.
        AccountHold => "account_hold",
        /// `route.account_for`: the provider cannot tell the account's quota.
        AccountQuotaUnknown => "account_quota_unknown",
        /// `route.account_for`: no account has headroom;
        /// `detail.reset_at_ms` when one is known.
        AccountPoolExhausted => "account_pool_exhausted",
        /// `route.account_for`: the accounts left are held in reserve.
        AccountPoolReserveLimited => "account_pool_reserve_limited",
        /// `route.account_for`: the allocator cannot choose right now;
        /// transient, but not timed.
        AccountSelectionUnavailable => "account_selection_unavailable",
        /// `route.account_for`: the admission id was already used with a
        /// different body.
        AdmissionConflict => "admission_conflict",
        /// `route.account_for`: `expected_pin.generation` is not the pin's
        /// current generation.
        PinGenerationConflict => "pin_generation_conflict",
        /// A consumer's own refusal of its work when the catalog lists no
        /// routing provider. Never answered by a provider.
        RoutingUnavailable => "routing_unavailable",
        /// A consumer's own refusal of its work when the catalog lists more
        /// than one routing provider; `detail.modules` names them. Never
        /// answered by a provider.
        RoutingAmbiguous => "routing_ambiguous",
    }
}

/// Codes a consumer answers for its own work, never a provider.
pub const CONSUMER_CODES: &[&str] = &["routing_unavailable", "routing_ambiguous"];

open_set! {
    /// Why a `pin` cannot be served (`pin_unavailable`'s `detail.reason`).
    pub enum PinUnavailableReason {
        /// The pin is not `provider/model[:variant]`.
        Invalid => "invalid",
        UnknownProvider => "unknown_provider",
        UnknownModel => "unknown_model",
        InvalidVariant => "invalid_variant",
        /// The pin's route key is in the request's `exclude`.
        Excluded => "excluded",
        NotAvailable => "not_available",
        CredentialUnentitled => "credential_unentitled",
        CredentialBillingExhausted => "credential_billing_exhausted",
        /// Timed: carries `reset_at_ms`.
        Cooldown => "cooldown",
        /// Timed when the reset is known.
        QuotaExhausted => "quota_exhausted",
        /// Timed: carries `reset_at_ms`.
        Backoff => "backoff",
    }
}

open_set! {
    /// Why nothing meets a `route.select` (`no_route`'s `detail.reason`).
    pub enum NoRouteReason {
        NoAvailableModel => "no_available_model",
        /// A requirement such as `tools`, `tool_capabilities`, `modalities`
        /// or `structured_output` is met by no route.
        MissingCapability => "missing_capability",
        ContextTooSmall => "context_too_small",
        /// Every route that would serve is cooling down or out of quota;
        /// carries `reset_at_ms` when the earliest end is known.
        QuotaOrCooldown => "quota_or_cooldown",
        /// Every candidate falls short of the requirements.
        AllInadequate => "all_inadequate",
        /// Every entry that would serve has all its eligible accounts
        /// blocked, and none can be held under the provider's hold rules;
        /// carries `reset_at_ms` when the earliest reset across entries is
        /// known.
        AccountsBlocked => "accounts_blocked",
    }
}

/// A refusal body: `{code, message, detail}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,
    /// Always an object on the wire. A body that sends `null` or leaves it
    /// out decodes as an empty object.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub detail: Map<String, Value>,
}

fn null_as_empty<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Map<String, Value>, D::Error> {
    Ok(Option::<Map<String, Value>>::deserialize(deserializer)?.unwrap_or_default())
}

impl Refusal {
    /// A refusal with an empty detail.
    pub fn new(code: RefusalCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: Map::new(),
        }
    }

    /// A refusal whose detail is `detail` encoded as an object.
    pub fn with<D: Serialize>(code: RefusalCode, message: impl Into<String>, detail: &D) -> Self {
        let detail = match serde_json::to_value(detail) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        Self {
            code,
            message: message.into(),
            detail,
        }
    }

    /// `invalid_request {field}`.
    pub fn invalid_request(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::with(
            RefusalCode::InvalidRequest,
            message,
            &FieldDetail {
                field: field.into(),
            },
        )
    }

    /// `invalid_requirements {field}`.
    pub fn invalid_requirements(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::with(
            RefusalCode::InvalidRequirements,
            message,
            &FieldDetail {
                field: field.into(),
            },
        )
    }

    /// `unknown_consumer {consumer}`.
    pub fn unknown_consumer(consumer: impl Into<String>) -> Self {
        let consumer = consumer.into();
        Self::with(
            RefusalCode::UnknownConsumer,
            format!("no consumer {consumer:?} is configured"),
            &UnknownConsumerDetail { consumer },
        )
    }

    /// `consumer_not_owned {consumer}`. Terminal, never timed.
    pub fn consumer_not_owned(consumer: impl Into<String>) -> Self {
        let consumer = consumer.into();
        Self::with(
            RefusalCode::ConsumerNotOwned,
            format!("the caller and its scope do not own consumer {consumer:?}"),
            &ConsumerNotOwnedDetail { consumer },
        )
    }

    /// `pin_unavailable {pin, reason, reset_at_ms?}`.
    pub fn pin_unavailable(
        pin: impl Into<String>,
        reason: PinUnavailableReason,
        reset_at_ms: Option<u64>,
    ) -> Self {
        let pin = pin.into();
        Self::with(
            RefusalCode::PinUnavailable,
            format!("the pin {pin:?} cannot be served: {reason}"),
            &PinUnavailableDetail {
                pin,
                reason,
                reset_at_ms,
            },
        )
    }

    /// `no_route {reason, reset_at_ms?}`.
    pub fn no_route(reason: NoRouteReason, reset_at_ms: Option<u64>) -> Self {
        Self::with(
            RefusalCode::NoRoute,
            format!("no route meets the request: {reason}"),
            &NoRouteDetail {
                reason,
                reset_at_ms,
            },
        )
    }

    /// `unknown_decision`, worded the same whether or not the decision
    /// exists.
    pub fn unknown_decision() -> Self {
        Self::new(
            RefusalCode::UnknownDecision,
            "no such decision for this caller",
        )
    }

    /// `account_hold {until_ms}`.
    pub fn account_hold(until_ms: u64) -> Self {
        Self::with(
            RefusalCode::AccountHold,
            "the account is held",
            &AccountHoldDetail { until_ms },
        )
    }

    /// `account_pool_exhausted {reset_at_ms?}`.
    pub fn account_pool_exhausted(reset_at_ms: Option<u64>) -> Self {
        Self::with(
            RefusalCode::AccountPoolExhausted,
            "no account has headroom",
            &ResetDetail { reset_at_ms },
        )
    }

    /// `routing_ambiguous {modules}`: a consumer's own refusal.
    pub fn routing_ambiguous(modules: Vec<String>) -> Self {
        Self::with(
            RefusalCode::RoutingAmbiguous,
            "more than one routing provider is listed",
            &RoutingAmbiguousDetail { modules },
        )
    }

    /// Decode the detail as `D`.
    pub fn detail_as<D: DeserializeOwned>(&self) -> Option<D> {
        serde_json::from_value(Value::Object(self.detail.clone())).ok()
    }

    /// When the refusal's cause ends, if it is timed: `reset_at_ms` for
    /// `pin_unavailable`, `no_route` and `account_pool_exhausted`,
    /// `until_ms` for `account_hold`. `None` for every other code, an
    /// unknown code included, whatever its detail carries.
    pub fn wait_until_ms(&self) -> Option<u64> {
        let key = match self.code {
            RefusalCode::PinUnavailable
            | RefusalCode::NoRoute
            | RefusalCode::AccountPoolExhausted => "reset_at_ms",
            RefusalCode::AccountHold => "until_ms",
            _ => return None,
        };
        self.detail.get(key).and_then(Value::as_u64)
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Refusal {}

/// The detail of `invalid_request` and `invalid_requirements`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDetail {
    /// The member refused: a request member's name, or for
    /// `invalid_requirements` the requirement's name as it was sent.
    pub field: String,
}

/// The detail of `unknown_consumer`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownConsumerDetail {
    pub consumer: String,
}

/// The detail of the terminal `consumer_not_owned` refusal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerNotOwnedDetail {
    pub consumer: String,
}

/// The detail of `pin_unavailable`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinUnavailableDetail {
    /// The pin as the request sent it.
    pub pin: String,
    pub reason: PinUnavailableReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<u64>,
}

/// The detail of `no_route`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoRouteDetail {
    pub reason: NoRouteReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<u64>,
}

/// The detail of `account_hold`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountHoldDetail {
    pub until_ms: u64,
}

/// The detail of `account_pool_exhausted`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<u64>,
}

/// The detail of `routing_ambiguous`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingAmbiguousDetail {
    /// The routing provider modules the catalog lists.
    pub modules: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_not_owned_names_the_consumer_and_is_terminal() {
        let mut refusal = Refusal::consumer_not_owned("example.curate");
        assert_eq!(refusal.code, RefusalCode::ConsumerNotOwned);
        assert_eq!(
            refusal
                .detail_as::<ConsumerNotOwnedDetail>()
                .unwrap()
                .consumer,
            "example.curate"
        );
        refusal
            .detail
            .insert("reset_at_ms".into(), serde_json::json!(99));
        assert_eq!(refusal.wait_until_ms(), None);
    }

    #[test]
    fn an_unknown_code_is_kept_and_never_timed() {
        let refusal: Refusal = serde_json::from_str(
            r#"{"code":"rate_shaped","message":"m","detail":{"reset_at_ms":5}}"#,
        )
        .unwrap();
        assert_eq!(refusal.code, RefusalCode::Other("rate_shaped".into()));
        assert_eq!(refusal.wait_until_ms(), None);
        assert_eq!(
            serde_json::to_string(&refusal).unwrap(),
            r#"{"code":"rate_shaped","message":"m","detail":{"reset_at_ms":5}}"#
        );
    }

    #[test]
    fn null_or_absent_detail_is_an_empty_object() {
        for body in [
            r#"{"code":"no_route","message":"m"}"#,
            r#"{"code":"no_route","message":"m","detail":null}"#,
        ] {
            let refusal: Refusal = serde_json::from_str(body).unwrap();
            assert!(refusal.detail.is_empty());
            assert_eq!(
                serde_json::to_string(&refusal).unwrap(),
                r#"{"code":"no_route","message":"m","detail":{}}"#
            );
        }
    }

    #[test]
    fn timed_codes_read_their_time() {
        let pin = Refusal::pin_unavailable("p/m", PinUnavailableReason::Cooldown, Some(9));
        assert_eq!(pin.wait_until_ms(), Some(9));
        assert_eq!(
            pin.detail_as::<PinUnavailableDetail>().unwrap().reason,
            PinUnavailableReason::Cooldown
        );
        assert_eq!(Refusal::account_hold(7).wait_until_ms(), Some(7));
        assert_eq!(
            Refusal::no_route(NoRouteReason::AllInadequate, None).wait_until_ms(),
            None
        );
        assert_eq!(Refusal::unknown_decision().wait_until_ms(), None);
    }

    #[test]
    fn unknown_reasons_decode_open() {
        let detail: NoRouteDetail = serde_json::from_str(r#"{"reason":"region_closed"}"#).unwrap();
        assert_eq!(detail.reason, NoRouteReason::Other("region_closed".into()));
        let detail: PinUnavailableDetail =
            serde_json::from_str(r#"{"pin":"p/m","reason":"retired","reset_at_ms":null}"#).unwrap();
        assert_eq!(detail.reason, PinUnavailableReason::Other("retired".into()));
        assert_eq!(detail.reset_at_ms, None);
    }

    #[test]
    fn consumer_codes_are_known_codes() {
        for code in CONSUMER_CODES {
            assert!(RefusalCode::parse(code).is_known(), "{code}");
        }
    }
}
