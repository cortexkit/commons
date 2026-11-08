//! The neutral part model. Reads present every message as these parts, and
//! views address them, whatever the host stores (OpenCode parts, Pi journal
//! rows, Anthropic, OpenAI or Gemini blocks). The runner maps its host format
//! both ways; a provider never sees host JSON.
//!
//! The kinds cover every content type of the AI SDK's provider prompt
//! (`LanguageModelV4`), which every model provider maps to: text, reasoning,
//! tool calls with their results, files of any media type, and
//! provider-specific items the runner must send back as they are.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One part of a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Part {
    /// Stable for the life of the message, unique within it. Views address
    /// parts by it.
    pub id: String,
    /// The part's tag number, when it has one (`is_numbered` in the
    /// `cortexkit-compaction-view` crate). The runner counts it: the
    /// checkpoint of the session's latest compaction marker plus the part's
    /// position among the numbered parts from the marker's first kept entry
    /// on (T1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub tag: Option<u64>,
    /// What the model provider checks when the part is sent back.
    #[serde(default, skip_serializing_if = "Seal::is_none")]
    pub seal: Seal,
    /// Optional count metadata in the page or notice's `token_family`.
    /// Reserved in the envelope for every kind, including unknown kinds;
    /// conforming senders use it only as W4.11 allows. Never included in a
    /// checksum; absent means estimate. Decoding checks shape, not conformance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<crate::tokens::PartTokens>,
    /// Checksum of the part without `id`, `tag`, `digest` and `tokens`
    /// ([`body_digest`]). A replace edit names the checksum it was computed
    /// from, so it never lands on a changed part.
    pub digest: String,
    #[serde(flatten)]
    pub body: PartBody,
}

impl Part {
    /// Build a part from its full body, computing its checksum. Pass the full
    /// form, never a compact one. Changing a hashed field afterwards doesn't
    /// refresh `digest`.
    pub fn new(id: impl Into<String>, body: PartBody) -> Self {
        let seal = Seal::None;
        let digest = body_digest(seal.clone(), &body);
        Self {
            id: id.into(),
            tag: None,
            tokens: None,
            seal,
            digest,
            body,
        }
    }

    pub fn with_tag(mut self, tag: u64) -> Self {
        self.tag = Some(tag);
        self
    }

    pub fn with_tokens(mut self, tokens: crate::tokens::PartTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    /// Set the seal and recompute the checksum, so the body must be full.
    pub fn with_seal(mut self, seal: Seal) -> Self {
        self.digest = body_digest(seal.clone(), &self.body);
        self.seal = seal;
        self
    }

    /// The compact form of a full part: tool output, the text of sends
    /// delivered with a tool result, and reasoning text are left out, and
    /// only their sizes remain. The checksum stays the one computed over W7's
    /// full-detail projection, so an edit computed from compact detail still names
    /// the exact part. Notices carry parts in this form.
    pub fn compact(&self) -> Part {
        let mut body = self.body.clone();
        match &mut body {
            PartBody::Tool(tool) => {
                tool.output = None;
                for delivered in &mut tool.delivered {
                    delivered.text = None;
                }
            }
            PartBody::Reasoning { text, .. } => *text = None,
            PartBody::Text { .. }
            | PartBody::File(_)
            | PartBody::Opaque(_)
            | PartBody::Unknown { .. } => {}
        }
        Part {
            id: self.id.clone(),
            tag: self.tag,
            tokens: self.tokens,
            seal: self.seal.clone(),
            digest: self.digest.clone(),
            body,
        }
    }
}

open_tagged! {
/// What a part holds.
#[derive(Clone, Debug, PartialEq)]
pub enum PartBody, "kind" {
    Text => "text" {
        text: String,
    },
    Reasoning => "reasoning" {
        /// The visible reasoning text or summary, when the model provider
        /// returned any. Left out in the compact form.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// The size of the visible text in bytes, in both forms.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(deserialize_with = "crate::integer::optional")]
        text_bytes: Option<u64>,
    },
    /// A tool call and its result as one item, at the call's position, even
    /// when the host stores the call and the result separately. Dropping a
    /// tool part removes both, so a call is never separated from its result.
    Tool => "tool"(payload: ToolPart),
    /// Media of any type, images included. Metadata only: reads never return
    /// a transcript file's bytes (inserted images are separate, see W6.2).
    File => "file"(payload: FileMedia),
    /// Provider-specific content the runner sends back as it is (the AI SDK's
    /// `custom`). It can only be dropped.
    Opaque => "opaque"(payload: Opaque),
}
}

/// What the model provider checks when a part is sent back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum Seal {
    /// Nothing to check.
    #[default]
    None,
    /// A provider signature binds the part to what came before it: Anthropic
    /// thinking and redacted thinking, Gemini thought signatures.
    /// [`crate::rules`] says which edits that forbids.
    Signed,
    /// Opaque provider content, not bound to earlier history: OpenAI
    /// reasoning with encrypted content.
    Encrypted,
    /// An unrecognised wire value, kept verbatim; handled as CONTRACT.md
    /// W1.5 says.
    Unknown(String),
}

