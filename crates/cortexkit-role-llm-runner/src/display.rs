//! Display rows: the canonical, schema-neutral form of one stored message
//! that `session.history` serves with `form: "display"` (CONTRACT.md §4.7).
//!
//! A row is what an app shows: who spoke, the message's text, and its parts
//! in order (text, reasoning, tool calls with their arguments, tool results,
//! files), with media described by the same refs descriptors mode serves and
//! never inlined. Each serializer profile has one pure projection from a
//! stored body to a row ([`anthropic_messages`], [`opencode_aisdk`], [`opencode_v2`], [`pi`], or
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
use crate::serializer_profile::{ANTHROPIC_MESSAGES, OPENCODE_AISDK, OPENCODE_V2, PI};

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

/// Convenience bounds for message-list clients: 2 KiB arguments, 16 KiB
/// results and 64 KiB error results. A caller wanting the whole message
/// sends no `limits`.
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
    /// Verified relay header hidden from this row's text, for a "via chat" label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<crate::attribution::Attribution>,
    /// Notices riding this row's tool results, in delivery order, outside checksums.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<crate::read::NoticeFact>,
    /// References captured when this message was stored, never added to its body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_refs: Option<crate::output_refs::Descriptor>,
    /// Who spoke, as the stored body says: `user`, `assistant`, `tool`,
    /// `system` or another runner value.
    pub role: String,
    /// The message's text parts, joined as `run.result` joins them
    /// ([`crate::run::join_text_parts`]). Reasoning is not part of it.
    pub text: String,
    /// Every projected part, in the stored message's order. Bookkeeping
    /// parts are dropped.
    #[serde(default)]
    pub parts: Vec<DisplayPart>,
    /// The recorded model that wrote the message. A message written through
    /// the runner's own run always has one. Imported or adopted messages may
    /// lack a recorded model: absent means unknown, never a guess, a default
    /// or the session's current model. Readers must handle absence as unknown
    /// before a runner omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Exact recorded model replacement on this assistant reply. Absent makes
    /// no claim; outside stored bytes and digests. See CONTRACT §4.7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_fallback: Option<ModelFallback>,
    /// The `client_message_id` of the send that created this user message,
    /// kept by the runner beside the stored body, never inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_message_id: Option<String>,
    /// For a user message: the attested module id of whoever sent it (the
    /// route principal the send arrived on), where the runner recorded it,
    /// kept beside the stored body and never inside it. Absent for a prompt
    /// a person typed into the host directly.
    /// With [`crate::capabilities::MESSAGE_SENDERS`], module-principal sends
    /// stored while the group is declared have this binding by `message_started`.
    /// An absent sender never proves who wrote the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    /// Agent whose words this module carries; outside bodies and checksums.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaks_for: Option<crate::message_marks::SpeaksFor>,
    /// Positive automatic-message mark; false is absent on the wire.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notice: bool,
    /// Owner-marked notice carrying a person's words; outside bodies and checksums.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
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
    pub fn with_speaks_for(mut self, speaks_for: crate::message_marks::SpeaksFor) -> Self {
        self.speaks_for = Some(speaks_for);
        self
    }

    pub fn with_notice(mut self, notice: bool) -> Self {
        self.notice = notice;
        self
    }

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
            attribution: None,
            notices: Vec::new(),
            role: role.into(),
            output_refs: None,
            text,
            parts,
            model: None,
            model_fallback: None,
            client_message_id: None,
            sender: None,
            speaks_for: None,
            notice: false,
            keep: false,
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

    /// Attach a recorded fallback only after readers support it (CONTRACT §4.7).
    pub fn with_model_fallback(mut self, fallback: ModelFallback) -> Self {
        self.model_fallback = Some(fallback);
        self
    }

    /// Validate a fallback fact against this row's role and model-id rules.
    pub fn check_model_fallback(&self) -> Result<(), &'static str> {
        if let Some(fallback) = &self.model_fallback {
            fallback.check(&self.role)?;
        }
        Ok(())
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

/// An exactly recorded replacement of the requested model for an assistant reply.
/// Both ids use the same form as [`DisplayRow::model`]; if provider-qualified,
/// qualify both bare marker ids with the reply's own provider. Absence of the
/// fact means no claim. The source must be a well-formed documented boundary
/// marker or the runner's own durable replacement record (CONTRACT §4.7).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "ModelFallbackWire")]
#[non_exhaustive]
pub struct ModelFallback {
    pub requested: String,
    pub answered: String,
}

#[derive(Deserialize)]
struct ModelFallbackWire {
    requested: String,
    answered: String,
}

impl TryFrom<ModelFallbackWire> for ModelFallback {
    type Error = &'static str;

    fn try_from(wire: ModelFallbackWire) -> Result<Self, Self::Error> {
        Self::new(wire.requested, wire.answered)
    }
}

impl ModelFallback {
    pub fn new(
        requested: impl Into<String>,
        answered: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let fact = Self {
            requested: requested.into(),
            answered: answered.into(),
        };
        fact.check("assistant")?;
        Ok(fact)
    }

    /// Validate constructed values as well as the entry's known role.
    pub fn check(&self, role: &str) -> Result<(), &'static str> {
        if role != "assistant" {
            return Err("model_fallback is allowed only on an assistant entry");
        }
        if self.requested.is_empty() || self.answered.is_empty() {
            return Err("model_fallback requires two non-empty model ids");
        }
        if self.requested == self.answered {
            return Err("model_fallback model ids must differ");
        }
        Ok(())
    }
}

/// One part of a [`DisplayRow`], tagged by `type`. Every part carries
/// `part`, the runner-defined key of the stored part it comes from, so a live
/// delta naming a part finds its place in the stored row. The projection helpers
/// below choose default keys; they do not prescribe a serializer profile's keys.
/// On a user or assistant row, `host: true` means host-written, never the
/// sender's or model's words.
/// Native `synthetic` rows are entirely host text, marked or not; marking is
/// optional and useful only to attach a code.
/// Shared projections never set it; runners opt in with [`Self::with_host`].
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

impl DisplayPart {
    /// The stored part key, shared with read display facts.
    pub fn part(&self) -> &str {
        match self {
            Self::Text(part) | Self::Reasoning(part) => &part.part,
            Self::ToolCall(part) => &part.part,
            Self::ToolResult(part) => &part.part,
            Self::File(part) => &part.part,
            Self::Other(part) => &part.part,
        }
    }

    /// Mark a user, assistant or native synthetic part from exact recorded knowledge.
    /// Emit this fact only after every reader has shipped support (CONTRACT §4.7).
    /// For synthetic parts, first have readers pass the synthetic host-code samples.
    pub fn with_host(mut self, host: bool) -> Self {
        match &mut self {
            Self::Text(part) | Self::Reasoning(part) => part.host = host,
            Self::ToolCall(part) => part.host = host,
            Self::ToolResult(part) => part.host = host,
            Self::File(part) => part.host = host,
            Self::Other(part) => part.host = host,
        }
        if !host {
            self.set_host_code(None);
        }
        self
    }

    /// Set both the host mark and its optional presentation code. Use the same
    /// exact recorded match for both; unknown codes remain valid (CONTRACT §4.7).
    /// Invalid code shapes are refused when decoding or by [`host_codes::check`].
    pub fn with_host_code(mut self, code: impl Into<String>) -> Self {
        self = self.with_host(true);
        self.set_host_code(Some(code.into()));
        self
    }

    fn set_host_code(&mut self, code: Option<String>) {
        match self {
            Self::Text(part) | Self::Reasoning(part) => part.host_code = code,
            Self::ToolCall(part) => part.host_code = code,
            Self::ToolResult(part) => part.host_code = code,
            Self::File(part) => part.host_code = code,
            Self::Other(part) => part.host_code = code,
        }
    }

    /// Optional open-vocabulary presentation code; fall back to the part's text.
    pub fn host_code(&self) -> Option<&str> {
        match self {
            Self::Text(part) | Self::Reasoning(part) => part.host_code.as_deref(),
            Self::ToolCall(part) => part.host_code.as_deref(),
            Self::ToolResult(part) => part.host_code.as_deref(),
            Self::File(part) => part.host_code.as_deref(),
            Self::Other(part) => part.host_code.as_deref(),
        }
    }

    /// Whether this part is marked as host-written. An absent mark is false.
    /// A row with native role `synthetic` is host text even without this mark.
    pub fn host(&self) -> bool {
        match self {
            Self::Text(part) | Self::Reasoning(part) => part.host,
            Self::ToolCall(part) => part.host,
            Self::ToolResult(part) => part.host,
            Self::File(part) => part.host,
            Self::Other(part) => part.host,
        }
    }
}

/// Open presentation codes for exactly identified host parts, never model-byte edits.
pub mod host_codes {
    use serde::{Deserialize, Deserializer};

    /// Claude Code's whole block `[Request interrupted by user]\n`.
    pub const INTERRUPTED: &str = "interrupted";
    /// The exact host text represented by [`INTERRUPTED`].
    pub const INTERRUPTED_TEXT: &str = "[Request interrupted by user]\n";
    /// Claude Code's whole block `[Request interrupted by user for tool use]\n`.
    pub const INTERRUPTED_FOR_TOOL_USE: &str = "interrupted_for_tool_use";
    /// The exact host text represented by [`INTERRUPTED_FOR_TOOL_USE`].
    pub const INTERRUPTED_FOR_TOOL_USE_TEXT: &str = "[Request interrupted by user for tool use]\n";

