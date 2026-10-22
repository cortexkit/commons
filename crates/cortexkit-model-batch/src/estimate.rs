//! The conservative cost estimate a runner checks against a batch's
//! `max_cost_usd` before each provider call, and the money arithmetic it
//! needs (`RULES.md` §5).
//!
//! Text tokens are bounded by UTF-8 byte length: a tokenizer that works on
//! bytes never makes more tokens than there are bytes, so the bound never
//! underestimates. A module that has the provider's tokenizer may pass
//! exact counts instead ([`Tokens::Exact`]). The fixed costs a role cannot
//! see (the module's prompt template, the tokens an image costs) are
//! required inputs, never defaulted to zero. Money is integer nanodollars,
//! the catalog's own unit ([`RateNanosPerMtok`]), with checked arithmetic:
//! an overflow refuses, it never wraps.

use serde::Serialize;
use serde_json::Number;

pub use cortexkit_model_catalog::RateNanosPerMtok;

use crate::identity::{canonical_json, IdentityError};

/// Nanodollars in one dollar.
pub const NANOS_PER_USD: u64 = 1_000_000_000;

/// Tokens in the unit a rate is priced per (one million).
pub const TOKENS_PER_MTOK: u64 = 1_000_000;

/// A token count for one part of a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tokens {
    /// Counted with the provider's own tokenizer.
    Exact(u64),
    /// An upper bound, from the part's byte length.
    Bound(u64),
}

impl Tokens {
    /// The byte bound of `text`.
    pub fn bound_of_text(text: &str) -> Self {
        Self::Bound(text.len() as u64)
    }

    /// The byte bound of `value`'s RFC 8785 (JCS) canonical JSON: the bound
    /// of a schema (a role's fields or questions) as the provider receives
    /// it.
    pub fn bound_of_json<T: Serialize + ?Sized>(value: &T) -> Result<Self, IdentityError> {
        Ok(Self::Bound(canonical_json(value)?.len() as u64))
    }

    pub fn count(self) -> u64 {
        match self {
            Self::Exact(count) | Self::Bound(count) => count,
        }
    }

    pub fn is_exact(self) -> bool {
        matches!(self, Self::Exact(_))
    }
}

/// What one item's provider call is estimated from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemCostInput {
    /// The item's state.
    pub state: Tokens,
    /// The schema sent with every item (fields or questions).
    pub schema: Tokens,
    /// How many images the item carries.
    pub images: u64,
    /// The module's bound on the tokens one image costs. Required; a
    /// module that does not know it states a bound, never zero.
    pub image_tokens_each: u64,
    /// The module's bound on its per-request overhead (prompt template,
    /// system text). Required; never defaulted to zero.
    pub overhead_tokens: u64,
    /// The provider's per-pass token budget, when it splits a schema into
    /// passes; `None` for a provider that answers in one pass.
    pub pass_tokens: Option<u64>,
}

impl ItemCostInput {
    pub fn new(
        state: Tokens,
        schema: Tokens,
        image_tokens_each: u64,
        overhead_tokens: u64,
    ) -> Self {
        Self {
            state,
            schema,
            images: 0,
            image_tokens_each,
            overhead_tokens,
            pass_tokens: None,
        }
    }

    pub fn with_images(mut self, images: u64) -> Self {
        self.images = images;
        self
    }

    pub fn with_pass_tokens(mut self, pass_tokens: u64) -> Self {
        self.pass_tokens = Some(pass_tokens);
        self
    }
}

/// An item's estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Estimate {
    /// An upper bound on the item's input cost.
    Bounded {
        /// Passes the provider is assumed to make, at least 1.
        passes: u64,
        /// Input tokens billed over every pass.
        input_tokens: u64,
        /// Their cost, rounded up to a whole nanodollar.
        cost_nanos: u64,
    },
    /// The state's byte bound reaches a pass's token budget, so the number
    /// of passes, and with it the cost, has no upper bound. Against a
    /// ceiling the item is `cost_exceeded`; with no ceiling it is sent.
    Unbounded,
}

/// Why an item could not be estimated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstimateError {
    /// The state's exact count reaches the pass budget: the item cannot fit
    /// in a pass, and is refused `invalid_item` before any call.
    DoesNotFit { state_tokens: u64, pass_tokens: u64 },
    /// The arithmetic overflowed. The item is refused; a runner never wraps.
    Overflow,
    /// The catalog rate is negative, which no catalog publishes.
    NegativeRate,
}

impl std::fmt::Display for EstimateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DoesNotFit {
                state_tokens,
                pass_tokens,
            } => write!(
                f,
                "the state is {state_tokens} tokens; a pass holds {pass_tokens}"
            ),
            Self::Overflow => f.write_str("the cost estimate overflows"),
            Self::NegativeRate => f.write_str("the catalog rate is negative"),
        }
    }
}

impl std::error::Error for EstimateError {}

