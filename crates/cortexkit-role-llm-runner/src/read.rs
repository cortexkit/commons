//! `session.read` and `session.head`: the transcript reads.
//!
//! Three read modes share one request ([`ReadRequest::mode`]):
//!
//! - **tail**: no cursor. The newest page. This is the contract, not a
//!   default a runner may change.
//! - **range**: `from_ordinal`. Messages from that ordinal forward.
//! - **after**: `after_mid` with its `lineage_id`. Messages strictly after
//!   that message. Refused by name as `lineage_changed` or `unknown_mid`,
//!   never answered from the tail instead.
//!
//! A page stops at `limit` messages or at the byte cap, whichever comes
//! first, and carries `next_from_ordinal` whenever it stops before the end
//! of the transcript. Every page carries the session's `lineage_id`; a read
//! that names another lineage is refused `lineage_changed`.
//!
//! A page is an ordinal range and nothing more: it is not aligned to turns,
//! runs or tool-call boundaries, so a tool call and its result may arrive on
//! different pages.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// The `field` an `invalid_params` refusal names when a read combines
/// `after_mid` with `from_ordinal`, or names `after_mid` without
/// `lineage_id`.
pub mod fields {
    pub const FROM_ORDINAL: &str = "from_ordinal";
    pub const LINEAGE_ID: &str = "lineage_id";
    pub const VIEW: &str = "view";
}

/// Which list of messages a read pages through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadView {
    /// The transcript as written, with each message's final values. The
    /// default.
    Raw,
    /// The canonical message list the model is given after hooks and
    /// compaction. Served only by a runner that declares `model_view`.
    Model,
}

fn deserialize_view<'de, D>(deserializer: D) -> Result<Option<ReadView>, D::Error>
where
    D: Deserializer<'de>,
{
    crate::strict::optional(deserializer, fields::VIEW, "\"raw\" or \"model\"")
}

/// The `session.read` request.
///
/// Strict on unknown fields: absence is meaningful here (no cursor means
/// "the newest page"), so a misspelled cursor must be refused rather than
/// read as a tail request. A runner refuses a field it does not serve with
/// `invalid_params` naming it.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`ReadRequest::tail`], [`ReadRequest::range`] or [`ReadRequest::after`]
/// and the `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReadRequest {
    /// Read forward from this ordinal. Excludes `after_mid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ordinal: Option<u64>,
    /// Read strictly after this message id. Requires `lineage_id`. A `mid`
    /// is opaque to consumers: only ever a value a page returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_mid: Option<String>,
    /// The lineage the cursor came from. A read naming a lineage other than
    /// the session's current one is refused `lineage_changed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// At most this many messages. The runner may cap it lower.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// At most this many bytes of messages. The runner may cap it lower.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    /// Add `originals` to every message a hook changed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_originals: bool,
    /// The view to page through. Absent means `raw`. The value is strict: an
    /// unknown view is refused naming `view`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_view"
    )]
    pub view: Option<ReadView>,
}

/// The mode a well-formed [`ReadRequest`] selects: the newest page, a range
/// from an ordinal, or the messages after a message id in a lineage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadMode<'a> {
    Tail,
    Range { from_ordinal: u64 },
    After { mid: &'a str, lineage_id: &'a str },
}

impl ReadRequest {
    /// The newest page.
    pub fn tail() -> Self {
        Self::default()
    }

    /// Messages from `from_ordinal` forward.
    pub fn range(from_ordinal: u64) -> Self {
        Self {
            from_ordinal: Some(from_ordinal),
            ..Self::default()
        }
    }

    /// Messages strictly after `mid` in `lineage_id`.
    pub fn after(mid: impl Into<String>, lineage_id: impl Into<String>) -> Self {
        Self {
            after_mid: Some(mid.into()),
            lineage_id: Some(lineage_id.into()),
            ..Self::default()
        }
    }

