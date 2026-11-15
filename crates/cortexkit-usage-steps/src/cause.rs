//! One shared explanation for each prompt rewrite.

use crate::{Cause, StepRow, Verdict};

/// Explain a warning, bust or full bust using the first known matching rule.
/// `prev` is the same session's previous row in feed order, warm-ups included.
/// The caller must compare rows from the same runner: digests are runner-local.
/// Missing facts skip their comparison; unknown never counts as changed.
/// The usage-feed provider computes this once and serves it as [`StepRow::cause`].
///
/// Precedence: account/provider/model, cache expiry (bust or full bust only),
/// tools/system, applied view, signed floor, then unexplained. A changed account
/// or model empties the whole cache. Expiry can empty it too, or expire a shorter
/// conversation breakpoint while the longer-lived prefix survives. After these
/// cache-wide or lifetime explanations, compare the prompt prefix from its head.
/// Expiry uses the request start, not the feed's general occurrence timestamp;
/// the gap must strictly exceed the previous request's shortest breakpoint TTL.
pub fn rewrite_cause(prev: Option<&StepRow>, row: &StepRow) -> Option<Cause> {
    if !matches!(
        row.verdict,
        Verdict::Warning | Verdict::Bust | Verdict::FullBust
    ) {
        return None;
    }
    let Some(prev) = prev else {
        return Some(Cause::Unexplained);
    };
    if changed(&prev.provider, &row.provider)
        || changed(&prev.model, &row.model)
        || changed(&prev.account_ref, &row.account_ref)
    {
        return Some(Cause::AccountOrModelChanged);
    }
    if matches!(row.verdict, Verdict::Bust | Verdict::FullBust) {
        if let (Some(start), Some(finished), Some(ttl)) =
            (row.started_at_ms, prev.finished_at_ms, prev.cache_ttl_ms)
        {
            if start.checked_sub(finished).is_some_and(|gap| gap > ttl) {
                return Some(Cause::CacheExpired);
            }
        }
    }
    if changed(&prev.tools_digest, &row.tools_digest)
        || changed(&prev.system_digest, &row.system_digest)
    {
        return Some(Cause::ToolsOrSystemChanged);
    }
    if changed(&prev.view_version, &row.view_version) {
        return Some(Cause::ViewChanged);
    }
    if changed(&prev.signed_floor, &row.signed_floor) {
        return Some(Cause::SignedFloorMoved);
    }
    Some(Cause::Unexplained)
}

fn changed<T: PartialEq>(prev: &Option<T>, row: &Option<T>) -> bool {
    matches!((prev, row), (Some(prev), Some(row)) if prev != row)
}
