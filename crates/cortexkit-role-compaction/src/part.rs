//! The neutral part model. Reads present every message as these parts, and
//! views address them, whatever the host stores (OpenCode parts, Pi journal
//! rows, Anthropic, OpenAI or Gemini blocks). The runner maps its host format
//! both ways; a provider never sees host JSON.
//!
//! The kinds cover every content type of the AI SDK's provider prompt
//! (`LanguageModelV4`), which every model provider maps to: text, reasoning,
//! tool calls with their results, files of any media type, and
//! provider-specific items the runner must send back as they are.

use std::fmt;
use std::sync::{Arc, OnceLock};

use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::digest::Digest;
use crate::text::Text;

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
    pub digest: Digest,
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
                if let Some(extras) = tool.extras.as_deref_mut() {
                    for delivered in &mut extras.delivered {
                        delivered.text = None;
                    }
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
        text: Text,
    },
    Reasoning => "reasoning" {
        /// The visible reasoning text or summary, when the model provider
        /// returned any. Left out in the compact form.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<Text>,
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

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ToolPart {
    pub call_id: String,
    pub name: String,
    /// The call's arguments, as the model wrote them: usually an object, but
    /// any JSON value.
    pub input: ToolInput,
    /// The model provider ran the tool itself (web search, code execution,
    /// image generation), and its result is part of the assistant message.
    /// The host replays such a call as it is, so it can only be dropped.
    pub provider_executed: bool,
    pub status: ToolStatus,
    /// The result's text: the output when completed, the error text on
    /// `error`, the reason on `denied`. Absent while the call is running, and
    /// in the compact form.
    pub output: Option<Text>,
    pub output_format: OutputFormat,
    /// The size of `output` in bytes, in both forms. Absent while the call is
    /// running.
    pub output_bytes: Option<u64>,
    /// Attachments, approval and delivered text, which few calls carry, in
    /// one box so a tool part stays small. Never an empty box: `None` exactly
    /// when all three are empty. Read them with [`ToolPart::attachments`],
    /// [`ToolPart::approval`] and [`ToolPart::delivered`]; on the wire they are
    /// fields of the tool part.
    extras: Option<Box<ToolExtras>>,
}

/// What a tool part carries besides its call and its result text.
#[derive(Clone, Debug, Default, PartialEq)]
struct ToolExtras {
    attachments: Vec<Attachment>,
    approval: Option<Approval>,
    delivered: Vec<Delivered>,
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
/// More forms may be added; the `forms` list decoder skips strings it doesn't
/// know.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RideForm {
    ToolResult,
    TurnSystem,
    /// A turn-scoped system message the provider clears at the next
    /// user-role message. A ride delivered this way renders only on a route
    /// that clears and is omitted on any other (the view library's
    /// `cleared_ride`).
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
    pub text: Option<Text>,
    #[serde(deserialize_with = "crate::integer::required")]
    pub text_bytes: u64,
}

impl Delivered {
    pub fn new(sender: impl Into<String>, text: impl Into<Text>) -> Self {
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
pub fn body_digest(seal: Seal, body: &PartBody) -> Digest {
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
    crate::digest::digest_of(&value)
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
        input: impl Into<ToolInput>,
        status: ToolStatus,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            input: input.into(),
            status,
            provider_executed: Default::default(),
            output: None,
            output_format: Default::default(),
            output_bytes: None,
            extras: None,
        }
    }

    pub fn with_provider_executed(mut self, provider_executed: bool) -> Self {
        self.provider_executed = provider_executed;
        self
    }

    /// Set the output text. This doesn't set `output_bytes`.
    pub fn with_output(mut self, output: impl Into<Text>) -> Self {
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
        self.set_attachments(attachments);
        self
    }

    pub fn with_approval(mut self, approval: Approval) -> Self {
        self.set_approval(Some(approval));
        self
    }

    pub fn with_delivered(mut self, delivered: Vec<Delivered>) -> Self {
        self.set_delivered(delivered);
        self
    }

    /// Files and opaque items the result carries besides its text; empty when
    /// it carries none.
    pub fn attachments(&self) -> &[Attachment] {
        self.extras
            .as_deref()
            .map_or(&[][..], |extras| &extras.attachments)
    }

    /// A provider-executed call's approval answer.
    pub fn approval(&self) -> Option<&Approval> {
        self.extras.as_deref()?.approval.as_ref()
    }

    /// Text sent with delivery `tool_result` and rendered after this result,
    /// in delivery order; `None` when none was.
    pub fn delivered(&self) -> Option<&[Delivered]> {
        let delivered = &self.extras.as_deref()?.delivered;
        (!delivered.is_empty()).then_some(delivered.as_slice())
    }

    /// Replace the attachments; an empty list removes them.
    pub fn set_attachments(&mut self, attachments: Vec<Attachment>) {
        self.edit_extras(|extras| extras.attachments = attachments);
    }

    /// Replace the approval answer; `None` removes it.
    pub fn set_approval(&mut self, approval: Option<Approval>) {
        self.edit_extras(|extras| extras.approval = approval);
    }

    /// Replace the delivered text; an empty list removes it.
    pub fn set_delivered(&mut self, delivered: Vec<Delivered>) {
        self.edit_extras(|extras| extras.delivered = delivered);
    }

    /// The delivered items, to edit in place (their `tokens`, say); an empty
    /// slice when there are none. A slice can't change length, so editing it
    /// never adds or removes the box.
    pub fn delivered_mut(&mut self) -> &mut [Delivered] {
        match self.extras.as_deref_mut() {
            Some(extras) => &mut extras.delivered,
            None => &mut [],
        }
    }

    /// Every change to the lists' lengths goes through here, so the box exists
    /// exactly when one of them holds something: it is allocated only when an
    /// edit leaves something in it, and freed when an edit empties it.
    fn edit_extras<R>(&mut self, edit: impl FnOnce(&mut ToolExtras) -> R) -> R {
        match self.extras.as_deref_mut() {
            Some(extras) => {
                let out = edit(extras);
                if extras.is_empty() {
                    self.extras = None;
                }
                out
            }
            None => {
                // Empty vectors and `None` allocate nothing, so trying an edit
                // here costs no allocation when it leaves the fields empty.
                let mut extras = ToolExtras::default();
                let out = edit(&mut extras);
                if !extras.is_empty() {
                    self.extras = Some(Box::new(extras));
                }
                out
            }
        }
    }
}

impl ToolExtras {
    fn is_empty(&self) -> bool {
        self.attachments.is_empty() && self.approval.is_none() && self.delivered.is_empty()
    }
}

impl PartialEq for ToolPart {
    /// Field by field, reading the three boxed fields through their readers.
    fn eq(&self, other: &Self) -> bool {
        self.call_id == other.call_id
            && self.name == other.name
            && self.input == other.input
            && self.provider_executed == other.provider_executed
            && self.status == other.status
            && self.output == other.output
            && self.output_format == other.output_format
            && self.output_bytes == other.output_bytes
            && self.attachments() == other.attachments()
            && self.approval() == other.approval()
            && self.delivered() == other.delivered()
    }
}

/// A tool part as it travels: every field flat, in wire order, with the same
/// defaults as before the extras were boxed. Decoding goes through it, so
/// the box changes neither what a decoder accepts nor its error messages, and
/// nothing is buffered on the way.
#[derive(Deserialize)]
#[serde(rename = "ToolPart")]
struct ToolPartFields {
    call_id: String,
    name: String,
    input: ToolInput,
    #[serde(default)]
    provider_executed: bool,
    status: ToolStatus,
    #[serde(default)]
    output: Option<Text>,
    #[serde(default)]
    output_format: OutputFormat,
    #[serde(default)]
    #[serde(deserialize_with = "crate::integer::optional")]
    output_bytes: Option<u64>,
    #[serde(default)]
    attachments: Vec<Attachment>,
    #[serde(default)]
    approval: Option<Approval>,
    #[serde(default)]
    delivered: Vec<Delivered>,
}

/// The same fields, borrowed, for encoding: the same names, order and
/// omissions as before the extras were boxed.
#[derive(Serialize)]
#[serde(rename = "ToolPart")]
struct ToolPartWire<'a> {
    call_id: &'a str,
    name: &'a str,
    input: &'a ToolInput,
    #[serde(skip_serializing_if = "is_false")]
    provider_executed: bool,
    status: &'a ToolStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<&'a Text>,
    #[serde(skip_serializing_if = "is_text_format")]
    output_format: &'a OutputFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_bytes: Option<u64>,
    #[serde(skip_serializing_if = "is_empty_list")]
    attachments: &'a [Attachment],
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<&'a Approval>,
    #[serde(skip_serializing_if = "is_empty_list")]
    delivered: &'a [Delivered],
}

