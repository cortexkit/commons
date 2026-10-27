//! Role failures retain their existing vocabulary and refusal shape. Serving
//! modules must enforce tool-admission checks before dispatching to a role.

use std::fmt;

use cortexkit_model_batch::errors as batch;
pub use cortexkit_model_batch::errors::{ErrorClass, ItemError, Refusal, RefusalDetail};
pub use cortexkit_role_classifier::errors as classifier;
pub use cortexkit_role_extractor::errors as extractor;

pub const SCOPE_REQUIRED: &str = "scope_required";
pub const FLOW_SCOPE_REQUIRED: &str = "flow_scope_required";
pub const COST_UNPRICEABLE: &str = "cost_unpriceable";
pub const OUTPUT_INVALID: &str = "output_invalid";
pub const NO_ROUTE: &str = "no_route";
/// The provider answered, but nothing gives the billed cost: a count the
/// per-token price rests on is missing and there is no provider billing figure.
/// The answer is withheld rather than returned with a guessed cost.
pub const COST_UNMEASURED: &str = "cost_unmeasured";

/// Permanent pre-dispatch refusals common to all three tools.
pub const TOOL_ADMISSION_CODES: &[&str] = &[SCOPE_REQUIRED, FLOW_SCOPE_REQUIRED, COST_UNPRICEABLE];

/// Permanent failures common to all three tools, raised after the provider
/// answered.
pub const TOOL_RESULT_CODES: &[&str] = &[COST_UNMEASURED];

/// Completion's applicable model-batch codes plus its two completion-only codes.
/// Batch identity and role-specific model-kind errors do not apply to completion.
pub const COMPLETE_CODES: &[&str] = &[
    batch::INVALID_PARAMS,
    batch::MODEL_UNKNOWN,
    batch::COST_EXCEEDED,
    batch::ACCOUNT_WALLED,
    batch::AUTH_FAILED,
    batch::MODEL_UNAVAILABLE,
    batch::ACCOUNT_LIMIT_EXHAUSTED,
    batch::ACCOUNT_NOT_ENTITLED,
    batch::RATE_LIMITED,
    batch::PROVIDER_ERROR,
    batch::PROVIDER_REFUSED,
    batch::INVALID_ITEM,
    batch::ANSWER_TOO_LARGE,
    OUTPUT_INVALID,
    NO_ROUTE,
];

/// The published class, or None for a code outside this vocabulary. Decoding
/// refusals remains open, just as it is in the roles.
pub fn class(code: &str) -> Option<ErrorClass> {
    if TOOL_ADMISSION_CODES.contains(&code)
        || TOOL_RESULT_CODES.contains(&code)
        || [OUTPUT_INVALID, NO_ROUTE].contains(&code)
    {
        Some(ErrorClass::Permanent)
    } else {
        batch::admission_class(code).or_else(|| batch::item_class(code))
    }
}

/// Build a refusal without guessing an unknown code's class.
pub fn refusal(code: &str, message: impl Into<String>) -> Option<Refusal> {
    class(code).map(|class| Refusal::new(code, message).with_detail(RefusalDetail::new(class)))
}

/// The `detail` member carrying a refusal's measured billed cost in USD.
/// Present only when a provider call was made and its billed cost was
/// measured: the same number a success would have carried. The caller
/// charges exactly this amount for the refusal.
pub const COST_USD_KEY: &str = "cost_usd";

/// The `detail` member marking a refusal whose spend may have reached the
/// provider but was never measured (for example a timeout or a connection
/// lost after the request was sent). Always `true` when present. The
/// caller charges the request's `max_cost_usd` for the refusal.
pub const COST_UNKNOWN_KEY: &str = "cost_unknown";

/// The `detail` member marking a refusal that spent nothing. Always `true`
/// when present: it is the serving module's promise that the refusal cost
/// nothing, for example every pre-dispatch refusal, or a provider error
/// the provider does not bill. The caller charges nothing for the refusal.
pub const NO_SPEND_KEY: &str = "no_spend";

/// What the caller charges for a refusal of any of the three tools, as
/// [`refusal_spend`] reads it from the refusal's code and spend members.
/// A repeat with the same call key returns the same refusal with the same
/// spend members, and is never billed twice.
///
/// The serving module marks every refusal with exactly one of the three
/// spend markers (`detail.cost_usd`, the `cost_unmeasured` code, or
/// `detail.cost_unknown`) or the explicit `detail.no_spend` mark. A refusal
/// with none of them is from a producer that does not follow this rule;
/// the caller applies its own policy to it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum RefusalSpend {
    /// A provider call was made and its billed cost was measured
    /// (`detail.cost_usd`, a non-negative finite number). The caller charges
    /// exactly the carried cost.
    Measured(f64),
    /// The caller charges the request's `max_cost_usd`. This is the spend
    /// for `cost_unmeasured` (the code itself, carrying no spend members)
    /// and for a refusal marked `detail.cost_unknown`.
    ChargeCeiling,
    /// The refusal is explicitly marked `detail.no_spend`, so the caller
    /// charges nothing.
    NoSpend,
    /// The refusal carries no spend marker at all: its producer does not
    /// follow this rule, so the caller cannot tell a no-spend refusal from
    /// an unmarked one. The caller applies its own policy to it.
    Undeclared,
}

