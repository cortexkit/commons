//! Display rows: the canonical, schema-neutral form of one stored message
//! that `session.history` serves with `form: "display"` (CONTRACT.md §4.7).
//!
//! A row is what an app shows: who spoke, the message's text, and its parts
//! in order (text, reasoning, tool calls with their arguments, tool results,
//! files), with media described by the same refs descriptors mode serves and
//! never inlined. Each serializer profile has one pure projection from a
//! stored body to a row ([`anthropic_messages`], [`opencode_aisdk`], [`pi`], or
//! [`project`] by profile name). A runner calls the projection and then fills
//! the fields its stored body lacks ([`DisplayRow::with_model`],
//! [`DisplayRow::with_client_message_id`], [`DisplayRow::with_sender`], and
//! under `call_identity` each tool call's [`ToolCallPart::with_call_key`] and
//! [`ToolCallPart::with_schema_pin`]); a
//! reader of several runners needs no runner's schema.
//!
//! Cuts are the caller's choice: [`Limits`] bounds tool-call arguments and
//! tool results in bytes, cut back to a UTF-8 character boundary, with
//! `arguments_truncated` or `truncated` set on what was cut. Without limits
//! nothing is cut: a runner never applies a default of its own.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::read::descriptors::Reference;
use crate::serializer_profile::{ANTHROPIC_MESSAGES, OPENCODE_AISDK, PI};

/// Byte bounds on a display row's tool payloads, chosen by the caller of a
/// display read or a live subscription. Strict: a misspelled member would
/// otherwise silently leave a payload whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// The most bytes of a tool call's `arguments` text a row carries.
    pub arguments_bytes: u64,
    /// The most bytes of a tool result's `content` a row carries, when the
    /// result is not an error.
    pub result_bytes: u64,
    /// The most bytes of a tool result's `content` a row carries, when the
    /// result is an error.
    pub error_result_bytes: u64,
}

impl Limits {
    pub const fn new(arguments_bytes: u64, result_bytes: u64, error_result_bytes: u64) -> Self {
        Self {
            arguments_bytes,
            result_bytes,
            error_result_bytes,
        }
    }
}

/// The bounds CortexKit's own phone and desktop clients (the Alfonso apps)
/// read a session's message list with: 2 KiB of arguments, 16 KiB of a
/// result and 64 KiB of an error result. A caller that wants a whole
/// message sends no `limits` at all.
pub const APP_LIMITS: Limits = Limits::new(2_048, 16_384, 65_536);

/// `text` cut to at most `max` bytes, ending on a UTF-8 character boundary,
/// and whether anything was cut. Text that fits is returned whole.
pub fn cut(text: &str, max: u64) -> (String, bool) {
    let max = usize::try_from(max).unwrap_or(usize::MAX);
    if text.len() <= max {
        return (text.to_owned(), false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

/// The grammar of a send's `client_message_id`: 1 to 64 characters of
/// `[A-Za-z0-9_-]`.
pub fn client_message_id_valid(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// One stored message as an app shows it. Decoded leniently: an unknown
/// member is ignored and an unknown part type decodes as
/// [`DisplayPart::Other`].
///
/// Non-exhaustive: build one with [`DisplayRow::new`] (or a projection) and
/// the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DisplayRow {
    /// References captured when this message was stored, never added to its body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_refs: Option<crate::output_refs::Descriptor>,
    /// Who spoke, as the stored body says: `user`, `assistant`, `tool`,
    /// `system` or another runner value.
    pub role: String,
    /// The message's text parts, joined as `run.result` joins them
    /// ([`crate::run::join_text_parts`]). Reasoning is not part of it.
    pub text: String,
    /// Every part, in the stored message's order.
    #[serde(default)]
    pub parts: Vec<DisplayPart>,
    /// The recorded model that wrote the message. A message written through
    /// the runner's own run always has one. Imported or adopted messages may
    /// lack a recorded model: absent means unknown, never a guess, a default
    /// or the session's current model. Readers must handle absence as unknown
    /// before a runner omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The `client_message_id` of the send that created this user message,
    /// kept by the runner beside the stored body, never inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_message_id: Option<String>,
    /// For a user message: the attested module id of whoever sent it (the
    /// route principal the send arrived on), where the runner recorded it,
    /// kept beside the stored body and never inside it. Absent for a prompt
    /// a person typed into the host directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    /// For an assistant message a run wrote, on a runner declaring
    /// `rich_ui_stamp`: the rich UI stamp that run's frozen plan carries,
    /// copied verbatim (CONTRACT §9.17). Absent on an assistant row of a
    /// runner declaring the group means unstamped: render the text as plain
    /// Markdown and never infer a stamp. Never on a user, steered, tool,
    /// imported or adopted row. Readers accept the member whether or not the
    /// runner declares the group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich_ui: Option<crate::rich_ui::RichUiStamp>,
}

impl DisplayRow {
    pub fn with_output_refs(mut self, descriptor: crate::output_refs::Descriptor) -> Self {
        self.output_refs = Some(descriptor);
        self
    }
    /// A row of `role` with `parts`, its `text` joined from the text parts.
    pub fn new(role: impl Into<String>, parts: Vec<DisplayPart>) -> Self {
        let text = crate::run::join_text_parts(parts.iter().filter_map(|part| match part {
            DisplayPart::Text(text) => Some(text.text.as_str()),
            _ => None,
        }));
        Self {
            role: role.into(),
            output_refs: None,
            text,
            parts,
            model: None,
            client_message_id: None,
            sender: None,
            rich_ui: None,
        }
    }

    /// Stamp this assistant row with its run's rich UI stamp.
    pub fn with_rich_ui(mut self, rich_ui: crate::rich_ui::RichUiStamp) -> Self {
        self.rich_ui = Some(rich_ui);
        self
    }

    /// Fill `model` where the projection left it absent; a model the body
    /// states is kept.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        if self.model.is_none() {
            self.model = Some(model.into());
        }
        self
    }

    pub fn with_client_message_id(mut self, id: impl Into<String>) -> Self {
        self.client_message_id = Some(id.into());
        self
    }

    /// Name the module that sent this user message.
    pub fn with_sender(mut self, module: impl Into<String>) -> Self {
        self.sender = Some(module.into());
        self
    }

    /// The row's tool calls, in order, for a runner filling what the stored
    /// body lacks (`call_key` and `schema_pin` under `call_identity`).
    pub fn tool_calls_mut(&mut self) -> impl Iterator<Item = &mut ToolCallPart> {
        self.parts.iter_mut().filter_map(|part| match part {
            DisplayPart::ToolCall(call) => Some(call),
            _ => None,
        })
    }
}

/// One part of a [`DisplayRow`], tagged by `type`. Every part carries
/// `part`, the runner-defined key of the stored part it comes from, so a live
/// delta naming a part finds its place in the stored row. The projection helpers
/// below choose default keys; they do not prescribe a serializer profile's keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DisplayPart {
    Text(TextPart),
    Reasoning(TextPart),
    ToolCall(ToolCallPart),
    ToolResult(ToolResultPart),
    File(FilePart),
    /// A part of a type the projection does not show, named by its stored
    /// type. Also what a reader decodes a part type it does not know into.
    Other(OtherPart),
}