    /// OpenCode 2's native synthetic continuation after a cut stream.
    pub const CONTINUED_AFTER_INTERRUPTION: &str = "continued_after_interruption";
    /// Exact whole text for [`CONTINUED_AFTER_INTERRUPTION`], with no trailing newline.
    pub const CONTINUED_AFTER_INTERRUPTION_TEXT: &str = "The previous response was interrupted. Continue from where you left off without repeating completed content.";
    /// OpenCode 2's native synthetic continuation after an output-limit stop.
    pub const CONTINUED_AFTER_OUTPUT_LIMIT: &str = "continued_after_output_limit";
    /// Exact whole text for [`CONTINUED_AFTER_OUTPUT_LIMIT`], with no trailing newline.
    pub const CONTINUED_AFTER_OUTPUT_LIMIT_TEXT: &str = "Your last response hit the output token limit (stop reason: length). Do not apologize, recap, or repeat yourself. Break the remaining work into smaller pieces.";
    /// OpenCode 2's native synthetic continuation after a server restart.
    pub const CONTINUED_AFTER_RESTART: &str = "continued_after_restart";
    /// Exact whole text for [`CONTINUED_AFTER_RESTART`], with no trailing newline.
    pub const CONTINUED_AFTER_RESTART_TEXT: &str = "The server restarted while you were working. Continue from where you left off without repeating completed work.";

    /// Bookkeeping parts written by the host's fallback-model plugin, not the model.
    pub const SERVER_FALLBACK: &str = "server_fallback";
    /// From the plugin's documented stored form: match only at the start of a
    /// signed reasoning part's signature, never its text or a substring.
    pub const SERVER_FALLBACK_PREFIX: &str = "cortexkit-server-fallback-v1:";
    /// From the plugin's documented stored form: match only at the start of a
    /// signed reasoning part's signature, never its text or a substring.
    pub const SERVER_FALLBACK_USAGE_PREFIX: &str = "cortexkit-server-fallback-usage-v1:";

    /// Check display members constructed in memory, including concrete part
    /// structs. The vocabulary is open; only shape and the host mark are checked.
    pub fn check(host: bool, code: Option<&str>) -> Result<(), &'static str> {
        if let Some(code) = code {
            if !host {
                return Err("host_code requires host: true");
            }
            if code.is_empty()
                || code.len() > 64
                || !code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err("host_code must be 1-64 bytes of lowercase ASCII letters, digits or _");
            }
        }
        Ok(())
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<String>, D::Error> {
        let code = String::deserialize(deserializer)?;
        check(true, Some(&code)).map_err(serde::de::Error::custom)?;
        Ok(Some(code))
    }
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
        if value.get("host_code").is_some() && value.get("host") != Some(&Value::Bool(true)) {
            return Err(serde::de::Error::custom("host_code requires host: true"));
        }
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
            _ => Self::Other(OtherPart {
                host_code: match value.get("host_code") {
                    Some(code) => {
                        let code: String = typed::<_, D::Error>(code.clone())?;
                        host_codes::check(true, Some(&code)).map_err(serde::de::Error::custom)?;
                        Some(code)
                    }
                    None => None,
                },
                host: typed::<_, D::Error>(
                    value.get("host").cloned().unwrap_or(Value::Bool(false)),
                )?,
                ..OtherPart::new(part, kind)
            }),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct TextPart {
    /// Host-written on a user, assistant or synthetic row; see [`DisplayPart::with_host`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host: bool,
    /// Optional host presentation code; requires `host: true`. See [`host_codes`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "host_codes::deserialize"
    )]
    pub host_code: Option<String>,
    pub part: String,
    pub text: String,
}

impl TextPart {
    pub fn new(part: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            host: false,
            host_code: None,
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
    /// Host-written on a user, assistant or synthetic row; see [`DisplayPart::with_host`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host: bool,
    /// Optional host presentation code; requires `host: true`. See [`host_codes`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "host_codes::deserialize"
    )]
    pub host_code: Option<String>,
    /// The opted-in plan call's references, separate from model arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_refs: Option<crate::output_refs::Descriptor>,
    pub part: String,
    /// The model's id for the call, as the stored body carries it. Not
    /// unique in a session: a row is found by `mid` plus this id.
    pub tool_call_id: String,
    pub name: String,
    /// The model provider ran this tool itself. Its result reaches the model
    /// as a provider item or not at all, never as tool-result text. Absent
    /// means false: not provider-run, or not reported.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub provider_executed: bool,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub arguments_truncated: bool,
    /// The call's key (CONTRACT.md §11.3), on a runner declaring
    /// `call_identity`: the value `session.read`'s attribution and
    /// `run.tool_calls` expose for the same call, once the call is prepared
    /// or dispatched. Absent means unknown, never empty. Opaque to readers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    /// The attested module id of the runner that minted this imported call's key.
    /// Only with `call_identity`; absent for the serving runner's own calls (§9.16).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::call_carrier::deserialize"
    )]
    pub call_carrier: Option<String>,
    /// The `schema_pin` of the tool definition the call was dispatched
    /// against, frozen at dispatch, on a runner declaring `call_identity`.
    /// Absent for a call never dispatched (denied, or refused before
    /// dispatch), for a runner-native tool with no provider definition, and
    /// on any runner without the group. Opaque to readers: never parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_pin: Option<String>,
    /// Dispatch time in integer milliseconds since the Unix epoch, when stored.
    /// Never inferred from another clock or another call in a parallel batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Time this call's answer was recorded, in Unix epoch milliseconds.
    /// A parallel batch uses each call's own answer time, not the batch's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// Why the call ended without an answer: `interrupted`, `cancelled`,
    /// `denied` or `not_executed`. Unknown values are retained unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unanswered: Option<String>,
}

/// A tool result. `content` is its text: a string result as stored, or the
/// text blocks of a structured result joined like a row's text. Media in
/// the result is in `files`. A bad file is recorded in `unprojected` instead
/// of hiding the tool's text, which is the content the person reads.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolResultPart {
    /// Host-written on a user, assistant or synthetic row; see [`DisplayPart::with_host`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host: bool,
    /// Optional host presentation code; requires `host: true`. See [`host_codes`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "host_codes::deserialize"
    )]
    pub host_code: Option<String>,
    pub part: String,
    pub tool_call_id: String,
    /// The model provider ran this tool itself. Its result reaches the model
    /// as a provider item or not at all, never as tool-result text. Absent
    /// means false: not provider-run, or not reported.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub provider_executed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    pub content: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FilePart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unprojected: Vec<UnprojectedPart>,
}

/// A tool-result file that could not be projected. Reasons are fixed library
/// text from [`projection_reasons`], never text taken from the stored body.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct UnprojectedPart {
    pub kind: String,
    pub reason: String,
}

impl UnprojectedPart {
    pub fn new(kind: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            reason: reason.into(),
        }
    }
}

/// Stable reasons for a part that could not be projected. These strings never
/// include body content or diagnostics from a decoder.
pub mod projection_reasons {
    pub const INVALID_MEDIA_REF: &str = "invalid media_ref";
    pub const DATA_NOT_BASE64: &str = "data is not base64";
    pub const INVALID_FILE_URL: &str = "file url is neither a string nor a media_ref";
    pub const MISSING_FILE_DATA: &str = "file has no hydrated data";
    pub const INVALID_TEXT: &str = "text is neither a string nor a text_ref";
    pub const INVALID_SHELL: &str = "shell command or output is not a string";
    pub const INVALID_LOCATION: &str = "location has no directory";
    pub const INVALID_SUMMARY: &str = "branch summary is not a string";
    pub const UNSUPPORTED_PART_KIND: &str = "unsupported part type";

    /// Every reason above, so a reader can tell a known reason from one a
    /// newer library added.
    pub const ALL: &[&str] = &[
        INVALID_MEDIA_REF,
        DATA_NOT_BASE64,
        INVALID_FILE_URL,
        MISSING_FILE_DATA,
        INVALID_TEXT,
        INVALID_SHELL,
        INVALID_LOCATION,
        INVALID_SUMMARY,
        UNSUPPORTED_PART_KIND,
    ];
}

