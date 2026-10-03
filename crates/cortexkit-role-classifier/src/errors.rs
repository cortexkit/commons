//! Error codes, their classes, and the two error shapes of `classify.run`.
//!
//! An admission refusal answers the whole request with an `ERROR` frame
//! whose body is `{code, message, detail}` ([`Refusal`]); it writes nothing
//! and dispatches nothing. After admission the batch never fails as a
//! whole: an item that could not be answered carries an [`ItemError`] in
//! its place in the reply.
//!
//! Codes are open strings on the wire: a code this crate does not list is
//! kept as it was sent. Classes decode open too ([`ErrorClass::Other`]); a
//! caller that meets an unknown class treats it as permanent
//! ([`ErrorClass::effective`]), which is the safe direction.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Number;

// ---- admission refusals ---------------------------------------------------

/// A malformed request, or a limit exceeded. `detail.field` names the
/// field, and `detail.limit` the limit where one applies. Permanent.
pub const INVALID_PARAMS: &str = "invalid_params";

/// The `batch_id` names a recorded batch whose body differs.
/// `detail.field` names the first differing field. Permanent.
pub const BATCH_ID_REUSE: &str = "batch_id_reuse";

/// The model is not in the runner's catalog. `detail.model` names it.
/// Permanent.
pub const MODEL_UNKNOWN: &str = "model_unknown";

/// The model's catalog row is not a classifier. `detail.model` and
/// `detail.kind` name it and its kind. Permanent.
pub const MODEL_NOT_CLASSIFIER: &str = "model_not_classifier";

/// As an admission refusal: the batch's estimated cost is over
/// `max_cost_usd`; `detail.estimate_usd` and `detail.max_cost_usd` carry
/// both. As a per-item error: answering the item would cross the batch's
/// ceiling. Permanent either way.
pub const COST_EXCEEDED: &str = "cost_exceeded";

/// The provider account is at its limit. `detail.resets_at_ms` when known.
/// Transient.
pub const ACCOUNT_WALLED: &str = "account_walled";

/// Another call holds this `batch_id`. `detail.retry_after_ms` says when to
/// re-send. Transient. Nothing was written or dispatched.
pub const BATCH_IN_PROGRESS: &str = "batch_in_progress";

/// The provider refused the module's credentials (HTTP 401 or 403).
/// Permanent. As an admission refusal: the call's first provider call met
/// it, so the whole call is refused and nothing is written. As a per-item
/// error: it stopped the call, and every item the call left unanswered
/// carries it. Never stored, so a re-send after a re-login asks the
/// provider again.
pub const AUTH_FAILED: &str = "auth_failed";

/// The provider does not serve the model (HTTP 404). Permanent. As an
/// admission refusal, `detail.model` names the model; the call's first
/// provider call met it, so the whole call is refused and nothing is
/// written. As a per-item error: it stopped the call, and every item the
/// call left unanswered carries it. Never stored, so a re-send after a
/// catalog fix asks the provider again.
pub const MODEL_UNAVAILABLE: &str = "model_unavailable";

/// Every admission refusal code, in the contract's table order.
pub const ADMISSION_CODES: &[&str] = &[
    INVALID_PARAMS,
    BATCH_ID_REUSE,
    MODEL_UNKNOWN,
    MODEL_NOT_CLASSIFIER,
    COST_EXCEEDED,
    ACCOUNT_WALLED,
    BATCH_IN_PROGRESS,
    AUTH_FAILED,
    MODEL_UNAVAILABLE,
];

// ---- per-item errors ------------------------------------------------------

/// The provider answered 429 or 529 after the runner's own retries.
/// `retry_after_ms` when the provider gave one. Transient. It stops the
/// call: the items after it that would need a provider call are not sent,
/// and carry `rate_limited` too, unstored, so a re-send retries them.
pub const RATE_LIMITED: &str = "rate_limited";

