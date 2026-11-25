//! The pre-call spend check for one item (`CONTRACT.md` §6.9): the shared
//! conservative estimate (`cortexkit-model-batch`, its `RULES.md` §5) with
//! this role's inputs, and the item error an item gets when it is not sent.
//!
//! Clairic bills the schema once and the item's state and images on every
//! model call. The role bounds the number of model calls by the number of
//! fields; no measured reply has needed more calls than fields. The
//! provider's reported `passes` is not a count of model calls.

use cortexkit_model_batch::estimate::{
    crosses_ceiling, estimate_item, Estimate, EstimateError, ItemCostInput, RateNanosPerMtok,
    Tokens,
};
pub use cortexkit_model_batch::{
    absent::input_cost_nanos,
    estimate::{
        cost_nanos, nanos_to_usd, nanos_to_usd_text, usd_to_nanos, Rounding, Tokens as TokenCount,
    },
};

use crate::{
    describe::ModelEntry,
    errors::{ItemError, COST_EXCEEDED, INVALID_ITEM},
    identity::IdentityError,
    request::ExtractRequest,
};

/// The bounds a module supplies for what the role cannot see. All are
/// required; none defaults to zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleBounds {
    /// The tokens one image costs at most in the schema and separately on
    /// each model call. The bound must cover the largest image admitted.
    pub image_tokens_each: u64,
    /// The upper bound on the provider's schema text for one field or image.
    /// With an exact fields count rather than their byte bound, this must
    /// cover more of the provider's reported schema tokens.
    pub schema_tokens_each: u64,
    /// The module's per-model-call overhead: its prompt template and any
    /// system text, in tokens.
    pub overhead_tokens: u64,
}

/// The input to an extractor item's pre-call spend check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemSpendInput {
    /// All inputs to the shared estimator are representable.
    Estimable(ItemCostInput),
    /// The schema bound overflowed. The state still needs its fit check.
    SchemaOverflow { state: Tokens, pass_tokens: u64 },
}

/// The estimate input for item `index` of `request` on `model`: the
/// state's and the fields' byte lengths, unless the module supplies exact
/// counts (`exact_state`, `exact_schema`). The schema also bounds the
/// provider's text for each field and image, and one copy of the image
/// tokens. The request's field count bounds model calls (`max_passes` in
/// the shared estimator). Returns an error for a missing item, no fields,
/// or fields that cannot be canonicalized.
pub fn item_cost_input(
    request: &ExtractRequest,
    index: usize,
    model: &ModelEntry,
    bounds: ModuleBounds,
    exact_state: Option<u64>,
    exact_schema: Option<u64>,
) -> Result<ItemSpendInput, IdentityError> {
    let item = request
        .items
        .get(index)
        .ok_or_else(|| IdentityError(format!("there is no item {index}")))?;
    let state = exact_state.map_or_else(|| Tokens::bound_of_text(&item.state), Tokens::Exact);
    let fields = match exact_schema {
        Some(count) => Tokens::Exact(count),
        None => Tokens::bound_of_json(&request.fields)?,
    };
    let max_calls = request.fields.len() as u64;
    if max_calls == 0 {
        return Err(IdentityError("a batch has at least one field".into()));
    }
    let images = item.images.as_ref().map_or(0, |images| images.len() as u64);
    let schema_count = max_calls
        .checked_add(images)
        .and_then(|entries| entries.checked_mul(bounds.schema_tokens_each))
        .and_then(|text| fields.count().checked_add(text))
        .and_then(|count| count.checked_add(images.checked_mul(bounds.image_tokens_each)?));
    let Some(schema_count) = schema_count else {
        return Ok(ItemSpendInput::SchemaOverflow {
            state,
            pass_tokens: model.pass_tokens,
        });
    };
    let schema = if schema_count == fields.count() {
        fields
    } else {
        Tokens::Bound(schema_count)
    };
    Ok(ItemSpendInput::Estimable(
        ItemCostInput::new(
            state,
            schema,
            bounds.image_tokens_each,
            bounds.overhead_tokens,
        )
        .with_images(images)
        .with_max_passes(max_calls)
        .with_pass_tokens(model.pass_tokens),
    ))
}