open_string! { Seal {
    None => "none",
    Signed => "signed",
    Encrypted => "encrypted",
} }

impl Seal {
    pub fn is_none(&self) -> bool {
        *self == Seal::None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolPart {
    pub call_id: String,
    pub name: String,
    /// The call's arguments, as the model wrote them: usually an object, but
    /// any JSON value.
    pub input: Value,
    /// The model provider ran the tool itself (web search, code execution,
    /// image generation), and its result is part of the assistant message.
    /// The host replays such a call as it is, so it can only be dropped.
    #[serde(default, skip_serializing_if = "is_false")]
    pub provider_executed: bool,
    pub status: ToolStatus,
    /// The result's text: the output when completed, the error text on
    /// `error`, the reason on `denied`. Absent while the call is running, and
    /// in the compact form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "OutputFormat::is_text")]
    pub output_format: OutputFormat,
    /// The size of `output` in bytes, in both forms. Absent while the call is
    /// running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub output_bytes: Option<u64>,
    /// Files and opaque items the result carries besides its text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// A provider-executed call's approval answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<Approval>,
    /// Text sent with delivery `tool_result` and rendered after this result,
    /// in delivery order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivered: Vec<Delivered>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum ToolStatus {
    /// Running, or waiting for approval.
    Running,
    Completed,
    Error,
    /// The call was not executed.
    Denied,
    /// An unrecognised wire value, kept verbatim; handled as CONTRACT.md
    /// W1.5 says.
    Unknown(String),
}

open_string! { ToolStatus {
    Running => "running",
    Completed => "completed",
    Error => "error",
    Denied => "denied",
} }

/// How a tool result's text is to be read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum OutputFormat {
    #[default]
    Text,
    /// `output` is the canonical JSON text of a JSON result.
    Json,
    /// An unrecognised wire value, kept verbatim; handled as CONTRACT.md
    /// W1.5 says.
    Unknown(String),
}

open_string! { OutputFormat {
    Text => "text",
    Json => "json",
} }

impl OutputFormat {
    pub fn is_text(&self) -> bool {
        *self == OutputFormat::Text
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Approval {
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A supported rendering form for a send delivered with a tool result.
/// More forms may be added; a decoder skips strings it doesn't know.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RideForm {
    ToolResult,
    TurnSystem,
    /// A turn-scoped system message the provider clears at the next
    /// user-role message. A runner renders it only on a route that clears;
    /// on any other route it's omitted (the view library's `cleared_ride`).
    TurnSystemCleared,
}

fn ride_forms<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<RideForm>>, D::Error> {
    let forms = Option::<Vec<String>>::deserialize(deserializer)?;
    Ok(forms.map(|forms| {
        let mut known: Vec<_> = forms
            .iter()
            .filter_map(|form| match form.as_str() {
                "tool_result" => Some(RideForm::ToolResult),
                "turn_system" => Some(RideForm::TurnSystem),
                "turn_system_cleared" => Some(RideForm::TurnSystemCleared),
                _ => None,
            })
            .collect();
        if known.is_empty() {
            known.push(RideForm::ToolResult);
        }
        known
    }))
}

/// Text a module sent with delivery `tool_result`, rendered alongside the
/// tool result it rode on, never as its own transcript entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Delivered {
    /// The module that sent it.
    pub sender: String,
    /// The send's id. Never backfilled on an entry already served.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_id: Option<String>,
    /// The send's delivery list restricted to ride forms, in list order.
    /// Absent means append only; unknown forms are skipped on decode.
    #[serde(
        default,
        deserialize_with = "ride_forms",
        skip_serializing_if = "Option::is_none"
    )]
    pub forms: Option<Vec<RideForm>>,
    /// Ride-text count in the page or notice's tokenizer family (W4.11).
    /// May arrive later; excluded from all checksums, as part counts are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<crate::tokens::TokenCount>,
    /// The plan template it was rendered from, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Left out in the compact form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(deserialize_with = "crate::integer::required")]
    pub text_bytes: u64,
}

impl Delivered {
    pub fn new(sender: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            sender: sender.into(),
            send_id: None,
            forms: None,
            tokens: None,
            template: None,
            text_bytes: text.len() as u64,
            text: Some(text),
        }
    }

    pub fn with_template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }

    pub fn with_send_id(mut self, send_id: impl Into<String>) -> Self {
        self.send_id = Some(send_id.into());
        self
    }

    pub fn with_tokens(mut self, tokens: crate::tokens::TokenCount) -> Self {
        self.tokens = Some(tokens);
        self
    }

    pub fn with_forms(mut self, forms: Vec<RideForm>) -> Self {
        self.forms = Some(if forms.is_empty() {
            vec![RideForm::ToolResult]
        } else {
            forms
        });
        self
    }

    /// Known forms in delivery order, with the safe append-only default.
    pub fn forms(&self) -> &[RideForm] {
        self.forms
            .as_deref()
            .filter(|forms| !forms.is_empty())
            .unwrap_or(&[RideForm::ToolResult])
    }
}