/// Why a refusal's spend members are invalid, as [`refusal_spend`] reports
/// it. A producer must never emit these shapes; a caller treats a refusal
/// that carries one as a malformed reply, not as a billed outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpendViolation {
    /// `detail` carries two spend members together. A refusal states at
    /// most one of `cost_usd`, `cost_unknown` and `no_spend`.
    BothPresent,
    /// `detail.cost_unknown` is present but is not `true`. Only `true` is
    /// ever a valid mark; an explicit `false` states nothing.
    UnknownNotTrue,
    /// `detail.no_spend` is present but is not `true`. Only `true` is ever
    /// a valid mark; an explicit `false` states nothing.
    NoSpendNotTrue,
    /// `detail.cost_usd` is not a non-negative finite number.
    InvalidCost,
    /// `cost_unmeasured` carries `cost_usd`, `cost_unknown` or `no_spend`.
    /// The code itself already states unmeasured spend at the ceiling, so it
    /// carries no spend members.
    UnmeasuredWithSpend,
}

impl fmt::Display for SpendViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BothPresent => write!(
                f,
                "refusal carries more than one of `{}`, `{}` and `{}`",
                COST_USD_KEY, COST_UNKNOWN_KEY, NO_SPEND_KEY
            ),
            Self::UnknownNotTrue => write!(
                f,
                "refusal carries `{}` with a value other than `true`",
                COST_UNKNOWN_KEY
            ),
            Self::NoSpendNotTrue => write!(
                f,
                "refusal carries `{}` with a value other than `true`",
                NO_SPEND_KEY
            ),
            Self::InvalidCost => write!(
                f,
                "refusal carries `{}` that is not a non-negative finite number",
                COST_USD_KEY
            ),
            Self::UnmeasuredWithSpend => write!(f, "`{COST_UNMEASURED}` carries a spend member"),
        }
    }
}

impl std::error::Error for SpendViolation {}

/// Read what the caller charges for a refusal of any of the three tools.
///
/// The serving module marks every refusal with exactly one spend marker:
/// measured (`detail.cost_usd`, charged exactly), `cost_unmeasured` (the
/// code itself, charged at the request's `max_cost_usd`), unknown
/// (`detail.cost_unknown` is `true`, charged at `max_cost_usd`), or no
/// spend (`detail.no_spend` is `true`, charged nothing). A serving module
/// that cannot prove a failure happened before anything reached the
/// provider marks it unknown rather than free: spend fails toward charging,
/// never toward free.
///
/// A refusal with no marker at all gives [`RefusalSpend::Undeclared`]: its
/// producer does not follow this rule, so the caller applies its own
/// policy to it.
///
/// Returns `Err` for the contract's invalid shapes: two markers together,
/// `cost_unknown` or `no_spend` other than `true`, a `cost_usd` that is not
/// a non-negative finite number, or `cost_unmeasured` carrying a spend
/// member.
///
/// A repeat with the same call key returns the same refusal with the same
/// spend members, and is never billed twice.
pub fn refusal_spend(refusal: &Refusal) -> Result<RefusalSpend, SpendViolation> {
    let detail = &refusal.detail;
    let markers = usize::from(refusal.code == COST_UNMEASURED)
        + usize::from(detail.cost_usd.is_some())
        + usize::from(detail.cost_unknown.is_some())
        + usize::from(detail.no_spend.is_some());
    if markers > 1 {
        if refusal.code == COST_UNMEASURED {
            return Err(SpendViolation::UnmeasuredWithSpend);
        }
        return Err(SpendViolation::BothPresent);
    }
    if refusal.code == COST_UNMEASURED {
        return Ok(RefusalSpend::ChargeCeiling);
    }
    if let Some(cost) = detail.cost_usd {
        if !cost.is_finite() || cost < 0.0 {
            return Err(SpendViolation::InvalidCost);
        }
        return Ok(RefusalSpend::Measured(cost));
    }
    if let Some(unknown) = detail.cost_unknown {
        if unknown {
            return Ok(RefusalSpend::ChargeCeiling);
        }
        return Err(SpendViolation::UnknownNotTrue);
    }
    if let Some(free) = detail.no_spend {
        if free {
            return Ok(RefusalSpend::NoSpend);
        }
        return Err(SpendViolation::NoSpendNotTrue);
    }
    Ok(RefusalSpend::Undeclared)
}