/// The known part types, as `type` spells them.
pub mod part_types {
    pub const TEXT: &str = "text";
    pub const REASONING: &str = "reasoning";
    pub const TOOL_CALL: &str = "tool_call";
    pub const TOOL_RESULT: &str = "tool_result";
    pub const FILE: &str = "file";
    pub const OTHER: &str = "other";
}

impl<'de> Deserialize<'de> for DisplayPart {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let part = value
            .get("part")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        fn typed<T: serde::de::DeserializeOwned, E: serde::de::Error>(
            value: Value,
        ) -> Result<T, E> {
            serde_json::from_value(value).map_err(E::custom)
        }
        Ok(match kind.as_str() {
            part_types::TEXT => Self::Text(typed::<_, D::Error>(value)?),
            part_types::REASONING => Self::Reasoning(typed::<_, D::Error>(value)?),
            part_types::TOOL_CALL => Self::ToolCall(typed::<_, D::Error>(value)?),
            part_types::TOOL_RESULT => Self::ToolResult(typed::<_, D::Error>(value)?),
            part_types::FILE => Self::File(typed::<_, D::Error>(value)?),
            part_types::OTHER => Self::Other(typed::<_, D::Error>(value)?),
            _ => Self::Other(OtherPart { part, kind }),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct TextPart {
    pub part: String,
    pub text: String,
}

impl TextPart {
    pub fn new(part: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            part: part.into(),
            text: text.into(),
        }
    }
}

/// A tool call. `arguments` is the call's arguments as JSON text (compact,
/// as `serde_json` writes it), so a cut can bound it in bytes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolCallPart {
    /// The opted-in plan call's references, separate from model arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_refs: Option<crate::output_refs::Descriptor>,
    pub part: String,
    /// The model's id for the call, as the stored body carries it. Not
    /// unique in a session: a row is found by `mid` plus this id.
    pub tool_call_id: String,
    pub name: String,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub arguments_truncated: bool,
    /// The call's key (CONTRACT.md §11.3), on a runner declaring
    /// `call_identity`: the value `session.read`'s attribution and
    /// `run.tool_calls` expose for the same call, once the call is prepared
    /// or dispatched. Absent means unknown, never empty. Opaque to readers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    /// The `schema_pin` of the tool definition the call was dispatched
    /// against, frozen at dispatch, on a runner declaring `call_identity`.
    /// Absent for a call never dispatched (denied, or refused before
    /// dispatch), for a runner-native tool with no provider definition, and
    /// on any runner without the group. Opaque to readers: never parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_pin: Option<String>,
}

/// A tool result. `content` is its text: a string result as stored, or the
/// text blocks of a structured result joined like a row's text. Media in
/// the result is in `files`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolResultPart {
    pub part: String,
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    pub content: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FilePart>,
}

/// A file or media item: byte-backed media as a `media_ref` (never its
/// bytes), or a URL the body names, kept as written.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct FilePart {
    /// The key of the stored part, empty for a file nested in a tool result.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub part: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<Reference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct OtherPart {
    #[serde(default)]
    pub part: String,
    /// The stored part's own type.
    #[serde(default)]
    pub kind: String,
}

impl ToolCallPart {
    pub fn with_output_refs(mut self, descriptor: crate::output_refs::Descriptor) -> Self {
        self.output_refs = Some(descriptor);
        self
    }

