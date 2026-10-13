//! `agent_data.status {refs, cutId}`: the step this `cutId` has reached
//! here. A read, used by the driver to resolve an op whose outcome is
//! unknown (a lost reply, a timeout, a dropped connection).

use serde::{Deserialize, Serialize};

use crate::step::Step;

/// The `status` request: `{refs, cutId}`.
pub type StatusRequest = crate::request::CutRequest;

/// The `status` reply: `{step}`. A `cutId` this participant never saw
/// answers `none`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StatusReply {
    pub step: Step,
}

impl StatusReply {
    pub fn new(step: Step) -> Self {
        Self { step }
    }
}