/// A file or media item: byte-backed media as a `media_ref` (never its
/// bytes), or a URL the body names, kept as written.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct FilePart {
    /// Host-written on a user, assistant or synthetic row; see [`DisplayPart::with_host`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host: bool,
    /// Optional host presentation code; requires `host: true`. See [`host_codes`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "host_codes::deserialize"
    )]
    pub host_code: Option<String>,
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
    /// Host-written on a user, assistant or synthetic row; see [`DisplayPart::with_host`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host: bool,
    /// Optional host presentation code; requires `host: true`. See [`host_codes`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "host_codes::deserialize"
    )]
    pub host_code: Option<String>,
    #[serde(default)]
    pub part: String,
    /// The stored part's own type.
    #[serde(default)]
    pub kind: String,
    /// A standalone part is itself the content, so keep its key and native
    /// type when projection fails, with a reason from [`projection_reasons`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ToolCallPart {
    /// Name the other runner that minted the call's key; never set for local calls.
    pub fn with_call_carrier(mut self, call_carrier: impl Into<String>) -> Self {
        self.call_carrier = Some(call_carrier.into());
        self
    }

    /// Record whether the model provider ran this tool itself.
    pub fn with_provider_executed(mut self, provider_executed: bool) -> Self {
        self.provider_executed = provider_executed;
        self
    }

    /// Record this call's stored dispatch time, in Unix epoch milliseconds.
    pub fn with_started_at(mut self, started_at: u64) -> Self {
        self.started_at = Some(started_at);
        self
    }

    /// Record this call's stored answer time, in Unix epoch milliseconds.
    pub fn with_finished_at(mut self, finished_at: u64) -> Self {
        self.finished_at = Some(finished_at);
        self
    }

    /// Record an unanswered outcome, preserving even an unknown value.
    pub fn with_unanswered(mut self, unanswered: impl Into<String>) -> Self {
        self.unanswered = Some(unanswered.into());
        self
    }

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
    pub fn with_unprojected(mut self, unprojected: Vec<UnprojectedPart>) -> Self {
        self.unprojected = unprojected;
        self
    }

    /// Record whether the model provider ran this tool itself.
    pub fn with_provider_executed(mut self, provider_executed: bool) -> Self {
        self.provider_executed = provider_executed;
        self
    }

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
            host: false,
            host_code: None,
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
            host: false,
            host_code: None,
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
            host: false,
            host_code: None,
            part: part.into(),
            kind: kind.into(),
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
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

/// Whether this crate has a display projection for the serializer profile
/// named `profile`: the profiles [`project`] accepts. For any other profile,
/// such as the runner-owned profile, the runner supplies its own projection.
/// The contract doesn't prescribe that profile's raw-body-to-row mapping, but
/// its other display-row requirements still apply.
pub fn projects(profile: &str) -> bool {
    matches!(
        profile,
        ANTHROPIC_MESSAGES | OPENCODE_AISDK | OPENCODE_V2 | PI
    )
}

/// The row for `body` under the serializer profile named `profile`, cut by
/// `limits` when given. Refused for a profile where [`projects`] is false.
pub fn project(
    profile: &str,
    body: &Value,
    limits: Option<&Limits>,
) -> Result<DisplayRow, ProjectionError> {
    match profile {
        ANTHROPIC_MESSAGES => anthropic_messages(body, limits),
        OPENCODE_AISDK => opencode_aisdk(body, limits),
        OPENCODE_V2 => opencode_v2(body, limits),
        PI => pi(body, limits),
        other => Err(ProjectionError::new(format!(
            "{other} is not a serializer profile this crate projects"
        ))),
    }
}

/// Project recorded attribution without ever exposing its trusted frame to
/// older apps. The stored body remains untouched. A mismatched prefix or byte
/// count leaves the text whole and carries no fact.
pub fn project_with_attribution(
    profile: &str,
    body: &Value,
    limits: Option<&Limits>,
    attribution: Option<&crate::attribution::Attribution>,
) -> Result<DisplayRow, ProjectionError> {
    let mut row = project(profile, body, limits)?;
    if let Some(fact) = attribution {
        let content = match profile {
            ANTHROPIC_MESSAGES => Some(body["content"].clone()),
            OPENCODE_AISDK => Some(body["parts"].clone()),
            OPENCODE_V2 => opencode_v2_display_content(body).ok().map(Value::Array),
            PI => crate::serializer_profile::pi::projected_content(body).ok(),
            _ => None,
        };
        let stored_text = content.as_ref().and_then(|content| {
            if fact.part == 0 && content.is_string() {
                content.as_str()
            } else {
                content
                    .as_array()?
                    .get(fact.part as usize)?
                    .get("text")?
                    .as_str()
            }
        });
        if !stored_text.is_some_and(|text| crate::attribution::verify(fact, text)) {
            return Ok(row);
        }
        let key = if profile == OPENCODE_AISDK {
            body.get("parts")
                .and_then(Value::as_array)
                .and_then(|parts| parts.get(fact.part as usize))
                .and_then(|part| part.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| fact.part.to_string())
        } else {
            fact.part.to_string()
        };
        if let Some(DisplayPart::Text(text)) = row
            .parts
            .iter_mut()
            .find(|part| matches!(part, DisplayPart::Text(text) if text.part == key))
        {
            if crate::attribution::verify(fact, &text.text) {
                text.text.drain(..fact.header_bytes as usize);
                row.attribution = Some(fact.clone());
                row.text =
                    crate::run::join_text_parts(row.parts.iter().filter_map(|part| match part {
                        DisplayPart::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    }));
            }
        }
    }
    Ok(row)
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
        host: false,
        host_code: None,
        output_refs: None,
        part,
        tool_call_id,
        name,
        provider_executed: false,
        arguments,
        arguments_truncated,
        call_key: None,
        call_carrier: None,
        schema_pin: None,
        started_at: None,
        finished_at: None,
        unanswered: None,
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
        host: false,
        host_code: None,
        part,
        tool_call_id,
        provider_executed: false,
        is_error,
        content,
        truncated,
        files,
        unprojected: Vec::new(),
    })
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

// Native file containers can imply the file type without a `type` member.
fn file_kind(value: &Value) -> &str {
    value.get("type").and_then(Value::as_str).unwrap_or("file")
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
            if source["data"]["type"] == "media_ref" {
                let reference = serde_json::from_value(source["data"].clone())
                    .map_err(|_| ProjectionError::new(projection_reasons::INVALID_MEDIA_REF))?;
                return Ok(Some(FilePart::media(part, reference)));
            }
            let bytes = base64_decode(str_of(source, "data"))
                .ok_or_else(|| ProjectionError::new(projection_reasons::DATA_NOT_BASE64))?;
            let reference = Reference::media(
                (!part.is_empty()).then(|| part.clone()),
                media_kind(&mime),
                &mime,
                &bytes,
                None,
            );
            Ok(Some(FilePart {
                host: false,
                host_code: None,
                part,
                media: Some(reference),
                url: None,
                mime: None,
                filename: None,
            }))
        }
        "url" => Ok(Some(FilePart {
            host: false,
            host_code: None,
            part,
            media: None,
            url: Some(str_of(source, "url").to_owned()),
            mime: (!mime.is_empty()).then_some(mime),
            filename: None,
        })),
        _ => Ok(None),
    }
}

/// Extract readable OpenAI Responses summary text from a stored
/// `redacted_reasoning {provider_format, data}` part. Accepts only the
/// `open_ai_responses` and `open_ai_responses_chatgpt` formats, with `data` a
/// string holding the Responses item as JSON. Returns `None` for any other
/// part, or when no non-empty summary text exists. Opaque content never
/// contributes to the returned text.
pub fn reasoning_summary(part: &Value) -> Option<String> {
    use crate::archive_prunes_reasoning::provider_state_formats::{
        OPEN_AI_RESPONSES, OPEN_AI_RESPONSES_CHATGPT,
    };
    if str_of(part, "type") != "redacted_reasoning"
        || !matches!(
            str_of(part, "provider_format"),
            OPEN_AI_RESPONSES | OPEN_AI_RESPONSES_CHATGPT
        )
    {
        return None;
    }
    let item: Value = serde_json::from_str(part.get("data")?.as_str()?).ok()?;
    let texts: Vec<_> = item
        .get("summary")?
        .as_array()?
        .iter()
        .filter_map(|entry| {
            (str_of(entry, "type") == "summary_text")
                .then(|| entry.get("text").and_then(Value::as_str))
                .flatten()
        })
        .collect();
    if texts.iter().all(|text| text.is_empty()) {
        return None;
    }
    Some(texts.join("\n\n"))
}

fn redacted_reasoning(part: String, stored: &Value) -> DisplayPart {
    match reasoning_summary(stored) {
        Some(text) => DisplayPart::Reasoning(TextPart::new(part, text)),
        None => other(part, "redacted_reasoning"),
    }
}

fn other(part: String, kind: &str) -> DisplayPart {
    DisplayPart::Other(OtherPart::new(part, kind))
}

fn projected_file(
    file: Result<FilePart, ProjectionError>,
    part: String,
    kind: &str,
) -> DisplayPart {
    match file {
        Ok(file) => DisplayPart::File(file),
        Err(error) => DisplayPart::Other(OtherPart::new(part, kind).with_reason(error.message)),
    }
}

fn tool_file(
    file: Result<Option<FilePart>, ProjectionError>,
    kind: &str,
    files: &mut Vec<FilePart>,
    unprojected: &mut Vec<UnprojectedPart>,
) {
    match file {
        Ok(Some(file)) => files.push(file),
        Ok(None) => {}
        Err(error) => unprojected.push(UnprojectedPart::new(kind, error.message)),
    }
}