    /// Name the call's key, once the call is prepared or dispatched.
    pub fn with_call_key(mut self, call_key: impl Into<String>) -> Self {
        self.call_key = Some(call_key.into());
        self
    }

    /// Name the pin of the definition the call was dispatched against, as
    /// recorded at dispatch.
    pub fn with_schema_pin(mut self, schema_pin: impl Into<String>) -> Self {
        self.schema_pin = Some(schema_pin.into());
        self
    }
    /// A call whose `arguments` are `arguments` as compact JSON text, cut
    /// by `limits` when given.
    pub fn new(
        part: impl Into<String>,
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: &Value,
        limits: Option<&Limits>,
    ) -> Self {
        match arguments_part(
            part.into(),
            tool_call_id.into(),
            name.into(),
            arguments,
            limits,
        ) {
            DisplayPart::ToolCall(call) => call,
            _ => unreachable!("arguments_part builds a tool call"),
        }
    }
}

impl ToolResultPart {
    /// A result whose content is `content`, cut by `limits` (by
    /// `error_result_bytes` when `is_error`) when given.
    pub fn new(
        part: impl Into<String>,
        tool_call_id: impl Into<String>,
        is_error: bool,
        content: &str,
        files: Vec<FilePart>,
        limits: Option<&Limits>,
    ) -> Self {
        match result_part(
            part.into(),
            tool_call_id.into(),
            is_error,
            content,
            files,
            limits,
        ) {
            DisplayPart::ToolResult(result) => result,
            _ => unreachable!("result_part builds a tool result"),
        }
    }
}

impl FilePart {
    /// Byte-backed media, described by `media` (a `media_ref`).
    pub fn media(part: impl Into<String>, media: Reference) -> Self {
        Self {
            part: part.into(),
            media: Some(media),
            url: None,
            mime: None,
            filename: None,
        }
    }

    /// A file the body names by URL, kept as written.
    pub fn url(part: impl Into<String>, url: impl Into<String>, mime: Option<String>) -> Self {
        Self {
            part: part.into(),
            media: None,
            url: Some(url.into()),
            mime,
            filename: None,
        }
    }

    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }
}

impl OtherPart {
    pub fn new(part: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            part: part.into(),
            kind: kind.into(),
        }
    }
}

/// Why a stored body has no row: it does not follow the profile's schema,
/// or the profile is not one this crate defines.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProjectionError {
    pub message: String,
}

impl ProjectionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProjectionError {}

/// The row for `body` under the serializer profile named `profile`, cut by
/// `limits` when given.
pub fn project(
    profile: &str,
    body: &Value,
    limits: Option<&Limits>,
) -> Result<DisplayRow, ProjectionError> {
    match profile {
        ANTHROPIC_MESSAGES => anthropic_messages(body, limits),
        OPENCODE_AISDK => opencode_aisdk(body, limits),
        PI => pi(body, limits),
        other => Err(ProjectionError::new(format!(
            "{other} is not a serializer profile this crate projects"
        ))),
    }
}

fn arguments_part(
    part: String,
    tool_call_id: String,
    name: String,
    arguments: &Value,
    limits: Option<&Limits>,
) -> DisplayPart {
    let text = serde_json::to_string(arguments).expect("JSON values serialize");
    let (arguments, arguments_truncated) = match limits {
        Some(limits) => cut(&text, limits.arguments_bytes),
        None => (text, false),
    };
    DisplayPart::ToolCall(ToolCallPart {
        output_refs: None,
        part,
        tool_call_id,
        name,
        arguments,
        arguments_truncated,
        call_key: None,
        schema_pin: None,
    })
}

fn result_part(
    part: String,
    tool_call_id: String,
    is_error: bool,
    text: &str,
    files: Vec<FilePart>,
    limits: Option<&Limits>,
) -> DisplayPart {
    let bound = limits.map(|limits| {
        if is_error {
            limits.error_result_bytes
        } else {
            limits.result_bytes
        }
    });
    let (content, truncated) = match bound {
        Some(bound) => cut(text, bound),
        None => (text.to_owned(), false),
    };
    DisplayPart::ToolResult(ToolResultPart {
        part,
        tool_call_id,
        is_error,
        content,
        truncated,
        files,
    })
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The descriptor `kind` for a media type, as descriptors mode names it.
fn media_kind(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "image"
    } else if mime == "application/pdf" {
        "pdf"
    } else if mime.starts_with("audio/") {
        "audio"
    } else {
        "file"
    }
}

/// An Anthropic `image` or `document` block as a file: base64 media as a
/// `media_ref`, a URL source as its URL. `None` for a source this
/// projection does not describe (a text-sourced document, say).
fn anthropic_file(block: &Value, part: String) -> Result<Option<FilePart>, ProjectionError> {
    let source = &block["source"];
    let mime = str_of(source, "media_type").to_owned();
    match str_of(source, "type") {
        "base64" => {
            let bytes = base64_decode(str_of(source, "data"))
                .ok_or_else(|| ProjectionError::new("a media block's data is not base64"))?;
            let reference = Reference::media(
                (!part.is_empty()).then(|| part.clone()),
                media_kind(&mime),
                &mime,
                &bytes,
                None,
            );
            Ok(Some(FilePart {
                part,
                media: Some(reference),
                url: None,
                mime: None,
                filename: None,
            }))
        }
        "url" => Ok(Some(FilePart {
            part,
            media: None,
            url: Some(str_of(source, "url").to_owned()),
            mime: (!mime.is_empty()).then_some(mime),
            filename: None,
        })),
        _ => Ok(None),
    }
}

