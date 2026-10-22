//! Chunked uploads and session-scoped media reads (§9.12). Storage and caller
//! admission belong to the runner; these checks operate on decoded bytes.

use crate::content_digest::{is_sha256_hex, sha256_hex};
use serde::{Deserialize, Serialize};

/// Base64 expands this to less than the daemon's 4 MiB frame limit.
pub const CHUNK_MAX: usize = 2_097_152;
pub const UNREFERENCED_RETENTION_SECONDS: u64 = 3_600;

macro_rules! constructor_arg {
    (String) => { impl Into<String> };
    ($number:ident) => { $number };
}

macro_rules! constructor_value {
    ($value:ident, String) => {
        $value.into()
    };
    ($value:ident, $number:ident) => {
        $value
    };
}

macro_rules! shape {
    ($(#[$meta:meta])* $name:ident { $($field:ident : $ty:ident),+ $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
        $(#[$meta])*
        #[non_exhaustive]
        pub struct $name { $(pub $field: $ty),+ }
        impl $name {
            pub fn new($($field: constructor_arg!($ty)),+) -> Self {
                Self { $($field: constructor_value!($field, $ty)),+ }
            }
            $(pub fn $field(&self) -> &$ty { &self.$field })+
        }
    };
}

shape!(
    #[serde(deny_unknown_fields)]
    UploadBeginRequest {
        mime: String,
        bytes: u64,
        sha256: String
    }
);
shape!(UploadBeginReply { upload_id: String });
shape!(
    #[serde(deny_unknown_fields)]
    UploadAppendRequest {
        upload_id: String,
        offset: u64,
        data: String
    }
);
shape!(UploadAppendReply { received: u64 });
shape!(
    #[serde(deny_unknown_fields)]
    UploadStatusRequest { upload_id: String }
);
shape!(
    #[serde(deny_unknown_fields)]
    UploadCommitRequest { upload_id: String }
);
shape!(UploadStatusReply {
    state: String,
    received: u64
});
shape!(UploadCommitReply {
    state: String,
    sha256: String,
    mime: String,
    bytes: u64
});
shape!(
    #[serde(deny_unknown_fields)]
    MediaReadRequest {
        sha256: String,
        offset: u64,
        length: u64
    }
);

/// `data` is absent only for a zero-length metadata probe.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct MediaReadReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    pub size: u64,
    pub mime: String,
}

impl MediaReadReply {
    pub fn new(size: u64, mime: impl Into<String>) -> Self {
        Self {
            data: None,
            size,
            mime: mime.into(),
        }
    }
    pub fn with_data(mut self, data: impl Into<String>) -> Self {
        self.data = Some(data.into());
        self
    }
    pub fn data(&self) -> Option<&str> {
        self.data.as_deref()
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn mime(&self) -> &str {
        &self.mime
    }
}

/// The optional ordered `session.send.content` contains upload references,
/// after the prompt's text. The upload must already be committed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ContentPart {
    Upload { upload_id: String },
}

impl ContentPart {
    pub fn upload(upload_id: impl Into<String>) -> Self {
        Self::Upload {
            upload_id: upload_id.into(),
        }
    }
    pub fn upload_id(&self) -> &str {
        match self {
            Self::Upload { upload_id } => upload_id,
        }
    }
}

pub mod states {
    pub const OPEN: &str = "open";
    pub const COMMITTED: &str = "committed";
    pub const GONE: &str = "gone";
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UploadState {
    Open,
    Committed,
    Gone,
    Unknown(String),
}

impl UploadState {
    pub fn parse(state: &str) -> Self {
        match state {
            states::OPEN => Self::Open,
            states::COMMITTED => Self::Committed,
            states::GONE => Self::Gone,
            other => Self::Unknown(other.into()),
        }
    }
}

impl UploadStatusReply {
    pub fn state_kind(&self) -> UploadState {
        UploadState::parse(&self.state)
    }
}

impl UploadBeginRequest {
    /// Malformed metadata is refused before allocating storage.
    pub fn check(&self) -> Result<(), &'static str> {
        if !is_sha256_hex(&self.sha256) {
            return Err("sha256");
        }
        if self.mime.is_empty() {
            return Err("mime");
        }
        Ok(())
    }
}

impl MediaReadRequest {
    pub fn check(&self, size: u64) -> Result<(), &'static str> {
        if self.length > CHUNK_MAX as u64 {
            return Err("length");
        }
        if self.offset > size {
            return Err("offset");
        }
        Ok(())
    }
}