fn is_text_format(format: &&OutputFormat) -> bool {
    format.is_text()
}

fn is_empty_list<T>(items: &&[T]) -> bool {
    items.is_empty()
}

impl Serialize for ToolPart {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ToolPartWire {
            call_id: &self.call_id,
            name: &self.name,
            input: &self.input,
            provider_executed: self.provider_executed,
            status: &self.status,
            output: self.output.as_ref(),
            output_format: &self.output_format,
            output_bytes: self.output_bytes,
            attachments: self.attachments(),
            approval: self.approval(),
            delivered: self.delivered().unwrap_or_default(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ToolPart {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = ToolPartFields::deserialize(deserializer)?;
        let extras = ToolExtras {
            attachments: fields.attachments,
            approval: fields.approval,
            delivered: fields.delivered,
        };
        Ok(Self {
            call_id: fields.call_id,
            name: fields.name,
            input: fields.input,
            provider_executed: fields.provider_executed,
            status: fields.status,
            output: fields.output,
            output_format: fields.output_format,
            output_bytes: fields.output_bytes,
            extras: (!extras.is_empty()).then(|| Box::new(extras)),
        })
    }
}

/// A tool call's arguments: the JSON text of exactly one value, usually an
/// object, held exactly as it was built and never re-encoded for storage.
/// Clones share the text.
///
/// Its canonical digest is the SHA-256 of the value's RFC 8785 (JCS) bytes,
/// computed the first time it is needed and then kept with the input. Clones
/// carry it. An input is immutable: nothing changes its text after it is
/// built, so the kept digest never goes stale. Equality compares the canonical
/// digests, so two inputs that differ only in whitespace, key order, escapes
/// or number spelling are equal.
///
/// It serializes as the value itself, the held text written verbatim, and
/// deserializes from any JSON value. Part and entry checksums hash the whole
/// part's JCS bytes, the input's included, as before; they don't use the
/// canonical digest.
#[derive(Clone)]
pub struct ToolInput {
    text: Text,
    /// The canonical digest's 32 bytes, once something has asked for them.
    canonical: OnceLock<[u8; 32]>,
}

/// Text that is not exactly one JSON value: empty, invalid, or followed by
/// more than whitespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidToolInput(String);

impl fmt::Display for InvalidToolInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tool input is not one JSON value: {}", self.0)
    }
}

