//! Provider attempt results on `route.report`. Unknown statuses count nothing.

use crate::{errors::Refusal, report::Outcome};
use serde::{Deserialize, Serialize};

open_set! {
    /// Closed for producers; readers keep unknown statuses without effects.
    pub enum AttemptStatus {
        Served => "served",
        ServedEmpty => "served_empty",
        Quota => "quota",
        Throttled => "throttled",
        Credential => "credential",
        Unavailable => "unavailable",
        Rejected => "rejected",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AttemptOutcome {
    pub status: AttemptStatus,
    /// A wait named by the provider, valid only for throttled attempts. When
    /// absent, the wait is unknown and routing chooses its own backoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl AttemptOutcome {
    pub fn new(status: AttemptStatus) -> Self {
        Self {
            status,
            retry_after_ms: None,
        }
    }
    pub fn throttled(retry_after_ms: u64) -> Self {
        Self {
            status: AttemptStatus::Throttled,
            retry_after_ms: Some(retry_after_ms),
        }
    }

    /// The legacy companion is deliberately harmless for rejected and unknown
    /// statuses: cancelled is the closest legacy value with no route penalty.
    pub fn legacy_outcome(&self) -> Outcome {
        match self.status {
            AttemptStatus::Served | AttemptStatus::ServedEmpty => Outcome::Completed,
            AttemptStatus::Quota
            | AttemptStatus::Throttled
            | AttemptStatus::Credential
            | AttemptStatus::Unavailable => Outcome::Failed,
            _ => Outcome::Cancelled,
        }
    }

    /// Rejected caller requests and unfamiliar statuses never count against a route.
    pub fn counts_against_route(&self) -> bool {
        matches!(
            self.status,
            AttemptStatus::Quota
                | AttemptStatus::Throttled
                | AttemptStatus::Credential
                | AttemptStatus::Unavailable
        )
    }

    pub fn check(&self, legacy: &Outcome) -> Result<(), Refusal> {
        if self.status != AttemptStatus::Throttled && self.retry_after_ms.is_some() {
            return Err(Refusal::invalid_request(
                "attempt_outcome.retry_after_ms",
                "retry_after_ms is valid only for throttled",
            ));
        }
        if self.status.is_known() && *legacy != self.legacy_outcome() {
            return Err(Refusal::invalid_request(
                "outcome",
                "outcome disagrees with attempt_outcome",
            ));
        }
        Ok(())
    }

    /// Producers use only the closed status set. Record/request readers still
    /// accept future statuses and apply no effects to them.
    pub fn check_producer(&self) -> Result<(), Refusal> {
        if !self.status.is_known() {
            return Err(Refusal::invalid_request(
                "attempt_outcome.status",
                "unknown producer status",
            ));
        }
        self.check(&self.legacy_outcome())
    }
}
