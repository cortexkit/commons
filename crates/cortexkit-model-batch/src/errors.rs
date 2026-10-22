//! The error vocabulary every model-batch role shares: codes, their classes,
//! and the two error shapes of a batch call.
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
//!
//! One vocabulary serves every role, so each role's own "this catalog row is
//! not my kind" code ([`MODEL_NOT_CLASSIFIER`], [`MODEL_NOT_EXTRACTOR`]) is
//! listed here too. Which codes a role actually answers with is the role's
//! list (`ADMISSION_CODES` and `ITEM_CODES` in each role crate).

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

/// `classifier/v1`: the model's catalog row is not a classifier.
/// `detail.model` and `detail.kind` name it and its kind. Permanent.
pub const MODEL_NOT_CLASSIFIER: &str = "model_not_classifier";

/// `extractor/v1`: the model's catalog row is not an extraction model,
/// including a row that states no kind. `detail.model` names it, and
/// `detail.kind` its kind when the row has one. Permanent.
pub const MODEL_NOT_EXTRACTOR: &str = "model_not_extractor";

/// As an admission refusal: the batch's estimated cost is over
/// `max_cost_usd`; `detail.estimate_usd` and `detail.max_cost_usd` carry
/// both. As a per-item error: answering the item would cross the batch's
/// ceiling. Permanent either way.
pub const COST_EXCEEDED: &str = "cost_exceeded";

/// The provider account is at its limit, as the runner's own spend guard
/// knows before dispatch. `detail.resets_at_ms` when known. Transient.
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

/// The provider does not serve the model (HTTP 404, or a Workers AI "no
/// such model" code). Permanent. As an admission refusal, `detail.model`
/// names the model; the call's first provider call met it, so the whole
/// call is refused and nothing is written. As a per-item error: it stopped
/// the call, and every item the call left unanswered carries it. Never
/// stored, so a re-send after a catalog fix asks the provider again.
pub const MODEL_UNAVAILABLE: &str = "model_unavailable";

/// The provider account used up its daily free allocation (Workers AI code
/// 3036). Transient: the allocation resets. Batch-level, handled like
/// [`AUTH_FAILED`]: an admission refusal when the call's first provider
/// call meets it, otherwise it stops the call and every item left
/// unanswered carries it. Never stored, so a re-send after the reset
/// answers the item.
pub const ACCOUNT_LIMIT_EXHAUSTED: &str = "account_limit_exhausted";

/// The provider account may not use the model: it needs a paid plan
/// (Workers AI 5035), has not accepted the model's agreement (5016), or is
/// blocked (3023). Permanent until an operator acts. Batch-level, handled
/// like [`AUTH_FAILED`]: an admission refusal when the call's first
/// provider call meets it, otherwise it stops the call and every item left
/// unanswered carries it. Never stored.
pub const ACCOUNT_NOT_ENTITLED: &str = "account_not_entitled";

// ---- per-item errors ------------------------------------------------------

/// The provider answered 429 or 529 after the runner's own retries.
/// `retry_after_ms` when the provider gave one. Transient. It stops the
/// call: the items after it that would need a provider call are not sent,
/// and carry `rate_limited` too, unstored, so a re-send retries them.
pub const RATE_LIMITED: &str = "rate_limited";

/// The provider answered 5xx or 408, or the transport failed, after the
/// runner's own retries. Transient.
pub const PROVIDER_ERROR: &str = "provider_error";

/// The provider refused the content. Permanent.
pub const PROVIDER_REFUSED: &str = "provider_refused";

/// The provider rejected this item's input (400, 413, 422, or another 4xx
/// the contract does not map elsewhere). Permanent.
pub const INVALID_ITEM: &str = "invalid_item";

/// The provider answered, but one answer is larger than the bound the
/// module states for the model, so the module does not forward it.
/// `detail.field` names the field, `detail.limit` the bound in force and
/// `detail.size` the answer's size in bytes. Permanent and stored; a caller
/// who wants another try re-sends under a new `batch_id`. `classifier/v1`
/// states no answer bound and never answers with this code.
pub const ANSWER_TOO_LARGE: &str = "answer_too_large";