impl std::error::Error for InvalidToolInput {}

impl ToolInput {
    /// Take `text` as it is, after checking once that it is exactly one JSON
    /// value (whitespace around it allowed) that parses into a [`Value`]:
    /// every number in range, every escape valid. The check builds nothing.
    pub fn from_text(text: impl Into<Text>) -> Result<Self, InvalidToolInput> {
        let text = text.into();
        serde_json::from_str::<CheckedValue>(&text)
            .map_err(|error| InvalidToolInput(error.to_string()))?;
        Ok(Self {
            text,
            canonical: OnceLock::new(),
        })
    }

    /// The text exactly as it was built.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The held text. Cloning it shares the text, it doesn't copy it.
    pub fn as_text(&self) -> &Text {
        &self.text
    }

    /// The SHA-256 of the value's RFC 8785 (JCS) bytes, as llm-runner's
    /// `content_digest` computes it for the parsed value. The first call
    /// computes it; later calls, and clones, read the kept bytes. Inputs that
    /// are equal JSON have the same canonical digest, however they are
    /// spelled.
    pub fn canonical_digest(&self) -> Digest {
        Digest::from_bytes(*self.canonical())
    }

    fn canonical(&self) -> &[u8; 32] {
        self.canonical
            .get_or_init(|| canonical_digest(&self.parse()))
    }

    /// Whether the value is an object. Reads only the first character.
    pub fn is_object(&self) -> bool {
        self.text
            .trim_start_matches([' ', '\t', '\n', '\r'])
            .starts_with('{')
    }

    /// The value, parsed.
    pub fn parse(&self) -> Value {
        serde_json::from_str(&self.text).expect("tool input is checked when built")
    }

    /// The value of the object's member `key`, parsing only that member.
    /// `None` when the input is not an object or has no such member. When
    /// `key` repeats, the last one counts, as in [`ToolInput::parse`].
    pub fn get(&self, key: &str) -> Option<Value> {
        if !self.is_object() {
            return None;
        }
        let mut deserializer = serde_json::Deserializer::from_str(&self.text);
        Member(key)
            .deserialize(&mut deserializer)
            .expect("tool input is checked when built")
    }
}

/// The canonical digest of `value`: the bytes llm-runner's `content_digest`
/// writes as hex, from the same JCS encoder and hash, without formatting them.
/// A test pins it to `content_digest`. JCS encodes every [`Value`]: its
/// numbers are finite and its keys are strings.
fn canonical_digest(value: &Value) -> [u8; 32] {
    use sha2::Digest as _;
    let canonical = serde_jcs::to_vec(value).expect("a JSON value always has a JCS encoding");
    sha2::Sha256::digest(canonical).into()
}

impl From<Value> for ToolInput {
    /// The value's compact JSON text.
    fn from(value: Value) -> Self {
        Self::from(&value)
    }
}

impl From<&Value> for ToolInput {
    /// The value's compact JSON text.
    fn from(value: &Value) -> Self {
        Self {
            text: Text::from(serde_json::to_string(value).expect("a JSON value always serializes")),
            canonical: OnceLock::new(),
        }
    }
}

impl From<Map<String, Value>> for ToolInput {
    /// The object's compact JSON text.
    fn from(object: Map<String, Value>) -> Self {
        Self::from(Value::Object(object))
    }
}

impl TryFrom<Text> for ToolInput {
    type Error = InvalidToolInput;

    fn try_from(text: Text) -> Result<Self, InvalidToolInput> {
        Self::from_text(text)
    }
}

impl TryFrom<Arc<str>> for ToolInput {
    type Error = InvalidToolInput;

    fn try_from(text: Arc<str>) -> Result<Self, InvalidToolInput> {
        Self::from_text(text)
    }
}

impl TryFrom<String> for ToolInput {
    type Error = InvalidToolInput;

    fn try_from(text: String) -> Result<Self, InvalidToolInput> {
        Self::from_text(text)
    }
}

impl TryFrom<&str> for ToolInput {
    type Error = InvalidToolInput;

    fn try_from(text: &str) -> Result<Self, InvalidToolInput> {
        Self::from_text(text)
    }
}

impl From<ToolInput> for Text {
    fn from(input: ToolInput) -> Self {
        input.text
    }
}

impl PartialEq for ToolInput {
    /// Equal when the canonical digests are: spelling doesn't count. The same
    /// text is the same value, so it needs no digest.
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text || self.canonical() == other.canonical()
    }
}

impl Eq for ToolInput {}

impl PartialEq<Value> for ToolInput {
    /// Equal when the canonical digests are.
    fn eq(&self, other: &Value) -> bool {
        *self.canonical() == canonical_digest(other)
    }
}