/// The provider answered 5xx, or the transport failed, after the runner's
/// own retries. Transient.
pub const PROVIDER_ERROR: &str = "provider_error";

/// The provider refused the content. Permanent.
pub const PROVIDER_REFUSED: &str = "provider_refused";

/// The provider rejected this item's input (400, 413, 422, or another 4xx
/// the contract does not map elsewhere). Permanent.
pub const INVALID_ITEM: &str = "invalid_item";

/// Every per-item error code, in the contract's table order.
/// `cost_exceeded`, `auth_failed` and `model_unavailable` are both
/// admission refusals and per-item errors.
pub const ITEM_CODES: &[&str] = &[
    RATE_LIMITED,
    PROVIDER_ERROR,
    PROVIDER_REFUSED,
    INVALID_ITEM,
    COST_EXCEEDED,
    AUTH_FAILED,
    MODEL_UNAVAILABLE,
];

/// The class of an admission refusal code this crate lists, `None` for any
/// other code.
pub fn admission_class(code: &str) -> Option<ErrorClass> {
    match code {
        INVALID_PARAMS | BATCH_ID_REUSE | MODEL_UNKNOWN | MODEL_NOT_CLASSIFIER | COST_EXCEEDED
        | AUTH_FAILED | MODEL_UNAVAILABLE => Some(ErrorClass::Permanent),
        ACCOUNT_WALLED | BATCH_IN_PROGRESS => Some(ErrorClass::Transient),
        _ => None,
    }
}

/// The class of a per-item error code this crate lists, `None` for any
/// other code.
pub fn item_class(code: &str) -> Option<ErrorClass> {
    match code {
        RATE_LIMITED | PROVIDER_ERROR => Some(ErrorClass::Transient),
        PROVIDER_REFUSED | INVALID_ITEM | COST_EXCEEDED | AUTH_FAILED | MODEL_UNAVAILABLE => {
            Some(ErrorClass::Permanent)
        }
        _ => None,
    }
}

// ---- classes --------------------------------------------------------------

/// Class names. The set is fixed for `classifier/v1`; a new class means a
/// new role version.
pub mod classes {
    /// The same request may succeed later.
    pub const TRANSIENT: &str = "transient";
    /// The same request meets the same answer.
    pub const PERMANENT: &str = "permanent";
}

/// An error class. Decodes open: a value other than [`classes`] is kept in
/// [`ErrorClass::Other`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    Transient,
    Permanent,
    Other(String),
}

impl ErrorClass {
    pub fn parse(class: &str) -> Self {
        match class {
            classes::TRANSIENT => Self::Transient,
            classes::PERMANENT => Self::Permanent,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Transient => classes::TRANSIENT,
            Self::Permanent => classes::PERMANENT,
            Self::Other(other) => other,
        }
    }

    /// The class a caller acts on: an unknown class is treated as
    /// permanent, so a caller never retries what it cannot classify.
    pub fn effective(&self) -> Self {
        match self {
            Self::Transient => Self::Transient,
            Self::Permanent | Self::Other(_) => Self::Permanent,
        }
    }

    /// Whether the class, as a caller acts on it, is transient.
    pub fn is_transient(&self) -> bool {
        self.effective() == Self::Transient
    }
}

impl fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorClass {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorClass {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(deserializer)?))
    }
}

// ---- per-item error -------------------------------------------------------