/// The codes that say nothing about an item and stop the call, beyond
/// [`AUTH_FAILED`] and [`MODEL_UNAVAILABLE`]: the Workers AI account codes.
/// Neither is ever stored as an item's outcome.
pub const BATCH_LEVEL_CODES: &[&str] = &[ACCOUNT_LIMIT_EXHAUSTED, ACCOUNT_NOT_ENTITLED];

/// Every admission refusal code the vocabulary lists. A role answers with
/// the subset its contract's table names (its own `ADMISSION_CODES`).
pub const ADMISSION_VOCABULARY: &[&str] = &[
    INVALID_PARAMS,
    BATCH_ID_REUSE,
    MODEL_UNKNOWN,
    MODEL_NOT_CLASSIFIER,
    MODEL_NOT_EXTRACTOR,
    COST_EXCEEDED,
    ACCOUNT_WALLED,
    BATCH_IN_PROGRESS,
    AUTH_FAILED,
    MODEL_UNAVAILABLE,
    ACCOUNT_LIMIT_EXHAUSTED,
    ACCOUNT_NOT_ENTITLED,
];

/// Every per-item error code the vocabulary lists. A role answers with the
/// subset its contract's table names (its own `ITEM_CODES`).
pub const ITEM_VOCABULARY: &[&str] = &[
    RATE_LIMITED,
    PROVIDER_ERROR,
    PROVIDER_REFUSED,
    INVALID_ITEM,
    COST_EXCEEDED,
    AUTH_FAILED,
    MODEL_UNAVAILABLE,
    ACCOUNT_LIMIT_EXHAUSTED,
    ACCOUNT_NOT_ENTITLED,
    ANSWER_TOO_LARGE,
];

/// The class of an admission refusal code this crate lists, `None` for any
/// other code.
pub fn admission_class(code: &str) -> Option<ErrorClass> {
    match code {
        INVALID_PARAMS | BATCH_ID_REUSE | MODEL_UNKNOWN | MODEL_NOT_CLASSIFIER
        | MODEL_NOT_EXTRACTOR | COST_EXCEEDED | AUTH_FAILED | MODEL_UNAVAILABLE
        | ACCOUNT_NOT_ENTITLED => Some(ErrorClass::Permanent),
        ACCOUNT_WALLED | BATCH_IN_PROGRESS | ACCOUNT_LIMIT_EXHAUSTED => Some(ErrorClass::Transient),
        _ => None,
    }
}

/// The class of a per-item error code this crate lists, `None` for any
/// other code.
pub fn item_class(code: &str) -> Option<ErrorClass> {
    match code {
        RATE_LIMITED | PROVIDER_ERROR | ACCOUNT_LIMIT_EXHAUSTED => Some(ErrorClass::Transient),
        PROVIDER_REFUSED | INVALID_ITEM | COST_EXCEEDED | AUTH_FAILED | MODEL_UNAVAILABLE
        | ACCOUNT_NOT_ENTITLED | ANSWER_TOO_LARGE => Some(ErrorClass::Permanent),
        _ => None,
    }
}

// ---- classes --------------------------------------------------------------

/// Class names. The set is fixed for every `v1` role; a new class means a
/// new role version.
pub mod classes {
    /// The same request may succeed later.
    pub const TRANSIENT: &str = "transient";
    /// The same request meets the same answer.
    pub const PERMANENT: &str = "permanent";
}

/// An error class. Decodes open: a value other than [`classes`] is kept in
/// [`ErrorClass::Other`]. Non-exhaustive, so a match needs a wildcard arm
/// and adding a variant later is not a breaking change.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
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

/// The optional `detail` of an item error. Only [`ANSWER_TOO_LARGE`] sends
/// one today; every other item error leaves it out, so its wire form is
/// `{code, class, message, retry_after_ms?}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ItemErrorDetail {
    /// The field the error is about, as the role names it in a request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The bound in force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// The size that broke the bound, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// An item's error in a batch reply: `{code, class, message,
/// retry_after_ms?, detail?}`.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ItemError::new`] or [`ItemError::for_code`] and the `with_*` setters,
/// or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ItemError {
    /// A per-item code this crate lists; any other value decodes as a
    /// plain string.
    pub code: String,
    pub class: ErrorClass,
    pub message: String,
    /// When the provider said how long to wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    /// Present only on codes that define one ([`ANSWER_TOO_LARGE`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<ItemErrorDetail>,
}