fn other(part: String, kind: &str) -> DisplayPart {
    DisplayPart::Other(OtherPart {
        part,
        kind: kind.to_owned(),
    })
}

/// The row for an `anthropic-messages` body (CONTRACT.md §4.4): a Messages
/// API message, `{role, content}`, with `content` a string or an array of
/// blocks. `text` and `thinking` blocks become text and reasoning,
/// `tool_use` a tool call, `tool_result` a tool result (its nested text
/// joined, its nested images and documents as files), `image` and
/// `document` files. Any other block, a `redacted_thinking` or a
/// text-sourced document included, is an `other` part. The body names no
/// model, so the runner fills `model` itself.
pub fn anthropic_messages(
    body: &Value,
    limits: Option<&Limits>,
) -> Result<DisplayRow, ProjectionError> {
    let role = body
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| ProjectionError::new("an anthropic-messages body has no role"))?;
    let blocks = match body.get("content") {
        Some(Value::String(text)) => {
            return Ok(DisplayRow::new(
                role,
                vec![DisplayPart::Text(TextPart::new("0", text.as_str()))],
            ))
        }
        Some(Value::Array(blocks)) => blocks,
        _ => {
            return Err(ProjectionError::new(
                "an anthropic-messages body's content is neither a string nor an array",
            ))
        }
    };
    let mut parts = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let key = index.to_string();
        let kind = str_of(block, "type");
        parts.push(match kind {
            "text" => DisplayPart::Text(TextPart::new(key, str_of(block, "text"))),
            "thinking" => DisplayPart::Reasoning(TextPart::new(key, str_of(block, "thinking"))),
            "tool_use" => arguments_part(
                key,
                str_of(block, "id").to_owned(),
                str_of(block, "name").to_owned(),
                block.get("input").unwrap_or(&Value::Object(Map::new())),
                limits,
            ),
            "tool_result" => {
                let mut texts = Vec::new();
                let mut files = Vec::new();
                match block.get("content") {
                    Some(Value::String(text)) => texts.push(text.clone()),
                    Some(Value::Array(nested)) => {
                        for inner in nested {
                            match str_of(inner, "type") {
                                "text" => texts.push(str_of(inner, "text").to_owned()),
                                "image" | "document" => {
                                    if let Some(file) = anthropic_file(inner, String::new())? {
                                        files.push(file);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                result_part(
                    key,
                    str_of(block, "tool_use_id").to_owned(),
                    block.get("is_error") == Some(&Value::Bool(true)),
                    &crate::run::join_text_parts(texts),
                    files,
                    limits,
                )
            }
            "image" | "document" => match anthropic_file(block, key.clone())? {
                Some(file) => DisplayPart::File(file),
                None => other(key, kind),
            },
            _ => other(key, kind),
        });
    }
    Ok(DisplayRow::new(role, parts))
}

/// OpenCode's part types that record the host's own bookkeeping (step
/// boundaries, file snapshots, patches) rather than anything said. A row
/// leaves them out.
pub const OPENCODE_BOOKKEEPING_PARTS: &[&str] = &["step-start", "step-finish", "snapshot", "patch"];

/// `provider/model` from an object carrying OpenCode's `providerID` and
/// `modelID`.
fn opencode_model(value: &Value) -> Option<String> {
    let provider = value.get("providerID")?.as_str()?;
    let model = value.get("modelID")?.as_str()?;
    Some(format!("{provider}/{model}"))
}

/// An OpenCode file part (`{type: "file", mime, url, filename?}`): a
/// `data:` URL holding base64 as a `media_ref`, any other URL as written.
fn opencode_file(value: &Value, part: String) -> Result<FilePart, ProjectionError> {
    let mime = str_of(value, "mime").to_owned();
    let url = str_of(value, "url");
    let filename = value
        .get("filename")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(data) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .map(|(_, data)| data)
    {
        let bytes = base64_decode(data)
            .ok_or_else(|| ProjectionError::new("a file part's data URL is not base64"))?;
        let reference = Reference::media(
            (!part.is_empty()).then(|| part.clone()),
            media_kind(&mime),
            &mime,
            &bytes,
            None,
        );
        return Ok(FilePart {
            part,
            media: Some(reference),
            url: None,
            mime: None,
            filename,
        });
    }
    Ok(FilePart {
        part,
        media: None,
        url: Some(url.to_owned()),
        mime: (!mime.is_empty()).then_some(mime),
        filename,
    })
}

/// The row for an `opencode-aisdk` body (CONTRACT.md §4.4): OpenCode's
/// stored message `{info, parts}`. `info.role` is the role; the model is
/// `info.providerID/info.modelID` on an assistant message and
/// `info.model.providerID/modelID` on a user message. `text` and
/// `reasoning` parts become text and reasoning; a `tool` part becomes its
/// call (`callID`, `tool`, `state.input`) and, once `state.status` is
/// `completed` or `error`, its result (`state.output` with
/// `state.attachments` as files, or `state.error` as an error); a `file`
/// part a file. The bookkeeping parts ([`OPENCODE_BOOKKEEPING_PARTS`]) are
/// left out, and any other part is an `other` part.
pub fn opencode_aisdk(
    body: &Value,
    limits: Option<&Limits>,
) -> Result<DisplayRow, ProjectionError> {
    let info = body
        .get("info")
        .filter(|info| info.is_object())
        .ok_or_else(|| ProjectionError::new("an opencode-aisdk body has no info"))?;
    let role = info
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| ProjectionError::new("an opencode-aisdk body has no info.role"))?;
    let stored = body
        .get("parts")
        .and_then(Value::as_array)
        .ok_or_else(|| ProjectionError::new("an opencode-aisdk body has no parts"))?;
    let mut parts = Vec::new();
    for (index, part) in stored.iter().enumerate() {
        let key = part
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| index.to_string(), str::to_owned);
        let kind = str_of(part, "type");
        match kind {
            "text" => parts.push(DisplayPart::Text(TextPart::new(key, str_of(part, "text")))),
            "reasoning" => parts.push(DisplayPart::Reasoning(TextPart::new(
                key,
                str_of(part, "text"),
            ))),
            "tool" => {
                let state = &part["state"];
                let call_id = str_of(part, "callID").to_owned();
                parts.push(arguments_part(
                    key.clone(),
                    call_id.clone(),
                    str_of(part, "tool").to_owned(),
                    state.get("input").unwrap_or(&Value::Object(Map::new())),
                    limits,
                ));
                match str_of(state, "status") {
                    "completed" => {
                        let files = state
                            .get("attachments")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .map(|file| opencode_file(file, String::new()))
                            .collect::<Result<Vec<_>, _>>()?;
                        parts.push(result_part(
                            key,
                            call_id,
                            false,
                            str_of(state, "output"),
                            files,
                            limits,
                        ));
                    }
                    "error" => parts.push(result_part(
                        key,
                        call_id,
                        true,
                        str_of(state, "error"),
                        Vec::new(),
                        limits,
                    )),
                    _ => {}
                }
            }
            "file" => parts.push(DisplayPart::File(opencode_file(part, key)?)),
            kind if OPENCODE_BOOKKEEPING_PARTS.contains(&kind) => {}
            _ => parts.push(other(key, kind)),
        }
    }
    let mut row = DisplayRow::new(role, parts);
    row.model = opencode_model(info).or_else(|| info.get("model").and_then(opencode_model));
    Ok(row)
}

/// A Pi image is described, never inlined. Existing descriptor payloads are
/// kept rather than treated as base64 again.
fn pi_image(image: &Value, part: String) -> Result<FilePart, ProjectionError> {
    if image["type"] == "media_ref" {
        let reference = serde_json::from_value(image.clone())
            .map_err(|e| ProjectionError::new(format!("invalid pi media_ref: {e}")))?;
        return Ok(FilePart::media(part, reference));
    }
    if image["data"].is_object() {
        return pi_image(&image["data"], part);
    }
    let bytes = base64_decode(str_of(image, "data"))
        .ok_or_else(|| ProjectionError::new("a pi image's data is not base64"))?;
    let mime = str_of(image, "mimeType");
    Ok(FilePart::media(
        part.clone(),
        Reference::media(
            (!part.is_empty()).then_some(part),
            "image",
            mime,
            &bytes,
            None,
        ),
    ))
}

/// A wrapped Pi journal row, using its runtime role, not the user role produced
/// for model requests. `custom_message` is `custom`, `branch_summary` is
/// `branchSummary`, and shell executions remain `bashExecution`. Shell text
/// shows command, output and status. Images use refs descriptors. Control rows,
/// including compaction summaries, are refused because they are never served.
/// Context edits belong to session projection/lineage handling, not raw rows.
pub fn pi(body: &Value, limits: Option<&Limits>) -> Result<DisplayRow, ProjectionError> {
    let role = match str_of(body, "type") {
        "custom_message" => "custom",
        "branch_summary" => "branchSummary",
        "message" => match str_of(&body["message"], "role") {
            role @ ("system" | "user" | "assistant" | "toolResult" | "bashExecution") => role,
            _ => {
                return Err(ProjectionError::new(
                    "a pi message has no supported runtime role",
                ))
            }
        },
        _ => {
            return Err(ProjectionError::new(
                "a pi display body is not an ordinal-bearing entry",
            ))
        }
    };
    // Exclusion controls model context, not visibility of a persisted shell run.
    let mut visible = body.clone();
    if role == "bashExecution" {
        visible["message"]["excludeFromContext"] = Value::Bool(false);
    }
    let content =
        crate::serializer_profile::pi::projected_content(&visible).map_err(ProjectionError::new)?;
    let blocks = match content {
        Value::String(text) => vec![serde_json::json!({"type":"text", "text":text})],
        Value::Array(blocks) => blocks,
        Value::Null => Vec::new(),
        reference => vec![reference],
    };
    let mut parts = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let key = index.to_string();
        parts.push(match str_of(block, "type") {
            "text" => DisplayPart::Text(TextPart::new(key, str_of(block, "text"))),
            "thinking" => DisplayPart::Reasoning(TextPart::new(key, str_of(block, "thinking"))),
            "toolCall" => arguments_part(
                key,
                str_of(block, "id").to_owned(),
                str_of(block, "name").to_owned(),
                &block["arguments"],
                limits,
            ),
            "image" | "media_ref" => DisplayPart::File(pi_image(
                block,
                if role == "toolResult" {
                    String::new()
                } else {
                    key
                },
            )?),
            kind => other(key, kind),
        });
    }
    if role == "toolResult" {
        let mut texts = Vec::new();
        let mut files = Vec::new();
        for part in parts {
            match part {
                DisplayPart::Text(text) => texts.push(text.text),
                DisplayPart::File(file) => files.push(file),
                _ => {}
            }
        }
        parts = vec![result_part(
            "0".into(),
            str_of(&body["message"], "toolCallId").to_owned(),
            body["message"]["isError"] == true,
            &crate::run::join_text_parts(texts),
            files,
            limits,
        )];
    }
    Ok(DisplayRow::new(role, parts))
}

/// Standard base64 (RFC 4648 §4), padding optional. `None` for anything
/// else, whitespace included.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let trimmed = text.trim_end_matches('=');
    if text.len() - trimmed.len() > 2 || trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    for chunk in trimmed.as_bytes().chunks(4) {
        let mut acc = 0u32;
        for &byte in chunk {
            acc = (acc << 6) | sextet(byte)?;
        }
        acc <<= 6 * (4 - chunk.len() as u32);
        let bytes = acc.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

/// One `session.history` entry in display form: the envelope's position
/// and tags, and the row in place of the stored body.
///
/// Non-exhaustive: build one with [`DisplayEntry::new`] and the `with_*`
/// setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DisplayEntry {
    /// As on a raw history entry: present only where the runner has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u64>,
    pub mid: String,
    pub written_at: u64,
    pub row: DisplayRow,
    /// As on a raw history entry: only at or after the lineage's
    /// `first_ordinal`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<crate::tags::PartTag>,
}

impl DisplayEntry {
    pub fn new(mid: impl Into<String>, written_at: u64, row: DisplayRow) -> Self {
        Self {
            ordinal: None,
            mid: mid.into(),
            written_at,
            row,
            tags: Vec::new(),
        }
    }

    /// The display entry for a raw history `entry`, with `row` in place of
    /// its body.
    pub fn from_history(entry: &crate::history::HistoryEntry, row: DisplayRow) -> Self {
        Self {
            ordinal: entry.ordinal,
            mid: entry.mid.clone(),
            written_at: entry.written_at,
            row,
            tags: entry.tags.clone(),
        }
    }

    pub fn with_ordinal(mut self, ordinal: u64) -> Self {
        self.ordinal = Some(ordinal);
        self
    }

    pub fn with_tags(mut self, tags: Vec<crate::tags::PartTag>) -> Self {
        self.tags = tags;
        self
    }
}

/// A `session.history` page in display form: [`crate::history::HistoryPage`]
/// with display entries. Decoded leniently.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DisplayPage {
    /// Original identities of the pruned stored parts represented on this page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pruned_reasoning: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    /// Oldest first.
    pub messages: Vec<DisplayEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_version: Option<u64>,
}