/// Returns the refusal code as a plain string, which a runner sends as is and
/// an older reader decodes as an unknown code. Check the decoded length, never
/// base64's character count, before accepting any bytes.
pub fn check_chunk_size(raw_bytes: usize) -> Result<(), &'static str> {
    if raw_bytes > CHUNK_MAX {
        Err("upload_chunk_too_large")
    } else {
        Ok(())
    }
}

/// Validate an append without changing storage. Offset refusals carry the
/// runner's current `received`, allowing a lost-reply retry to resume.
pub fn check_append(
    received: u64,
    declared: u64,
    offset: u64,
    raw_bytes: usize,
) -> Result<(), &'static str> {
    check_chunk_size(raw_bytes)?;
    if offset != received {
        return Err("upload_offset_mismatch");
    }
    if received
        .checked_add(raw_bytes as u64)
        .is_none_or(|end| end > declared)
    {
        return Err("upload_size_mismatch");
    }
    Ok(())
}

/// Verify the complete upload before making its id usable by a send.
pub fn verify_commit(
    declared_bytes: u64,
    declared_sha256: &str,
    bytes: &[u8],
) -> Result<(), &'static str> {
    if bytes.len() as u64 != declared_bytes {
        return Err("upload_size_mismatch");
    }
    if sha256_hex(bytes) != declared_sha256 {
        return Err("upload_sha256_mismatch");
    }
    Ok(())
}

/// One grammar for reads and model fallback. Callers supply the reason for
/// their context (`not_requested` for default reads, `unsupported` for a
/// model that cannot take the media). Hashes are lowercase SHA-256 metadata.
pub fn marker(kind: &str, mime: &str, bytes: u64, sha256: &str, reason: &str) -> String {
    let prefix: String = sha256.chars().take(12).collect();
    format!(
        "[media not shown: kind={kind} mime={mime} bytes={bytes} sha256={prefix} reason={reason}]"
    )
}

