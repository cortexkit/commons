//! The pre-call spend check for one item (`CONTRACT.md` §6.9): the shared
//! conservative estimate (`cortexkit-model-batch`, its `RULES.md` §5) with
//! this role's inputs, and the item error an item gets when it is not sent.
//!
//! The provider runs a batch's fields in one or more passes and bills the
//! item's state on every pass. The role bounds the passes by the number of
//! fields: Clairic ran 40 fields in 40 passes (live probe 05 of
//! 2026-10-22) and nine fields in two (probe 01), and never more passes
//! than fields.

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

/// The bounds a module supplies for what the role cannot see. Both are
/// required; neither defaults to zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleBounds {
    /// The tokens one image costs at most, billed per pass. Measured for
    /// Clairic at one point only: a 64x64 PNG reported `image_tokens` 64
    /// and raised `schema_tokens` by 41 (probe 12).
    pub image_tokens_each: u64,
    /// The module's per-request overhead: its prompt template and any
    /// system text, in tokens.
    pub overhead_tokens: u64,
}

/// The estimate input for item `index` of `request` on `model`: the
/// state's and the fields' byte bounds, unless the module passes exact
/// counts (`exact_state`, `exact_schema`) from the provider's tokenizer,
/// and at most one pass per field (`max_passes` is the request's field
/// count). An error when there is no such item, or the request has no
/// fields (admission refuses that before any estimate).
pub fn item_cost_input(
    request: &ExtractRequest,
    index: usize,
    model: &ModelEntry,
    bounds: ModuleBounds,
    exact_state: Option<u64>,
    exact_schema: Option<u64>,
) -> Result<ItemCostInput, IdentityError> {
    let item = request
        .items
        .get(index)
        .ok_or_else(|| IdentityError(format!("there is no item {index}")))?;
    let state = exact_state.map_or_else(|| Tokens::bound_of_text(&item.state), Tokens::Exact);
    let schema = match exact_schema {
        Some(count) => Tokens::Exact(count),
        None => Tokens::bound_of_json(&request.fields)?,
    };
    if request.fields.is_empty() {
        return Err(IdentityError("a batch has at least one field".into()));
    }
    let images = item.images.as_ref().map_or(0, |images| images.len() as u64);
    Ok(ItemCostInput::new(
        state,
        schema,
        bounds.image_tokens_each,
        bounds.overhead_tokens,
    )
    .with_images(images)
    .with_max_passes(request.fields.len() as u64)
    .with_pass_tokens(model.pass_tokens))
}

/// Decide whether an item may be sent, before its provider call.
///
/// - The state's exact count reaches `pass_tokens`: the item cannot fit in
///   a pass, `invalid_item`.
/// - With a ceiling (`ceiling_nanos`) and a priced model (`input_rate`): an
///   item whose estimate would cross the ceiling after `spent_nanos` is
///   `cost_exceeded`. An estimate that overflows, or one the module's
///   bounds make impossible (zero passes), crosses any ceiling. A state
///   whose byte bound reaches `pass_tokens` is estimated like any other.
/// - Otherwise the item is sent, and the estimate is returned when there
///   is one.
// The error is the item's own outcome, returned by value so a runner can
// store it as is; it is built once per item, so its size does not matter.
#[allow(clippy::result_large_err)]
pub fn check_before_call(
    input: &ItemCostInput,
    input_rate: Option<RateNanosPerMtok>,
    spent_nanos: u64,
    ceiling_nanos: Option<u64>,
) -> Result<Option<Estimate>, ItemError> {
    let estimate = estimate_item(input, input_rate.unwrap_or(0));
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
    use crate::errors::ErrorClass;
    use indexmap::IndexMap;

    fn input(state: TokenCount) -> ItemCostInput {
        ItemCostInput::new(state, Tokens::Bound(100), 0, 50).with_pass_tokens(8192)
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
    fn every_field_may_bill_the_state_again() {
        use crate::{
            describe::ModelEntry,
            field::{Field, FieldType},
            request::Item,
        };
        let fields = (0..40)
            .map(|n| (format!("f{n:02}"), Field::new(FieldType::Text)))
            .collect();
        let request = ExtractRequest::new("b", "m", fields, vec![Item::new("x".repeat(231))]);
        let model = ModelEntry::new("m", 1, 64, vec![FieldType::Text], 8192, 1000, "h");
        let bounds = ModuleBounds {
            image_tokens_each: 0,
            overhead_tokens: 0,
        };
        let input = item_cost_input(&request, 0, &model, bounds, Some(85), Some(26_488)).unwrap();
        assert_eq!(input.max_passes, 40);
        // Clairic's probe 05: 40 fields, 40 passes, input_tokens 29,888.
        let estimate = check_before_call(&input, Some(110_000_000), 0, None)
            .unwrap()
            .unwrap();
        assert_eq!(estimate.input_tokens, 29_888);
        let empty = ExtractRequest::new("b", "m", IndexMap::new(), vec![Item::new("s")]);
        assert!(item_cost_input(&empty, 0, &model, bounds, None, None).is_err());
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
