use serde::{Deserialize, Serialize};

use crate::{Cursor, RequestSize, StepSource, Verdict};

/// One step in the feed. Rows carry no content: never prompt text, tool names
/// or error text.
///
/// A row's identity is `harness` plus `step_ref`. When any field of a step
/// changes, the provider sends the whole row again at a new cursor; the last
/// row for an identity wins, and a reader replaces, never appends (see
/// [`crate::LatestRows`]).
///
/// Optional members are omitted when unknown and are never zero for unknown.
/// `output` includes reasoning: `reasoning` is a breakdown of `output`, never
/// an addition to it, so `reasoning <= output` whenever both are present (see
/// [`StepRow::check_counts`]). `cache_write` is the sum of every cache
/// lifetime. Unknown members are
/// ignored when decoding; [`StepRow::check_closed`] names them for the
/// provider's own tests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StepRow {
    /// This row's feed position.
    pub cursor: Cursor,
    /// The producer: a coding harness or a runner's module id. For a step a
    /// runner announced, see [`live_row_identity`].
    pub harness: String,
    pub source: StepSource,
    /// The producer's own id for the step. For a step a runner announced, see
    /// [`live_row_identity`].
    pub step_ref: String,
    pub session: String,
    /// Runner steps only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Unix milliseconds. Steps without a time are not in the feed.
    pub occurred_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Uncached input tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    /// The sum of every cache lifetime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
    /// Present only when the source split cache writes by lifetime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_5m: Option<u64>,
    /// Present only when the source split cache writes by lifetime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<u64>,
    /// Output tokens, reasoning included. Charge it once; never add
    /// `reasoning` on top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    /// The part of `output` the provider reports as reasoning: a breakdown,
    /// never an addition. Absent means not reported, not zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    /// The producer's finish or stop reason, as a classification string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    /// Request body length in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body_len: Option<u64>,
    /// Input plus cache read plus cache write.
    pub prompt: u64,
    /// This step's cache read divided by the expected cache read: a
    /// non-negative number, never rounded or clamped, so it exceeds 1 when a
    /// step reads more than expected. Absent when there was nothing to
    /// compare with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<f64>,
    pub verdict: Verdict,
    pub prompt_dropped: bool,
    /// On a prompt drop, present when both compared steps report body lengths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_size: Option<RequestSize>,
}

/// A row whose counts break a contract rule. Readers still decode such a row;
/// this is for the provider's own tests.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BadCounts {
    /// `reasoning` is a breakdown of `output`, so it can never exceed it.
    ReasoningExceedsOutput {
        harness: String,
        step_ref: String,
        reasoning: u64,
        output: u64,
    },
}

impl std::fmt::Display for BadCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReasoningExceedsOutput {
                harness,
                step_ref,
                reasoning,
                output,
            } => write!(
                f,
                "row {harness} {step_ref}: reasoning {reasoning} exceeds output {output}"
            ),
        }
    }
}

impl std::error::Error for BadCounts {}

/// The identity `(harness, step_ref)` of the row the provider serves for a step
/// a runner announced on the bus (`step.finished.v1`), from that event's
/// members. Plain strings, so it needs nothing from the runner contract crate.
///
/// The live row must carry the identity of the row the provider later records
/// from a durable read, or last-row-wins could not replace it:
///
/// - With a `harness_step_ref` (the runner declares `history_key: "harness"`
///   and the host's own store is the durable source): `harness` is the
///   event's `harness`, the hosting program, and `step_ref` is the host's
///   record id.
/// - Without one: `harness` is the event's `runner`, the runner's module id,
///   and `step_ref` is `<run_id>#<step_id>#<attempt>`, with `attempt` in
///   decimal and 1-based, as the runner contract pins it. Each attempt
///   actually sent is its own row, because a billed retry is spend and its
///   cache behaviour differs. For a runner that is its own durable source
///   (`history_key: "step_ref"`) this is the identity the provider gives the
///   attempt when it reads the runner's step log. For a host-keyed runner
///   with no host record, the row stays live-only.
pub fn live_row_identity(
    runner: &str,
    harness: &str,
    harness_step_ref: Option<&str>,
    run_id: &str,
    step_id: &str,
    attempt: u64,
) -> (String, String) {
    match harness_step_ref {
        Some(host_record) => (harness.to_owned(), host_record.to_owned()),
        None => (runner.to_owned(), format!("{run_id}#{step_id}#{attempt}")),
    }
}

/// A JSON value that is not within a published v1 member list.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotClosed {
    /// The value checked is not a JSON object.
    NotAnObject { object: &'static str },
    /// The object carries a member outside its published list.
    UnknownMember {
        object: &'static str,
        member: String,
    },
}

impl std::fmt::Display for NotClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnObject { object } => write!(f, "{object} is not a JSON object"),
            Self::UnknownMember { object, member } => {
                write!(f, "{object} carries unknown member `{member}`")
            }
        }
    }
}

impl std::error::Error for NotClosed {}