    /// Check the cursor names `lineage_id`.
    pub fn with_lineage_id(mut self, lineage_id: impl Into<String>) -> Self {
        self.lineage_id = Some(lineage_id.into());
        self
    }

    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }

    pub fn with_include_originals(mut self, include_originals: bool) -> Self {
        self.include_originals = include_originals;
        self
    }

    pub fn with_view(mut self, view: ReadView) -> Self {
        self.view = Some(view);
        self
    }

    /// The read mode, or the field an `invalid_params` refusal names:
    /// `from_ordinal` when it is combined with `after_mid`, `lineage_id`
    /// when `after_mid` comes without it.
    pub fn mode(&self) -> Result<ReadMode<'_>, &'static str> {
        match (&self.after_mid, self.from_ordinal, &self.lineage_id) {
            (Some(_), Some(_), _) => Err(fields::FROM_ORDINAL),
            (Some(_), None, None) => Err(fields::LINEAGE_ID),
            (Some(mid), None, Some(lineage_id)) => Ok(ReadMode::After { mid, lineage_id }),
            (None, Some(from_ordinal), _) => Ok(ReadMode::Range { from_ordinal }),
            (None, None, _) => Ok(ReadMode::Tail),
        }
    }
}

/// A `session.read` page. Decoded leniently: unknown fields are ignored.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ReadPage::new`], [`ReadPage::no_lineage`] and the `with_*` setters, or
/// decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReadPage {
    pub messages: Vec<ReadMessage>,
    /// The session's lineage, on every page of a session that has one.
    /// Absent means the session has no lineage yet: it was never written,
    /// and the page is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// Present whenever the page stopped before the end of the transcript,
    /// by count or by bytes: the ordinal to read from next. A single message
    /// larger than the byte cap comes alone on its page, over the cap, with
    /// this set if more follow; a message is never truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_from_ordinal: Option<u64>,
    /// The position of the last durable event covered by the same snapshot
    /// as this page: a JSON object, opaque to consumers. A subscription from
    /// it continues strictly after it. Absent on a session with no events
    /// yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Map<String, Value>>,
}

impl ReadPage {
    /// A page of a session whose lineage is `lineage_id`.
    pub fn new(lineage_id: impl Into<String>, messages: Vec<ReadMessage>) -> Self {
        Self {
            messages,
            lineage_id: Some(lineage_id.into()),
            next_from_ordinal: None,
            head: None,
        }
    }

    /// The page of a session that was never written: empty, with no lineage
    /// and no `next_from_ordinal`.
    pub fn no_lineage() -> Self {
        Self {
            messages: Vec::new(),
            lineage_id: None,
            next_from_ordinal: None,
            head: None,
        }
    }

    pub fn with_next_from_ordinal(mut self, next_from_ordinal: u64) -> Self {
        self.next_from_ordinal = Some(next_from_ordinal);
        self
    }

    pub fn with_head(mut self, head: Map<String, Value>) -> Self {
        self.head = Some(head);
        self
    }

    /// Whether the page is consistent with its lineage: a page without a
    /// lineage holds no messages and no `next_from_ordinal`, because a
    /// session that has messages has a lineage.
    pub fn lineage_consistent(&self) -> bool {
        self.lineage_id.is_some() || (self.messages.is_empty() && self.next_from_ordinal.is_none())
    }
}

/// One message on a page.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ReadMessage::new`] and the `with_*` setters, or decode one.
/// External callers cannot construct one with a struct literal:
///
/// ```compile_fail,E0639
/// use cortexkit_role_llm_runner::read::ReadMessage;
/// let message = ReadMessage {
///     ordinal: 0,
///     mid: "m0".into(),
///     message: serde_json::json!({}),
///     run: None,
///     originals: None,
///     tool_calls: Vec::new(),
/// };
/// ```
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReadMessage {
    /// The message's position in the transcript. Never reused or
    /// renumbered within a lineage.
    pub ordinal: u64,
    /// The message's id, opaque to consumers. Never reused within a
    /// lineage.
    pub mid: String,
    /// The message itself, carrying its final values: what later steps
    /// render and what executed. Its schema is the runner's.
    pub message: Value,
    /// Which run produced the message. Present on a runner that declares
    /// `run_ops`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunAttribution>,
    /// What hooks changed, oldest first. Present only when the request set
    /// `include_originals` and a hook changed this message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originals: Option<Vec<Original>>,
    /// One entry per tool call in the message. Present on a runner that
    /// declares `dispatch_attribution`, for messages that carry tool calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallAttribution>,
}

