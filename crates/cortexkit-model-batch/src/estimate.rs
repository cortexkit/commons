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

/// A price in integer nanodollars per million tokens. It is the same type as
/// the model catalog's rate (`cortexkit-model-catalog`'s `RateNanosPerMtok`,
/// an `i64`), so a catalog price passes in as it is, without this crate
/// depending on the catalog.
pub type RateNanosPerMtok = i64;

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
    /// The item's state, billed on every pass.
    pub state: Tokens,
    /// The schema sent with every item (fields or questions), billed once
    /// across all passes.
    pub schema: Tokens,
    /// How many images the item carries.
    pub images: u64,
    /// The module's bound on the tokens one image costs. Required; a
    /// module that does not know it states a bound, never zero.
    pub image_tokens_each: u64,
    /// The module's bound on its per-request overhead (prompt template,
    /// system text). Required; never defaulted to zero.
    pub overhead_tokens: u64,
    /// The module's upper bound on how many passes the provider makes for
    /// the item, at least 1. 1 for a provider that answers in one pass.
    pub max_passes: u64,
    /// The provider's per-pass token budget, when it states one. Used only
    /// to check that an exactly counted state fits in a pass; it does not
    /// decide how many passes are billed.
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
            max_passes: 1,
            pass_tokens: None,
        }
    }

    pub fn with_images(mut self, images: u64) -> Self {
        self.images = images;
        self
    }

    /// The module's upper bound on the passes the provider makes. Zero is
    /// refused when the item is estimated ([`EstimateError::NoPasses`]).
    pub fn with_max_passes(mut self, max_passes: u64) -> Self {
        self.max_passes = max_passes;
        self
    }

    pub fn with_pass_tokens(mut self, pass_tokens: u64) -> Self {
        self.pass_tokens = Some(pass_tokens);
        self
    }
}

/// An item's estimate: an upper bound on its input cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Estimate {
    /// The passes the estimate bills: the input's `max_passes`.
    pub max_passes: u64,
    /// Input tokens billed over every pass.
    pub input_tokens: u64,
    /// Their cost, rounded up to a whole nanodollar.
    pub cost_nanos: u64,
}

impl Estimate {
    pub fn new(max_passes: u64, input_tokens: u64, cost_nanos: u64) -> Self {
        Self {
            max_passes,
            input_tokens,
            cost_nanos,
        }
    }
}

/// Why an item could not be estimated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstimateError {
    /// The state's exact count reaches the pass budget: the item cannot fit
    /// in a pass, and is refused `invalid_item` before any call.
    DoesNotFit { state_tokens: u64, pass_tokens: u64 },
    /// `max_passes` is zero: every provider call makes at least one pass,
    /// so the module's bound is wrong, not the item.
    NoPasses,
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
            Self::NoPasses => {
                f.write_str("max_passes is zero; a provider call makes at least one pass")
            }
            Self::Overflow => f.write_str("the cost estimate overflows"),
            Self::NegativeRate => f.write_str("the catalog rate is negative"),
        }
    }
}

impl std::error::Error for EstimateError {}

/// The cost of `tokens` input tokens at `rate` nanodollars per million,
/// rounded up to a whole nanodollar.
pub fn cost_nanos(tokens: u64, rate: RateNanosPerMtok) -> Result<u64, EstimateError> {
    let rate = u64::try_from(rate).map_err(|_| EstimateError::NegativeRate)?;
    let product = u128::from(tokens) * u128::from(rate);
    let nanos = product.div_ceil(u128::from(TOKENS_PER_MTOK));
    u64::try_from(nanos).map_err(|_| EstimateError::Overflow)
}

/// Estimate one item's input cost (`RULES.md` §5):
///
/// ```text
/// input tokens = schema + max_passes x (state + images x image_tokens_each + overhead)
/// cost         = input tokens x input rate
/// ```
///
/// The schema bills once across all passes and the state bills on every
/// pass, as Cloudflare's Clairic was measured to bill. Images and overhead
/// are charged on every pass too, which can only overstate the cost.
///
/// `pass_tokens` decides nothing here but the fit check: a state counted
/// exactly at or past it is [`EstimateError::DoesNotFit`]. A byte-bound
/// state is estimated whatever its size, since the formula bounds its cost
/// whether the provider refuses the state or truncates it.
pub fn estimate_item(
    input: &ItemCostInput,
    input_rate: RateNanosPerMtok,
) -> Result<Estimate, EstimateError> {
    let state = input.state.count();
    if let (Some(pass_tokens), true) = (input.pass_tokens, input.state.is_exact()) {
        if state >= pass_tokens {
            return Err(EstimateError::DoesNotFit {
                state_tokens: state,
                pass_tokens,
            });
        }
    }
    if input.max_passes == 0 {
        return Err(EstimateError::NoPasses);
    }
    let image_tokens = input
        .images
        .checked_mul(input.image_tokens_each)
        .ok_or(EstimateError::Overflow)?;
    let per_pass = [image_tokens, input.overhead_tokens]
        .into_iter()
        .try_fold(state, u64::checked_add)
        .ok_or(EstimateError::Overflow)?;
    let input_tokens = per_pass
        .checked_mul(input.max_passes)
        .and_then(|passes| passes.checked_add(input.schema.count()))
        .ok_or(EstimateError::Overflow)?;
    Ok(Estimate {
        max_passes: input.max_passes,
        input_tokens,
        cost_nanos: cost_nanos(input_tokens, input_rate)?,
    })
}

