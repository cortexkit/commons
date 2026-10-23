//! Immutable references to the tool outputs in the request that wrote a reply.
//!
//! Runners supply the durably stored request numbering, never the current
//! lineage, and retain returned snapshots as long as any record names them.

use std::collections::BTreeMap;

use cortexkit_rich_ui_syntax::{check_token_slot, scan_references, Mode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::content_digest::sha256_hex;

pub const PLAN_FIELD: &str = "plan.items[].output_refs";
pub const MAX_REFERENCES: usize = 64;
pub const MAX_SNAPSHOT_BYTES: usize = 1_048_576;
pub const MAX_SET_BYTES: usize = 4_194_304;
pub const MAX_DESCRIPTOR_BYTES: usize = 4_096;
pub const MAX_PAGE_RECORDS: usize = 32;
pub const MAX_PAGE_BYTES: usize = 16_384;
pub const MAX_READ_BYTES: u64 = 131_072;

/// A reference-bearing top-level argument from a frozen provider catalog.
/// Kinds remain open strings so catalog metadata can be decoded independently
/// of the provider's Rust API.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReferenceSlot {
    name: String,
    kind: String,
}

impl ReferenceSlot {
    pub fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }
}

/// Check every supplied plan before any write, including explicit false.
/// An absent member means false; a present member must be a boolean.
pub fn check_plan(plan: &Value, advertised: bool) -> Result<(), &'static str> {
    if let Some(items) = plan.get("items").and_then(Value::as_array) {
        for item in items {
            if let Some(value) = item.get("output_refs") {
                if !advertised || !value.is_boolean() {
                    return Err(PLAN_FIELD);
                }
            }
        }
    }
    Ok(())
}

/// Descriptor carried only on display rows and opted-in tool call entries.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Descriptor {
    pub set_id: String,
    pub mid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    pub dialect: String,
    pub count: u64,
    pub digest: String,
}

impl Descriptor {
    pub fn new(
        set_id: impl Into<String>,
        mid: impl Into<String>,
        dialect: impl Into<String>,
        records: &[Record],
    ) -> Self {
        Self {
            set_id: set_id.into(),
            mid: mid.into(),
            call_key: None,
            dialect: dialect.into(),
            count: records.len() as u64,
            digest: records_digest(records),
        }
    }
    pub fn with_call_key(mut self, key: impl Into<String>) -> Self {
        self.call_key = Some(key.into());
        self
    }
    pub fn check(&self, records: &[Record]) -> bool {
        serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= MAX_DESCRIPTOR_BYTES)
            && self.count == records.len() as u64
            && self.digest == records_digest(records)
    }
}

/// One occurrence, in scanner encounter order. Unknown target metadata is
/// null, not guessed from later requests. State and reason are open strings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Record {
    pub ref_id: String,
    pub tag: Option<String>,
    pub part: Option<String>,
    pub call_key: Option<String>,
    pub tool: Option<String>,
    pub provider: Option<String>,
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Record {
    pub fn unresolved(
        ref_id: impl Into<String>,
        tag: Option<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            ref_id: ref_id.into(),
            tag,
            part: None,
            call_key: None,
            tool: None,
            provider: None,
            format: None,
            mime: None,
            bytes: None,
            sha256: None,
            state: "unresolved".into(),
            reason: Some(reason.into()),
        }
    }
}

/// JCS (RFC 8785) JSON array of records in source encounter order. Fields with
/// unknown values remain null; absent optional mime/reason members stay absent.
pub fn canonical_records(records: &[Record]) -> Vec<u8> {
    serde_jcs::to_vec(records).expect("records contain only JSON strings and integers")
}

pub fn records_digest(records: &[Record]) -> String {
    sha256_hex(&canonical_records(records))
}

/// The stored part a tag named in the writing request. Runners must supply
/// stored bytes, not the rendered text with its tag prefix or injected labels.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Target {
    pub part: String,
    pub call_key: Option<String>,
    pub tool: Option<String>,
    pub provider: Option<String>,
    pub format: String,
    pub mime: Option<String>,
    pub content: Vec<u8>,
    pub held: bool,
}