impl ReadMessage {
    pub fn new(ordinal: u64, mid: impl Into<String>, message: Value) -> Self {
        Self {
            ordinal,
            mid: mid.into(),
            message,
            run: None,
            originals: None,
            tool_calls: Vec::new(),
        }
    }

    pub fn with_run(mut self, run: RunAttribution) -> Self {
        self.run = Some(run);
        self
    }

    pub fn with_originals(mut self, originals: Vec<Original>) -> Self {
        self.originals = Some(originals);
        self
    }

    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCallAttribution>) -> Self {
        self.tool_calls = tool_calls;
        self
    }
}

/// The run and episode that produced a message, and whether the message is
/// that run's final one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RunAttribution {
    pub run_id: String,
    /// The episode within the session, opaque to consumers.
    pub episode: String,
    /// Whether this is the run's final message.
    #[serde(rename = "final")]
    pub is_final: bool,
}

/// One hooked field's value before a hook changed it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Original {
    /// The message field the hook changed, as the runner's message schema
    /// names it.
    pub field: String,
    /// The value before the change.
    pub value: Value,
    /// The module whose hook changed it.
    pub provider: String,
}

/// Where one tool call went.
///
/// Non-exhaustive so later optional members are additive: use
/// [`ToolCallAttribution::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolCallAttribution {
    /// The model's id for the call. Display only: it is not unique.
    pub tool_call_id: String,
    /// The call's key, unique per runner: present once the call was
    /// prepared or dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
    /// The module the call was dispatched to. Absent for a call never
    /// dispatched (denied, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_to: Option<String>,
    /// The call has a durable dispatch intent and no result: it may or may
    /// not have run.
    #[serde(default)]
    pub indeterminate: bool,
}

impl ToolCallAttribution {
    pub fn new(tool_call_id: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            call_key: None,
            dispatched_to: None,
            indeterminate: false,
        }
    }

    pub fn with_call_key(mut self, call_key: impl Into<String>) -> Self {
        self.call_key = Some(call_key.into());
        self
    }

    pub fn with_dispatched_to(mut self, module: impl Into<String>) -> Self {
        self.dispatched_to = Some(module.into());
        self
    }

    pub fn with_indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }
}

/// The `session.head` request. It takes no fields, and refuses any.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeadRequest {}

/// The `session.head` answer: where the transcript stands, without message
/// bodies. Decoded leniently.
///
/// Non-exhaustive so later optional members are additive: use
/// [`HeadMeta::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HeadMeta {
    pub lineage_id: String,
    /// The newest message's ordinal. Absent on an empty transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ordinal: Option<u64>,
    /// The same opaque position a page's `head` carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Map<String, Value>>,
    /// The session's most recent run. Absent before the first run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_state: Option<LastRunState>,
    /// When the transcript last changed, in milliseconds since the Unix
    /// epoch.
    pub updated_at: u64,
}

impl HeadMeta {
    pub fn new(lineage_id: impl Into<String>, updated_at: u64) -> Self {
        Self {
            lineage_id: lineage_id.into(),
            last_ordinal: None,
            head: None,
            last_run_state: None,
            updated_at,
        }
    }

    pub fn with_last_ordinal(mut self, last_ordinal: u64) -> Self {
        self.last_ordinal = Some(last_ordinal);
        self
    }

    pub fn with_head(mut self, head: Map<String, Value>) -> Self {
        self.head = Some(head);
        self
    }

    pub fn with_last_run_state(mut self, last_run_state: LastRunState) -> Self {
        self.last_run_state = Some(last_run_state);
        self
    }
}

