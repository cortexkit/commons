//! The request shape seven ops share, `{refs, cutId}`, and [`Request`], one
//! value for any of the ten ops, decoded from a method name and its params.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    busy::BusyRequest,
    describe::DescribeRequest,
    errors::{decode_request, ErrorBody, RefusalCode},
    refs::{CutId, Refs},
    release::ReleaseRequest,
    Op,
};

/// `{refs, cutId}`: the request of `status`, `prepare`, `export`,
/// `diverged`, `import`, `activate` and `abort`. Strict: an unknown member
/// is refused.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CutRequest {
    pub refs: Refs,
    #[serde(rename = "cutId")]
    pub cut_id: CutId,
}

impl CutRequest {
    pub fn new(refs: Refs, cut_id: CutId) -> Self {
        Self { refs, cut_id }
    }
}

/// Any of the ten requests, tagged by its op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Describe(DescribeRequest),
    Busy(BusyRequest),
    Status(CutRequest),
    Prepare(CutRequest),
    Export(CutRequest),
    Diverged(CutRequest),
    Import(CutRequest),
    Activate(CutRequest),
    Abort(CutRequest),
    Release(ReleaseRequest),
}

impl Request {
    /// Decodes `params` as the request of the op named `method`. An unknown
    /// method or params that do not decode are refused `invalid_request`.
    pub fn decode(method: &str, params: &Value) -> Result<Self, ErrorBody> {
        let op = Op::from_method(method).ok_or_else(|| {
            ErrorBody::new(
                RefusalCode::InvalidRequest,
                format!("{method:?} is not an agent-data/v1 op"),
            )
        })?;
        Ok(match op {
            Op::Describe => Self::Describe(decode_request(params)?),
            Op::Busy => Self::Busy(decode_request(params)?),
            Op::Status => Self::Status(decode_request(params)?),
            Op::Prepare => Self::Prepare(decode_request(params)?),
            Op::Export => Self::Export(decode_request(params)?),
            Op::Diverged => Self::Diverged(decode_request(params)?),
            Op::Import => Self::Import(decode_request(params)?),
            Op::Activate => Self::Activate(decode_request(params)?),
            Op::Abort => Self::Abort(decode_request(params)?),
            Op::Release => Self::Release(decode_request(params)?),
        })
    }

    pub fn op(&self) -> Op {
        match self {
            Self::Describe(_) => Op::Describe,
            Self::Busy(_) => Op::Busy,
            Self::Status(_) => Op::Status,
            Self::Prepare(_) => Op::Prepare,
            Self::Export(_) => Op::Export,
            Self::Diverged(_) => Op::Diverged,
            Self::Import(_) => Op::Import,
            Self::Activate(_) => Op::Activate,
            Self::Abort(_) => Op::Abort,
            Self::Release(_) => Op::Release,
        }
    }

    /// The request's params as JSON, the inverse of [`Request::decode`].
    pub fn params(&self) -> Value {
        let value = match self {
            Self::Describe(r) => serde_json::to_value(r),
            Self::Busy(r) => serde_json::to_value(r),
            Self::Status(r)
            | Self::Prepare(r)
            | Self::Export(r)
            | Self::Diverged(r)
            | Self::Import(r)
            | Self::Activate(r)
            | Self::Abort(r) => serde_json::to_value(r),
            Self::Release(r) => serde_json::to_value(r),
        };
        value.expect("a request always serializes")
    }

    /// The request's `cutId`; `describe` and `busy` carry none.
    pub fn cut_id(&self) -> Option<&CutId> {
        match self {
            Self::Describe(_) | Self::Busy(_) => None,
            Self::Status(r)
            | Self::Prepare(r)
            | Self::Export(r)
            | Self::Diverged(r)
            | Self::Import(r)
            | Self::Activate(r)
            | Self::Abort(r) => Some(&r.cut_id),
            Self::Release(r) => Some(&r.cut_id),
        }
    }

    /// The same request with its refs sorted, so two requests that differ
    /// only in ref order compare equal. Used by idempotency, which compares
    /// refs as a set.
    pub fn with_sorted_refs(&self) -> Request {
        let mut request = self.clone();
        match &mut request {
            Self::Describe(_) => {}
            Self::Busy(r) => r.refs = r.refs.sorted(),
            Self::Status(r)
            | Self::Prepare(r)
            | Self::Export(r)
            | Self::Diverged(r)
            | Self::Import(r)
            | Self::Activate(r)
            | Self::Abort(r) => r.refs = r.refs.sorted(),
            Self::Release(r) => r.refs = r.refs.sorted(),
        }
        request
    }

    /// The request's refs; `describe` carries none.
    pub fn refs(&self) -> Option<&Refs> {
        match self {
            Self::Describe(_) => None,
            Self::Busy(r) => Some(&r.refs),
            Self::Status(r)
            | Self::Prepare(r)
            | Self::Export(r)
            | Self::Diverged(r)
            | Self::Import(r)
            | Self::Activate(r)
            | Self::Abort(r) => Some(&r.refs),
            Self::Release(r) => Some(&r.refs),
        }
    }
}