impl Target {
    /// A non-tool part; references to it record `unresolved` /
    /// `not_a_tool_output`, even if it has text.
    pub fn text(part: impl Into<String>, content: Vec<u8>) -> Self {
        Self {
            part: part.into(),
            call_key: None,
            tool: None,
            provider: None,
            format: "text".into(),
            mime: None,
            content,
            held: false,
        }
    }
    pub fn with_tool(
        mut self,
        call_key: impl Into<String>,
        tool: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        self.call_key = Some(call_key.into());
        self.tool = Some(tool.into());
        self.provider = Some(provider.into());
        self
    }
    pub fn with_image(mut self, mime: impl Into<String>) -> Self {
        self.format = "image".into();
        self.mime = Some(mime.into());
        self
    }
    pub fn held(mut self) -> Self {
        self.held = true;
        self
    }
}

/// A deterministic store projection. Persist its inputs at the writing commit
/// or persist its output there. Snapshots are keyed by SHA-256 for deduplication.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Resolution {
    pub records: Vec<Record>,
    pub snapshots: BTreeMap<String, Vec<u8>>,
    used_bytes: usize,
}

impl Resolution {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scan one model text part. Reuse the resolution across the message's
    /// parts so both caps apply to the whole message, not each part.
    pub fn reply(
        &mut self,
        source: &str,
        request: &BTreeMap<u64, Target>,
        mint: &mut impl FnMut() -> String,
    ) {
        for reference in scan_references(source, Mode::Reply).references {
            self.push(Some(reference.n), request, mint);
        }
    }

    /// Scan only declared string slots of an opted-in plan tool. Send-only
    /// tools have no catalog or plan item and must pass false. Missing and
    /// non-string arguments are not scanned.
    pub fn call(
        &mut self,
        arguments: &Value,
        slots: &[ReferenceSlot],
        enabled: bool,
        request: &BTreeMap<u64, Target>,
        mint: &mut impl FnMut() -> String,
    ) {
        if !enabled {
            return;
        }
        for slot in slots {
            let Some(source) = arguments.get(&slot.name).and_then(Value::as_str) else {
                continue;
            };
            if slot.kind == "token" {
                self.push(
                    check_token_slot(source).map(|reference| reference.n),
                    request,
                    mint,
                );
            } else if slot
                .kind
                .strip_prefix("source:")
                .is_some_and(|dialect| !dialect.is_empty())
            {
                for reference in scan_references(source, Mode::Program).references {
                    self.push(Some(reference.n), request, mint);
                }
            }
        }
    }

    fn push(
        &mut self,
        tag: Option<String>,
        request: &BTreeMap<u64, Target>,
        mint: &mut impl FnMut() -> String,
    ) {
        let reason = if self.records.len() >= MAX_REFERENCES {
            "too_many_refs"
        } else if tag.is_none() {
            "not_a_reference"
        } else {
            "unknown_tag"
        };
        let mut record = Record::unresolved(mint(), tag.clone(), reason);
        if self.records.len() < MAX_REFERENCES {
            if let Some(target) = tag
                .and_then(|tag| tag.parse::<u64>().ok())
                .and_then(|tag| request.get(&tag))
            {
                record.part = Some(target.part.clone());
                if target.call_key.is_none() {
                    record.reason = Some("not_a_tool_output".into());
                } else {
                    record.call_key.clone_from(&target.call_key);
                    record.tool.clone_from(&target.tool);
                    record.provider.clone_from(&target.provider);
                    record.format = Some(target.format.clone());
                    record.mime.clone_from(&target.mime);
                    record.bytes = Some(target.content.len() as u64);
                    let digest = sha256_hex(&target.content);
                    record.sha256 = Some(digest.clone());
                    record.reason = None;
                    if target.content.len() > MAX_SNAPSHOT_BYTES
                        || self.used_bytes + target.content.len() > MAX_SET_BYTES
                    {
                        record.state = "too_large".into();
                    } else {
                        record.state = if target.held { "held" } else { "resolved" }.into();
                        self.used_bytes += target.content.len();
                        self.snapshots
                            .entry(digest)
                            .or_insert_with(|| target.content.clone());
                    }
                }
            }
        }
        self.records.push(record);
    }
}