/// An item a tool result carries besides its text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Attachment {
    /// Stable within its tool part; a replace of `attachments` keeps a subset
    /// by these ids.
    pub id: String,
    #[serde(flatten)]
    pub item: AttachmentItem,
}

open_tagged! {
#[derive(Clone, Debug, PartialEq)]
pub enum AttachmentItem, "kind" {
    File => "file"(payload: FileMedia),
    Opaque => "opaque"(payload: Opaque),
}
}

/// A file of any media type. A provider only needs to know what it is and how
/// big; the bytes stay with the runner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FileMedia {
    /// The media type, for example `image/png`, `video/mp4`, `application/pdf`.
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub transport: Transport,
    /// Size in bytes, when the runner knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub size: Option<u64>,
    /// Pixels, for images and video, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub width: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub height: Option<u64>,
    /// For audio and video, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub duration_ms: Option<u64>,
    /// The model produced it as part of its reasoning (the AI SDK's
    /// `reasoning-file`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub reasoning: bool,
}

impl FileMedia {
    /// A file sent as bytes, with no size details beyond its byte count.
    pub fn inline(mime: impl Into<String>, size: Option<u64>) -> Self {
        Self {
            mime: mime.into(),
            name: None,
            transport: Transport::Inline,
            size,
            width: None,
            height: None,
            duration_ms: None,
            reasoning: false,
        }
    }

    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }
}

/// How the host sends a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum Transport {
    /// Bytes.
    Inline,
    Url,
    /// A model provider's file id.
    Reference,
    Text,
    /// An unrecognised wire value, kept verbatim; handled as CONTRACT.md
    /// W1.5 says.
    Unknown(String),
}

open_string! { Transport {
    Inline => "inline",
    Url => "url",
    Reference => "reference",
    Text => "text",
} }

/// Provider-specific content, sent back as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Opaque {
    /// `provider.type`, for example `anthropic.compaction`.
    pub provider_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub size: Option<u64>,
}

/// Checksum of a part without its `id`, `tag`, `digest`, part `tokens` or
/// delivered-item `tokens`: its seal (left out when `none`), kind and kind
/// fields. Pass the full body: a compact body hashes to a different value.
pub fn body_digest(seal: Seal, body: &PartBody) -> String {
    #[derive(Serialize)]
    struct Digested<'a> {
        #[serde(skip_serializing_if = "Seal::is_none")]
        seal: Seal,
        #[serde(flatten)]
        body: &'a PartBody,
    }
    let mut value =
        serde_json::to_value(Digested { seal, body }).expect("contract types always serialize");
    strip_delivered_tokens(&mut value);
    crate::digest::content_digest(&value)
}

/// Strip only known tool ride-count metadata, never opaque same-named fields.
pub(crate) fn strip_delivered_tokens(part: &mut Value) {
    if part["kind"] == "tool" {
        for delivered in part
            .get_mut("delivered")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            delivered.as_object_mut().unwrap().remove("tokens");
        }
    }
}

impl ToolPart {
    /// Build the value with its required fields.
    pub fn new(
        call_id: impl Into<String>,
        name: impl Into<String>,
        input: Value,
        status: ToolStatus,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            input,
            status,
            provider_executed: Default::default(),
            output: None,
            output_format: Default::default(),
            output_bytes: None,
            attachments: Vec::new(),
            approval: None,
            delivered: Vec::new(),
        }
    }

    pub fn with_provider_executed(mut self, provider_executed: bool) -> Self {
        self.provider_executed = provider_executed;
        self
    }

    /// Set the output text. This doesn't set `output_bytes`.
    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.output = Some(output.into());
        self
    }

    pub fn with_output_format(mut self, output_format: OutputFormat) -> Self {
        self.output_format = output_format;
        self
    }

    pub fn with_output_bytes(mut self, output_bytes: u64) -> Self {
        self.output_bytes = Some(output_bytes);
        self
    }

    pub fn with_attachments(mut self, attachments: Vec<Attachment>) -> Self {
        self.attachments = attachments;
        self
    }

    pub fn with_approval(mut self, approval: Approval) -> Self {
        self.approval = Some(approval);
        self
    }

    pub fn with_delivered(mut self, delivered: Vec<Delivered>) -> Self {
        self.delivered = delivered;
        self
    }
}