/// The state of a session's most recent run. Describes that run, never the
/// transcript as a whole.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct LastRunState {
    pub run_id: String,
    /// A run state ([`crate::run::states`]); any other value decodes as a
    /// plain string.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};

    #[test]
    fn request_vectors_decode_round_trip_and_select_their_mode() {
        let file = vectors::load("read-requests.json");
        for case in vectors::cases(&file, "accepted") {
            let name = case["name"].as_str().unwrap();
            let request: ReadRequest = vectors::round_trip(name, &case["request"]);
            let mode = match request.mode() {
                Ok(ReadMode::Tail) => "tail",
                Ok(ReadMode::Range { .. }) => "range",
                Ok(ReadMode::After { .. }) => "after",
                Err(field) => panic!("{name}: refused naming {field}"),
            };
            assert_eq!(mode, case["mode"].as_str().unwrap(), "{name}");
        }
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            match serde_json::from_value::<ReadRequest>(case["request"].clone()) {
                Err(error) => assert!(error.to_string().contains(field), "{name}: {error}"),
                Ok(request) => assert_eq!(request.mode(), Err(field), "{name}"),
            }
            assert_eq!(case["refusal"]["code"], errors::INVALID_PARAMS, "{name}");
            assert_eq!(
                errors::refused_field(errors::INVALID_PARAMS, Some(&case["refusal"]["detail"])),
                Some(field),
                "{name}"
            );
        }
    }

    #[test]
    fn page_vectors_decode_and_round_trip() {
        let file = vectors::load("read-pages.json");
        for case in vectors::cases(&file, "pages") {
            let name = case["name"].as_str().unwrap();
            let page: ReadPage = vectors::round_trip(name, &case["page"]);
            if case["stopped_early"].as_bool().unwrap() {
                assert!(page.next_from_ordinal.is_some(), "{name}");
            }
            assert!(page.lineage_consistent(), "{name}");
            if let Some(cap) = case.get("max_bytes").and_then(Value::as_u64) {
                // An oversize message comes alone, whole, over the cap.
                assert_eq!(page.messages.len(), 1, "{name}");
                let bytes = serde_json::to_vec(&page.messages[0]).unwrap().len() as u64;
                assert!(bytes > cap, "{name}: {bytes} bytes is not over {cap}");
            }
        }
        for case in vectors::cases(&file, "inconsistent") {
            let name = case["name"].as_str().unwrap();
            let page: ReadPage = serde_json::from_value(case["page"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!page.lineage_consistent(), "{name}");
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            serde_json::from_value::<ReadPage>(case["page"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for case in vectors::cases(&file, "undecodable") {
            let name = case["name"].as_str().unwrap();
            assert!(
                serde_json::from_value::<ReadPage>(case["page"].clone()).is_err(),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            let code = case["refusal"]["code"].as_str().unwrap();
            assert!(
                [errors::LINEAGE_CHANGED, errors::UNKNOWN_MID].contains(&code),
                "{name}"
            );
            assert!(!errors::is_retryable(code), "{name}");
            serde_json::from_value::<ReadRequest>(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn head_vectors_decode_and_round_trip() {
        let file = vectors::load("head.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<HeadRequest>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "refused_requests") {
            assert!(
                serde_json::from_value::<HeadRequest>(case["request"].clone()).is_err(),
                "{}",
                case["name"]
            );
        }
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            vectors::round_trip::<HeadMeta>(name, &case["answer"]);
            assert!(case["answer"].get("messages").is_none(), "{name}");
        }
    }

    #[test]
    fn builders_match_the_decoded_shapes() {
        let request = ReadRequest::after("m7", "lin-1")
            .with_limit(50)
            .with_max_bytes(65_536)
            .with_include_originals(true)
            .with_view(ReadView::Raw);
        assert_eq!(
            request.mode(),
            Ok(ReadMode::After {
                mid: "m7",
                lineage_id: "lin-1"
            })
        );
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<ReadRequest>(encoded).unwrap(),
            request
        );

        let message = ReadMessage::new(3, "m3", serde_json::json!({"role": "assistant"}))
            .with_run(RunAttribution {
                run_id: "r1".into(),
                episode: "e1".into(),
                is_final: true,
            })
            .with_tool_calls(vec![ToolCallAttribution::new("call_0")
                .with_call_key("lin-1:42")
                .with_dispatched_to("aft")
                .with_indeterminate(true)]);
        assert!(ReadPage::no_lineage().lineage_consistent());
        let page = ReadPage::new("lin-1", vec![message])
            .with_next_from_ordinal(4)
            .with_head(serde_json::json!({"seq": 9}).as_object().unwrap().clone());
        let encoded = serde_json::to_value(&page).unwrap();
        assert_eq!(serde_json::from_value::<ReadPage>(encoded).unwrap(), page);
        assert_eq!(
            ReadRequest::range(5).with_lineage_id("lin-1").mode(),
            Ok(ReadMode::Range { from_ordinal: 5 })
        );
    }
}