/// The row for an `anthropic-messages` body (CONTRACT.md §4.4): a Messages
/// API message, `{role, content}`, with `content` a string or an array of
/// blocks. `text` and `thinking` blocks become text and reasoning,
/// `tool_use` a tool call, `tool_result` a tool result (its nested text
/// joined, its nested images and documents as files), `image` and
/// `document` files. A Responses `redacted_reasoning` block becomes a reasoning
/// part when [`reasoning_summary`] returns text, and an `other` part when it
/// returns `None`. Any other block, a `redacted_thinking` or a
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
            "redacted_reasoning" => redacted_reasoning(key, block),
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
                let mut unprojected = Vec::new();
                match block.get("content") {
                    Some(Value::String(text)) => texts.push(text.clone()),
                    Some(Value::Array(nested)) => {
                        for inner in nested {
                            match str_of(inner, "type") {
                                "text" => texts.push(str_of(inner, "text").to_owned()),
                                "image" | "document" => {
                                    tool_file(
                                        anthropic_file(inner, String::new()),
                                        str_of(inner, "type"),
                                        &mut files,
                                        &mut unprojected,
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                DisplayPart::ToolResult(
                    ToolResultPart::new(
                        key,
                        str_of(block, "tool_use_id").to_owned(),
                        block.get("is_error") == Some(&Value::Bool(true)),
                        &crate::run::join_text_parts(texts),
                        files,
                        limits,
                    )
                    .with_unprojected(unprojected),
                )
            }
            "image" | "document" => match anthropic_file(block, key.clone()) {
                Ok(Some(file)) => DisplayPart::File(file),
                Ok(None) => other(key, kind),
                Err(error) => {
                    DisplayPart::Other(OtherPart::new(key, kind).with_reason(error.message))
                }
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
/// `data:` URL holding base64 as a `media_ref`, an existing `media_ref`
/// unchanged, and any other URL as written.
fn opencode_file(value: &Value, part: String) -> Result<FilePart, ProjectionError> {
    let mime = str_of(value, "mime").to_owned();
    let filename = value
        .get("filename")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let url = &value["url"];
    if url["type"] == "media_ref" {
        let reference = serde_json::from_value(url.clone())
            .map_err(|_| ProjectionError::new(projection_reasons::INVALID_MEDIA_REF))?;
        let mut output = FilePart::media(part, reference);
        output.filename = filename;
        return Ok(output);
    }
    let url = url
        .as_str()
        .ok_or_else(|| ProjectionError::new(projection_reasons::INVALID_FILE_URL))?;
    if let Some(data) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .map(|(_, data)| data)
    {
        let bytes = base64_decode(data)
            .ok_or_else(|| ProjectionError::new(projection_reasons::DATA_NOT_BASE64))?;
        let reference = Reference::media(
            (!part.is_empty()).then(|| part.clone()),
            media_kind(&mime),
            &mime,
            &bytes,
            None,
        );
        return Ok(FilePart {
            host: false,
            host_code: None,
            part,
            media: Some(reference),
            url: None,
            mime: None,
            filename,
        });
    }
    Ok(FilePart {
        host: false,
        host_code: None,
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
/// part a file. Part-level `metadata.providerExecuted` marks both the call
/// and result as provider-executed. The bookkeeping parts ([`OPENCODE_BOOKKEEPING_PARTS`]) are
/// left out. A Responses `redacted_reasoning` part becomes a reasoning part when
/// [`reasoning_summary`] returns text, and an `other` part when it returns
/// `None`; any other part is an `other` part.
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
            "redacted_reasoning" => parts.push(redacted_reasoning(key, part)),
            "text" => parts.push(DisplayPart::Text(TextPart::new(key, str_of(part, "text")))),
            "reasoning" => parts.push(DisplayPart::Reasoning(TextPart::new(
                key,
                str_of(part, "text"),
            ))),
            "tool" => {
                let state = &part["state"];
                let call_id = str_of(part, "callID").to_owned();
                let provider_executed = part["metadata"]["providerExecuted"] == true;
                parts.push(DisplayPart::ToolCall(
                    ToolCallPart::new(
                        key.clone(),
                        call_id.clone(),
                        str_of(part, "tool"),
                        state.get("input").unwrap_or(&Value::Object(Map::new())),
                        limits,
                    )
                    .with_provider_executed(provider_executed),
                ));
                if let status @ ("completed" | "error") = str_of(state, "status") {
                    let is_error = status == "error";
                    let mut files = Vec::new();
                    let mut unprojected = Vec::new();
                    for file in state
                        .get("attachments")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        tool_file(
                            opencode_file(file, String::new()).map(Some),
                            file_kind(file),
                            &mut files,
                            &mut unprojected,
                        );
                    }
                    parts.push(DisplayPart::ToolResult(
                        ToolResultPart::new(
                            key,
                            call_id,
                            is_error,
                            str_of(state, if is_error { "error" } else { "output" }),
                            files,
                            limits,
                        )
                        .with_provider_executed(provider_executed)
                        .with_unprojected(unprojected),
                    ));
                }
            }
            "file" => parts.push(projected_file(opencode_file(part, key.clone()), key, kind)),
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
            .map_err(|_| ProjectionError::new(projection_reasons::INVALID_MEDIA_REF))?;
        return Ok(FilePart::media(part, reference));
    }
    if image["data"].is_object() {
        return pi_image(&image["data"], part);
    }
    let bytes = base64_decode(str_of(image, "data"))
        .ok_or_else(|| ProjectionError::new(projection_reasons::DATA_NOT_BASE64))?;
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

fn opencode_v2_file(file: &Value, key: String) -> Result<FilePart, ProjectionError> {
    if let Some(uri) = file.get("uri").and_then(Value::as_str) {
        return opencode_file(
            &serde_json::json!({"url":uri, "mime":file["mime"], "filename":file["name"]}),
            key,
        );
    }
    let payload = file
        .get("uri")
        .filter(|uri| uri["type"] == "media_ref")
        .or_else(|| file.get("data"))
        .unwrap_or(file);
    if payload["type"] == "media_ref" {
        let reference = serde_json::from_value(payload.clone())
            .map_err(|_| ProjectionError::new(projection_reasons::INVALID_MEDIA_REF))?;
        let mut output = FilePart::media(key, reference);
        output.filename = file.get("name").and_then(Value::as_str).map(str::to_owned);
        return Ok(output);
    }
    let mime = str_of(file, "mime");
    let data = payload
        .as_str()
        .ok_or_else(|| ProjectionError::new(projection_reasons::MISSING_FILE_DATA))?;
    let bytes = base64_decode(data)
        .ok_or_else(|| ProjectionError::new(projection_reasons::DATA_NOT_BASE64))?;
    let mut output = FilePart::media(
        key.clone(),
        Reference::media(
            (!key.is_empty()).then_some(key),
            media_kind(mime),
            mime,
            &bytes,
            None,
        ),
    );
    output.filename = file.get("name").and_then(Value::as_str).map(str::to_owned);
    Ok(output)
}

fn opencode_v2_display_content(body: &Value) -> Result<Vec<Value>, ProjectionError> {
    let role = str_of(body, "type");
    if role == "compaction" {
        return Err(ProjectionError::new(
            "an opencode-v2 compaction is never served",
        ));
    }
    // Model-context validation rejects an entire body for one bad native slot.
    // Display keeps the slot so its failure cannot hide the neighbouring text.
    let array = |name: &str| -> Result<Vec<Value>, ProjectionError> {
        match body.get(name) {
            None => Ok(Vec::new()),
            Some(Value::Array(values)) => Ok(values.clone()),
            _ => Err(ProjectionError::new(
                "an opencode-v2 content container is not an array",
            )),
        }
    };
    let text = |value: &Value| serde_json::json!({"type":"text", "text":value});
    Ok(match role {
        "assistant" => array("content")?,
        "user" => {
            let mut content = Vec::new();
            for skill in array("skills")? {
                if let Some(value) = skill.get("text") {
                    content.push(text(value));
                }
            }
            if let Some(value) = body.get("text").filter(|value| value.as_str() != Some("")) {
                content.push(text(value));
            }
            content.extend(array("files")?);
            content
        }
        "synthetic" | "system" | "skill" => vec![text(&body["text"])],
        "shell" | "location-switched" => {
            let mut visible = body.clone();
            if role == "shell" {
                // Background execution is omitted from model context, not from display.
                visible["metadata"] = serde_json::json!({"background":false});
            }
            crate::serializer_profile::opencode_v2::projected_content(&visible)
                .map_err(|_| {
                    ProjectionError::new(if role == "shell" {
                        projection_reasons::INVALID_SHELL
                    } else {
                        projection_reasons::INVALID_LOCATION
                    })
                })?
                .as_array()
                .expect("native units")
                .clone()
        }
        "model-switched" | "agent-switched" | "idle" => Vec::new(),
        _ => {
            return Err(ProjectionError::new(
                "opencode-v2 body has no supported native type",
            ))
        }
    })
}

/// OpenCode 2's decoded message (`data` plus native `id` and `type`). Notices
/// keep their native type rather than adopting their model-request authority.
/// Compaction checkpoints are never served. Tool calls and outcomes share a
/// native slot, although display uses two parts to show them separately.
/// A tool slot's `executed` flag marks both parts as provider-executed.
pub fn opencode_v2(body: &Value, limits: Option<&Limits>) -> Result<DisplayRow, ProjectionError> {
    let role = str_of(body, "type");
    let content = match opencode_v2_display_content(body) {
        Ok(content) => content,
        Err(error) if matches!(role, "shell" | "location-switched") => {
            return Ok(DisplayRow::new(
                role,
                vec![DisplayPart::Other(
                    OtherPart::new("0", role).with_reason(error.message),
                )],
            ));
        }
        Err(error) => return Err(error),
    };
    let mut parts = Vec::new();
    for (index, slot) in content.iter().enumerate() {
        let key = index.to_string();
        if role == "user"
            && (slot.get("mime").is_some() || matches!(str_of(slot, "type"), "file" | "media_ref"))
        {
            parts.push(projected_file(
                opencode_v2_file(slot, key.clone()),
                key,
                file_kind(slot),
            ));
            continue;
        }
        match str_of(slot, "type") {
            "text" => {
                let value = if role == "system" {
                    body.get("description").unwrap_or(&slot["text"])
                } else {
                    &slot["text"]
                };
                parts.push(match value.as_str() {
                    Some(text) => DisplayPart::Text(TextPart::new(key, text)),
                    None if value["type"] == "text_ref" => other(key, "text_ref"),
                    None => DisplayPart::Other(
                        OtherPart::new(key, "text").with_reason(projection_reasons::INVALID_TEXT),
                    ),
                });
            }
            "reasoning" => parts.push(DisplayPart::Reasoning(TextPart::new(
                key,
                str_of(slot, "text"),
            ))),
            "tool" => {
                let state = &slot["state"];
                let call_id = str_of(slot, "id").to_owned();
                let provider_executed = slot["executed"] == true;
                parts.push(DisplayPart::ToolCall(
                    ToolCallPart::new(
                        key.clone(),
                        call_id.clone(),
                        str_of(slot, "name"),
                        &state["input"],
                        limits,
                    )
                    .with_provider_executed(provider_executed),
                ));
                let status = str_of(state, "status");
                if matches!(status, "completed" | "error") {
                    let mut texts = Vec::new();
                    let mut files = Vec::new();
                    let mut unprojected = Vec::new();
                    if let Some(content) = state["content"].as_array() {
                        for item in content {
                            if item["type"] == "text" {
                                texts.push(str_of(item, "text").to_owned());
                            } else if item.get("mime").is_some()
                                || matches!(str_of(item, "type"), "file" | "media_ref")
                            {
                                tool_file(
                                    opencode_v2_file(item, String::new()).map(Some),
                                    file_kind(item),
                                    &mut files,
                                    &mut unprojected,
                                );
                            }
                        }
                    }
                    if status == "error" {
                        let error = &state["error"];
                        let message = error
                            .as_str()
                            .or_else(|| error["message"].as_str())
                            .or_else(|| error["data"]["message"].as_str());
                        texts.push(
                            message
                                .map(str::to_owned)
                                .unwrap_or_else(|| error.to_string()),
                        );
                    }
                    parts.push(DisplayPart::ToolResult(
                        ToolResultPart::new(
                            key,
                            call_id,
                            status == "error",
                            &crate::run::join_text_parts(texts),
                            files,
                            limits,
                        )
                        .with_provider_executed(provider_executed)
                        .with_unprojected(unprojected),
                    ));
                }
            }
            "media_ref" => parts.push(projected_file(
                opencode_v2_file(slot, key.clone()),
                key,
                "media_ref",
            )),
            kind if role == "assistant" => parts.push(DisplayPart::Other(
                OtherPart::new(key, kind).with_reason(projection_reasons::UNSUPPORTED_PART_KIND),
            )),
            kind => parts.push(other(key, kind)),
        }
    }
    let mut row = DisplayRow::new(role, parts);
    if role == "assistant" {
        if let (Some(provider), Some(model)) = (
            body["model"]["providerID"].as_str(),
            body["model"]["id"].as_str(),
        ) {
            row.model = Some(format!("{provider}/{model}"));
        }
    }
    Ok(row)
}

/// A wrapped Pi journal row, using its runtime role, not the user role produced
/// for model requests. `custom_message` is `custom`, `branch_summary` is
/// `branchSummary`, and shell executions remain `bashExecution`. Shell text
/// shows command, output and status. Images use refs descriptors. Control rows,
/// including compaction summaries, are refused because they are never served.
/// Context edits belong to session projection/history_version handling, not raw rows.
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
    let (content, failed_native_part) =
        match crate::serializer_profile::pi::projected_content(&visible) {
            Ok(content) => (content, None),
            Err(_) if role == "bashExecution" => {
                let mut blocks = vec![Value::Null];
                if let Some(images) = body["message"]["images"].as_array() {
                    blocks.extend(images.iter().cloned());
                }
                (
                    Value::Array(blocks),
                    Some(("bashExecution", projection_reasons::INVALID_SHELL)),
                )
            }
            Err(_) if role == "branchSummary" => (
                Value::Array(vec![Value::Null]),
                Some(("branch_summary", projection_reasons::INVALID_SUMMARY)),
            ),
            Err(error) => return Err(ProjectionError::new(error)),
        };
    let blocks = match content {
        Value::String(text) => vec![serde_json::json!({"type":"text", "text":text})],
        Value::Array(blocks) => blocks,
        Value::Null => Vec::new(),
        reference => vec![reference],
    };
    let mut parts = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let key = index.to_string();
        if let Some((kind, reason)) = failed_native_part.filter(|_| index == 0) {
            parts.push(DisplayPart::Other(
                OtherPart::new(key, kind).with_reason(reason),
            ));
            continue;
        }
        parts.push(match str_of(block, "type") {
            "redacted_reasoning" => redacted_reasoning(key, block),
            "text" => DisplayPart::Text(TextPart::new(key, str_of(block, "text"))),
            "thinking" => DisplayPart::Reasoning(TextPart::new(key, str_of(block, "thinking"))),
            "toolCall" => arguments_part(
                key,
                str_of(block, "id").to_owned(),
                str_of(block, "name").to_owned(),
                &block["arguments"],
                limits,
            ),
            kind @ ("image" | "media_ref") => projected_file(
                pi_image(
                    block,
                    if role == "toolResult" {
                        String::new()
                    } else {
                        key.clone()
                    },
                ),
                key,
                kind,
            ),
            kind => other(key, kind),
        });
    }
    if role == "toolResult" {
        let mut texts = Vec::new();
        let mut files = Vec::new();
        let mut unprojected = Vec::new();
        for part in parts {
            match part {
                DisplayPart::Text(text) => texts.push(text.text),
                DisplayPart::File(file) => files.push(file),
                DisplayPart::Other(OtherPart {
                    kind,
                    reason: Some(reason),
                    ..
                }) => {
                    unprojected.push(UnprojectedPart::new(kind, reason));
                }
                _ => {}
            }
        }
        parts = vec![DisplayPart::ToolResult(
            ToolResultPart::new(
                "0",
                str_of(&body["message"], "toolCallId").to_owned(),
                body["message"]["isError"] == true,
                &crate::run::join_text_parts(texts),
                files,
                limits,
            )
            .with_unprojected(unprojected),
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
    let padding = text.len() - trimmed.len();
    if trimmed.len() % 4 == 1 || (padding != 0 && (!text.len().is_multiple_of(4) || padding > 2)) {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    for chunk in trimmed.as_bytes().chunks(4) {
        let mut acc = 0u32;
        for &byte in chunk {
            acc = (acc << 6) | sextet(byte)?;
        }
        // Unused low bits in the final partial quantum must be zero.
        if (chunk.len() == 2 && acc & 0x0f != 0) || (chunk.len() == 3 && acc & 0x03 != 0) {
            return None;
        }
        acc <<= 6 * (4 - chunk.len() as u32);
        let bytes = acc.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

/// One `session.history` entry in display form: the envelope's position
/// and served part keys, and the row in place of the stored body.
///
/// Non-exhaustive: build one with [`DisplayEntry::new`] and the `with_*`
/// setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DisplayEntry {
    /// The stored-order keys of the parts represented by this entry.
    /// Required for an archive-pruning runner; no keys are invented by a reader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_keys: Option<Vec<String>>,
    /// Required one-based position on the active path, as on raw history.
    pub ordinal: u64,
    pub mid: String,
    pub written_at: u64,
    pub row: DisplayRow,
}

impl DisplayEntry {
    pub fn with_part_keys(mut self, keys: Vec<String>) -> Self {
        self.part_keys = Some(keys);
        self
    }
    pub fn new(ordinal: u64, mid: impl Into<String>, written_at: u64, row: DisplayRow) -> Self {
        Self {
            part_keys: None,
            ordinal,
            mid: mid.into(),
            written_at,
            row,
        }
    }

    /// The display entry for a raw history `entry`, with `row` in place of
    /// its body.
    pub fn from_history(entry: &crate::history::HistoryEntry, row: DisplayRow) -> Self {
        Self {
            part_keys: entry
                .part_digests
                .as_ref()
                .map(|rows| rows.iter().map(|row| row.part.clone()).collect())
                .or_else(|| entry.part_keys.clone()),
            ordinal: entry.ordinal,
            mid: entry.mid.clone(),
            written_at: entry.written_at,
            row,
        }
    }

    pub fn with_ordinal(mut self, ordinal: u64) -> Self {
        self.ordinal = ordinal;
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
    pub incarnation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newer: Option<String>,
    pub history_version: u64,
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
            incarnation: page.incarnation.clone(),
            older: page.older.clone(),
            newer: page.newer.clone(),
            history_version: page.history_version,
            pruned_reasoning: page.pruned_reasoning.clone(),
        })
    }

    /// The rules a raw page shows on its own ([`crate::history::HistoryPage::check`]),
    /// checked on this page's envelope.
    pub fn check(&self) -> Result<(), crate::history::HistoryPageProblem> {
        let skeleton = crate::history::HistoryPage {
            messages: self
                .messages
                .iter()
                .map(|entry| {
                    let mut raw = crate::history::HistoryEntry::new(
                        entry.ordinal,
                        &entry.mid,
                        entry.written_at,
                        Value::Null,
                    );
                    raw.ordinal = entry.ordinal;
                    raw
                })
                .collect(),
            incarnation: self.incarnation.clone(),
            older: self.older.clone(),
            newer: self.newer.clone(),
            history_version: self.history_version,
            pruned_reasoning: self.pruned_reasoning.clone(),
        };
        skeleton.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parts_reader_vectors_preserve_host_members() {
        let file = vectors::load("display-host-parts.json");
        for case in file["readers"].as_array().unwrap() {
            let entry: DisplayEntry = serde_json::from_value(case["entry"].clone()).unwrap();
            let expected = case.get("canonical_entry").unwrap_or(&case["entry"]);
            assert_eq!(
                serde_json::to_value(&entry).unwrap(),
                *expected,
                "{}",
                case["name"]
            );
            let person_words = if entry.row.role == "user" {
                crate::run::join_text_parts(entry.row.parts.iter().filter_map(|part| match part {
                    DisplayPart::Text(text) if !part.host() => Some(text.text.as_str()),
                    _ => None,
                }))
            } else {
                String::new()
            };
            assert_eq!(person_words, case["person_words"], "{}", case["name"]);

            let page_value = serde_json::json!({
                "messages": [case["entry"]], "incarnation": "host-fixture", "history_version": 0
            });
            let page: DisplayPage = serde_json::from_value(page_value).unwrap();
            assert_eq!(page.messages, vec![entry]);
        }
    }

    #[test]
    fn host_parts_require_explicit_runner_marking() {
        let file = vectors::load("display-host-parts.json");
        let body = file["body"].clone();
        for case in file["rows"].as_array().unwrap() {
            let mut row = project(ANTHROPIC_MESSAGES, &body, None).unwrap();
            assert!(row.parts.iter().all(|part| !part.host()));
            if case["mark_host"] == true {
                row.parts[0] = row.parts[0].clone().with_host(true);
            }
            assert_eq!(
                serde_json::to_value(row).unwrap(),
                case["row"],
                "{}",
                case["name"]
            );
        }
        assert_eq!(body, file["body"], "display facts never modify model bytes");

        for role in ["user", "synthetic", "system", "skill"] {
            let native =
                serde_json::json!({"type": role, "text": "Host-looking text", "synthetic": true});
            let row = project(OPENCODE_V2, &native, None).unwrap();
            assert_eq!(row.role, role);
            assert!(row.parts.iter().all(|part| !part.host()));
            assert!(serde_json::to_value(&row).unwrap()["parts"][0]
                .get("host")
                .is_none());
            assert_eq!(
                project_with_attribution(OPENCODE_V2, &native, None, None).unwrap(),
                row
            );
        }
    }

    #[test]
    fn host_parts_all_variants_default_false_and_round_trip_true() {
        let parts = vec![
            DisplayPart::Text(TextPart::new("0", "text")),
            DisplayPart::Reasoning(TextPart::new("1", "reasoning")),
            DisplayPart::ToolCall(ToolCallPart::new(
                "2",
                "call",
                "tool",
                &serde_json::json!({}),
                None,
            )),
            DisplayPart::ToolResult(ToolResultPart::new(
                "2",
                "call",
                false,
                "result",
                Vec::new(),
                None,
            )),
            DisplayPart::File(FilePart::url("3", "https://example.com/file", None)),
            DisplayPart::Other(OtherPart::new("4", "opaque")),
        ];
        for part in parts {
            let unmarked = serde_json::to_value(&part).unwrap();
            assert!(!part.host());
            assert!(unmarked.get("host").is_none());
            let marked = part.with_host(true);
            let value = serde_json::to_value(&marked).unwrap();
            assert_eq!(value["host"], true);
            let decoded: DisplayPart = serde_json::from_value(value.clone()).unwrap();
            assert!(decoded.host());
            assert_eq!(decoded, marked);
            assert_eq!(
                serde_json::to_value(decoded.with_host(false)).unwrap(),
                unmarked
            );
            let mut false_value = value.clone();
            false_value["host"] = Value::Bool(false);
            let decoded_false: DisplayPart = serde_json::from_value(false_value).unwrap();
            assert_eq!(serde_json::to_value(decoded_false).unwrap(), unmarked);
            let mut invalid = value;
            invalid["host"] = Value::String("true".into());
            assert!(serde_json::from_value::<DisplayPart>(invalid).is_err());
        }
        let unknown: DisplayPart = serde_json::from_value(serde_json::json!({
            "type": "future_part", "part": "5", "host": true
        }))
        .unwrap();
        assert!(matches!(&unknown, DisplayPart::Other(part) if part.kind == "future_part"));
        assert!(
            unknown.host(),
            "an unknown part must not lose its host origin"
        );
    }

    #[test]
    fn host_parts_older_reader_accepts_the_new_member() {
        // The pre-host required entry/row/text shape, with serde's existing
        // unknown-member tolerance. Optional fields remain unknown to this reader.
        #[derive(Deserialize)]
        struct OlderTextPart {
            part: String,
            text: String,
        }
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum OlderPart {
            Text(OlderTextPart),
        }
        #[derive(Deserialize)]
        struct OlderRow {
            role: String,
            text: String,
            parts: Vec<OlderPart>,
        }
        #[derive(Deserialize)]
        struct OlderEntry {
            ordinal: u64,
            mid: String,
            written_at: u64,
            row: OlderRow,
        }
        let file = vectors::load("display-host-parts.json");
        let mixed = &file["readers"][0]["entry"];
        let older: OlderEntry = serde_json::from_value(mixed.clone()).unwrap();
        assert_eq!(older.ordinal, 7);
        assert_eq!(older.mid, "mixed-user-entry");
        assert_eq!(older.written_at, 1_700_000_000_000);
        assert_eq!(older.row.role, "user");
        assert_eq!(older.row.text, mixed["row"]["text"]);
        assert_eq!(older.row.parts.len(), 2);
        let OlderPart::Text(first) = &older.row.parts[0];
        assert_eq!(first.part, "0");
        assert_eq!(first.text, "[Request interrupted by user]\n");
    }

    #[test]
    fn reasoning_summary_reads_only_responses_summary_texts() {
        use serde_json::json;
        let mut part = json!({"type":"redacted_reasoning",
            "provider_format":"open_ai_responses",
            "data":json!({"type":"reasoning", "id":"rs_invented",
                "summary":[{"type":"summary_text", "text":"First observation."},
                    {"type":"ignored", "text":"Do not show this."},
                    {"type":"summary_text", "text":"Second observation."}],
                "encrypted_content":"invented-opaque-bytes"}).to_string()});
        for format in ["open_ai_responses", "open_ai_responses_chatgpt"] {
            part["provider_format"] = json!(format);
            assert_eq!(
                reasoning_summary(&part).as_deref(),
                Some("First observation.\n\nSecond observation.")
            );
        }
        part["provider_format"] = json!("other-provider");
        assert_eq!(reasoning_summary(&part), None);
        part["provider_format"] = json!("open_ai_responses");
        part["type"] = json!("reasoning");
        assert_eq!(reasoning_summary(&part), None);
        part["type"] = json!("redacted_reasoning");
        for data in [
            "not JSON",
            "{}",
            r#"{"summary":[]}"#,
            r#"{"summary":[{"type":"summary_text","text":""}],"encrypted_content":"invented"}"#,
        ] {
            part["data"] = json!(data);
            assert_eq!(reasoning_summary(&part), None, "{data}");
        }
    }

    #[test]
    fn reasoning_summary_projects_without_adding_stored_parts() {
        use serde_json::json;
        let part = json!({"type":"redacted_reasoning",
            "provider_format":"open_ai_responses_chatgpt",
            "data":r#"{"summary":[{"type":"summary_text","text":"A visible thought."}],"encrypted_content":"invented"}"#});
        let bodies = [
            (
                ANTHROPIC_MESSAGES,
                json!({"role":"assistant", "content":[part.clone()]}),
            ),
            (
                OPENCODE_AISDK,
                json!({"info":{"role":"assistant"}, "parts":[part.clone()]}),
            ),
            (
                PI,
                json!({"type":"message", "message":{"role":"assistant", "content":[part]}}),
            ),
        ];
        for (profile, body) in bodies {
            let original = body.clone();
            let row = project(profile, &body, None).unwrap();
            assert_eq!(body, original);
            assert_eq!(
                row.parts,
                vec![DisplayPart::Reasoning(TextPart::new(
                    "0",
                    "A visible thought."
                ))],
                "{profile}"
            );
            assert_eq!(row.text, "");
        }
    }

    #[test]
    fn base64_decode_accepts_canonical_padded_and_unpadded_input() {
        for (text, expected) in [
            ("", ""),
            ("TQ==", "M"),
            ("TQ", "M"),
            ("TWE=", "Ma"),
            ("TWE", "Ma"),
            ("TWFu", "Man"),
            ("TWFuTQ==", "ManM"),
        ] {
            assert_eq!(base64_decode(text).as_deref(), Some(expected.as_bytes()));
        }
    }

    #[test]
    fn base64_decode_rejects_bad_padding() {
        for text in [
            "=", "==", "===", "====", "TQ=", "TQ===", "TWE==", "TWFu=", "T=Q=", "TQ==TQ==", "A",
            "TQ==\n", "TQ-_",
        ] {
            assert_eq!(base64_decode(text), None, "accepted {text:?}");
        }
    }

    #[test]
    fn base64_decode_rejects_nonzero_trailing_bits() {
        for text in ["TR==", "TR", "TWF=", "TWF"] {
            assert_eq!(base64_decode(text), None, "accepted {text:?}");
        }
    }

    #[test]
    fn projects_is_true_for_the_four_profiles_and_project_refuses_others() {
        for profile in [ANTHROPIC_MESSAGES, OPENCODE_AISDK, OPENCODE_V2, PI] {
            assert!(projects(profile), "{profile}");
        }
        for profile in [crate::serializer_profile::OWNED_BROCA, "unknown"] {
            assert!(!projects(profile), "{profile}");
            let refused = project(profile, &serde_json::json!({}), None).unwrap_err();
            assert!(
                refused
                    .to_string()
                    .contains("not a serializer profile this crate projects"),
                "{refused}"
            );
        }
    }
    use crate::vectors;

    fn limits_of(case: &Value) -> Option<Limits> {
        case.get("limits")
            .map(|limits| serde_json::from_value(limits.clone()).expect("limits decode"))
    }

    fn check_failure_vectors(section: &str) {
        let file = vectors::load("display-unprojected.json");
        for case in vectors::cases(&file, section) {
            let name = case["name"].as_str().unwrap();
            let row = project(case["profile"].as_str().unwrap(), &case["body"], None)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(serde_json::to_value(&row).unwrap(), case["row"], "{name}");
            assert_eq!(
                vectors::round_trip::<DisplayRow>(name, &case["row"]),
                row,
                "{name}"
            );
        }
    }

    /// Every carried reason in the failure vectors is one of the published
    /// constants, so no reason can quote a transcript value.
    #[test]
    fn carried_reasons_are_published_constants() {
        let file = vectors::load("display-unprojected.json");
        let mut seen = 0;
        let mut check = |value: &Value| {
            let reason = value.as_str().expect("a reason is a string");
            assert!(
                projection_reasons::ALL.contains(&reason),
                "unpublished reason {reason:?}"
            );
            seen += 1;
        };
        for section in [
            "tool_files",
            "standalone",
            "native_failures",
            "carried_refusals",
        ] {
            for case in vectors::cases(&file, section) {
                for part in case["row"]["parts"].as_array().into_iter().flatten() {
                    if let Some(reason) = part.get("reason") {
                        check(reason);
                    }
                    for item in part["unprojected"].as_array().into_iter().flatten() {
                        check(&item["reason"]);
                    }
                }
            }
        }
        assert!(seen > 0, "the failure vectors carry no reason");
    }

    #[test]
    fn display_tool_results_carry_bad_files() {
        check_failure_vectors("tool_files");
    }

    #[test]
    fn display_error_tool_results_carry_bad_files() {
        let file = vectors::load("display-unprojected.json");
        for case in vectors::cases(&file, "tool_files") {
            let profile = case["profile"].as_str().unwrap();
            if !matches!(profile, OPENCODE_V2 | OPENCODE_AISDK) {
                continue;
            }
            let mut body = case["body"].clone();
            let state = if profile == OPENCODE_V2 {
                &mut body["content"][0]["state"]
            } else {
                &mut body["parts"][0]["state"]
            };
            state["status"] = Value::String("error".into());
            state["error"] = Value::String("tool failed".into());
            let mut expected = case["row"].clone();
            expected["parts"][1]["is_error"] = Value::Bool(true);
            expected["parts"][1]["content"] = Value::String(
                if profile == OPENCODE_V2 {
                    "readable resulttool failed"
                } else {
                    "tool failed"
                }
                .into(),
            );
            assert_eq!(
                serde_json::to_value(project(profile, &body, None).unwrap()).unwrap(),
                expected,
                "{profile}"
            );
        }
    }

    #[test]
    fn display_standalone_parts_carry_failures() {
        check_failure_vectors("standalone");
        check_failure_vectors("native_failures");
        check_failure_vectors("carried_refusals");
    }

    #[test]
    fn display_attribution_still_hides_frame_with_a_bad_neighbour() {
        let stored = format!("{}readable message", crate::attribution::header("chat"));
        let fact = crate::attribution::verified(1, "chat", &stored).unwrap();
        let body = serde_json::json!({"type":"assistant", "content":[
            {"type":"future-part"}, {"type":"text", "text":stored}
        ]});
        let row = project_with_attribution(OPENCODE_V2, &body, None, Some(&fact)).unwrap();
        assert_eq!(row.text, "readable message");
        assert_eq!(row.attribution, Some(fact));
        assert_eq!(
            row.parts[1],
            DisplayPart::Text(TextPart::new("1", "readable message"))
        );
        assert!(
            matches!(&row.parts[0], DisplayPart::Other(part) if part.part == "0" && part.reason.as_deref() == Some(projection_reasons::UNSUPPORTED_PART_KIND))
        );
    }

    #[test]
    fn display_unknown_shapes_still_refuse() {
        let file = vectors::load("display-unprojected.json");
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            let error =
                project(case["profile"].as_str().unwrap(), &case["body"], None).expect_err(name);
            assert!(!error.message.contains("private-body-sentinel!"), "{name}");
        }
    }

    #[test]
    fn display_never_served_bodies_still_refuse() {
        let file = vectors::load("display-unprojected.json");
        for case in vectors::cases(&file, "never_served") {
            let error = project(case["profile"].as_str().unwrap(), &case["body"], None)
                .expect_err(case["name"].as_str().unwrap());
            assert_eq!(error.message, case["reason"].as_str().unwrap());
        }
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
        // Preserve the published fixture bytes, but explicitly check the two
        // old per-image refusals against the newly carried display rows.
        let failures = vectors::load("display-unprojected.json");
        let carried: Vec<_> = vectors::cases(&failures, "carried_refusals")
            .iter()
            .filter(|case| case["profile"] == profile)
            .collect();
        let mut carried_count = 0;
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            if let Some(expected) = carried.iter().find(|expected| expected["name"] == name) {
                assert_eq!(
                    case["body"], expected["body"],
                    "{name}: historical body changed"
                );
                assert_eq!(
                    serde_json::to_value(project(profile, &case["body"], None).unwrap()).unwrap(),
                    expected["row"],
                    "{name}"
                );
                carried_count += 1;
            } else {
                assert!(project(profile, &case["body"], None).is_err(), "{name}");
            }
        }
        assert_eq!(
            carried_count,
            carried.len(),
            "{profile}: carried refusal is not a historical fixture"
        );
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
    fn opencode_v2_vectors_project() {
        check_profile("display-opencode-v2.json", OPENCODE_V2);
    }

    #[test]
    fn opencode_aisdk_vectors_project() {
        check_profile("display-opencode-aisdk.json", OPENCODE_AISDK);
    }

    #[test]
    fn opencode_file_refuses_non_string_urls_and_invalid_references() {
        for url in [
            Value::Null,
            serde_json::json!(false),
            serde_json::json!(42),
            serde_json::json!([]),
            serde_json::json!({"type":"unknown"}),
            serde_json::json!({"type":"media_ref"}),
        ] {
            let file = serde_json::json!({"type":"file", "mime":"image/jpeg", "url":url});
            let error = opencode_file(&file, "0".to_owned()).unwrap_err();
            let expected = if file["url"]["type"] == "media_ref" {
                projection_reasons::INVALID_MEDIA_REF
            } else {
                projection_reasons::INVALID_FILE_URL
            };
            assert_eq!(error.message, expected);
        }
    }

    #[test]
    fn display_reasons_are_fixed_and_contain_no_body_bytes() {
        use projection_reasons::*;
        use serde_json::json;
        const SECRET: &str = "private-body-sentinel!";
        let allowed = [
            INVALID_MEDIA_REF,
            DATA_NOT_BASE64,
            INVALID_FILE_URL,
            MISSING_FILE_DATA,
            INVALID_TEXT,
            INVALID_SHELL,
            INVALID_LOCATION,
            INVALID_SUMMARY,
            UNSUPPORTED_PART_KIND,
        ];
        let mut reasons = Vec::new();
        let bad_ref = json!({"type":"media_ref", "kind":"image", "mime":"image/png", "byte_size":SECRET, "sha256":"b1ff9c8ea3a780bad09b346c423d2d0e46815926879b18e841d928376a946640"});
        let cases = [
            (
                anthropic_file(
                    &json!({"source":{"type":"base64", "data":bad_ref}}),
                    "0".into(),
                )
                .unwrap_err(),
                INVALID_MEDIA_REF,
            ),
            (
                anthropic_file(
                    &json!({"source":{"type":"base64", "data":SECRET}}),
                    "0".into(),
                )
                .unwrap_err(),
                DATA_NOT_BASE64,
            ),
            (
                opencode_file(&json!({"url":bad_ref}), "0".into()).unwrap_err(),
                INVALID_MEDIA_REF,
            ),
            (
                opencode_file(&json!({"url":{"secret":SECRET}}), "0".into()).unwrap_err(),
                INVALID_FILE_URL,
            ),
            (
                opencode_file(
                    &json!({"url":format!("data:image/png;base64,{SECRET}")}),
                    "0".into(),
                )
                .unwrap_err(),
                DATA_NOT_BASE64,
            ),
            (
                opencode_v2_file(&json!({"data":bad_ref}), "0".into()).unwrap_err(),
                INVALID_MEDIA_REF,
            ),
            (
                opencode_v2_file(&json!({"data":{"secret":SECRET}}), "0".into()).unwrap_err(),
                MISSING_FILE_DATA,
            ),
            (
                opencode_v2_file(&json!({"data":SECRET}), "0".into()).unwrap_err(),
                DATA_NOT_BASE64,
            ),
            (
                pi_image(&bad_ref, "0".into()).unwrap_err(),
                INVALID_MEDIA_REF,
            ),
            (
                pi_image(&json!({"data":SECRET}), "0".into()).unwrap_err(),
                DATA_NOT_BASE64,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.message, expected);
            reasons.push(error.message);
        }
        let file = vectors::load("display-unprojected.json");
        for section in [
            "tool_files",
            "standalone",
            "native_failures",
            "carried_refusals",
        ] {
            for case in vectors::cases(&file, section) {
                let row = project(case["profile"].as_str().unwrap(), &case["body"], None).unwrap();
                for part in row.parts {
                    match part {
                        DisplayPart::ToolResult(result) => reasons
                            .extend(result.unprojected.into_iter().map(|failure| failure.reason)),
                        DisplayPart::Other(OtherPart {
                            reason: Some(reason),
                            ..
                        }) => reasons.push(reason),
                        _ => {}
                    }
                }
            }
        }
        for reason in &reasons {
            assert!(allowed.contains(&reason.as_str()), "{reason}");
            assert!(!reason.contains(SECRET));
        }
        for reason in allowed {
            assert!(
                reasons.iter().any(|seen| seen == reason),
                "untested reason: {reason}"
            );
        }
    }

    #[test]
    fn display_clean_rows_omit_failure_fields() {
        let result = ToolResultPart::new("0", "call", false, "readable", vec![], None);
        let other = OtherPart::new("1", "unknown");
        assert_eq!(
            serde_json::to_string(&result).unwrap(),
            r#"{"part":"0","tool_call_id":"call","content":"readable"}"#
        );
        assert_eq!(
            serde_json::to_string(&other).unwrap(),
            r#"{"part":"1","kind":"unknown"}"#
        );
        assert!(
            serde_json::from_value::<ToolResultPart>(serde_json::to_value(result).unwrap())
                .unwrap()
                .unprojected
                .is_empty()
        );
        assert!(
            serde_json::from_value::<OtherPart>(serde_json::to_value(other).unwrap())
                .unwrap()
                .reason
                .is_none()
        );
    }

    #[test]
    fn anthropic_described_media_keeps_payload_metadata() {
        use serde_json::json;

        for (kind, mime) in [("image", "image/jpeg"), ("document", "application/pdf")] {
            let reference = json!({
                "type":"media_ref", "kind":if kind == "image" { "image" } else { "pdf" },
                "mime":mime, "byte_size":3,
                "sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "part":"stored-file"
            });
            let file = json!({"type":kind,
                "source":{"type":"base64", "media_type":mime, "data":reference}});
            let body = json!({"role":"user", "content":[file,
                {"type":"tool_result", "tool_use_id":"invented-call", "content":[
                    {"type":"text", "text":"An invented result"}, file
                ]}
            ]});
            let row = anthropic_messages(&body, None).unwrap();
            assert_eq!(
                serde_json::to_value(row).unwrap(),
                json!({"role":"user", "text":"", "parts":[
                    {"type":"file", "part":"0", "media":reference},
                    {"type":"tool_result", "part":"1", "tool_call_id":"invented-call",
                     "content":"An invented result", "files":[{"media":reference}]}
                ]})
            );
        }
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
                host: false,
                host_code: None,
                part: "3".into(),
                kind: "hologram".into(),
                reason: None,
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
            assert_eq!(page.check(), Ok(()), "{name}");
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
            let head = crate::read::HeadMeta::empty(1, "log-1").with_input(input);
            assert!(
                head.snapshot_consistent(),
                "{name}: input is outside the active path rule"
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
    fn tool_times_and_unanswered_outcomes_round_trip_without_inventing_times() {
        let file: Value = serde_json::from_str(include_str!(
            "../test-vectors/llm-runner-v1/display-tool-times.json"
        ))
        .unwrap();
        for case in file["rows"].as_array().unwrap() {
            let row: DisplayRow = vectors::round_trip(case["name"].as_str().unwrap(), &case["row"]);
            for part in &row.parts {
                let DisplayPart::ToolCall(call) = part else {
                    panic!("fixture must hold tool calls");
                };
                let mut built = ToolCallPart::new(
                    &call.part,
                    &call.tool_call_id,
                    &call.name,
                    &serde_json::json!({}),
                    None,
                );
                assert_eq!(built.started_at, None);
                assert_eq!(built.finished_at, None);
                assert_eq!(built.unanswered, None);
                if let Some(time) = call.started_at {
                    built = built.with_started_at(time);
                }
                if let Some(time) = call.finished_at {
                    built = built.with_finished_at(time);
                }
                if let Some(reason) = &call.unanswered {
                    built = built.with_unanswered(reason);
                }
                assert_eq!(&built, call);
            }
        }
    }

    #[test]
    fn provider_execution_flags_default_to_false_and_round_trip_on_both_parts() {
        let call = ToolCallPart::new("0", "call_1", "web_search", &serde_json::json!({}), None);
        let result = ToolResultPart::new("0", "call_1", false, "record", Vec::new(), None);
        assert!(!call.provider_executed);
        assert!(!result.provider_executed);
        for (part, legacy) in [
            (
                DisplayPart::ToolCall(call),
                serde_json::json!({
                    "type": "tool_call", "part": "0", "tool_call_id": "call_1",
                    "name": "web_search", "arguments": "{}"
                }),
            ),
            (
                DisplayPart::ToolResult(result),
                serde_json::json!({
                    "type": "tool_result", "part": "0", "tool_call_id": "call_1",
                    "content": "record"
                }),
            ),
        ] {
            assert_eq!(serde_json::to_value(&part).unwrap(), legacy);
            assert_eq!(
                serde_json::from_value::<DisplayPart>(legacy.clone()).unwrap(),
                part
            );
            let mut explicit_false = legacy.clone();
            explicit_false["provider_executed"] = Value::Bool(false);
            assert_eq!(
                serde_json::from_value::<DisplayPart>(explicit_false).unwrap(),
                part
            );
            let provider_part = match part {
                DisplayPart::ToolCall(call) => {
                    DisplayPart::ToolCall(call.with_provider_executed(true))
                }
                DisplayPart::ToolResult(result) => {
                    DisplayPart::ToolResult(result.with_provider_executed(true))
                }
                _ => unreachable!(),
            };
            let mut expected = legacy.clone();
            expected["provider_executed"] = Value::Bool(true);
            assert_eq!(serde_json::to_value(&provider_part).unwrap(), expected);
            assert_eq!(
                serde_json::from_value::<DisplayPart>(expected).unwrap(),
                provider_part
            );
            let local_part = match provider_part {
                DisplayPart::ToolCall(call) => {
                    DisplayPart::ToolCall(call.with_provider_executed(false))
                }
                DisplayPart::ToolResult(result) => {
                    DisplayPart::ToolResult(result.with_provider_executed(false))
                }
                _ => unreachable!(),
            };
            assert_eq!(serde_json::to_value(local_part).unwrap(), legacy);
        }
    }

    #[test]
    fn existing_display_vectors_keep_their_bytes_without_tool_times() {
        for file in [
            "display-anthropic-messages.json",
            "display-opencode-aisdk.json",
            "display-opencode-v2.json",
            "display-pi.json",
        ] {
            let file = vectors::load(file);
            for case in vectors::cases(&file, "rows") {
                let row: DisplayRow = serde_json::from_value(case["row"].clone()).unwrap();
                for part in &row.parts {
                    if let DisplayPart::ToolCall(call) = part {
                        assert_eq!(
                            (call.started_at, call.finished_at, &call.unanswered),
                            (None, None, &None)
                        );
                    }
                }
                assert_eq!(
                    serde_json::to_vec(&serde_json::to_value(row).unwrap()).unwrap(),
                    serde_json::to_vec(&case["row"]).unwrap(),
                    "{}",
                    case["name"]
                );
            }
        }
    }

    #[test]
    fn a_display_page_checks_like_a_raw_page() {
        let row = DisplayRow::new("user", vec![DisplayPart::Text(TextPart::new("0", "hi"))]);
        let page = DisplayPage {
            pruned_reasoning: Vec::new(),
            messages: vec![
                DisplayEntry::new(2, "m1", 1, row.clone()),
                DisplayEntry::new(1, "m0", 1, row),
            ],
            incarnation: "log-1".into(),
            older: None,
            newer: None,
            history_version: 1,
        };
        assert_eq!(
            page.check(),
            Err(crate::history::HistoryPageProblem::OrdinalsNotRising(
                "m0".into()
            ))
        );
    }
}