impl DisplayPage {
    pub fn with_pruned_reasoning(
        mut self,
        parts: Vec<crate::archive_prunes_reasoning::PrunedReasoning>,
    ) -> Self {
        self.pruned_reasoning = parts;
        self
    }
    /// The display form of a raw `page`, each entry's row from `row`.
    pub fn from_history<F, E>(page: &crate::history::HistoryPage, mut row: F) -> Result<Self, E>
    where
        F: FnMut(&crate::history::HistoryEntry) -> Result<DisplayRow, E>,
    {
        Ok(Self {
            messages: page
                .messages
                .iter()
                .map(|entry| row(entry).map(|row| DisplayEntry::from_history(entry, row)))
                .collect::<Result<_, E>>()?,
            lineage_id: page.lineage_id.clone(),
            older: page.older.clone(),
            newer: page.newer.clone(),
            history_version: page.history_version,
            pruned_reasoning: page.pruned_reasoning.clone(),
        })
    }

    /// The rules a raw page shows on its own ([`crate::history::HistoryPage::check`]),
    /// checked on this page's envelope.
    pub fn check(
        &self,
        first_ordinal: Option<u64>,
    ) -> Result<(), crate::history::HistoryPageProblem> {
        let skeleton = crate::history::HistoryPage {
            messages: self
                .messages
                .iter()
                .map(|entry| {
                    let mut raw = crate::history::HistoryEntry::new(
                        &entry.mid,
                        entry.written_at,
                        Value::Null,
                    )
                    .with_tags(entry.tags.clone());
                    raw.ordinal = entry.ordinal;
                    raw
                })
                .collect(),
            lineage_id: self.lineage_id.clone(),
            older: self.older.clone(),
            newer: self.newer.clone(),
            history_version: self.history_version,
            pruned_reasoning: self.pruned_reasoning.clone(),
        };
        skeleton.check(first_ordinal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    fn limits_of(case: &Value) -> Option<Limits> {
        case.get("limits")
            .map(|limits| serde_json::from_value(limits.clone()).expect("limits decode"))
    }

    fn check_profile(file: &str, profile: &str) {
        let file = vectors::load(file);
        assert_eq!(file["profile"], profile);
        let mut cut_any = false;
        for case in vectors::cases(&file, "rows") {
            let name = case["name"].as_str().unwrap();
            let limits = limits_of(case);
            let row = project(profile, &case["body"], limits.as_ref())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(serde_json::to_value(&row).unwrap(), case["row"], "{name}");
            let decoded: DisplayRow = vectors::round_trip(name, &case["row"]);
            assert_eq!(decoded, row, "{name}");
            let has_cut = row.parts.iter().any(|part| match part {
                DisplayPart::ToolCall(call) => call.arguments_truncated,
                DisplayPart::ToolResult(result) => result.truncated,
                _ => false,
            });
            if limits.is_none() {
                assert!(!has_cut, "{name}: an uncut row carries no flag");
            } else {
                cut_any |= has_cut;
            }
        }
        assert!(cut_any, "{profile}: no vector cuts anything");
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            assert!(project(profile, &case["body"], None).is_err(), "{name}");
        }
    }

    #[test]
    fn anthropic_messages_vectors_project() {
        check_profile("display-anthropic-messages.json", ANTHROPIC_MESSAGES);
    }

    #[test]
    fn pi_vectors_project() {
        check_profile("display-pi.json", PI);
    }

    #[test]
    fn opencode_aisdk_vectors_project() {
        check_profile("display-opencode-aisdk.json", OPENCODE_AISDK);
    }

    #[test]
    fn a_cut_ends_on_a_character_boundary_and_says_so() {
        assert_eq!(cut("héllo", 2), ("h".to_owned(), true));
        assert_eq!(cut("héllo", 3), ("hé".to_owned(), true));
        assert_eq!(cut("🦀", 3), (String::new(), true));
        assert_eq!(cut("abc", 3), ("abc".to_owned(), false));
        assert_eq!(cut("", 0), (String::new(), false));
        assert_eq!(APP_LIMITS, Limits::new(2048, 16384, 65536));
        assert!(serde_json::from_value::<Limits>(
            serde_json::json!({"arguments_bytes": 1, "result_bytes": 1, "error_result_bytes": 1, "extra": 1})
        )
        .is_err());
        assert!(serde_json::from_value::<Limits>(
            serde_json::json!({"arguments_bytes": 1, "result_bytes": 1})
        )
        .is_err());
    }

    #[test]
    fn client_message_ids_follow_their_grammar() {
        assert!(client_message_id_valid("a"));
        assert!(client_message_id_valid(&"A_-9".repeat(16)));
        for bad in ["", "a b", "é", "a.b", &"a".repeat(65)] {
            assert!(!client_message_id_valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(base64_decode("aGk=").unwrap(), b"hi");
        assert_eq!(base64_decode("aGk").unwrap(), b"hi");
        assert_eq!(base64_decode("AAEC").unwrap(), [0, 1, 2]);
        assert_eq!(base64_decode("").unwrap(), b"");
        assert!(base64_decode("a").is_none());
        assert!(base64_decode("a b=").is_none());
    }

    #[test]
    fn an_unknown_part_type_decodes_as_other_and_runner_fields_fill_once() {
        let row: DisplayRow = serde_json::from_value(serde_json::json!({
            "role": "assistant", "text": "", "extra": 1,
            "parts": [{"type": "hologram", "part": "3", "beam": true}]
        }))
        .unwrap();
        assert_eq!(
            row.parts,
            vec![DisplayPart::Other(OtherPart {
                part: "3".into(),
                kind: "hologram".into()
            })]
        );
        let row = row.with_model("acme/m1").with_model("acme/m2");
        assert_eq!(row.model.as_deref(), Some("acme/m1"));
    }

    #[test]
    fn display_request_page_head_and_send_vectors() {
        use crate::history::{HistoryForm, HistoryMode, HistoryRequest};
        let file = vectors::load("display-requests.json");
        for case in vectors::cases(&file, "history_requests") {
            let name = case["name"].as_str().unwrap();
            let request: HistoryRequest = vectors::round_trip(name, &case["request"]);
            let form = match request.form() {
                HistoryForm::Raw => "raw",
                HistoryForm::Display => "display",
            };
            assert_eq!(form, case["form"], "{name}");
            let mode = match request.mode().unwrap_or_else(|f| panic!("{name}: {f}")) {
                HistoryMode::Newest => "newest",
                HistoryMode::Cursor(_) => "cursor",
                HistoryMode::FromMid(_) => "from_mid",
                HistoryMode::BeforeMid(_) => "before_mid",
                HistoryMode::FromOrdinal(_) => "from_ordinal",
                HistoryMode::BeforeOrdinal(_) => "before_ordinal",
            };
            assert_eq!(mode, case["mode"], "{name}");
        }
        for case in vectors::cases(&file, "history_refused") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            match serde_json::from_value::<HistoryRequest>(case["request"].clone()) {
                Err(error) => assert!(error.to_string().contains(field), "{name}: {error}"),
                Ok(request) => assert_eq!(request.mode(), Err(field), "{name}"),
            }
        }
        for case in vectors::cases(&file, "pages") {
            let name = case["name"].as_str().unwrap();
            let page: DisplayPage = vectors::round_trip(name, &case["page"]);
            assert_eq!(page.check(case["first_ordinal"].as_u64()), Ok(()), "{name}");
            // Where a vector names each row's sender, the decoded row says
            // the same; a row it leaves out names none.
            if let Some(senders) = case.get("senders") {
                for entry in &page.messages {
                    assert_eq!(
                        entry.row.sender.as_deref(),
                        senders[entry.mid.as_str()].as_str(),
                        "{name}: {}",
                        entry.mid
                    );
                }
            }
        }
        for case in vectors::cases(&file, "head_inputs") {
            let name = case["name"].as_str().unwrap();
            let input: crate::read::HeadInput = vectors::round_trip(name, &case["input"]);
            assert_eq!(
                input.carries_attachments(),
                case["carries"].as_bool(),
                "{name}"
            );
            let head = crate::read::HeadMeta::no_lineage().with_input(input);
            assert!(
                head.lineage_consistent(),
                "{name}: input is outside the lineage rule"
            );
        }
        let declared = [crate::capabilities::CLIENT_MESSAGE_ID];
        for case in vectors::cases(&file, "sends") {
            let name = case["name"].as_str().unwrap();
            let send: crate::send::SendRequest = vectors::round_trip(name, &case["send"]);
            let id = send.client_message_id(&declared).unwrap();
            assert_eq!(id, case["id"].as_str(), "{name}");
            assert!(send.same_client_message_id(id), "{name}");
            assert!(!send.same_client_message_id(Some("other")), "{name}");
            if id.is_some() {
                // A runner without the group refuses the member by name.
                assert_eq!(
                    send.client_message_id::<&str>(&[]),
                    Err(crate::send::CLIENT_MESSAGE_ID_FIELD),
                    "{name}"
                );
            }
        }
        for case in vectors::cases(&file, "sends_refused") {
            let name = case["name"].as_str().unwrap();
            let send: crate::send::SendRequest =
                serde_json::from_value(case["send"].clone()).unwrap();
            assert_eq!(
                send.client_message_id(&declared),
                Err(crate::send::CLIENT_MESSAGE_ID_FIELD),
                "{name}"
            );
        }
    }

    /// Each `call_identity` row decodes to the members its case names, and
    /// the setters rebuild its exact bytes from the row without them. A
    /// member the case leaves out is absent, never empty.
    #[test]
    fn call_identity_rows_carry_their_members_or_leave_them_out() {
        let file = vectors::load("call-identity.json");
        let mut seen = (false, false, false);
        for case in vectors::cases(&file, "rows") {
            let name = case["name"].as_str().unwrap();
            let row: DisplayRow = vectors::round_trip(name, &case["row"]);
            let mut calls = row.parts.iter().filter_map(|part| match part {
                DisplayPart::ToolCall(call) => Some(call),
                _ => None,
            });
            let call = calls.next().expect("each row holds a tool call");
            assert_eq!(
                call.call_key.as_deref(),
                case["call_key"].as_str(),
                "{name}"
            );
            assert_eq!(
                call.schema_pin.as_deref(),
                case["schema_pin"].as_str(),
                "{name}"
            );
            assert_ne!(call.call_key.as_deref(), Some(""), "{name}");
            assert_ne!(call.schema_pin.as_deref(), Some(""), "{name}");
            match (&call.call_key, &call.schema_pin) {
                (Some(_), Some(_)) => seen.0 = true,
                (Some(_), None) => seen.1 = true,
                (None, None) => seen.2 = true,
                (None, Some(_)) => panic!("{name}: a pin without a key"),
            }
            // Rebuild the row: the projection's bare call, then the setters.
            let arguments: Value = serde_json::from_str(&call.arguments).unwrap();
            let mut built =
                ToolCallPart::new(&call.part, &call.tool_call_id, &call.name, &arguments, None);
            assert_eq!(built.call_key, None, "{name}");
            assert_eq!(built.schema_pin, None, "{name}");
            if let Some(key) = &call.call_key {
                built = built.with_call_key(key);
            }
            if let Some(pin) = &call.schema_pin {
                built = built.with_schema_pin(pin);
            }
            let mut rebuilt = DisplayRow::new(&row.role, vec![DisplayPart::ToolCall(built)]);
            if let Some(model) = &row.model {
                rebuilt = rebuilt.with_model(model);
            }
            assert_eq!(
                serde_json::to_value(&rebuilt).unwrap(),
                case["row"],
                "{name}"
            );
            // Clearing both through the row leaves neither member on the wire.
            let mut cleared = row.clone();
            for call in cleared.tool_calls_mut() {
                call.call_key = None;
                call.schema_pin = None;
            }
            let text = serde_json::to_string(&cleared).unwrap();
            assert!(
                !text.contains("call_key") && !text.contains("schema_pin"),
                "{name}: {text}"
            );
        }
        assert_eq!(seen, (true, true, true), "every shape has a vector");
    }

    #[test]
    fn a_display_page_checks_like_a_raw_page() {
        let row = DisplayRow::new("user", vec![DisplayPart::Text(TextPart::new("0", "hi"))]);
        let page = DisplayPage {
            pruned_reasoning: Vec::new(),
            messages: vec![
                DisplayEntry::new("m1", 1, row.clone()).with_ordinal(1),
                DisplayEntry::new("m0", 1, row).with_ordinal(0),
            ],
            lineage_id: Some("lin".into()),
            older: None,
            newer: None,
            history_version: Some(1),
        };
        assert_eq!(
            page.check(Some(0)),
            Err(crate::history::HistoryPageProblem::OrdinalsNotRising(
                "m0".into()
            ))
        );
    }
}