impl ItemError {
    pub fn new(code: impl Into<String>, class: ErrorClass, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            class,
            message: message.into(),
            retry_after_ms: None,
            detail: None,
        }
    }

    /// An error with the class the vocabulary gives `code`; a code this
    /// crate does not list gets [`ErrorClass::Permanent`].
    pub fn for_code(code: &str, message: impl Into<String>) -> Self {
        Self::new(
            code,
            item_class(code).unwrap_or(ErrorClass::Permanent),
            message,
        )
    }

    /// `answer_too_large` for the answer to `field`, `size` bytes against a
    /// bound of `limit`.
    pub fn answer_too_large(field: impl Into<String>, limit: u64, size: u64) -> Self {
        let field = field.into();
        let mut error = Self::for_code(
            ANSWER_TOO_LARGE,
            format!("the answer to {field} is {size} bytes; at most {limit}"),
        );
        error.detail = Some(ItemErrorDetail {
            field: Some(field),
            limit: Some(limit),
            size: Some(size),
        });
        error
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub fn with_detail(mut self, detail: ItemErrorDetail) -> Self {
        self.detail = Some(detail);
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
    /// error is, except the ones that say nothing about the item:
    /// `auth_failed`, `model_unavailable`, `account_limit_exhausted` and
    /// `account_not_entitled`. The item stays unanswered, and a re-send
    /// (after a re-login, a catalog fix, the allocation's reset or an
    /// operator's fix) asks the provider again.
    ///
    /// This answers for an item the provider actually answered with this
    /// error. A `rate_limited` item is stored like any transient error; the
    /// items after it that a rate limit stopped the call from sending carry
    /// `rate_limited` in the reply but are never recorded, which is the
    /// runner's rule for unsent items and not something this code decides.
    pub fn is_stored(&self) -> bool {
        !is_batch_level(&self.code)
    }

    /// Whether this error, once an item ends with it, stops the call: no
    /// further item is sent to the provider in that call, and every item
    /// the call leaves unanswered carries the same code. True for the
    /// batch-level codes and `rate_limited`.
    pub fn stops_the_call(&self) -> bool {
        is_batch_level(&self.code) || self.code == RATE_LIMITED
    }
}

/// Whether `code` says nothing about the item it came back for: never
/// stored, stops the call, and refuses the call at admission when its first
/// provider call meets it.
pub fn is_batch_level(code: &str) -> bool {
    matches!(
        code,
        AUTH_FAILED | MODEL_UNAVAILABLE | ACCOUNT_LIMIT_EXHAUSTED | ACCOUNT_NOT_ENTITLED
    )
}

// ---- admission refusal ----------------------------------------------------

/// The `detail` of an admission refusal. Which members are present depends
/// on the code (see the role contract's table); `class` is always present,
/// so a caller that meets an unknown code still knows whether to retry.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RefusalDetail {
    pub class: ErrorClass,
    /// The offending or differing request field, as a path: a map key
    /// joined with a dot, an array index in brackets (`items[2].state`,
    /// `questions.urgent.type`). `params` names the request as a whole, and
    /// is used only when no narrower path applies. Each role contract lists
    /// its paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The limit the field exceeded, where one applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The kind of a catalog row that is not the role's kind, when the row
    /// states one.
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
    /// On `auth_failed`, the id of the credential the runner presented (for
    /// example `apikey:cloudflare`), when the runner knows it, so an operator
    /// can tell which login to fix. Always an id, never the secret itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
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
            credential: None,
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

    pub fn with_credential(mut self, credential: impl Into<String>) -> Self {
        self.credential = Some(credential.into());
        self
    }
}

/// An admission refusal: the body of the `ERROR` frame a batch call answers
/// with, `{code, message, detail}`, the shape of `subc_protocol::ErrorBody`
/// with a typed detail.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Refusal {
    /// An admission code this crate lists; any other value decodes as a
    /// plain string.
    pub code: String,
    pub message: String,
    /// Boxed so a `Result<_, Refusal>` stays small.
    pub detail: Box<RefusalDetail>,
}