/// How many passes the provider makes for a schema of `schema` tokens with a
/// state of `state` tokens, at `pass_tokens` per pass:
/// `ceil(schema / (pass_tokens - state))`, at least 1. `None` when the state
/// leaves no room in a pass (`state >= pass_tokens`).
pub fn passes(schema: u64, state: u64, pass_tokens: u64) -> Option<u64> {
    let room = pass_tokens.checked_sub(state).filter(|room| *room > 0)?;
    Some(schema.div_ceil(room).max(1))
}

/// The cost of `tokens` input tokens at `rate` nanodollars per million,
/// rounded up to a whole nanodollar.
pub fn cost_nanos(tokens: u64, rate: RateNanosPerMtok) -> Result<u64, EstimateError> {
    let rate = u64::try_from(rate).map_err(|_| EstimateError::NegativeRate)?;
    let product = u128::from(tokens) * u128::from(rate);
    let nanos = product.div_ceil(u128::from(TOKENS_PER_MTOK));
    u64::try_from(nanos).map_err(|_| EstimateError::Overflow)
}

/// Estimate one item's input cost: `(state + schema + image tokens +
/// overhead) x passes x input rate`. Each pass is assumed to bill the whole
/// request again, which is the conservative reading while the provider's
/// own billing of later passes is not yet measured.
pub fn estimate_item(
    input: &ItemCostInput,
    input_rate: RateNanosPerMtok,
) -> Result<Estimate, EstimateError> {
    let state = input.state.count();
    let schema = input.schema.count();
    let passes = match input.pass_tokens {
        None => 1,
        Some(pass_tokens) => match passes(schema, state, pass_tokens) {
            Some(passes) => passes,
            None if input.state.is_exact() => {
                return Err(EstimateError::DoesNotFit {
                    state_tokens: state,
                    pass_tokens,
                })
            }
            None => return Ok(Estimate::Unbounded),
        },
    };
    let image_tokens = input
        .images
        .checked_mul(input.image_tokens_each)
        .ok_or(EstimateError::Overflow)?;
    let per_pass = [schema, image_tokens, input.overhead_tokens]
        .into_iter()
        .try_fold(state, u64::checked_add)
        .ok_or(EstimateError::Overflow)?;
    let input_tokens = per_pass
        .checked_mul(passes)
        .ok_or(EstimateError::Overflow)?;
    Ok(Estimate::Bounded {
        passes,
        input_tokens,
        cost_nanos: cost_nanos(input_tokens, input_rate)?,
    })
}

/// Whether an item estimated at `estimate` would cross `ceiling_nanos` after
/// `spent_nanos` already recorded for the batch. An unbounded estimate, or a
/// sum that overflows, crosses any ceiling.
pub fn crosses_ceiling(spent_nanos: u64, estimate: &Estimate, ceiling_nanos: u64) -> bool {
    match estimate {
        Estimate::Unbounded => true,
        Estimate::Bounded { cost_nanos, .. } => spent_nanos
            .checked_add(*cost_nanos)
            .is_none_or(|total| total > ceiling_nanos),
    }
}

/// Which way [`usd_to_nanos`] rounds a value finer than a nanodollar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// Toward zero: for a ceiling, so it is never raised.
    Down,
    /// Away from zero: for a cost, so it is never lowered.
    Up,
}

/// A JSON number of USD as whole nanodollars, by decimal string scaling of
/// the number's text (never through floating-point arithmetic). `None` for
/// a negative number or one too large for `u64` nanodollars.
pub fn usd_to_nanos(usd: &Number, rounding: Rounding) -> Option<u64> {
    decimal_to_nanos(&usd.to_string(), rounding)
}

fn decimal_to_nanos(text: &str, rounding: Rounding) -> Option<u64> {
    if let Some(magnitude) = text.strip_prefix('-') {
        // Only a negative zero is a valid amount.
        let zero = magnitude
            .split(['e', 'E'])
            .next()
            .is_some_and(|mantissa| mantissa.bytes().all(|b| matches!(b, b'0' | b'.')));
        return zero.then_some(0);
    }
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(at) => (&text[..at], text[at + 1..].parse::<i64>().ok()?),
        None => (text, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits: String = format!("{int}{frac}");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some(0);
    }
    // value = digits x 10^(exponent - frac.len()); nanos = value x 10^9.
    let scale = exponent.checked_sub(frac.len() as i64)?.checked_add(9)?;
    let whole: u128 = if digits.len() > 38 {
        return None;
    } else {
        digits.parse().ok()?
    };
    let nanos = if scale >= 0 {
        let factor = 10u128.checked_pow(u32::try_from(scale).ok()?)?;
        whole.checked_mul(factor)?
    } else {
        let shift = u32::try_from(-scale).ok()?;
        match 10u128.checked_pow(shift) {
            Some(divisor) => {
                let (quotient, remainder) = (whole / divisor, whole % divisor);
                if rounding == Rounding::Up && remainder > 0 {
                    quotient + 1
                } else {
                    quotient
                }
            }
            // Smaller than any nanodollar fraction `u128` can hold.
            None => u128::from(rounding == Rounding::Up),
        }
    };
    u64::try_from(nanos).ok()
}