impl PartialEq<ToolInput> for Value {
    fn eq(&self, other: &ToolInput) -> bool {
        other == self
    }
}

impl fmt::Debug for ToolInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ToolInput").field(&self.as_str()).finish()
    }
}

impl Serialize for ToolInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Borrowing the text as a raw value only scans it; nothing is copied.
        let raw: &RawValue = serde_json::from_str(&self.text).map_err(serde::ser::Error::custom)?;
        raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ToolInput {
    /// Keeps the value's text as the deserializer presents it: the source
    /// text from a JSON reader, the compact text from a [`Value`].
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Self::from_text(Box::<str>::from(raw)).map_err(serde::de::Error::custom)
    }
}

/// Accepts exactly what parsing into a [`Value`] accepts, without building
/// it: every number is read, so one out of range is refused, and every string,
/// keys included, is decoded, so an invalid escape is refused.
struct CheckedValue;

impl<'de> Deserialize<'de> for CheckedValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(CheckedValue)
    }
}

impl<'de> Visitor<'de> for CheckedValue {
    type Value = CheckedValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self, E> {
        Ok(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self, A::Error> {
        while seq.next_element::<CheckedValue>()?.is_some() {}
        Ok(self)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self, A::Error> {
        while map.next_key::<CheckedValue>()?.is_some() {
            map.next_value::<CheckedValue>()?;
        }
        Ok(self)
    }
}

/// Reads one member of an object, skipping the others without building them.
struct Member<'k>(&'k str);

impl<'de> DeserializeSeed<'de> for Member<'_> {
    type Value = Option<Value>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Option<Value>, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for Member<'_> {
    type Value = Option<Value>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<Value>, A::Error> {
        let mut found = None;
        while let Some(matches) = map.next_key_seed(KeyIs(self.0))? {
            if matches {
                found = Some(map.next_value::<Value>()?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(found)
    }
}

/// Whether an object key is the one wanted, compared without copying it.
struct KeyIs<'k>(&'k str);

impl<'de> DeserializeSeed<'de> for KeyIs<'_> {
    type Value = bool;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<bool, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for KeyIs<'_> {
    type Value = bool;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object key")
    }

    fn visit_str<E: serde::de::Error>(self, key: &str) -> Result<bool, E> {
        Ok(key == self.0)
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
            input: json!({"path": "a.rs"}).into(),
            provider_executed: false,
            status: ToolStatus::Completed,
            output: Some(output.into()),
            output_format: OutputFormat::Text,
            output_bytes: Some(output.len() as u64),
            extras: None,
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
            tool.set_delivered(vec![Delivered::new("example-notifier", "reminder")]);
        }
        let compact = Part::new("p2", body).with_tag(1).compact();
        let PartBody::Tool(tool) = compact.body else {
            panic!("a tool part stays one");
        };
        assert_eq!(tool.output, None);
        assert_eq!(tool.output_bytes, Some(6));
        let delivered = tool.delivered().unwrap();
        assert_eq!(delivered[0].text, None);
        assert_eq!(delivered[0].text_bytes, 8);
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

    fn tool_part(input: ToolInput) -> Part {
        Part::new(
            "p1",
            PartBody::Tool(
                ToolPart::new("call_1", "read", input, ToolStatus::Completed)
                    .with_output("ok")
                    .with_output_bytes(2),
            ),
        )
    }

    #[test]
    fn whitespace_and_key_order_change_neither_equality_nor_digests() {
        let compact = ToolInput::try_from(r#"{"limit":10,"path":"a.rs"}"#).unwrap();
        let written = "{ \"path\" : \"a.rs\",\n  \"limit\": 10 }";
        let spaced = ToolInput::try_from(written).unwrap();
        // Kept exactly as built.
        assert_eq!(spaced.as_str(), written);
        assert_ne!(compact.as_str(), spaced.as_str());
        assert_eq!(compact, spaced);
        assert_eq!(spaced, json!({"path": "a.rs", "limit": 10}));

        let one = tool_part(compact);
        let two = tool_part(spaced);
        assert_eq!(one, two);
        assert_eq!(one.digest, two.digest);
        // The digest of the part with `input` as a parsed JSON value, which
        // is how the crate computed it before tool input was held as text.
        let expected = crate::digest::content_digest(&json!({
            "kind": "tool", "call_id": "call_1", "name": "read",
            "input": {"path": "a.rs", "limit": 10},
            "status": "completed", "output": "ok", "output_bytes": 2,
        }));
        assert_eq!(one.digest, expected);
        let entry = |part: &Part| {
            crate::read::Entry::new(1, "m1", crate::read::Role::Assistant, 0, vec![part.clone()])
        };
        assert_eq!(entry(&one).digest, entry(&two).digest);
        assert_eq!(entry(&one), entry(&two));

        // A different value is not equal and hashes differently.
        let other = tool_part(ToolInput::try_from(r#"{"limit":11,"path":"a.rs"}"#).unwrap());
        assert_ne!(one, other);
        assert_ne!(one.digest, other.digest);
    }

    #[test]
    fn any_json_value_is_a_tool_input_and_digests_as_its_parsed_value() {
        for value in [
            json!({"path": "a.rs"}),
            json!(null),
            json!([1, "two", {"three": 3.5}]),
            json!("raw arguments"),
            json!(-1.25),
            json!(u64::MAX),
        ] {
            let part = tool_part(value.clone().into());
            let wire = serde_json::to_string(&part).unwrap();
            let decoded: Part = serde_json::from_str(&wire).unwrap();
            assert_eq!(serde_json::to_string(&decoded).unwrap(), wire, "{value}");
            let PartBody::Tool(tool) = &decoded.body else {
                panic!("a tool part stays one");
            };
            assert_eq!(tool.input.is_object(), value.is_object());
            assert_eq!(tool.input.parse(), value);
            let expected = crate::digest::content_digest(&json!({
                "kind": "tool", "call_id": "call_1", "name": "read", "input": value,
                "status": "completed", "output": "ok", "output_bytes": 2,
            }));
            assert_eq!(decoded.digest, expected, "{value}");
            assert_eq!(body_digest(Seal::None, &decoded.body), expected);
        }
    }

    #[test]
    fn a_tool_input_is_exactly_one_json_value() {
        for valid in ["{}", " {} \n", "[]", "null", "0", "\"\"", "1e300"] {
            assert!(ToolInput::try_from(valid).is_ok(), "{valid:?}");
        }
        for invalid in ["", " ", "{} {}", "{", "1e400", "{\"a\":1} x", "[1,]"] {
            assert!(ToolInput::try_from(invalid).is_err(), "{invalid:?}");
            assert!(
                serde_json::from_str::<ToolInput>(invalid).is_err(),
                "{invalid:?}"
            );
        }
        // A reader keeps the source text; writing it back writes the value.
        let read: ToolInput = serde_json::from_str(r#" {"b": 1, "a": [2]}"#).unwrap();
        assert_eq!(read.as_str(), r#"{"b": 1, "a": [2]}"#);
        assert_eq!(serde_json::to_string(&read).unwrap(), read.as_str());
        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            json!({"a": [2], "b": 1})
        );
    }

    #[test]
    fn members_are_read_without_parsing_the_rest() {
        let input = ToolInput::try_from(r#"{"path":"a.rs","n":{"x":[1]},"path":"b.rs"}"#).unwrap();
        // The last of a repeated key counts, as when parsed.
        assert_eq!(input.get("path"), Some(json!("b.rs")));
        assert_eq!(input.get("path"), input.parse().get("path").cloned());
        assert_eq!(input.get("n"), Some(json!({"x": [1]})));
        assert_eq!(input.get("missing"), None);
        assert_eq!(ToolInput::from(json!(["path"])).get("path"), None);
        assert_eq!(ToolInput::from(json!("path")).get("path"), None);
    }

    #[test]
    fn the_input_text_is_shared_not_copied() {
        let whole: Arc<str> = Arc::from(r#"{"path":"a.rs"}"#);
        let input = ToolInput::try_from(whole.clone()).unwrap();
        assert_eq!(input.as_str().as_ptr(), whole.as_ptr());
        let shared = input.as_text().clone();
        assert_eq!(shared.as_ptr(), whole.as_ptr());
        let part = tool_part(input);
        let compact = part.compact();
        let PartBody::Tool(tool) = &compact.body else {
            panic!("a tool part stays one");
        };
        assert_eq!(tool.input.as_str().as_ptr(), whole.as_ptr());
    }

    // Written by the crate before attachments, approval and delivered text were
    // boxed (cortexkit-role-compaction 0.3.5), for the parts `boxed_samples`
    // builds.
    const UNBOXED_GOLDENS: [(&str, &str); 5] = [
        (
            "none",
            r#"{"id":"p1","tag":3,"digest":"1689085d956769e27a6d524b360c865835e30a45a69f71370efa559524cfdd9b","kind":"tool","call_id":"call_1","name":"read","input":{"limit":10,"path":"a.rs"},"status":"completed","output":"fn main() {}","output_bytes":12}"#,
        ),
        (
            "attachments",
            r#"{"id":"p1","tag":3,"digest":"9168964c731df7826492d9dfe8d8b653bb0eca9b9615c73778a66b789d9ca5e7","kind":"tool","call_id":"call_1","name":"read","input":{"limit":10,"path":"a.rs"},"status":"completed","output":"fn main() {}","output_bytes":12,"attachments":[{"id":"a1","kind":"file","mime":"image/png","transport":"inline","size":1024}]}"#,
        ),
        (
            "approval",
            r#"{"id":"p1","tag":3,"digest":"710ade1d49fec69bed2e85d8e5c24a484d199baaaaf377800cffdb2137e23500","kind":"tool","call_id":"call_1","name":"read","input":{"limit":10,"path":"a.rs"},"provider_executed":true,"status":"completed","output":"fn main() {}","output_bytes":12,"approval":{"approved":false,"reason":"not now"}}"#,
        ),
        (
            "delivered",
            r#"{"id":"p1","tag":3,"digest":"4dffde1f78cc039a19224248fa84addba199920113db56c4c54aeaf518350d25","kind":"tool","call_id":"call_1","name":"read","input":{"limit":10,"path":"a.rs"},"status":"completed","output":"fn main() {}","output_bytes":12,"delivered":[{"sender":"example-notifier","forms":["turn_system"],"text":"a reminder","text_bytes":10}]}"#,
        ),
        (
            "all",
            r#"{"id":"p1","tag":3,"digest":"074b80a78f01742888de18ea62dba04ab2e0471b8c1642031df35eca58ed4db1","kind":"tool","call_id":"call_1","name":"read","input":{"limit":10,"path":"a.rs"},"provider_executed":true,"status":"completed","output":"fn main() {}","output_bytes":12,"attachments":[{"id":"a1","kind":"file","mime":"image/png","transport":"inline","size":1024}],"approval":{"approved":false,"reason":"not now"},"delivered":[{"sender":"example-notifier","forms":["turn_system"],"text":"a reminder","text_bytes":10}]}"#,
        ),
    ];

    fn boxed_samples() -> Vec<(&'static str, ToolPart)> {
        let base = || {
            ToolPart::new(
                "call_1",
                "read",
                json!({"path": "a.rs", "limit": 10}),
                ToolStatus::Completed,
            )
            .with_output("fn main() {}")
            .with_output_bytes(12)
        };
        let attachments = || {
            vec![Attachment::new(
                "a1",
                AttachmentItem::File(FileMedia::inline("image/png", Some(1024))),
            )]
        };
        let approval = || Approval::new(false).with_reason("not now");
        let delivered = || {
            vec![Delivered::new("example-notifier", "a reminder")
                .with_forms(vec![RideForm::TurnSystem])]
        };
        vec![
            ("none", base()),
            ("attachments", base().with_attachments(attachments())),
            (
                "approval",
                base()
                    .with_provider_executed(true)
                    .with_approval(approval()),
            ),
            ("delivered", base().with_delivered(delivered())),
            (
                "all",
                base()
                    .with_provider_executed(true)
                    .with_attachments(attachments())
                    .with_approval(approval())
                    .with_delivered(delivered()),
            ),
        ]
    }

    fn has_box(part: &Part) -> bool {
        match &part.body {
            PartBody::Tool(tool) => tool.extras.is_some(),
            _ => false,
        }
    }

    #[test]
    fn boxed_fields_keep_the_unboxed_bytes_and_digests() {
        for ((name, tool), (golden_name, golden)) in
            boxed_samples().into_iter().zip(UNBOXED_GOLDENS)
        {
            assert_eq!(name, golden_name);
            let part = Part::new("p1", PartBody::Tool(tool)).with_tag(3);
            assert_eq!(serde_json::to_string(&part).unwrap(), golden, "{name}");
            let decoded: Part = serde_json::from_str(golden).unwrap();
            assert_eq!(decoded, part, "{name}");
            assert_eq!(serde_json::to_string(&decoded).unwrap(), golden, "{name}");
            assert_eq!(has_box(&decoded), name != "none", "{name}");
            assert_eq!(has_box(&part), name != "none", "{name}");
        }
    }

    /// Every tool part in the shipped wire vectors encodes to the bytes the
    /// crate wrote before the three fields were boxed, and holds a box exactly
    /// when it carries one of them. The fixture holds each distinct tool part
    /// of `test-vectors/compaction-v2` with the encoding version 0.3.5 wrote
    /// for it.
    #[test]
    fn every_vector_tool_part_keeps_its_bytes_and_boxes_only_what_it_carries() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/unboxed_tool_parts.jsonl");
        let fixture = std::fs::read_to_string(fixture).unwrap();
        let mut boxed = 0;
        for line in fixture.lines() {
            let case: Value = serde_json::from_str(line).unwrap();
            let unforked = case["unforked"].as_str().unwrap();
            let part: Part = serde_json::from_value(case["wire"].clone()).unwrap();
            assert_eq!(serde_json::to_string(&part).unwrap(), unforked);
            let written: Value = serde_json::from_str(unforked).unwrap();
            let carries = ["attachments", "approval", "delivered"]
                .iter()
                .any(|field| written.get(*field).is_some());
            assert_eq!(has_box(&part), carries, "{unforked}");
            boxed += usize::from(carries);
        }
        assert!(
            fixture.lines().count() >= 8,
            "the fixture covers the vectors"
        );
        assert!(boxed > 0, "some vector tool part carries delivered text");
    }

    /// No public path leaves an empty box, so equality and checksums never
    /// see one.
    #[test]
    fn no_path_leaves_an_empty_box() {
        let plain = || {
            ToolPart::new("call_1", "read", json!({}), ToolStatus::Completed)
                .with_output("ok")
                .with_output_bytes(2)
        };
        let ride = || vec![Delivered::new("example-notifier", "a reminder")];
        let file = || {
            vec![Attachment::new(
                "a1",
                AttachmentItem::File(FileMedia::inline("image/png", None)),
            )]
        };
        let reference = plain();
        let mut paths = vec![
            ("new", plain()),
            ("with empty attachments", plain().with_attachments(vec![])),
            ("with empty delivered", plain().with_delivered(vec![])),
            (
                "cleared attachments",
                plain().with_attachments(file()).with_attachments(vec![]),
            ),
            (
                "cleared delivered",
                plain().with_delivered(ride()).with_delivered(vec![]),
            ),
            ("cleared approval", {
                let mut tool = plain().with_approval(Approval::new(true));
                tool.set_approval(None);
                tool
            }),
            ("cleared one by one", {
                let mut tool = plain()
                    .with_attachments(file())
                    .with_approval(Approval::new(true))
                    .with_delivered(ride());
                tool.set_attachments(Vec::new());
                assert!(tool.extras.is_some());
                tool.set_delivered(Vec::new());
                assert!(tool.extras.is_some());
                tool.set_approval(None);
                tool
            }),
            ("edited through delivered_mut", {
                let mut tool = plain();
                assert!(tool.delivered_mut().is_empty());
                tool
            }),
            ("cloned", plain().clone()),
        ];
        let reference_wire = serde_json::to_value(&reference).unwrap();
        for wire in [
            json!({"attachments": [], "delivered": []}),
            json!({"approval": null}),
            json!({"attachments": [], "approval": null, "delivered": []}),
        ] {
            let mut wire_with_empties = reference_wire.clone();
            wire_with_empties
                .as_object_mut()
                .unwrap()
                .extend(wire.as_object().unwrap().clone());
            let decoded: ToolPart = serde_json::from_value(wire_with_empties).unwrap();
            paths.push(("decoded with empty fields", decoded));
        }
        let digest = Part::new("p1", PartBody::Tool(reference.clone())).digest;
        for (path, tool) in paths {
            assert!(tool.extras.is_none(), "{path}");
            assert_eq!(tool, reference, "{path}");
            assert_eq!(
                serde_json::to_value(&tool).unwrap(),
                reference_wire,
                "{path}"
            );
            assert_eq!(
                Part::new("p1", PartBody::Tool(tool)).digest,
                digest,
                "{path}"
            );
        }
        let compacted = Part::new("p1", PartBody::Tool(plain())).compact();
        assert!(!has_box(&compacted), "compacted");
    }

    #[test]
    fn delivered_mut_edits_in_place_and_never_adds_or_removes_the_box() {
        let mut tool = ToolPart::new("call_1", "read", json!({}), ToolStatus::Completed);
        for delivered in tool.delivered_mut() {
            delivered.tokens = Some(crate::tokens::TokenCount::new(1));
        }
        assert!(tool.extras.is_none());
        assert_eq!(tool.delivered(), None);

        tool.set_delivered(vec![Delivered::new("example-notifier", "a reminder")]);
        let before = Part::new("p1", PartBody::Tool(tool.clone())).digest;
        tool.delivered_mut()[0].tokens = Some(crate::tokens::TokenCount::new(3));
        assert!(tool.extras.is_some());
        assert_eq!(
            tool.delivered().unwrap()[0].tokens,
            Some(crate::tokens::TokenCount::new(3))
        );
        // Delivered items' token counts are metadata: the checksum leaves them out.
        assert_eq!(Part::new("p1", PartBody::Tool(tool)).digest, before);
    }

    /// Part and entry digests computed by cortexkit-role-compaction 0.3.5,
    /// with each input parsed into a `Value`. The candidate builds the same
    /// parts from the input text as written and must give the same values.
    const DIGEST_PINS: [(&str, &str, &str); 8] = [
        (
            r#"{"path":"a.rs","limit":10}"#,
            "731168fdb85d35722048db6205db8866a5a5107997c35e3a23b75b45b311dd64",
            "854acac17856bd01e5f899378dd3d0d4f6395644fb2ef7297f760571e207ed58",
        ),
        (
            r#"{ "limit" : 10, "path" : "a.rs" }"#,
            "731168fdb85d35722048db6205db8866a5a5107997c35e3a23b75b45b311dd64",
            "854acac17856bd01e5f899378dd3d0d4f6395644fb2ef7297f760571e207ed58",
        ),
        (
            r#"[1, "two", null, {"b": 2, "a": 1}]"#,
            "936faac84b964b6ede42479f8f23cf74c6207aaa8fa17191da8f28905a1d5bee",
            "db807bf3adc1546e9bc5038c3181ac5fe14cfbed731dfd0eb1ca57292d590b81",
        ),
        (
            r#"null"#,
            "ff26dbd23e737b60d9ad2219adc03e9694442ee08431425524d51cd6c43dc996",
            "428b6688f5539eb9a2342c74ca2654486e2c158939b10d7d9f71b56323a0e858",
        ),
        (
            r#"1.0"#,
            "e9a78ee860ec10903ffa24649927d79a126b844356dc6d260f09a1173ef98552",
            "0fcd34eda04a8617d2b40a095a3916587fcfc7aaf0462fdb847922a7a5e209fa",
        ),
        (
            r#"{"n": 1.0, "m": 1e2, "k": -0.5}"#,
            "5003f984938dc3cb1b8cd7d4703e779dd36499bb0b4cccae41ffca41cf7eb6cf",
            "57b947ca6c3cf7efd878ea55b22a06e7671efc450c576f3ae9cd53c773a10a0d",
        ),
        (
            r#"{"s": "tab\t quote\" slash\/ e\u00e9 smile \ud83d\ude00 nul \u0000 bell \u0007"}"#,
            "ee699087944552b0c424f9593dc062c366d21ef97f08cd0ff9b781616b1733a6",
            "6577b0b0e15e3c7a00a98af8d15171c15a37d007f341fcb7868559f21be3c458",
        ),
        (
            r#"{"z": {"y": [3, 2, 1], "x": "é"}, "a": true}"#,
            "1c61b74ddf1799ae9bcb1f4163ca4a694e2ba35d45ab90782cf1796c6d96df18",
            "e085480f465420f5e99fd0627581400dd5921993da2c1a80ba7e91e67001c904",
        ),
    ];

    #[test]
    fn checksums_of_parts_and_entries_are_unchanged_for_any_spelling() {
        for (text, part_digest, entry_digest) in DIGEST_PINS {
            let input = ToolInput::try_from(text).unwrap();
            assert_eq!(input.as_str(), text);
            let tool = ToolPart::new("call_1", "read", input, ToolStatus::Completed)
                .with_output("ok")
                .with_output_bytes(2);
            let part = Part::new("0", PartBody::Tool(tool)).with_tag(1);
            let entry = crate::read::Entry::new(
                1,
                "m1",
                crate::read::Role::Assistant,
                0,
                vec![part.clone()],
            );
            assert_eq!(part.digest, part_digest, "{text}");
            assert_eq!(entry.digest, entry_digest, "{text}");
        }
    }

    #[test]
    fn the_canonical_digest_is_the_content_digest_of_the_value() {
        for (text, _, _) in DIGEST_PINS {
            let input = ToolInput::try_from(text).unwrap();
            let value: Value = serde_json::from_str(text).unwrap();
            assert_eq!(
                input.canonical_digest(),
                crate::digest::content_digest(&value),
                "{text}"
            );
            assert_eq!(
                ToolInput::from(&value).canonical_digest(),
                input.canonical_digest()
            );
            let decoded: ToolInput = serde_json::from_value(value).unwrap();
            assert_eq!(decoded.canonical_digest(), input.canonical_digest());
        }
        let spelled = |text: &str| ToolInput::try_from(text).unwrap().canonical_digest();
        assert_eq!(spelled(DIGEST_PINS[0].0), spelled(DIGEST_PINS[1].0));
        assert_eq!(spelled("1.0"), spelled("1"));
        assert_eq!(spelled(r#""\u00e9""#), spelled("\"é\""));
        assert_ne!(spelled("1"), spelled("2"));
        assert!(spelled("null").is_canonical());
    }

    /// Values at the edges of what JSON and JCS encode. Every one that builds
    /// an input must also give a canonical digest, equal to llm-runner's
    /// `content_digest` of the parsed value, so the digest computed on first
    /// use can never fail.
    #[test]
    fn awkward_values_digest_on_first_use_as_content_digest_does() {
        let deep_array = format!("{}{}", "[".repeat(120), "]".repeat(120));
        let deep_object = format!("{}1{}", r#"{"k":"#.repeat(100), "}".repeat(100));
        let awkward = [
            "18446744073709551615",
            "-9223372036854775808",
            "9007199254740993",
            "123456789012345678901234567890",
            "1e308",
            "-1.5e-300",
            "5e-324",
            "1E+2",
            "0.1",
            "-0",
            "-0.0",
            "1.0",
            r#""\u0000\u001f\"\\\/\b\f\n\r\t \ud83d\ude00 \u2028 \u00e9""#,
            r#"{"\u00e9\n": 1, "é\n": 2, "": [], "\ud83d\ude00": {}}"#,
            r#"{"b": [1, 2.5, -3e-7, true, null], "a": {"y": "z", "x": ""}}"#,
            "[]",
            "{}",
            r#""""#,
            &deep_array,
            &deep_object,
        ];
        for text in awkward {
            let value: Value = serde_json::from_str(text).unwrap();
            let input = ToolInput::try_from(text).unwrap();
            assert!(input.canonical.get().is_none(), "{text}");
            assert_eq!(
                input.canonical_digest(),
                crate::digest::content_digest(&value),
                "{text}"
            );
            assert_eq!(input, value, "{text}");
        }
    }

    /// An input builds exactly when its text parses into a `Value`, so the
    /// parse on first use never fails.
    #[test]
    fn an_input_builds_exactly_when_its_text_parses() {
        let too_deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        for text in [
            "1e400",
            "-1e400",
            r#""\ud800""#,
            r#""\udc00 x""#,
            r#"{"\ud800": 1}"#,
            r#""\x""#,
            "01",
            "-",
            "[1,]",
            r#"{"a" 1}"#,
            "\"\u{1}\"",
            &too_deep,
            "1 2",
            "",
            "null",
            "[1e2, -0, 0.5]",
        ] {
            assert_eq!(
                ToolInput::try_from(text).is_ok(),
                serde_json::from_str::<Value>(text).is_ok(),
                "{text}"
            );
        }
    }

    #[test]
    fn a_clone_carries_the_canonical_digest_once_computed() {
        let input = ToolInput::try_from(r#"{"b": 1, "a": 2}"#).unwrap();
        let early = input.clone();
        assert!(early.canonical.get().is_none());
        let digest = input.canonical_digest();
        let late = input.clone();
        assert_eq!(late.canonical.get(), digest.as_bytes());
        assert!(early.canonical.get().is_none());
        assert_eq!(early.canonical_digest(), digest);
        // The same text compares equal without computing a digest.
        let other = ToolInput::try_from(r#"{"b": 1, "a": 2}"#).unwrap();
        let again = ToolInput::try_from(r#"{"b": 1, "a": 2}"#).unwrap();
        assert_eq!(other, again);
        assert!(other.canonical.get().is_none() && again.canonical.get().is_none());
    }
}
