//! Admission rules every model-batch role shares (`RULES.md` §1, §5, §8, §9):
//! the `batch_id` grammar, count bounds, the request size, the catalog row,
//! and the `max_cost_usd` ceiling.

use serde_json::Number;

use crate::errors::Refusal;

/// Longest `batch_id`, in bytes.
pub const BATCH_ID_MAX_LEN: usize = 128;

/// Whether `batch_id` is 1 to [`BATCH_ID_MAX_LEN`] printable ASCII
/// characters (space through tilde).
pub fn is_batch_id(batch_id: &str) -> bool {
    !batch_id.is_empty()
        && batch_id.len() <= BATCH_ID_MAX_LEN
        && batch_id.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

/// `batch_id` checked, or `invalid_params` naming `batch_id`.
pub fn check_batch_id(batch_id: &str) -> Result<(), Refusal> {
    if is_batch_id(batch_id) {
        Ok(())
    } else {
        Err(Refusal::invalid_params(
            "batch_id",
            "batch_id is 1-128 printable ASCII characters",
        ))
    }
}

/// `count` within `min..=max`, or an over-limit refusal naming `field` and
/// the bound it broke. `what` names the counted things in the message.
pub fn bounded(
    count: usize,
    min: usize,
    max: usize,
    field: &str,
    what: &str,
) -> Result<(), Refusal> {
    if count < min {
        Err(Refusal::over_limit(
            field,
            min as u64,
            format!("{count} {what}; at least {min}"),
        ))
    } else if count > max {
        Err(Refusal::over_limit(
            field,
            max as u64,
            format!("{count} {what}; at most {max}"),
        ))
    } else {
        Ok(())
    }
}

/// Check a request body's size against the module's `max_request_bytes`.
/// The refusal names `params` and the limit: the size belongs to the
/// request as a whole, so no narrower path applies.
pub fn check_request_size(len: usize, max_request_bytes: u64) -> Result<(), Refusal> {
    if len as u64 > max_request_bytes {
        Err(Refusal::over_limit(
            "params",
            max_request_bytes,
            format!("the request is {len} bytes; at most {max_request_bytes}"),
        ))
    } else {
        Ok(())
    }
}

/// `max_cost_usd`, when present, is a number of USD, zero or more;
/// otherwise `invalid_params` naming `max_cost_usd`.
pub fn check_max_cost_usd(max_cost_usd: Option<&Number>) -> Result<(), Refusal> {
    match max_cost_usd {
        Some(max) if max.as_f64().is_none_or(|usd| usd < 0.0) => Err(Refusal::invalid_params(
            "max_cost_usd",
            "max_cost_usd is a number of USD, zero or more",
        )),
        _ => Ok(()),
    }
}

/// The ceiling a call works under, and the batch's recorded ceiling from
/// then on: the lower of the ceiling recorded so far and the one this call
/// carries. `None` when neither names one.
///
/// The recorded ceiling is not fixed at first admission. A runner records
/// this value durably before the call sends anything, so after any call the
/// recorded ceiling is `effective_ceiling(recorded, sent)`. That is what
/// makes a tightening stick: a later re-send carrying the original, higher
/// value works under the lower one, and a caller can tighten a batch's
/// ceiling but never loosen it.
pub fn effective_ceiling(recorded: Option<f64>, sent: Option<f64>) -> Option<f64> {
    match (recorded, sent) {
        (Some(recorded), Some(sent)) => Some(recorded.min(sent)),
        (one, None) | (None, one) => one,
    }
}

/// What the runner's catalog says about the requested model. Three cases,
/// so a row that states no kind is never mistaken for a missing row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogRow<'a> {
    /// The catalog has no row for the model.
    Absent,
    /// The row exists and states no `kind`.
    Kindless,
    /// The row states this `kind`.
    Kind(&'a str),
}

/// Check the model's catalog row against the role's `expected_kind`:
/// `model_unknown` for no row, `wrong_kind(model, kind)` for a row of
/// another kind or of none (`kind` is `None` then; the refusal never
/// invents one). The model is never substituted.
pub fn check_catalog_row(
    model: &str,
    row: CatalogRow<'_>,
    expected_kind: &str,
    wrong_kind: impl FnOnce(&str, Option<&str>) -> Refusal,
) -> Result<(), Refusal> {
    match row {
        CatalogRow::Absent => Err(Refusal::model_unknown(model)),
        CatalogRow::Kind(kind) if kind == expected_kind => Ok(()),
        CatalogRow::Kind(kind) => Err(wrong_kind(model, Some(kind))),
        CatalogRow::Kindless => Err(wrong_kind(model, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{MODEL_NOT_EXTRACTOR, MODEL_UNKNOWN};

    #[test]
    fn batch_ids_are_printable_ascii_up_to_128() {
        assert!(is_batch_id("b-1"));
        assert!(!is_batch_id(""));
        assert!(!is_batch_id("tab\there"));
        assert!(!is_batch_id(&"x".repeat(129)));
        assert!(is_batch_id(&"x".repeat(128)));
    }

    #[test]
    fn the_ceiling_tightens_and_never_loosens() {
        let mut recorded = None;
        let mut worked_under = Vec::new();
        for sent in [5.0, 1.0, 5.0] {
            recorded = effective_ceiling(recorded, Some(sent));
            worked_under.push(recorded);
        }
        assert_eq!(worked_under, [Some(5.0), Some(1.0), Some(1.0)]);
        assert_eq!(effective_ceiling(None, None), None);
    }

    #[test]
    fn a_kindless_row_is_not_a_missing_row() {
        let wrong = |model: &str, kind: Option<&str>| Refusal::model_not_extractor(model, kind);
        let absent = check_catalog_row("m", CatalogRow::Absent, "extraction", wrong).unwrap_err();
        assert_eq!(absent.code, MODEL_UNKNOWN);
        let kindless =
            check_catalog_row("m", CatalogRow::Kindless, "extraction", wrong).unwrap_err();
        assert_eq!(kindless.code, MODEL_NOT_EXTRACTOR);
        assert_eq!(kindless.detail.kind, None);
        assert!(
            check_catalog_row("m", CatalogRow::Kind("extraction"), "extraction", wrong).is_ok()
        );
    }
}