/// Join already-rendered text and media markers in stored part order.
pub fn join_tool_result(parts: &[String]) -> String {
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_vectors_pin_wire_bytes_and_markers() {
        let v: serde_json::Value =
            serde_json::from_str(crate::vectors::file("attachments.json").unwrap()).unwrap();
        fn roundtrip<T: serde::de::DeserializeOwned + Serialize>(v: &serde_json::Value) {
            let decoded: T = serde_json::from_value(v.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), *v);
        }
        for row in v["requests"].as_array().unwrap() {
            let params = &row["params"];
            match row["method"].as_str().unwrap() {
                "session.upload_begin" => roundtrip::<UploadBeginRequest>(params),
                "session.upload_append" => roundtrip::<UploadAppendRequest>(params),
                "session.upload_status" => roundtrip::<UploadStatusRequest>(params),
                "session.upload_commit" => roundtrip::<UploadCommitRequest>(params),
                "session.media_read" => roundtrip::<MediaReadRequest>(params),
                other => panic!("unhandled request {other}"),
            }
        }
        for row in v["replies"].as_array().unwrap() {
            let result = &row["result"];
            match row["method"].as_str().unwrap() {
                "session.upload_begin" => roundtrip::<UploadBeginReply>(result),
                "session.upload_append" => roundtrip::<UploadAppendReply>(result),
                "session.upload_status" => roundtrip::<UploadStatusReply>(result),
                "session.upload_commit" => roundtrip::<UploadCommitReply>(result),
                "session.media_read" => roundtrip::<MediaReadReply>(result),
                other => panic!("unhandled reply {other}"),
            }
        }
        roundtrip::<crate::send::SendRequest>(&v["send"]);
        roundtrip::<crate::read::ReadRequest>(&v["blocks_request"]);
        let describe = crate::describe::check_describe(&v["describe"]).unwrap();
        assert!(describe.declares(crate::capabilities::ATTACHMENTS));
        for op in crate::capabilities::group_ops(crate::capabilities::ATTACHMENTS).unwrap() {
            let mut broken = v["describe"].clone();
            broken["majors"][0]["ops"]
                .as_array_mut()
                .unwrap()
                .retain(|value| value != op);
            assert!(crate::describe::check_describe(&broken).unwrap_err().iter().any(|problem| matches!(problem, crate::describe::DescribeProblem::GroupIncomplete { capability, op: missing } if *capability == crate::capabilities::ATTACHMENTS && missing == op)));
        }
        assert_eq!(
            serde_json::to_string(&crate::send::SendRequest::new("Look", "s1")).unwrap(),
            v["legacy_send_bytes"]
        );
        for row in v["markers"].as_array().unwrap() {
            assert_eq!(
                marker(
                    row["kind"].as_str().unwrap(),
                    row["mime"].as_str().unwrap(),
                    row["bytes"].as_u64().unwrap(),
                    row["sha256"].as_str().unwrap(),
                    row["reason"].as_str().unwrap()
                ),
                row["text"]
            );
        }
        let result = &v["multipart_tool_result"];
        let parts: Vec<String> = result["stored"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| {
                if part["type"] == "text" {
                    part["text"].as_str().unwrap().into()
                } else {
                    marker(
                        part["kind"].as_str().unwrap(),
                        part["mime"].as_str().unwrap(),
                        part["bytes"].as_u64().unwrap(),
                        part["sha256"].as_str().unwrap(),
                        "not_requested",
                    )
                }
            })
            .collect();
        let mut rendered = result["stored"].clone();
        rendered["content"] = serde_json::json!(join_tool_result(&parts));
        assert_eq!(rendered, result["inline"]);
        for row in v["other_session_id"].as_array().unwrap() {
            match row["method"].as_str().unwrap() {
                "session.upload_status" => {
                    roundtrip::<UploadStatusRequest>(&row["params"]);
                    let status: UploadStatusReply =
                        serde_json::from_value(row["result"].clone()).unwrap();
                    assert_eq!(status.state_kind(), UploadState::Gone);
                    assert_eq!(status.received, 0);
                }
                "session.upload_commit" => roundtrip::<UploadCommitRequest>(&row["params"]),
                "session.upload_append" => roundtrip::<UploadAppendRequest>(&row["params"]),
                "session.send" => roundtrip::<crate::send::SendRequest>(&row["params"]),
                other => panic!("unhandled other-session vector {other}"),
            }
            if let Some(code) = row["error"]["code"].as_str() {
                assert!(crate::errors::is_unknown_upload(code));
            }
        }
        assert_eq!(
            UploadState::parse("future"),
            UploadState::Unknown("future".into())
        );
        let predicates: &[fn(&str) -> bool] = &[
            crate::errors::is_unknown_session,
            crate::errors::is_unknown_upload,
            crate::errors::is_upload_not_committed,
            crate::errors::is_upload_gone,
            crate::errors::is_upload_chunk_too_large,
            crate::errors::is_upload_offset_mismatch,
            crate::errors::is_upload_size_mismatch,
            crate::errors::is_upload_sha256_mismatch,
            crate::errors::is_media_unsupported,
            crate::errors::is_unknown_media,
        ];
        for (row, predicate) in v["refusals"].as_array().unwrap().iter().zip(predicates) {
            let code = row["code"].as_str().unwrap();
            assert!(crate::errors::CODES.contains(&code));
            assert!(!crate::errors::is_retryable(code));
            assert!(predicate(code));
            assert!(!predicate("future_refusal"));
        }
        assert!(serde_json::from_value::<UploadBeginRequest>(
            serde_json::json!({"session":"other","mime":"image/png","bytes":3,"sha256":"hash"})
        )
        .is_err());
    }

    #[test]
    fn chunk_cap_includes_exactly_two_mib() {
        assert_eq!(check_chunk_size(CHUNK_MAX), Ok(()));
        assert_eq!(
            check_chunk_size(CHUNK_MAX + 1),
            Err("upload_chunk_too_large")
        );
    }

    #[test]
    fn commit_checks_size_and_sha256() {
        assert_eq!(verify_commit(3, &sha256_hex(b"abc"), b"abc"), Ok(()));
        assert_eq!(
            verify_commit(2, &sha256_hex(b"abc"), b"abc"),
            Err("upload_size_mismatch")
        );
        assert_eq!(
            verify_commit(3, &sha256_hex(b"abd"), b"abc"),
            Err("upload_sha256_mismatch")
        );
    }

    #[test]
    fn append_refuses_replays_and_overruns() {
        assert_eq!(check_append(3, 4, 0, 1), Err("upload_offset_mismatch"));
        assert_eq!(check_append(3, 4, 3, 2), Err("upload_size_mismatch"));
        assert_eq!(
            check_append(u64::MAX, u64::MAX, u64::MAX, 1),
            Err("upload_size_mismatch")
        );
    }

    #[test]
    fn media_read_bounds_allow_probes_and_eof() {
        assert_eq!(MediaReadRequest::new("hash", 3_u64, 0_u64).check(3), Ok(()));
        assert_eq!(
            MediaReadRequest::new("hash", 4_u64, 0_u64).check(3),
            Err("offset")
        );
        assert_eq!(
            MediaReadRequest::new("hash", 0_u64, CHUNK_MAX as u64 + 1).check(3),
            Err("length")
        );
    }
}
