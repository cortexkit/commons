//! The pre-call spend check for one item (`CONTRACT.md` §6.9): the shared
//! conservative estimate (`cortexkit-model-batch`, its `RULES.md` §5) with
//! this role's inputs, and the item error an item gets when it is not sent.

use cortexkit_model_batch::estimate::{
    crosses_ceiling, estimate_item, Estimate, EstimateError, ItemCostInput, RateNanosPerMtok,
    Tokens,
};
pub use cortexkit_model_batch::estimate::{
    nanos_to_usd, nanos_to_usd_text, usd_to_nanos, Rounding, Tokens as TokenCount,
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
    /// The tokens one image costs at most. Not yet measured for Clairic.
    pub image_tokens_each: u64,
    /// The module's per-request overhead: its prompt template and any
    /// system text, in tokens.
    pub overhead_tokens: u64,
}

/// The estimate input for item `index` of `request` on `model`: the
/// state's and the fields' byte bounds, unless the module passes exact
/// counts (`exact_state`, `exact_schema`) from the provider's tokenizer.
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
    let images = item.images.as_ref().map_or(0, |images| images.len() as u64);
    Ok(ItemCostInput::new(
        state,
        schema,
        bounds.image_tokens_each,
        bounds.overhead_tokens,
    )
    .with_images(images)
    .with_pass_tokens(model.pass_tokens))
}

/// Decide whether an item may be sent, before its provider call.
///
/// - The state's exact count reaches `pass_tokens`: the item cannot fit in
///   a pass, `invalid_item`.
/// - With a ceiling (`ceiling_nanos`) and a priced model (`input_rate`): an
///   item whose estimate would cross the ceiling after `spent_nanos` is
///   `cost_exceeded`. An unbounded estimate (the state's byte bound reaches
///   a pass's budget) or an overflowing one crosses any ceiling.
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
        Ok(Estimate::Unbounded) => Err(ItemError::for_code(
            COST_EXCEEDED,
            "the cost estimate is unbounded because the state's byte bound reaches a pass's token budget; raise or remove max_cost_usd, or use a module that counts tokens exactly",
        )),
        Ok(bounded @ Estimate::Bounded { cost_nanos, .. })
            if crosses_ceiling(spent_nanos, &bounded, ceiling) =>
        {
            Err(ItemError::for_code(
                COST_EXCEEDED,
                format!(
                    "answering this item could cost {} USD, crossing max_cost_usd {} after {} USD spent",
                    nanos_to_usd_text(cost_nanos),
                    nanos_to_usd_text(ceiling),
                    nanos_to_usd_text(spent_nanos),
                ),
            ))
        }
        Ok(bounded) => Ok(Some(bounded)),
        Err(_) => Err(ItemError::for_code(
            COST_EXCEEDED,
            "the cost estimate overflows, so it crosses any ceiling",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorClass;

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
    fn a_byte_bound_past_the_pass_is_cost_exceeded_only_under_a_ceiling() {
        let long = input(Tokens::Bound(10_000));
        assert_eq!(
            check_before_call(&long, Some(110_000_000), 0, None),
            Ok(Some(Estimate::Unbounded))
        );
        let error = check_before_call(&long, Some(110_000_000), 0, Some(u64::MAX)).unwrap_err();
        assert_eq!(error.code, COST_EXCEEDED);
        assert_eq!(error.class, ErrorClass::Permanent);
        assert!(error.message.contains("unbounded"), "{}", error.message);
        assert!(error.message.contains("max_cost_usd"), "{}", error.message);
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