impl Approval {
    /// Build the value with its required fields.
    pub fn new(approved: bool) -> Self {
        Self {
            approved,
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

impl Attachment {
    /// Build the value with its required fields.
    pub fn new(id: impl Into<String>, item: AttachmentItem) -> Self {
        Self {
            id: id.into(),
            item,
        }
    }
}

impl FileMedia {
    /// Build the value with its required fields.
    pub fn new(mime: impl Into<String>, transport: Transport) -> Self {
        Self {
            mime: mime.into(),
            transport,
            name: None,
            size: None,
            width: None,
            height: None,
            duration_ms: None,
            reasoning: Default::default(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    pub fn with_width(mut self, width: u64) -> Self {
        self.width = Some(width);
        self
    }

    pub fn with_height(mut self, height: u64) -> Self {
        self.height = Some(height);
        self
    }

    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    pub fn with_reasoning(mut self, reasoning: bool) -> Self {
        self.reasoning = reasoning;
        self
    }
}

impl Opaque {
    /// Build the value with its required fields.
    pub fn new(provider_kind: impl Into<String>) -> Self {
        Self {
            provider_kind: provider_kind.into(),
            size: None,
        }
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(output: &str) -> PartBody {
        PartBody::Tool(ToolPart {
            call_id: "call_1".into(),
            name: "read".into(),
            input: json!({"path": "a.rs"}),
            provider_executed: false,
            status: ToolStatus::Completed,
            output: Some(output.into()),
            output_format: OutputFormat::Text,
            output_bytes: Some(output.len() as u64),
            attachments: vec![],
            approval: None,
            delivered: vec![],
        })
    }

    #[test]
    fn a_tool_part_round_trips_with_its_kind_inline() {
        let part = Part::new("p1", tool("fn main() {}")).with_tag(42);
        let wire = serde_json::to_value(&part).unwrap();
        assert_eq!(wire["kind"], "tool");
        assert_eq!(wire["tag"], 42);
        assert_eq!(wire["name"], "read");
        assert!(wire.get("seal").is_none());
        assert!(wire.get("provider_executed").is_none());
        let back: Part = serde_json::from_value(wire).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn files_and_opaque_items_round_trip_as_parts_and_attachments() {
        let video = PartBody::File(FileMedia {
            duration_ms: Some(4_000),
            ..FileMedia::inline("video/mp4", Some(1 << 20))
        });
        let part = Part::new("p1", video).with_seal(Seal::Signed);
        let wire = serde_json::to_value(&part).unwrap();
        assert_eq!(wire["kind"], "file");
        assert_eq!(wire["seal"], "signed");
        assert_eq!(serde_json::from_value::<Part>(wire).unwrap(), part);

        let attachment = Attachment {
            id: "a1".into(),
            item: AttachmentItem::Opaque(Opaque {
                provider_kind: "anthropic.container".into(),
                size: None,
            }),
        };
        let wire = serde_json::to_value(&attachment).unwrap();
        assert_eq!(wire["kind"], "opaque");
        assert_eq!(
            serde_json::from_value::<Attachment>(wire).unwrap(),
            attachment
        );
    }

    #[test]
    fn the_compact_form_keeps_sizes_and_the_full_checksum() {
        let full = Part::new(
            "p1",
            PartBody::Reasoning {
                text: Some("long thinking".into()),
                text_bytes: Some(13),
            },
        )
        .with_seal(Seal::Signed);
        let compact = full.compact();
        assert_eq!(compact.digest, full.digest);
        assert_eq!(
            compact.body,
            PartBody::Reasoning {
                text: None,
                text_bytes: Some(13),
            }
        );

        let mut body = tool("output");
        if let PartBody::Tool(tool) = &mut body {
            tool.delivered
                .push(Delivered::new("example-notifier", "reminder"));
        }
        let compact = Part::new("p2", body).with_tag(1).compact();
        let PartBody::Tool(tool) = compact.body else {
            panic!("a tool part stays one");
        };
        assert_eq!(tool.output, None);
        assert_eq!(tool.output_bytes, Some(6));
        assert_eq!(tool.delivered[0].text, None);
        assert_eq!(tool.delivered[0].text_bytes, 8);
    }

    #[test]
    fn the_digest_follows_the_body_and_seal_not_the_id_or_tag() {
        let body = PartBody::Text {
            text: "hello".into(),
        };
        let one = Part::new("a", body.clone()).with_tag(1);
        let two = Part::new("b", body.clone());
        assert_eq!(one.digest, two.digest);
        assert_ne!(
            one.digest,
            Part::new("a", body)
                .with_tag(1)
                .with_seal(Seal::Signed)
                .digest
        );
        let changed = Part::new(
            "a",
            PartBody::Text {
                text: "hello!".into(),
            },
        )
        .with_tag(1);
        assert_ne!(one.digest, changed.digest);
    }
}