/// Whole nanodollars as an exact decimal number of USD, for a message:
/// `0.00110396`, never an exponent form.
pub fn nanos_to_usd_text(nanos: u64) -> String {
    let whole = nanos / NANOS_PER_USD;
    let frac = nanos % NANOS_PER_USD;
    if frac == 0 {
        format!("{whole}")
    } else {
        let frac = format!("{frac:09}");
        format!("{whole}.{}", frac.trim_end_matches('0'))
    }
}

/// Whole nanodollars as a JSON number of USD, for a refusal's
/// `estimate_usd`: the exact decimal text, read as a JSON number.
pub fn nanos_to_usd(nanos: u64) -> Number {
    serde_json::from_str(&nanos_to_usd_text(nanos)).expect("a plain decimal is a JSON number")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(text: &str) -> Number {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn passes_are_at_least_one_and_never_divide_by_zero() {
        assert_eq!(passes(0, 10, 100), Some(1));
        assert_eq!(passes(90, 10, 100), Some(1));
        assert_eq!(passes(91, 10, 100), Some(2));
        assert_eq!(passes(180, 10, 100), Some(2));
        assert_eq!(passes(181, 10, 100), Some(3));
        assert_eq!(passes(1, 100, 100), None);
        assert_eq!(passes(1, 200, 100), None);
    }

    #[test]
    fn a_byte_bound_past_the_pass_is_unbounded_and_an_exact_count_does_not_fit() {
        let bound =
            ItemCostInput::new(Tokens::Bound(9000), Tokens::Bound(10), 0, 0).with_pass_tokens(8192);
        assert_eq!(estimate_item(&bound, 1000), Ok(Estimate::Unbounded));
        let exact =
            ItemCostInput::new(Tokens::Exact(9000), Tokens::Bound(10), 0, 0).with_pass_tokens(8192);
        assert_eq!(
            estimate_item(&exact, 1000),
            Err(EstimateError::DoesNotFit {
                state_tokens: 9000,
                pass_tokens: 8192
            })
        );
        assert!(crosses_ceiling(0, &Estimate::Unbounded, u64::MAX));
    }

    #[test]
    fn every_pass_bills_the_whole_request() {
        let input = ItemCostInput::new(Tokens::Bound(1000), Tokens::Bound(9000), 300, 50)
            .with_images(2)
            .with_pass_tokens(8192);
        // passes = ceil(9000 / 7192) = 2; per pass 1000 + 9000 + 600 + 50.
        assert_eq!(
            estimate_item(&input, 110_000_000),
            Ok(Estimate::Bounded {
                passes: 2,
                input_tokens: 21_300,
                cost_nanos: 2_343_000,
            })
        );
    }

    #[test]
    fn overflow_refuses() {
        let input = ItemCostInput::new(Tokens::Exact(u64::MAX), Tokens::Exact(1), 0, 0);
        assert_eq!(estimate_item(&input, 1), Err(EstimateError::Overflow));
        assert_eq!(cost_nanos(1, -1), Err(EstimateError::NegativeRate));
        assert!(crosses_ceiling(
            u64::MAX,
            &Estimate::Bounded {
                passes: 1,
                input_tokens: 1,
                cost_nanos: 1
            },
            u64::MAX
        ));
    }

    #[test]
    fn cost_rounds_up_to_a_nanodollar() {
        assert_eq!(cost_nanos(1, 1), Ok(1));
        assert_eq!(cost_nanos(1_000_000, 3), Ok(3));
        assert_eq!(cost_nanos(0, 5), Ok(0));
    }

    #[test]
    fn usd_scales_by_decimal_text() {
        assert_eq!(
            usd_to_nanos(&num("0.25"), Rounding::Down),
            Some(250_000_000)
        );
        assert_eq!(usd_to_nanos(&num("1e-7"), Rounding::Down), Some(100));
        assert_eq!(usd_to_nanos(&num("3"), Rounding::Down), Some(3_000_000_000));
        assert_eq!(usd_to_nanos(&num("1e-10"), Rounding::Down), Some(0));
        assert_eq!(usd_to_nanos(&num("1e-10"), Rounding::Up), Some(1));
        assert_eq!(usd_to_nanos(&num("0"), Rounding::Up), Some(0));
        assert_eq!(usd_to_nanos(&num("-1"), Rounding::Down), None);
        assert_eq!(usd_to_nanos(&num("1e300"), Rounding::Down), None);
        assert_eq!(nanos_to_usd(420_000_000).to_string(), "0.42");
        assert_eq!(nanos_to_usd(3_000_000_000).to_string(), "3");
        assert_eq!(nanos_to_usd(2_343_000).to_string(), "0.002343");
        assert_eq!(nanos_to_usd_text(36_960), "0.00003696");
    }
}