impl Refusal {
    /// A refusal with `code`, carrying the class the vocabulary gives it; a
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

    fn edit(mut self, edit: impl FnOnce(RefusalDetail) -> RefusalDetail) -> Self {
        let detail =
            std::mem::replace(&mut *self.detail, RefusalDetail::new(ErrorClass::Permanent));
        *self.detail = edit(detail);
        self
    }

    /// `invalid_params` naming `field`.
    pub fn invalid_params(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message).edit(|detail| detail.with_field(field))
    }

    /// `invalid_params` naming `field` and the `limit` it exceeded.
    pub fn over_limit(field: impl Into<String>, limit: u64, message: impl Into<String>) -> Self {
        Self::invalid_params(field, message).edit(|detail| detail.with_limit(limit))
    }

    /// `batch_id_reuse` naming the first differing field.
    pub fn batch_id_reuse(field: impl Into<String>) -> Self {
        let field = field.into();
        Self::new(
            BATCH_ID_REUSE,
            format!("batch_id names a recorded batch whose {field} differs"),
        )
        .edit(|detail| detail.with_field(field))
    }

    pub fn model_unknown(model: impl Into<String>) -> Self {
        let model = model.into();
        Self::new(MODEL_UNKNOWN, format!("{model} is not in the catalog"))
            .edit(|detail| detail.with_model(model))
    }

    /// `classifier/v1`'s `model_not_classifier`.
    pub fn model_not_classifier(model: impl Into<String>, kind: impl Into<String>) -> Self {
        let (model, kind) = (model.into(), kind.into());
        Self::new(
            MODEL_NOT_CLASSIFIER,
            format!("{model} is a {kind}, not a classifier"),
        )
        .edit(|detail| detail.with_model(model).with_kind(kind))
    }

    /// `extractor/v1`'s `model_not_extractor`. `kind` is the row's kind,
    /// `None` when the row states none; the refusal never invents one.
    pub fn model_not_extractor(model: impl Into<String>, kind: Option<&str>) -> Self {
        let model = model.into();
        let message = match kind {
            Some(kind) => format!("{model} is a {kind}, not an extraction model"),
            None => {
                format!("{model} states no kind in the catalog, so it is not an extraction model")
            }
        };
        let refusal =
            Self::new(MODEL_NOT_EXTRACTOR, message).edit(|detail| detail.with_model(model));
        match kind {
            Some(kind) => refusal.edit(|detail| detail.with_kind(kind)),
            None => refusal,
        }
    }

    pub fn cost_exceeded(estimate_usd: Number, max_cost_usd: Number) -> Self {
        Self::new(
            COST_EXCEEDED,
            format!("estimated cost {estimate_usd} USD is over max_cost_usd {max_cost_usd}"),
        )
        .edit(|detail| {
            detail
                .with_estimate_usd(estimate_usd)
                .with_max_cost_usd(max_cost_usd)
        })
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
        Self::new(
            MODEL_UNAVAILABLE,
            format!("the provider does not serve {model}"),
        )
        .edit(|detail| detail.with_model(model))
    }

    /// The call's first provider call met Workers AI code 3036.
    pub fn account_limit_exhausted() -> Self {
        Self::new(
            ACCOUNT_LIMIT_EXHAUSTED,
            "the provider account used up its daily allocation",
        )
    }

    /// The call's first provider call met Workers AI code 5035, 5016 or
    /// 3023.
    pub fn account_not_entitled() -> Self {
        Self::new(
            ACCOUNT_NOT_ENTITLED,
            "the provider account may not use this model",
        )
    }

    pub fn batch_in_progress(retry_after_ms: u64) -> Self {
        Self::new(BATCH_IN_PROGRESS, "another call holds this batch_id")
            .edit(|detail| detail.with_retry_after_ms(retry_after_ms))
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
    fn every_admission_code_has_a_class_and_the_account_limit_is_transient() {
        for code in ADMISSION_VOCABULARY {
            assert!(admission_class(code).is_some(), "{code}");
        }
        for code in ITEM_VOCABULARY {
            assert!(item_class(code).is_some(), "{code}");
        }
        for code in [ACCOUNT_WALLED, BATCH_IN_PROGRESS, ACCOUNT_LIMIT_EXHAUSTED] {
            assert_eq!(admission_class(code), Some(ErrorClass::Transient), "{code}");
        }
        assert_eq!(
            admission_class(ACCOUNT_NOT_ENTITLED),
            Some(ErrorClass::Permanent)
        );
        assert_eq!(admission_class("nope"), None);
    }

    #[test]
    fn item_classes() {
        for code in [RATE_LIMITED, PROVIDER_ERROR, ACCOUNT_LIMIT_EXHAUSTED] {
            assert_eq!(item_class(code), Some(ErrorClass::Transient), "{code}");
        }
        for code in [
            PROVIDER_REFUSED,
            INVALID_ITEM,
            COST_EXCEEDED,
            AUTH_FAILED,
            MODEL_UNAVAILABLE,
            ACCOUNT_NOT_ENTITLED,
            ANSWER_TOO_LARGE,
        ] {
            assert_eq!(item_class(code), Some(ErrorClass::Permanent), "{code}");
        }
    }

    #[test]
    fn batch_level_codes_are_never_stored_and_stop_the_call() {
        for code in [
            AUTH_FAILED,
            MODEL_UNAVAILABLE,
            ACCOUNT_LIMIT_EXHAUSTED,
            ACCOUNT_NOT_ENTITLED,
        ] {
            let error = ItemError::for_code(code, "m");
            assert!(!error.is_stored(), "{code}");
            assert!(error.stops_the_call(), "{code}");
        }
        let limited = ItemError::for_code(RATE_LIMITED, "m");
        assert!(limited.is_stored() && limited.stops_the_call());
        for code in [
            PROVIDER_ERROR,
            PROVIDER_REFUSED,
            INVALID_ITEM,
            COST_EXCEEDED,
            ANSWER_TOO_LARGE,
        ] {
            let error = ItemError::for_code(code, "m");
            assert!(error.is_stored(), "{code}");
            assert!(!error.stops_the_call(), "{code}");
        }
    }

    #[test]
    fn an_unknown_class_is_kept_and_acted_on_as_permanent() {
        let error: ItemError = serde_json::from_str(
            r#"{"code":"quota_gone","class":"degraded","message":"m","retry_after_ms":5}"#,
        )
        .unwrap();
        assert_eq!(error.class, ErrorClass::Other("degraded".into()));
        assert!(!error.is_retried_on_resend());
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"code":"quota_gone","class":"degraded","message":"m","retry_after_ms":5}"#
        );
    }

    #[test]
    fn answer_too_large_carries_field_limit_and_size() {
        let error = ItemError::answer_too_large("fields.total", 4096, 5000);
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"code":"answer_too_large","class":"permanent","message":"the answer to fields.total is 5000 bytes; at most 4096","detail":{"field":"fields.total","limit":4096,"size":5000}}"#
        );
        assert!(error.is_stored());
        assert!(!error.is_retried_on_resend());
    }

    #[test]
    fn model_not_extractor_never_invents_a_kind() {
        let kindless = Refusal::model_not_extractor("m", None);
        assert_eq!(kindless.code, MODEL_NOT_EXTRACTOR);
        assert_eq!(kindless.detail.model.as_deref(), Some("m"));
        assert_eq!(kindless.detail.kind, None);
        let chat = Refusal::model_not_extractor("m", Some("chat"));
        assert_eq!(chat.detail.kind.as_deref(), Some("chat"));
        assert_eq!(chat.detail.class, ErrorClass::Permanent);
    }

    #[test]
    fn refusals_carry_their_class_and_field() {
        let refusal = Refusal::over_limit("items", 100, "too many items");
        assert_eq!(refusal.code, INVALID_PARAMS);
        assert_eq!(refusal.field(), Some("items"));
        assert_eq!(refusal.detail.limit, Some(100));
        let busy = Refusal::batch_in_progress(250);
        assert_eq!(busy.detail.class, ErrorClass::Transient);
        assert_eq!(busy.detail.retry_after_ms, Some(250));
        assert_eq!(
            Refusal::account_limit_exhausted().detail.class,
            ErrorClass::Transient
        );
    }
}