/// An item's error in a `classify.run` reply: `{code, class, message,
/// retry_after_ms?}`.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ItemError::new`] or [`ItemError::for_code`] and the `with_*` setters,
/// or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ItemError {
    /// One of [`ITEM_CODES`]; any other value decodes as a plain string.
    pub code: String,
    pub class: ErrorClass,
    pub message: String,
    /// When the provider said how long to wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl ItemError {
    pub fn new(code: impl Into<String>, class: ErrorClass, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            class,
            message: message.into(),
            retry_after_ms: None,
        }
    }

    /// An error with the class the contract gives `code`; a code this crate
    /// does not list gets [`ErrorClass::Permanent`].
    pub fn for_code(code: &str, message: impl Into<String>) -> Self {
        Self::new(
            code,
            item_class(code).unwrap_or(ErrorClass::Permanent),
            message,
        )
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    /// Whether a re-send of the batch asks the provider again for an item
    /// whose stored outcome is this error. A stored transient error is
    /// retried and its new outcome replaces it; a stored permanent one (and
    /// one of an unknown class) is returned as stored, with no provider
    /// call.
    pub fn is_retried_on_resend(&self) -> bool {
        self.class.is_transient()
    }

    /// Whether a runner records this error as the item's outcome. Every
    /// error is, except `auth_failed` and `model_unavailable`: they say
    /// nothing about the item, so the item stays unanswered and a re-send
    /// (after a re-login or a catalog fix) asks the provider again. The
    /// items a rate limit kept from being sent are left unrecorded too,
    /// but that is decided by the call, not by the code.
    pub fn is_stored(&self) -> bool {
        !matches!(self.code.as_str(), AUTH_FAILED | MODEL_UNAVAILABLE)
    }

    /// Whether this error, once an item ends with it, stops the call: no
    /// further item is sent to the provider in that call, and every item
    /// the call leaves unanswered carries the same code. True for
    /// `auth_failed`, `model_unavailable` and `rate_limited`.
    pub fn stops_the_call(&self) -> bool {
        matches!(
            self.code.as_str(),
            AUTH_FAILED | MODEL_UNAVAILABLE | RATE_LIMITED
        )
    }
}

// ---- admission refusal ----------------------------------------------------

/// The `detail` of an admission refusal. Which members are present depends
/// on the code (see the contract's table); `class` is always present, so a
/// caller that meets an unknown code still knows whether to retry.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RefusalDetail {
    pub class: ErrorClass,
    /// The offending or differing request field, as a path (a map key
    /// joined with a dot, an array index in brackets): `batch_id`,
    /// `model`, `questions`, `questions.<id>`, `questions.<id>.type`,
    /// `questions.<id>.instructions`, `questions.<id>.criteria`, `items`,
    /// `items[i]`, `items[i].state`, `items[i].images`,
    /// `items[i].images[j]`, `max_cost_usd`, or `params` for the request
    /// as a whole. `params` is used only when none of the narrower paths
    /// applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The limit the field exceeded, where one applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The kind of a catalog row that is not a classifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_usd: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl RefusalDetail {
    pub fn new(class: ErrorClass) -> Self {
        Self {
            class,
            field: None,
            limit: None,
            model: None,
            kind: None,
            estimate_usd: None,
            max_cost_usd: None,
            resets_at_ms: None,
            retry_after_ms: None,
        }
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    pub fn with_estimate_usd(mut self, estimate_usd: Number) -> Self {
        self.estimate_usd = Some(estimate_usd);
        self
    }

    pub fn with_max_cost_usd(mut self, max_cost_usd: Number) -> Self {
        self.max_cost_usd = Some(max_cost_usd);
        self
    }

    pub fn with_resets_at_ms(mut self, resets_at_ms: u64) -> Self {
        self.resets_at_ms = Some(resets_at_ms);
        self
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }
}

/// An admission refusal: the body of the `ERROR` frame `classify.run`
/// answers with, `{code, message, detail}`, the shape of
/// `subc_protocol::ErrorBody` with a typed detail.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Refusal {
    /// One of [`ADMISSION_CODES`]; any other value decodes as a plain
    /// string.
    pub code: String,
    pub message: String,
    /// Boxed so a `Result<_, Refusal>` stays small.
    pub detail: Box<RefusalDetail>,
}