/// Session-bound list request. A cursor is the preceding record's opaque id.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ListRequest {
    pub set_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}
impl ListRequest {
    pub fn new(set_id: impl Into<String>) -> Self {
        Self {
            set_id: set_id.into(),
            after: None,
        }
    }
    pub fn after(mut self, id: impl Into<String>) -> Self {
        self.after = Some(id.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Page {
    pub records: Vec<Record>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}
impl Page {
    /// Page only a set already admitted on this session. The size limit covers
    /// the complete encoded JSON page, including its continuation cursor.
    pub fn from_records(records: &[Record], after: Option<&str>) -> Result<Self, &'static str> {
        let start = match after {
            None => 0,
            Some(id) => {
                records
                    .iter()
                    .position(|record| record.ref_id == id)
                    .ok_or("unknown_ref")?
                    + 1
            }
        };
        let mut page = Self {
            records: Vec::new(),
            next: None,
        };
        for (index, record) in records.iter().enumerate().skip(start) {
            let mut candidate = page.clone();
            candidate.records.push(record.clone());
            candidate.next = (index + 1 < records.len()).then(|| record.ref_id.clone());
            if candidate.records.len() > MAX_PAGE_RECORDS
                || serde_json::to_vec(&candidate).expect("JSON page").len() > MAX_PAGE_BYTES
            {
                break;
            }
            page = candidate;
        }
        if start < records.len() && page.records.is_empty() {
            return Err("reply_too_large");
        }
        Ok(page)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReadRequest {
    pub ref_id: String,
    pub offset: u64,
    pub length: u64,
}
impl ReadRequest {
    pub fn new(ref_id: impl Into<String>, offset: u64, length: u64) -> Self {
        Self {
            ref_id: ref_id.into(),
            offset,
            length,
        }
    }
    pub fn check(&self, size: u64) -> Result<(), &'static str> {
        if self.length > MAX_READ_BYTES {
            return Err("length");
        }
        if self.offset > size {
            return Err("offset");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReadReply {
    /// Base64 of the requested bytes, at most 131,072 decoded bytes.
    pub data: String,
    pub size: u64,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}
impl ReadReply {
    pub fn new(data: impl Into<String>, size: u64, format: impl Into<String>) -> Self {
        Self {
            data: data.into(),
            size,
            format: format.into(),
            mime: None,
        }
    }
    pub fn with_mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }
}

#[cfg(test)]
mod provider_parity {
    use super::ReferenceSlot;
    use cortexkit_role_tool_provider::catalog::ReferenceSlot as ProviderReferenceSlot;
    use serde_json::Value;

    fn check_slots(wire: &Value) {
        let runner: Vec<ReferenceSlot> = serde_json::from_value(wire.clone()).unwrap();
        let provider: Vec<ProviderReferenceSlot> = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(
            serde_json::to_vec(&runner).unwrap(),
            serde_json::to_vec(&provider).unwrap()
        );
        assert_eq!(serde_json::to_value(&runner).unwrap(), *wire);
        for (slot, provider) in runner.iter().zip(&provider) {
            let constructed = ReferenceSlot::new(&provider.name, &provider.kind);
            assert_eq!(slot.name(), provider.name);
            assert_eq!(slot.kind(), provider.kind);
            assert_eq!(
                serde_json::to_vec(&constructed).unwrap(),
                serde_json::to_vec(provider).unwrap()
            );
        }
    }

    #[test]
    fn reference_slots_match_provider_vector_bytes() {
        let vectors: Value = serde_json::from_str(
            cortexkit_role_tool_provider::vectors::file("reference-slots.json").unwrap(),
        )
        .unwrap();
        for slots in vectors["valid"].as_array().unwrap() {
            check_slots(slots);
        }
        for case in vectors["invalid"].as_array().unwrap() {
            check_slots(&case["slots"]);
        }
        check_slots(&vectors["show_ui"]["reference_slots"]);
    }
}