/// Whether an item estimated at `estimate` would cross `ceiling_nanos` after
/// `spent_nanos` already recorded for the batch. A sum that overflows
/// crosses any ceiling.
pub fn crosses_ceiling(spent_nanos: u64, estimate: &Estimate, ceiling_nanos: u64) -> bool {
    spent_nanos
        .checked_add(estimate.cost_nanos)
        .is_none_or(|total| total > ceiling_nanos)
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

    /// Cloudflare's Clairic, measured live on 2026-10-22 (probe 05): 40
    /// text fields ran in 40 passes and billed `input_tokens` 29,888 for a
    /// schema of 26,488 tokens and a state of 85, with no images.
    fn probe_05(state: Tokens, schema: Tokens) -> ItemCostInput {
        ItemCostInput::new(state, schema, 0, 0)
            .with_max_passes(40)
            .with_pass_tokens(8192)
    }

    #[test]
    fn exact_counts_estimate_what_clairic_billed_for_probe_05() {
        let input = probe_05(Tokens::Exact(85), Tokens::Exact(26_488));
        // 26,488 + 40 x 85: the schema once, the state on every pass.
        assert_eq!(
            estimate_item(&input, 110_000_000),
            Ok(Estimate::new(40, 29_888, 3_287_680))
        );
    }

    #[test]
    fn byte_bounds_never_underestimate_probe_05() {
        // The UTF-8 byte lengths of probe 05's fields (as sent, and as
        // canonical JSON: the same 51,089 bytes) and of its state.
        let input = probe_05(Tokens::Bound(231), Tokens::Bound(51_089));
        let estimate = estimate_item(&input, 110_000_000).unwrap();
        assert_eq!(estimate.input_tokens, 51_089 + 40 * 231);
        assert!(estimate.input_tokens >= 29_888);
    }

    #[test]
    fn a_large_state_over_many_passes_bills_on_every_pass() {
        // 40 small fields and a large state. Deriving passes from the pass
        // budget gives ceil(400 / (8192 - 4000)) = 1 pass and bills
        // (4000 + 400) x 1 = 4,400 tokens, while a provider that runs each
        // field in its own pass bills the state 40 times.
        let input = ItemCostInput::new(Tokens::Bound(4000), Tokens::Bound(400), 0, 0)
            .with_max_passes(40)
            .with_pass_tokens(8192);
        let estimate = estimate_item(&input, 1_000).unwrap();
        assert_eq!(estimate.input_tokens, 400 + 40 * 4000);
        assert!(estimate.input_tokens > 4_400);
    }

    #[test]
    fn images_and_overhead_bill_on_every_pass() {
        let input = ItemCostInput::new(Tokens::Bound(1000), Tokens::Bound(9000), 300, 50)
            .with_images(2)
            .with_max_passes(2)
            .with_pass_tokens(8192);
        // 9000 + 2 x (1000 + 2 x 300 + 50).
        assert_eq!(
            estimate_item(&input, 110_000_000),
            Ok(Estimate::new(2, 12_300, 1_353_000))
        );
        let one_pass = ItemCostInput::new(Tokens::Bound(1000), Tokens::Bound(9000), 300, 50);
        assert_eq!(
            one_pass.max_passes, 1,
            "one pass unless the module says more"
        );
        assert_eq!(estimate_item(&one_pass, 0).unwrap().input_tokens, 10_050);
    }

    #[test]
    fn a_byte_bound_past_the_pass_is_estimated_and_an_exact_count_does_not_fit() {
        let bound = ItemCostInput::new(Tokens::Bound(9000), Tokens::Bound(10), 0, 0)
            .with_max_passes(3)
            .with_pass_tokens(8192);
        assert_eq!(
            estimate_item(&bound, 1000),
            Ok(Estimate::new(3, 27_010, 28))
        );
        let exact =
            ItemCostInput::new(Tokens::Exact(9000), Tokens::Bound(10), 0, 0).with_pass_tokens(8192);
        assert_eq!(
            estimate_item(&exact, 1000),
            Err(EstimateError::DoesNotFit {
                state_tokens: 9000,
                pass_tokens: 8192
            })
        );
        let at_the_budget =
            ItemCostInput::new(Tokens::Exact(8192), Tokens::Bound(10), 0, 0).with_pass_tokens(8192);
        assert!(matches!(
            estimate_item(&at_the_budget, 1000),
            Err(EstimateError::DoesNotFit { .. })
        ));
    }

    #[test]
    fn zero_passes_are_refused() {
        let input = ItemCostInput::new(Tokens::Exact(1), Tokens::Exact(1), 0, 0).with_max_passes(0);
        assert_eq!(estimate_item(&input, 1), Err(EstimateError::NoPasses));
    }

    #[test]
    fn overflow_refuses() {
        let input = ItemCostInput::new(Tokens::Exact(u64::MAX), Tokens::Exact(1), 0, 0);
        assert_eq!(estimate_item(&input, 1), Err(EstimateError::Overflow));
        let passes =
            ItemCostInput::new(Tokens::Exact(2), Tokens::Exact(1), 0, 0).with_max_passes(u64::MAX);
        assert_eq!(estimate_item(&passes, 1), Err(EstimateError::Overflow));
        assert_eq!(cost_nanos(1, -1), Err(EstimateError::NegativeRate));
        assert!(crosses_ceiling(u64::MAX, &Estimate::new(1, 1, 1), u64::MAX));
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