impl Refusal {
    /// A refusal with `code`, carrying the class the contract gives it; a
    /// code this crate does not list gets [`ErrorClass::Permanent`].
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        let code = code.into();
        let class = admission_class(&code).unwrap_or(ErrorClass::Permanent);
        Self {
            code,
            message: message.into(),
            detail: Box::new(RefusalDetail::new(class)),
        }
    }

    pub fn with_detail(mut self, detail: RefusalDetail) -> Self {
        self.detail = Box::new(detail);
        self
    }

    /// `invalid_params` naming `field`.
    pub fn invalid_params(field: impl Into<String>, message: impl Into<String>) -> Self {
        let refusal = Self::new(INVALID_PARAMS, message);
        let detail = (*refusal.detail).clone().with_field(field);
        refusal.with_detail(detail)
    }

    /// `invalid_params` naming `field` and the `limit` it exceeded.
    pub fn over_limit(field: impl Into<String>, limit: u64, message: impl Into<String>) -> Self {
        let refusal = Self::invalid_params(field, message);
        let detail = (*refusal.detail).clone().with_limit(limit);
        refusal.with_detail(detail)
    }

    /// `batch_id_reuse` naming the first differing field.
    pub fn batch_id_reuse(field: impl Into<String>) -> Self {
        let field = field.into();
        let refusal = Self::new(
            BATCH_ID_REUSE,
            format!("batch_id names a recorded batch whose {field} differs"),
        );
        let detail = (*refusal.detail).clone().with_field(field);
        refusal.with_detail(detail)
    }

    pub fn model_unknown(model: impl Into<String>) -> Self {
        let model = model.into();
        let refusal = Self::new(MODEL_UNKNOWN, format!("{model} is not in the catalog"));
        let detail = (*refusal.detail).clone().with_model(model);
        refusal.with_detail(detail)
    }

    pub fn model_not_classifier(model: impl Into<String>, kind: impl Into<String>) -> Self {
        let (model, kind) = (model.into(), kind.into());
        let refusal = Self::new(
            MODEL_NOT_CLASSIFIER,
            format!("{model} is a {kind}, not a classifier"),
        );
        let detail = (*refusal.detail).clone().with_model(model).with_kind(kind);
        refusal.with_detail(detail)
    }

    pub fn cost_exceeded(estimate_usd: Number, max_cost_usd: Number) -> Self {
        let refusal = Self::new(
            COST_EXCEEDED,
            format!("estimated cost {estimate_usd} USD is over max_cost_usd {max_cost_usd}"),
        );
        let detail = (*refusal.detail)
            .clone()
            .with_estimate_usd(estimate_usd)
            .with_max_cost_usd(max_cost_usd);
        refusal.with_detail(detail)
    }

    pub fn account_walled() -> Self {
        Self::new(ACCOUNT_WALLED, "the provider account is at its limit")
    }

    /// The call's first provider call met a 401 or 403.
    pub fn auth_failed() -> Self {
        Self::new(AUTH_FAILED, "the provider refused the module's credentials")
    }

    /// The call's first provider call met a 404 for `model`.
    pub fn model_unavailable(model: impl Into<String>) -> Self {
        let model = model.into();
        let refusal = Self::new(
            MODEL_UNAVAILABLE,
            format!("the provider does not serve {model}"),
        );
        let detail = (*refusal.detail).clone().with_model(model);
        refusal.with_detail(detail)
    }

    pub fn batch_in_progress(retry_after_ms: u64) -> Self {
        let refusal = Self::new(BATCH_IN_PROGRESS, "another call holds this batch_id");
        let detail = (*refusal.detail)
            .clone()
            .with_retry_after_ms(retry_after_ms);
        refusal.with_detail(detail)
    }

    /// The refused field, if the detail names one.
    pub fn field(&self) -> Option<&str> {
        self.detail.field.as_deref()
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail.field {
            Some(field) => write!(f, "{} ({field}): {}", self.code, self.message),
            None => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for Refusal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_code_has_its_tables_class() {
        for code in ADMISSION_CODES {
            assert!(admission_class(code).is_some(), "{code}");
        }
        for code in ITEM_CODES {
            assert!(item_class(code).is_some(), "{code}");
        }
        assert_eq!(
            admission_class(BATCH_IN_PROGRESS),
            Some(ErrorClass::Transient)
        );
        assert_eq!(admission_class(ACCOUNT_WALLED), Some(ErrorClass::Transient));
        assert_eq!(item_class(RATE_LIMITED), Some(ErrorClass::Transient));
        assert_eq!(item_class(PROVIDER_ERROR), Some(ErrorClass::Transient));
        assert_eq!(item_class(PROVIDER_REFUSED), Some(ErrorClass::Permanent));
        assert_eq!(item_class(INVALID_ITEM), Some(ErrorClass::Permanent));
        assert_eq!(item_class(COST_EXCEEDED), Some(ErrorClass::Permanent));
        for code in [AUTH_FAILED, MODEL_UNAVAILABLE] {
            assert_eq!(item_class(code), Some(ErrorClass::Permanent), "{code}");
            assert_eq!(admission_class(code), Some(ErrorClass::Permanent), "{code}");
        }
        assert_eq!(admission_class("nope"), None);
    }

    #[test]
    fn an_unknown_class_is_kept_and_acted_on_as_permanent() {
        let error: ItemError = serde_json::from_str(
            r#"{"code":"quota_gone","class":"degraded","message":"m","retry_after_ms":5}"#,
        )
        .unwrap();
        assert_eq!(error.code, "quota_gone");
        assert_eq!(error.class, ErrorClass::Other("degraded".into()));
        assert_eq!(error.class.effective(), ErrorClass::Permanent);
        assert!(!error.is_retried_on_resend());
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"code":"quota_gone","class":"degraded","message":"m","retry_after_ms":5}"#
        );
    }

    #[test]
    fn stored_transient_errors_are_retried_and_permanent_ones_are_not() {
        for code in [RATE_LIMITED, PROVIDER_ERROR] {
            assert!(
                ItemError::for_code(code, "m").is_retried_on_resend(),
                "{code}"
            );
        }
        for code in [PROVIDER_REFUSED, INVALID_ITEM, COST_EXCEEDED] {
            assert!(
                !ItemError::for_code(code, "m").is_retried_on_resend(),
                "{code}"
            );
        }
    }

    #[test]
    fn auth_and_model_failures_are_never_stored_and_stop_the_call() {
        for code in [AUTH_FAILED, MODEL_UNAVAILABLE] {
            let error = ItemError::for_code(code, "m");
            assert!(!error.is_stored(), "{code}");
            assert!(error.stops_the_call(), "{code}");
        }
        assert!(ItemError::for_code(RATE_LIMITED, "m").stops_the_call());
        for code in [
            PROVIDER_ERROR,
            PROVIDER_REFUSED,
            INVALID_ITEM,
            COST_EXCEEDED,
        ] {
            let error = ItemError::for_code(code, "m");
            assert!(error.is_stored(), "{code}");
            assert!(!error.stops_the_call(), "{code}");
        }
        assert!(ItemError::for_code(RATE_LIMITED, "m").is_stored());
        assert_eq!(
            Refusal::model_unavailable("m").detail.model.as_deref(),
            Some("m")
        );
        assert_eq!(Refusal::auth_failed().detail.class, ErrorClass::Permanent);
    }

    #[test]
    fn refusals_carry_their_class_and_field() {
        let refusal = Refusal::over_limit("items", 100, "too many items");
        assert_eq!(refusal.code, INVALID_PARAMS);
        assert_eq!(refusal.field(), Some("items"));
        assert_eq!(refusal.detail.limit, Some(100));
        assert_eq!(refusal.detail.class, ErrorClass::Permanent);
        let busy = Refusal::batch_in_progress(250);
        assert_eq!(busy.detail.class, ErrorClass::Transient);
        assert_eq!(busy.detail.retry_after_ms, Some(250));
    }
}
