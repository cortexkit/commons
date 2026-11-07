//! Immutable display records for keyed wake sends. No storage or row lookup is performed here.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{schema::validate_id, ErrorBody, InvalidParams, Principal};

pub const WAKE_RECORD_WRITER_CAPABILITY: &str = "wake-record-writer/v1";
pub const WAKE_RECORD_DISPLAY_CAPABILITY: &str = "wake-record-display/v1";
pub const MAX_WAKE_RECORD_BYTES: usize = 16 * 1024;
pub const MAX_WAKE_ITEMS: usize = 64;
pub const MAX_WAKE_EXCERPT_CHARS: usize = 200;
pub const WAKE_RETENTION_DAYS: u64 = 180;

/// Registry identity and optional fallback name, using the fleet's agent_id/display_name concepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WakeAgent {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl WakeAgent {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            display_name: None,
        }
    }

    pub fn check(&self) -> Result<(), InvalidParams> {
        let suffix = self.agent_id.strip_prefix("agent_").unwrap_or("");
        if ![8, 16].contains(&suffix.len())
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(InvalidParams::new("agent_id"));
        }
        Ok(())
    }
}

/// An open item kind and its refs. Readers retain unfamiliar kinds for plain cards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WakeItem {
    pub kind: String,
    pub refs: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<WakeAgent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<WakeAgent>,
    pub at: i64,
    pub excerpt: String,
}

impl WakeItem {
    pub fn new(
        kind: impl Into<String>,
        refs: Map<String, Value>,
        at: i64,
        excerpt: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            refs,
            from: None,
            to: None,
            at,
            excerpt: excerpt.into(),
        }
    }

    pub fn check(&self) -> Result<(), InvalidParams> {
        if self.excerpt.chars().count() > MAX_WAKE_EXCERPT_CHARS {
            return Err(InvalidParams::new("excerpt"));
        }
        for (field, agent) in [("from", &self.from), ("to", &self.to)] {
            if let Some(agent) = agent {
                agent.check().map_err(|_| InvalidParams::new(field))?;
            }
        }
        match self.kind.as_str() {
            "peer_message" => {
                let id = self.id("id")?;
                if !id.starts_with("pm_") || id.len() == 3 {
                    return Err(InvalidParams::new("refs.id"));
                }
                if self.from.is_none() {
                    return Err(InvalidParams::new("from"));
                }
                if self.to.is_none() {
                    return Err(InvalidParams::new("to"));
                }
            }
            "room_post" => {
                self.id("room")?;
                let first = self.sequence("first_seq")?;
                let last = self.sequence("last_seq")?;
                if first > last {
                    return Err(InvalidParams::new("refs.last_seq"));
                }
            }
            "worker_question" => {
                self.id("ask")?;
                self.id("task")?;
            }
            "work_result" => {
                self.id("task")?;
                for field in ["run", "item"] {
                    if self.refs.contains_key(field) {
                        self.id(field)?;
                    }
                }
            }
            "sidekick_answer" => {
                self.id("id")?;
            }
            "approval_card" => {
                let id = self.id("id")?;
                if !(id.starts_with("acc_") || id.starts_with("el_"))
                    || id
                        .split_once('_')
                        .is_none_or(|(_, suffix)| suffix.is_empty())
                {
                    return Err(InvalidParams::new("refs.id"));
                }
            }
            "notice" => {
                self.id("family")?;
                if self
                    .refs
                    .get("title")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err(InvalidParams::new("refs.title"));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Writers use the closed vocabulary even though readers accept future kinds.
    pub fn check_writer(&self) -> Result<(), InvalidParams> {
        self.check()?;
        match self.kind.as_str() {
            "peer_message" | "room_post" | "worker_question" | "work_result"
            | "sidekick_answer" | "approval_card" | "notice" => Ok(()),
            _ => Err(InvalidParams::new("kind")),
        }
    }

    fn id(&self, field: &str) -> Result<&str, InvalidParams> {
        self.refs
            .get(field)
            .and_then(Value::as_str)
            .filter(|id| validate_id(id))
            .ok_or_else(|| InvalidParams::new(format!("refs.{field}")))
    }

    fn sequence(&self, field: &str) -> Result<u64, InvalidParams> {
        self.refs
            .get(field)
            .and_then(Value::as_u64)
            .ok_or_else(|| InvalidParams::new(format!("refs.{field}")))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WakeRecord {
    pub items: Vec<WakeItem>,
    #[serde(
        default,
        rename = "omittedItems",
        skip_serializing_if = "Option::is_none"
    )]
    pub omitted_items: Option<u64>,
}

impl WakeRecord {
    pub fn new(items: Vec<WakeItem>) -> Self {
        Self {
            items,
            omitted_items: None,
        }
    }

    /// Reader validation preserves unknown kinds but never accepts invalid known refs.
    pub fn check(&self) -> Result<(), ErrorBody> {
        self.bytes()?;
        for item in &self.items {
            item.check().map_err(|error| error.refusal())?;
        }
        Ok(())
    }

    pub fn check_writer(&self) -> Result<(), ErrorBody> {
        self.check()?;
        for item in &self.items {
            item.check_writer().map_err(|error| error.refusal())?;
        }
        Ok(())
    }

    fn bytes(&self) -> Result<Vec<u8>, ErrorBody> {
        let bytes = serde_jcs::to_vec(self).map_err(|_| InvalidParams::new("body").refusal())?;
        if self.items.len() > MAX_WAKE_ITEMS || bytes.len() > MAX_WAKE_RECORD_BYTES {
            return Err(
                ErrorBody::new(crate::DISPLAY_TOO_LARGE, "wake record exceeds the limit")
                    .with_detail(json!({"kind":"wake", "max_bytes":MAX_WAKE_RECORD_BYTES})),
            );
        }
        Ok(bytes)
    }
}

/// Separate params for the same display.write operation; identity comes only from the route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WakeWrite {
    pub kind: String,
    pub runner: Principal,
    pub session: String,
    pub send_id: String,
    pub body: WakeRecord,
}

impl WakeWrite {
    pub fn new(
        runner: Principal,
        session: impl Into<String>,
        send_id: impl Into<String>,
        body: WakeRecord,
    ) -> Self {
        Self {
            kind: "wake".into(),
            runner,
            session: session.into(),
            send_id: send_id.into(),
            body,
        }
    }

    /// The principal must be the daemon-attested route opener, never a payload claim.
    pub fn check(&self, writer: &Principal) -> Result<Vec<u8>, ErrorBody> {
        crate::check_writer(writer)?;
        if self.kind != "wake" {
            return Err(InvalidParams::new("kind").refusal());
        }
        for (field, value) in [("session", &self.session), ("send_id", &self.send_id)] {
            if !validate_id(value) {
                return Err(InvalidParams::new(field).refusal());
            }
        }
        self.body.check_writer()?;
        self.body.bytes()
    }
}

/// Session-qualified storage key, distinct from a row's cross-session lookup pair.
pub fn wake_idempotency_key(
    writer: &Principal,
    write: &WakeWrite,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_jcs::to_vec(&(&write.session, writer, &write.send_id))
}