pub(crate) fn check_members(
    object: &'static str,
    value: &serde_json::Value,
    allowed: &[&str],
) -> Result<(), NotClosed> {
    let members = value.as_object().ok_or(NotClosed::NotAnObject { object })?;
    match members.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(member) => Err(NotClosed::UnknownMember {
            object,
            member: member.clone(),
        }),
        None => Ok(()),
    }
}

impl StepRow {
    /// Members every row carries.
    pub const REQUIRED_MEMBERS: &'static [&'static str] = &[
        "cursor",
        "harness",
        "source",
        "step_ref",
        "session",
        "occurred_at_ms",
        "prompt",
        "verdict",
        "prompt_dropped",
    ];

    /// Members omitted when unknown.
    pub const OPTIONAL_MEMBERS: &'static [&'static str] = &[
        "run_id",
        "provider",
        "model",
        "input",
        "cache_read",
        "cache_write",
        "cache_write_5m",
        "cache_write_1h",
        "output",
        "reasoning",
        "finish",
        "context_limit",
        "request_body_len",
        "retention",
        "request_size",
    ];

    /// Builds a row from its required members; optional members start absent.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cursor: impl Into<Cursor>,
        harness: impl Into<String>,
        source: StepSource,
        step_ref: impl Into<String>,
        session: impl Into<String>,
        occurred_at_ms: u64,
        prompt: u64,
        verdict: Verdict,
        prompt_dropped: bool,
    ) -> Self {
        Self {
            cursor: cursor.into(),
            harness: harness.into(),
            source,
            step_ref: step_ref.into(),
            session: session.into(),
            run_id: None,
            occurred_at_ms,
            provider: None,
            model: None,
            input: None,
            cache_read: None,
            cache_write: None,
            cache_write_5m: None,
            cache_write_1h: None,
            output: None,
            reasoning: None,
            finish: None,
            context_limit: None,
            request_body_len: None,
            prompt,
            retention: None,
            verdict,
            prompt_dropped,
            request_size: None,
        }
    }

    /// The row's identity: `(harness, step_ref)`.
    pub fn identity(&self) -> (&str, &str) {
        (&self.harness, &self.step_ref)
    }

    /// Reports a member outside the published v1 row list, by name. For the
    /// provider's own tests: whatever it emits must stay within the published
    /// list until this crate updates it. Readers ignore unknown members
    /// instead, so decoding never calls this.
    pub fn check_closed(value: &serde_json::Value) -> Result<(), NotClosed> {
        let allowed: Vec<&str> = Self::REQUIRED_MEMBERS
            .iter()
            .chain(Self::OPTIONAL_MEMBERS)
            .copied()
            .collect();
        check_members("row", value, &allowed)
    }

    /// Reports a row whose `reasoning` exceeds its `output`, naming it by
    /// identity. `reasoning` is a breakdown inside `output`, so a row with
    /// both must have `reasoning <= output`. For the provider's own tests,
    /// like [`StepRow::check_closed`]; readers still decode such a row.
    pub fn check_counts(&self) -> Result<(), BadCounts> {
        match (self.reasoning, self.output) {
            (Some(reasoning), Some(output)) if reasoning > output => {
                Err(BadCounts::ReasoningExceedsOutput {
                    harness: self.harness.clone(),
                    step_ref: self.step_ref.clone(),
                    reasoning,
                    output,
                })
            }
            _ => Ok(()),
        }
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
    pub fn with_input(mut self, input: u64) -> Self {
        self.input = Some(input);
        self
    }
    pub fn with_cache_read(mut self, cache_read: u64) -> Self {
        self.cache_read = Some(cache_read);
        self
    }
    pub fn with_cache_write(mut self, cache_write: u64) -> Self {
        self.cache_write = Some(cache_write);
        self
    }
    pub fn with_cache_write_5m(mut self, cache_write_5m: u64) -> Self {
        self.cache_write_5m = Some(cache_write_5m);
        self
    }
    pub fn with_cache_write_1h(mut self, cache_write_1h: u64) -> Self {
        self.cache_write_1h = Some(cache_write_1h);
        self
    }
    pub fn with_output(mut self, output: u64) -> Self {
        self.output = Some(output);
        self
    }
    pub fn with_reasoning(mut self, reasoning: u64) -> Self {
        self.reasoning = Some(reasoning);
        self
    }
    pub fn with_finish(mut self, finish: impl Into<String>) -> Self {
        self.finish = Some(finish.into());
        self
    }
    pub fn with_context_limit(mut self, context_limit: u64) -> Self {
        self.context_limit = Some(context_limit);
        self
    }
    pub fn with_request_body_len(mut self, request_body_len: u64) -> Self {
        self.request_body_len = Some(request_body_len);
        self
    }
    pub fn with_retention(mut self, retention: f64) -> Self {
        self.retention = Some(retention);
        self
    }
    pub fn with_request_size(mut self, request_size: RequestSize) -> Self {
        self.request_size = Some(request_size);
        self
    }
}
