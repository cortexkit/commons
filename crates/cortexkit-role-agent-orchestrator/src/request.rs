//! Request envelopes pair each method with its own closed params type.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::errors::Refusal;
use crate::object::Object;
use crate::ops;
use crate::params::{
    CancelParams, FinishParams, ListParams, OpenParams, ProgressParams, SyncParams,
};

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    Open(OpenParams),
    Finish(FinishParams),
    Progress(ProgressParams),
    Cancel(CancelParams),
    Sync(SyncParams),
    List(ListParams),
}

#[derive(Serialize, Deserialize)]
#[serde(
    remote = "Request",
    tag = "method",
    content = "params",
    deny_unknown_fields
)]
enum WireRequest {
    #[serde(rename = "awaited_work.open")]
    Open(OpenParams),
    #[serde(rename = "awaited_work.finish")]
    Finish(FinishParams),
    #[serde(rename = "awaited_work.progress")]
    Progress(ProgressParams),
    #[serde(rename = "awaited_work.cancel")]
    Cancel(CancelParams),
    #[serde(rename = "awaited_work.sync")]
    Sync(SyncParams),
    #[serde(rename = "awaited_work.list")]
    List(ListParams),
}

impl Serialize for Request {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WireRequest::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        WireRequest::deserialize(Object(deserializer))
    }
}

impl Request {
    pub fn method(&self) -> &'static str {
        match self {
            Self::Open(_) => ops::AWAITED_WORK_OPEN,
            Self::Finish(_) => ops::AWAITED_WORK_FINISH,
            Self::Progress(_) => ops::AWAITED_WORK_PROGRESS,
            Self::Cancel(_) => ops::AWAITED_WORK_CANCEL,
            Self::Sync(_) => ops::AWAITED_WORK_SYNC,
            Self::List(_) => ops::AWAITED_WORK_LIST,
        }
    }

    /// Check params bounds after decoding. Scope, authority and stored state
    /// are the implementer's responsibility.
    pub fn validate(&self) -> Result<(), Refusal> {
        match self {
            Self::Open(params) => params.validate(),
            Self::Finish(params) => params.validate(),
            Self::Progress(params) => params.validate(),
            Self::Cancel(params) => params.validate(),
            Self::Sync(params) => params.validate(),
            Self::List(params) => params.validate(),
        }
    }
}
