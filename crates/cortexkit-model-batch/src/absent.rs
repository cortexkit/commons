//! Absent is not zero (`RULES.md` §10): a count or price the provider or
//! catalog did not state stays absent, in each item and in every sum.

use crate::estimate::{cost_nanos, EstimateError, RateNanosPerMtok};

/// `total + count`, where an absent count adds nothing and two absent
/// counts stay absent. Saturates rather than wrapping.
pub fn add_reported(total: Option<u64>, count: Option<u64>) -> Option<u64> {
    match (total, count) {
        (Some(total), Some(count)) => Some(total.saturating_add(count)),
        (total, None) => total,
        (None, count) => count,
    }
}

/// The sum of the counts that were reported; absent when none was.
pub fn sum_reported(counts: impl IntoIterator<Item = Option<u64>>) -> Option<u64> {
    counts.into_iter().fold(None, add_reported)
}

/// A batch's input-priced cost in nanodollars: every answered item's
/// reported `input_tokens` at `rate`. Absent when the model has no input
/// rate, or when any answered item did not report its input tokens: a
/// partial sum would read as the whole cost.
pub fn input_cost_nanos(
    answered_input_tokens: impl IntoIterator<Item = Option<u64>>,
    rate: Option<RateNanosPerMtok>,
) -> Result<Option<u64>, EstimateError> {
    let Some(rate) = rate else {
        return Ok(None);
    };
    let mut total: u64 = 0;
    for tokens in answered_input_tokens {
        let Some(tokens) = tokens else {
            return Ok(None);
        };
        total = total.checked_add(tokens).ok_or(EstimateError::Overflow)?;
    }
    cost_nanos(total, rate).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_counts_stay_absent() {
        assert_eq!(sum_reported([None, None]), None);
        assert_eq!(sum_reported([None, Some(5), None]), Some(5));
        assert_eq!(sum_reported([Some(0)]), Some(0));
        assert_eq!(add_reported(Some(u64::MAX), Some(1)), Some(u64::MAX));
    }

    #[test]
    fn a_missing_input_count_makes_the_cost_absent() {
        assert_eq!(
            input_cost_nanos([Some(10), Some(20)], Some(1_000_000)),
            Ok(Some(30))
        );
        assert_eq!(
            input_cost_nanos([Some(10), None], Some(1_000_000)),
            Ok(None)
        );
        assert_eq!(input_cost_nanos([Some(10)], None), Ok(None));
        assert_eq!(input_cost_nanos([Some(10)], Some(0)), Ok(Some(0)));
    }
}