/// Decide whether an item may be sent, before its provider call.
///
/// - The state's exact count reaches `pass_tokens`: the item cannot fit in
///   a pass, `invalid_item`.
/// - With a ceiling (`ceiling_nanos`) and a priced model (`input_rate`): an
///   item whose estimate would cross the ceiling after `spent_nanos` is
///   `cost_exceeded`. An estimate that overflows, or one the module's
///   bounds make impossible (zero model calls), crosses any ceiling. A state
///   whose byte bound reaches `pass_tokens` is estimated like any other.
/// - Otherwise the item is sent, and the estimate is returned when there
///   is one.
// The error is the item's own outcome, returned by value so a runner can
// store it as is; it is built once per item, so its size does not matter.
#[allow(clippy::result_large_err)]
pub fn check_before_call(
    input: &ItemSpendInput,
    input_rate: Option<RateNanosPerMtok>,
    spent_nanos: u64,
    ceiling_nanos: Option<u64>,
) -> Result<Option<Estimate>, ItemError> {
    let estimate = match input {
        ItemSpendInput::Estimable(input) => estimate_item(input, input_rate.unwrap_or(0)),
        ItemSpendInput::SchemaOverflow {
            state: Tokens::Exact(state_tokens),
            pass_tokens,
        } if state_tokens >= pass_tokens => Err(EstimateError::DoesNotFit {
            state_tokens: *state_tokens,
            pass_tokens: *pass_tokens,
        }),
        ItemSpendInput::SchemaOverflow { .. } => Err(EstimateError::Overflow),
    };
    if let Err(EstimateError::DoesNotFit {
        state_tokens,
        pass_tokens,
    }) = estimate
    {
        return Err(ItemError::for_code(
            INVALID_ITEM,
            format!(
                "the state is {state_tokens} tokens; one pass holds {pass_tokens}, so the item cannot fit"
            ),
        ));
    }
    let (Some(ceiling), Some(_)) = (ceiling_nanos, input_rate) else {
        return Ok(estimate.ok());
    };
    match estimate {
        Ok(estimate) if crosses_ceiling(spent_nanos, &estimate, ceiling) => {
            Err(ItemError::for_code(
                COST_EXCEEDED,
                format!(
                    "answering this item could cost {} USD, crossing max_cost_usd {} after {} USD spent",
                    nanos_to_usd_text(estimate.cost_nanos),
                    nanos_to_usd_text(ceiling),
                    nanos_to_usd_text(spent_nanos),
                ),
            ))
        }
        Ok(estimate) => Ok(Some(estimate)),
        Err(error) => Err(ItemError::for_code(
            COST_EXCEEDED,
            format!("the item cannot be estimated ({error}), so it crosses any ceiling"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::ErrorClass,
        field::{Field, FieldType},
        request::Item,
    };
    use indexmap::IndexMap;

    fn input(state: TokenCount) -> ItemSpendInput {
        ItemSpendInput::Estimable(
            ItemCostInput::new(state, Tokens::Bound(100), 0, 50).with_pass_tokens(8192),
        )
    }

    #[test]
    fn an_exact_state_past_the_pass_is_invalid_with_or_without_a_ceiling() {
        for ceiling in [None, Some(1_000_000)] {
            let error =
                check_before_call(&input(Tokens::Exact(9000)), Some(110_000_000), 0, ceiling)
                    .unwrap_err();
            assert_eq!(error.code, INVALID_ITEM);
            assert!(error.message.contains("8192"), "{}", error.message);
        }
    }

    #[test]
    fn a_byte_bound_past_the_pass_is_estimated_like_any_state() {
        let long = input(Tokens::Bound(10_000));
        // 100 + 1 x (10,000 + 50) tokens at 0.11 USD per million.
        let estimate = check_before_call(&long, Some(110_000_000), 0, None)
            .unwrap()
            .unwrap();
        assert_eq!(estimate.input_tokens, 10_150);
        assert_eq!(estimate.cost_nanos, 1_116_500);
        assert!(check_before_call(&long, Some(110_000_000), 0, Some(1_116_500)).is_ok());
        let error = check_before_call(&long, Some(110_000_000), 0, Some(1_116_499)).unwrap_err();
        assert_eq!(error.code, COST_EXCEEDED);
        assert_eq!(error.class, ErrorClass::Permanent);
        assert!(error.message.contains("max_cost_usd"), "{}", error.message);
    }

    #[test]
    fn every_field_may_need_its_own_model_call() {
        let fields = (0..40)
            .map(|n| (format!("f{n:02}"), Field::new(FieldType::Text)))
            .collect();
        let request = ExtractRequest::new("b", "m", fields, vec![Item::new("x".repeat(231))]);
        let model = ModelEntry::new("m", 1, 64, vec![FieldType::Text], 8192, 1000, "h");
        let bounds = ModuleBounds {
            image_tokens_each: 0,
            schema_tokens_each: 0,
            overhead_tokens: 0,
        };
        let input = item_cost_input(&request, 0, &model, bounds, Some(85), Some(26_488)).unwrap();
        let ItemSpendInput::Estimable(cost) = input else {
            panic!("the schema fits in the estimate");
        };
        assert_eq!(cost.schema, Tokens::Exact(26_488));
        // Clairic's probe 05: 40 fields, 40 model calls, input_tokens 29,888.
        let estimate = check_before_call(&input, Some(110_000_000), 0, None)
            .unwrap()
            .unwrap();
        assert_eq!(estimate.input_tokens, 29_888);
        assert_eq!(estimate.max_passes, 40);
        let empty = ExtractRequest::new("b", "m", IndexMap::new(), vec![Item::new("s")]);
        assert!(item_cost_input(&empty, 0, &model, bounds, None, None).is_err());
    }

    #[test]
    fn schema_overflow_crosses_any_ceiling_even_with_zero_tokens_per_call() {
        let fields = IndexMap::from([("f".into(), Field::new(FieldType::Text))]);
        let request = ExtractRequest::new("b", "m", fields, vec![Item::new("")]);
        let model = ModelEntry::new("m", 1, 64, vec![FieldType::Text], 8192, 1000, "h");
        let bounds = ModuleBounds {
            image_tokens_each: 0,
            schema_tokens_each: u64::MAX,
            overhead_tokens: 0,
        };
        let input = item_cost_input(&request, 0, &model, bounds, Some(0), Some(1)).unwrap();
        let error = check_before_call(&input, Some(0), 0, Some(u64::MAX)).unwrap_err();
        assert_eq!(error.code, COST_EXCEEDED);
        assert!(error.message.contains("overflows"), "{}", error.message);
        assert_eq!(check_before_call(&input, Some(0), 0, None).unwrap(), None);

        let input = item_cost_input(&request, 0, &model, bounds, Some(8192), Some(1)).unwrap();
        for ceiling in [None, Some(u64::MAX)] {
            let error = check_before_call(&input, Some(0), 0, ceiling).unwrap_err();
            assert_eq!(error.code, INVALID_ITEM);
        }
    }

    #[test]
    fn a_bounded_estimate_crosses_only_past_the_ceiling() {
        let short = input(Tokens::Bound(1000));
        // 1150 tokens at 0.11 USD per million: 126,500 nanodollars.
        assert!(check_before_call(&short, Some(110_000_000), 0, Some(126_500)).is_ok());
        let error = check_before_call(&short, Some(110_000_000), 1, Some(126_500)).unwrap_err();
        assert_eq!(error.code, COST_EXCEEDED);
        // An unpriced model has no cost to bound.
        assert!(check_before_call(&short, None, 0, Some(0)).is_ok());
    }
}
