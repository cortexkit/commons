//! Notice records for a module's own sends (contract §10).

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

use crate::{schema::validate_id, InvalidParams, Principal, DISPLAY_TOO_LARGE};

pub const NOTICE_RECORD_WRITER_CAPABILITY: &str = "notice-record-writer/v1";
pub const NOTICE_RECORD_DISPLAY_CAPABILITY: &str = "notice-record-display/v1";
pub const MAX_NOTICE_RECORD_BYTES: usize = 16 * 1024;
pub const MAX_NOTICE_KEY_BYTES: usize = 128;
/// Largest version that every JSON reader can represent exactly.
pub const MAX_NOTICE_VIEW_VERSION: u64 = (1 << 53) - 1;

pub fn valid_notice_view_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub fn valid_notice_view_version(version: u64) -> bool {
    (1..=MAX_NOTICE_VIEW_VERSION).contains(&version)
}

/// Parse the writer's unqualified `<name>:<version>`.
pub fn parse_notice_view(view: &str) -> Option<(&str, u64)> {
    let (name, version) = view.split_once(':')?;
    if !valid_notice_view_name(name)
        || version.starts_with('0')
        || !version.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let version = version.parse().ok()?;
    valid_notice_view_version(version).then_some((name, version))
}

/// Encode a notice view pin. When serving a record, use the attested module id.
pub fn notice_view_key(writer: &str, name: &str, version: u64) -> Option<String> {
    let key = format!("nv1:{writer}:{name}:{version}");
    parse_notice_view_key(&key).filter(|(owner, _, _)| *owner == writer)?;
    Some(key)
}

/// Parse a notice view pin into writer module id, view name and version.
/// Parsing a key does not attest the writer's identity.
pub fn parse_notice_view_key(key: &str) -> Option<(&str, &str, u64)> {
    if !validate_id(key) {
        return None;
    }
    let (writer, view) = key.strip_prefix("nv1:")?.split_once(':')?;
    let (name, version) = parse_notice_view(view)?;
    (!writer.is_empty()).then_some((writer, name, version))
}

fn check_key(key: Option<&str>) -> Result<(), InvalidParams> {
    if key.is_some_and(|key| key.len() > MAX_NOTICE_KEY_BYTES || !validate_id(key)) {
        return Err(InvalidParams::new("key"));
    }
    Ok(())
}

fn present_key<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

/// The sender-free body written before sending a notice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NoticeBody {
    pub view: String,
    #[serde(
        default,
        deserialize_with = "present_key",
        skip_serializing_if = "Option::is_none"
    )]
    pub key: Option<String>,
    pub data: Map<String, Value>,
}

impl NoticeBody {
    pub fn new(view: impl Into<String>, key: Option<String>, data: Map<String, Value>) -> Self {
        Self {
            view: view.into(),
            key,
            data,
        }
    }

    /// Validate the body and return its immutable storage bytes. The view's
    /// data schema is checked when drawing, not when the record is written.
    pub fn check(&self) -> Result<Vec<u8>, ErrorBody> {
        parse_notice_view(&self.view).ok_or_else(|| InvalidParams::new("view").refusal())?;
        check_key(self.key.as_deref()).map_err(|error| error.refusal())?;
        let bytes = serde_jcs::to_vec(self).map_err(|_| InvalidParams::new("body").refusal())?;
        if bytes.len() > MAX_NOTICE_RECORD_BYTES {
            return Err(
                ErrorBody::new(DISPLAY_TOO_LARGE, "notice record exceeds the limit")
                    .with_detail(json!({"kind": "notice", "max_bytes": MAX_NOTICE_RECORD_BYTES})),
            );
        }
        Ok(bytes)
    }
}

/// Notice params for both pushed `display.write` and pulled `display.read` replies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NoticeWrite {
    pub kind: String,
    pub runner: Principal,
    pub session: String,
    pub send_id: String,
    pub body: NoticeBody,
}

impl NoticeWrite {
    pub fn new(
        runner: Principal,
        session: impl Into<String>,
        send_id: impl Into<String>,
        body: NoticeBody,
    ) -> Self {
        Self {
            kind: "notice".into(),
            runner,
            session: session.into(),
            send_id: send_id.into(),
            body,
        }
    }

    /// `writer` is the daemon-attested module, never an identity from the body.
    pub fn check(&self, writer: &Principal) -> Result<Vec<u8>, ErrorBody> {
        crate::check_writer(writer)?;
        if self.kind != "notice" {
            return Err(InvalidParams::new("kind").refusal());
        }
        for (field, value) in [("session", &self.session), ("send_id", &self.send_id)] {
            if !validate_id(value) {
                return Err(InvalidParams::new(field).refusal());
            }
        }
        self.body.check()
    }
}

/// The immutable notice storage key; the destination runner is not part of it.
pub fn notice_idempotency_key(
    writer: &Principal,
    write: &NoticeWrite,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_jcs::to_vec(&(&write.session, writer, &write.send_id))
}

/// A notice record served on opt-in reads, after a successful join.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NoticeRecord {
    pub sender: String,
    pub view: String,
    #[serde(
        default,
        deserialize_with = "present_key",
        skip_serializing_if = "Option::is_none"
    )]
    pub key: Option<String>,
    pub data: Map<String, Value>,
}

impl NoticeRecord {
    /// Qualify the stored body's view with the verified writer. The caller
    /// still checks that the writer serves the resulting pin before joining.
    pub fn from_body(writer: &Principal, body: NoticeBody) -> Result<Self, ErrorBody> {
        crate::check_writer(writer)?;
        let Principal::Reserved { module_id } = writer else {
            unreachable!("check_writer requires a reserved principal");
        };
        let view = parse_notice_view(&body.view)
            .and_then(|(name, version)| notice_view_key(module_id, name, version))
            .ok_or_else(|| InvalidParams::new("view").refusal())?;
        check_key(body.key.as_deref()).map_err(|error| error.refusal())?;
        Ok(Self {
            sender: module_id.clone(),
            view,
            key: body.key,
            data: body.data,
        })
    }

    /// Check that the qualified view agrees with the served sender.
    pub fn check(&self) -> Result<(), InvalidParams> {
        if !parse_notice_view_key(&self.view).is_some_and(|(writer, _, _)| writer == self.sender) {
            return Err(InvalidParams::new("view"));
        }
        check_key(self.key.as_deref())
    }
}
