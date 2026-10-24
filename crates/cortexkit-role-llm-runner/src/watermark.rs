//! Durable progress of a same-computer log reader, scoped to the bind principal.

use serde::{Deserialize, Serialize};

pub const LOG_READ_WATERMARK_V1: &str = "log-read-watermark/v1";
pub const WATERMARK_GET: &str = "watermark.get";
pub const NOT_A_READ_SOURCE: &str = "not_a_read_source";

/// The daemon-attested caller is the only source; no module selector is accepted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WatermarkRequest {}

impl WatermarkRequest {
    pub fn new() -> Self {
        Self {}
    }
}

/// Every record the caller wrote strictly before this time was durably read.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct WatermarkReply {
    /// `None` is serialized as null: progress has not yet been established.
    pub watermark_ms: Option<u64>,
}

impl WatermarkReply {
    pub fn new(watermark_ms: Option<u64>) -> Self {
        Self { watermark_ms }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn watermark_vectors_and_strict_caller_request() {
        let v: Value =
            serde_json::from_str(crate::vectors::file("log-read-watermark.json").unwrap()).unwrap();
        assert_eq!(v["capability"], LOG_READ_WATERMARK_V1);
        assert_eq!(v["op"], WATERMARK_GET);
        let request: WatermarkRequest = serde_json::from_value(v["request"].clone()).unwrap();
        assert_eq!(request, WatermarkRequest::new());
        assert_eq!(serde_json::to_value(request).unwrap(), v["request"]);
        for invalid in v["invalid_requests"].as_array().unwrap() {
            assert!(serde_json::from_value::<WatermarkRequest>(invalid.clone()).is_err());
        }
        for reply in v["replies"].as_array().unwrap() {
            let decoded: WatermarkReply = serde_json::from_value(reply.clone()).unwrap();
            assert_eq!(serde_json::to_value(&decoded).unwrap(), *reply);
            assert_eq!(decoded, WatermarkReply::new(reply["watermark_ms"].as_u64()));
        }
        for invalid in [json!({"watermark_ms": -1}), json!({"watermark_ms":1.5})] {
            assert!(serde_json::from_value::<WatermarkReply>(invalid).is_err());
        }
        assert_eq!(v["refusal"]["code"], NOT_A_READ_SOURCE);
    }
}
