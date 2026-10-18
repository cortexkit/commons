//! The two ck-bus events of the `async` group (CONTRACT.md §20.2, §20.3).
//!
//! Both are claim checks: the broker carries a subject, an id, a digest and
//! string headers, never message or view content. An [`AdvanceNotice`] wakes
//! the provider, which then reads the new messages with `session.read`; a
//! [`ViewReady`] tells the runner which view to fetch with `view.get`.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::{finish, frame, is_sha256_hex, parse_decimal};

/// Event name of the runner's advance notice, under the runner's module id.
pub const ADVANCED_EVENT: &str = "compaction.advanced";
/// Event name of the provider's view-ready event, under the provider's
/// module id.
pub const VIEW_READY_EVENT: &str = "compaction.view_ready";
/// Both events are at version 1: [`event_subject`] ends their subjects in `.v1`.
pub const EVENT_VERSION: u32 = 1;

/// Upper bound on a notice's headers: the sum over every header this contract
/// defines of the name's and the value's length in bytes, the way the bus
/// counts a message's header size. The per-value bounds below keep every
/// well-formed notice under it (at most 1,659 bytes); a runner with more
/// events than fit in `events` splits them across consecutive notices.
pub const MAX_NOTICE_HEADER_BYTES: usize = 2048;
/// Upper bound on one header value, in bytes.
pub const MAX_HEADER_VALUE_BYTES: usize = 1024;
/// Upper bound on the number of tokens in `events`.
pub const MAX_EVENTS: usize = 32;
/// Upper bound on one event token, in bytes.
pub const MAX_EVENT_TOKEN_BYTES: usize = 128;
/// Upper bound on a `cut_id`, in bytes.
pub const MAX_CUT_ID_BYTES: usize = 64;
/// Upper bound on a `view_refused` reason, in bytes.
pub const MAX_REFUSAL_REASON_BYTES: usize = 32;
/// Upper bound on the `model` header, in bytes.
pub const MAX_MODEL_BYTES: usize = 256;

/// The domain string that starts an advance notice's digest input, so the
/// same headers can never produce a digest valid for another kind of record.
pub const NOTICE_DIGEST_DOMAIN: &str = "ck.compaction.advanced.v1";

/// Header names. Every value is ASCII; numbers are canonical decimal `u64`
/// (digits only, no sign, no leading zero except `0` itself).
pub mod headers {
    /// The session reference, `llm-runner/v1`'s `session_ref`: 32 lowercase
    /// hexadecimal characters.
    pub const SESSION_REF: &str = "session_ref";
    /// The notice's per-session sequence number, from 1, one higher for every
    /// notice of the session.
    pub const SEQ: &str = "seq";
    /// The newest ordinal written in the session's current lineage when the
    /// notice was recorded.
    pub const THROUGH_ORDINAL: &str = "through_ordinal";
    /// [`super::super::content_digest`] of the message at `through_ordinal`.
    pub const THROUGH_DIGEST: &str = "through_digest";
    /// Comma-separated event tokens, in the order they happened; omitted when
    /// there are none.
    pub const EVENTS: &str = "events";
    /// The runner's estimate, in tokens, of its next request built from its
    /// current view plus the messages after it.
    pub const EST_TOKENS: &str = "est_tokens";
    /// The current model's context window in tokens; omitted when unknown.
    pub const WINDOW: &str = "window";
    /// The current model id.
    pub const MODEL: &str = "model";
    /// The last response's measured usage,
    /// `<input>:<cache_read>:<cache_write>:<output>`; omitted before the first.
    pub const LAST_USAGE: &str = "last_usage";
    /// The version of the view the runner renders now; omitted before the
    /// first view. This is the runner's durable acknowledgement: every notice
    /// repeats it, so a lost notice loses no acknowledgement.
    pub const APPLIED_VIEW_VERSION: &str = "applied_view_version";
    /// A view's version.
    pub const VIEW_VERSION: &str = "view_version";
    /// The last ordinal a view covers.
    pub const COVERS_THROUGH: &str = "covers_through";
    /// A view's digest ([`crate::async_group::view::CompactionView::compute_digest`]).
    pub const DIGEST: &str = "digest";

    /// The headers of an advance notice, in ascending byte order: the order
    /// the notice digest frames them in.
    pub const NOTICE: &[&str] = &[
        APPLIED_VIEW_VERSION,
        EST_TOKENS,
        EVENTS,
        LAST_USAGE,
        MODEL,
        SEQ,
        SESSION_REF,
        THROUGH_DIGEST,
        THROUGH_ORDINAL,
        WINDOW,
    ];
}

/// The subject one module publishes one of these events on:
/// `ck.{account}.event.{module_id}.{event}.v1`. `cortexkit-bus-naming`'s
/// `event_subject` builds the same bytes and validates the tokens.
pub fn event_subject(account: &str, module_id: &str, event: &str) -> String {
    format!("ck.{account}.event.{module_id}.{event}.v{EVENT_VERSION}")
}

/// Whether `value` is an `llm-runner/v1` session reference: 32 lowercase
/// hexadecimal characters.
pub fn is_session_ref(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// What the runner reports about the host's native compaction marker for a
/// view's cut (CONTRACT.md §20.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerOutcome {
    /// The host's marker is written at the cut.
    Applied,
    /// The runner will write it later (for example at the host's next
    /// opportunity); a later notice reports the final outcome.
    Deferred,
    /// The runner cannot tell whether the host kept it.
    Unknown,
}

impl MarkerOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Deferred => "deferred",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "applied" => Some(Self::Applied),
            "deferred" => Some(Self::Deferred),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// Reasons a runner gives in `view_refused`. Open: a provider that meets
/// another reason treats the view as refused all the same.
pub mod refusal_reasons {
    /// Applying the view would separate a tool call from its result.
    pub const TOOL_PAIR_SPLIT: &str = "tool_pair_split";
    /// Applying the view would break the current model provider's role order.
    pub const ROLE_ORDER: &str = "role_order";
    /// The current model's own rules forbid the view and the runner cannot
    /// adapt it (for example, signed thinking it may not strip).
    pub const MODEL_RULES: &str = "model_rules";
}

/// One token of a notice's `events` header.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// `model_change`: the session's model changed.
    ModelChange,
    /// `surface_change`: the frozen system text or tool surface changed.
    SurfaceChange,
    /// `ttl_expired`: the prompt cache expired. Declared only by a runner that
    /// knows the cache lifetime it actually sent (CONTRACT.md §20.2).
    TtlExpired,
    /// `revert_below:<ordinal>`: the transcript was truncated, and every
    /// message at `ordinal` or above is gone.
    RevertBelow(u64),
    /// `marker_outcome:<applied|deferred|unknown>:<cut_id>`: the host's
    /// native marker for the cut of the view the same notice names in
    /// `applied_view_version`.
    MarkerOutcome {
        outcome: MarkerOutcome,
        cut_id: String,
    },
    /// `view_refused:<view_version>:<reason>`: the runner will not render
    /// this view and keeps rendering its current one. `reason` is one of
    /// [`refusal_reasons`] or another `[a-z0-9_]` word of at most 32 bytes.
    ViewRefused { view_version: u64, reason: String },
    /// `view_discarded:<view_version>`: the runner discarded this view
    /// because its coverage is no longer a prefix of the transcript. Always
    /// comes with `revert_below` in the same notice.
    ViewDiscarded(u64),
    /// A well-formed token this crate does not know. The provider ignores it.
    Unknown(String),
}

fn is_cut_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CUT_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// The generic token grammar every event follows, known or not: a name of
/// `[a-z][a-z0-9_]*`, then zero or more `:<argument>` segments of
/// `[A-Za-z0-9._-]+`, at most [`MAX_EVENT_TOKEN_BYTES`] in all.
fn is_event_token(token: &str) -> bool {
    if token.is_empty() || token.len() > MAX_EVENT_TOKEN_BYTES {
        return false;
    }
    let mut segments = token.split(':');
    let name = segments.next().unwrap_or_default();
    let name_ok = name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    name_ok
        && segments.all(|argument| {
            !argument.is_empty()
                && argument
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

impl Event {
    /// The token as it appears in `events`.
    pub fn token(&self) -> String {
        match self {
            Self::ModelChange => "model_change".into(),
            Self::SurfaceChange => "surface_change".into(),
            Self::TtlExpired => "ttl_expired".into(),
            Self::RevertBelow(ordinal) => format!("revert_below:{ordinal}"),
            Self::MarkerOutcome { outcome, cut_id } => {
                format!("marker_outcome:{}:{cut_id}", outcome.as_str())
            }
            Self::ViewRefused {
                view_version,
                reason,
            } => format!("view_refused:{view_version}:{reason}"),
            Self::ViewDiscarded(version) => format!("view_discarded:{version}"),
            Self::Unknown(token) => token.clone(),
        }
    }

    /// Parse one token. A known name with arguments it does not take is
    /// malformed; an unknown name that follows the generic grammar decodes as
    /// [`Event::Unknown`].
    pub fn parse(token: &str) -> Result<Self, NoticeProblem> {
        let malformed = || NoticeProblem::Malformed(headers::EVENTS);
        if !is_event_token(token) {
            return Err(malformed());
        }
        let (name, rest) = token.split_once(':').unwrap_or((token, ""));
        let version = |rest: &str| {
            parse_decimal(rest)
                .filter(|v| *v >= 1)
                .ok_or_else(malformed)
        };
        match name {
            "model_change" | "surface_change" | "ttl_expired" if !rest.is_empty() => {
                Err(malformed())
            }
            "model_change" => Ok(Self::ModelChange),
            "surface_change" => Ok(Self::SurfaceChange),
            "ttl_expired" => Ok(Self::TtlExpired),
            "revert_below" => parse_decimal(rest)
                .map(Self::RevertBelow)
                .ok_or_else(malformed),
            "view_refused" => {
                let (view_version, reason) = rest.split_once(':').ok_or_else(malformed)?;
                let view_version = version(view_version)?;
                let reason_ok = !reason.is_empty()
                    && reason.len() <= MAX_REFUSAL_REASON_BYTES
                    && reason
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
                if !reason_ok {
                    return Err(malformed());
                }
                Ok(Self::ViewRefused {
                    view_version,
                    reason: reason.to_owned(),
                })
            }
            "view_discarded" => version(rest).map(Self::ViewDiscarded),
            "marker_outcome" => {
                let (outcome, cut_id) = rest.split_once(':').ok_or_else(malformed)?;
                let outcome = MarkerOutcome::parse(outcome).ok_or_else(malformed)?;
                if !is_cut_id(cut_id) {
                    return Err(malformed());
                }
                Ok(Self::MarkerOutcome {
                    outcome,
                    cut_id: cut_id.to_owned(),
                })
            }
            _ => Ok(Self::Unknown(token.to_owned())),
        }
    }
}

/// The last response's measured usage, in tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LastUsage {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl LastUsage {
    pub fn new(input: u64, cache_read: u64, cache_write: u64, output: u64) -> Self {
        Self {
            input,
            cache_read,
            cache_write,
            output,
        }
    }

    fn encode(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.input, self.cache_read, self.cache_write, self.output
        )
    }

    fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split(':').map(parse_decimal);
        let usage = Self::new(
            parts.next()??,
            parts.next()??,
            parts.next()??,
            parts.next()??,
        );
        parts.next().is_none().then_some(usage)
    }
}

/// Why a notice or a view-ready event's headers are not acceptable.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NoticeProblem {
    /// A required header is absent.
    Missing(&'static str),
    /// A header's value does not follow its encoding.
    Malformed(&'static str),
    /// The same event token appears twice.
    DuplicateEvent(String),
    /// More than [`MAX_EVENTS`] event tokens.
    TooManyEvents,
    /// `view_discarded` without `revert_below` in the same notice.
    DiscardWithoutRevert,
    /// `marker_outcome` without `applied_view_version` in the same notice.
    MarkerWithoutAppliedView,
    /// The headers exceed [`MAX_NOTICE_HEADER_BYTES`].
    TooLarge(usize),
}

impl NoticeProblem {
    /// The problem's name as the vectors spell it, for example
    /// `missing:seq` or `malformed:events`.
    pub fn name(&self) -> String {
        match self {
            Self::Missing(header) => format!("missing:{header}"),
            Self::Malformed(header) => format!("malformed:{header}"),
            Self::DuplicateEvent(_) => "duplicate_event".into(),
            Self::TooManyEvents => "too_many_events".into(),
            Self::DiscardWithoutRevert => "view_discarded_without_revert_below".into(),
            Self::MarkerWithoutAppliedView => "marker_outcome_without_applied_view_version".into(),
            Self::TooLarge(_) => "too_large".into(),
        }
    }
}

fn required<'a>(
    map: &'a BTreeMap<String, String>,
    name: &'static str,
) -> Result<&'a str, NoticeProblem> {
    let value = map.get(name).ok_or(NoticeProblem::Missing(name))?;
    if value.len() > MAX_HEADER_VALUE_BYTES {
        return Err(NoticeProblem::Malformed(name));
    }
    Ok(value)
}

fn optional<'a>(
    map: &'a BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<&'a str>, NoticeProblem> {
    match map.get(name) {
        None => Ok(None),
        Some(value) if value.is_empty() || value.len() > MAX_HEADER_VALUE_BYTES => {
            Err(NoticeProblem::Malformed(name))
        }
        Some(value) => Ok(Some(value)),
    }
}

fn decimal(value: &str, name: &'static str) -> Result<u64, NoticeProblem> {
    parse_decimal(value).ok_or(NoticeProblem::Malformed(name))
}

fn is_model(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_MODEL_BYTES
        && value.bytes().all(|b| b.is_ascii_graphic())
}

/// The runner's advance notice: `ck.{acct}.event.<runner_module_id>.compaction.advanced.v1`,
/// id [`AdvanceNotice::message_id`], digest [`AdvanceNotice::digest`], and
/// these fields as string headers ([`AdvanceNotice::to_headers`]).
///
/// It carries no message content. It only wakes the provider, which reads
/// what is new with `session.read` from its own cursor (CONTRACT.md §20.2).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdvanceNotice {
    pub session_ref: String,
    pub seq: u64,
    pub through_ordinal: u64,
    pub through_digest: String,
    pub events: Vec<Event>,
    pub est_tokens: u64,
    pub model: String,
    pub window: Option<u64>,
    pub last_usage: Option<LastUsage>,
    pub applied_view_version: Option<u64>,
}

impl AdvanceNotice {
    pub fn new(
        session_ref: impl Into<String>,
        seq: u64,
        through_ordinal: u64,
        through_digest: impl Into<String>,
        est_tokens: u64,
        model: impl Into<String>,
    ) -> Self {
        Self {
            session_ref: session_ref.into(),
            seq,
            through_ordinal,
            through_digest: through_digest.into(),
            events: Vec::new(),
            est_tokens,
            model: model.into(),
            window: None,
            last_usage: None,
            applied_view_version: None,
        }
    }

    pub fn with_applied_view_version(mut self, view_version: u64) -> Self {
        self.applied_view_version = Some(view_version);
        self
    }

    pub fn with_events(mut self, events: Vec<Event>) -> Self {
        self.events = events;
        self
    }

    pub fn with_window(mut self, window: u64) -> Self {
        self.window = Some(window);
        self
    }

    pub fn with_last_usage(mut self, usage: LastUsage) -> Self {
        self.last_usage = Some(usage);
        self
    }

    /// The bus message id, `<session_ref>:<seq>`. A runner that republishes a
    /// notice after a crash reuses its id and headers, so the bus discards the
    /// duplicate.
    pub fn message_id(&self) -> String {
        format!("{}:{}", self.session_ref, self.seq)
    }

    /// The headers, keyed by name. Optional headers are omitted when absent.
    pub fn to_headers(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert(headers::SESSION_REF.into(), self.session_ref.clone());
        map.insert(headers::SEQ.into(), self.seq.to_string());
        map.insert(
            headers::THROUGH_ORDINAL.into(),
            self.through_ordinal.to_string(),
        );
        map.insert(headers::THROUGH_DIGEST.into(), self.through_digest.clone());
        if !self.events.is_empty() {
            let tokens: Vec<String> = self.events.iter().map(Event::token).collect();
            map.insert(headers::EVENTS.into(), tokens.join(","));
        }
        map.insert(headers::EST_TOKENS.into(), self.est_tokens.to_string());
        map.insert(headers::MODEL.into(), self.model.clone());
        if let Some(window) = self.window {
            map.insert(headers::WINDOW.into(), window.to_string());
        }
        if let Some(usage) = &self.last_usage {
            map.insert(headers::LAST_USAGE.into(), usage.encode());
        }
        if let Some(version) = self.applied_view_version {
            map.insert(headers::APPLIED_VIEW_VERSION.into(), version.to_string());
        }
        map
    }

    /// Decode and check a notice's headers. Headers this contract does not
    /// define are ignored.
    pub fn from_headers(map: &BTreeMap<String, String>) -> Result<Self, NoticeProblem> {
        let events = match optional(map, headers::EVENTS)? {
            None => Vec::new(),
            Some(value) => value
                .split(',')
                .map(Event::parse)
                .collect::<Result<Vec<_>, _>>()?,
        };
        let notice = Self {
            session_ref: required(map, headers::SESSION_REF)?.to_owned(),
            seq: decimal(required(map, headers::SEQ)?, headers::SEQ)?,
            through_ordinal: decimal(
                required(map, headers::THROUGH_ORDINAL)?,
                headers::THROUGH_ORDINAL,
            )?,
            through_digest: required(map, headers::THROUGH_DIGEST)?.to_owned(),
            events,
            est_tokens: decimal(required(map, headers::EST_TOKENS)?, headers::EST_TOKENS)?,
            model: required(map, headers::MODEL)?.to_owned(),
            window: optional(map, headers::WINDOW)?
                .map(|value| decimal(value, headers::WINDOW))
                .transpose()?,
            last_usage: optional(map, headers::LAST_USAGE)?
                .map(|value| {
                    LastUsage::parse(value).ok_or(NoticeProblem::Malformed(headers::LAST_USAGE))
                })
                .transpose()?,
            applied_view_version: optional(map, headers::APPLIED_VIEW_VERSION)?
                .map(|value| decimal(value, headers::APPLIED_VIEW_VERSION))
                .transpose()?,
        };
        notice.check()?;
        Ok(notice)
    }

    /// Check every rule a built or decoded notice must meet.
    pub fn check(&self) -> Result<(), NoticeProblem> {
        if !is_session_ref(&self.session_ref) {
            return Err(NoticeProblem::Malformed(headers::SESSION_REF));
        }
        if self.seq == 0 {
            return Err(NoticeProblem::Malformed(headers::SEQ));
        }
        if !is_sha256_hex(&self.through_digest) {
            return Err(NoticeProblem::Malformed(headers::THROUGH_DIGEST));
        }
        if !is_model(&self.model) {
            return Err(NoticeProblem::Malformed(headers::MODEL));
        }
        if self.events.len() > MAX_EVENTS {
            return Err(NoticeProblem::TooManyEvents);
        }
        let mut seen = std::collections::BTreeSet::new();
        for event in &self.events {
            let token = event.token();
            if Event::parse(&token).as_ref() != Ok(event) {
                return Err(NoticeProblem::Malformed(headers::EVENTS));
            }
            if !seen.insert(token.clone()) {
                return Err(NoticeProblem::DuplicateEvent(token));
            }
        }
        let discards = self
            .events
            .iter()
            .any(|event| matches!(event, Event::ViewDiscarded(_)));
        let reverts = self
            .events
            .iter()
            .any(|event| matches!(event, Event::RevertBelow(_)));
        if discards && !reverts {
            return Err(NoticeProblem::DiscardWithoutRevert);
        }
        if self.applied_view_version == Some(0) {
            return Err(NoticeProblem::Malformed(headers::APPLIED_VIEW_VERSION));
        }
        let marker = self
            .events
            .iter()
            .any(|event| matches!(event, Event::MarkerOutcome { .. }));
        if marker && self.applied_view_version.is_none() {
            return Err(NoticeProblem::MarkerWithoutAppliedView);
        }
        let headers = self.to_headers();
        if let Some((name, _)) = headers
            .iter()
            .find(|(_, value)| value.len() > MAX_HEADER_VALUE_BYTES)
        {
            let name = headers::NOTICE
                .iter()
                .find(|known| **known == name.as_str())
                .copied()
                .unwrap_or(headers::EVENTS);
            return Err(NoticeProblem::Malformed(name));
        }
        let size = header_bytes(&headers);
        if size > MAX_NOTICE_HEADER_BYTES {
            return Err(NoticeProblem::TooLarge(size));
        }
        Ok(())
    }

    /// The bus digest: SHA-256 over the canonical header set. The input is
    /// the framed domain [`NOTICE_DIGEST_DOMAIN`], then, for every header of
    /// [`headers::NOTICE`] the notice carries, in ascending byte order of its
    /// name, the framed name followed by the framed value. Each frame is an
    /// 8-byte big-endian length and the bytes. Lowercase hexadecimal.
    pub fn digest(&self) -> String {
        notice_digest(&self.to_headers())
    }
}

/// The notice digest over a header map, counting only the headers this
/// contract defines ([`AdvanceNotice::digest`]).
pub fn notice_digest(map: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    frame(&mut hasher, NOTICE_DIGEST_DOMAIN.as_bytes());
    for name in headers::NOTICE {
        if let Some(value) = map.get(*name) {
            frame(&mut hasher, name.as_bytes());
            frame(&mut hasher, value.as_bytes());
        }
    }
    finish(hasher)
}

/// The size the bus counts for a header map: every name's and value's length.
pub fn header_bytes(map: &BTreeMap<String, String>) -> usize {
    map.iter()
        .map(|(name, value)| name.len() + value.len())
        .sum()
}

/// The provider's view-ready event:
/// `ck.{acct}.event.<provider_module_id>.compaction.view_ready.v1`, id
/// `<session_ref>:<view_version>`, and as the bus digest the view's own
/// digest, so the event is a claim check for exactly one immutable view.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ViewReady {
    pub session_ref: String,
    pub view_version: u64,
    pub covers_through: u64,
    pub digest: String,
}

impl ViewReady {
    pub fn new(
        session_ref: impl Into<String>,
        view_version: u64,
        covers_through: u64,
        digest: impl Into<String>,
    ) -> Self {
        Self {
            session_ref: session_ref.into(),
            view_version,
            covers_through,
            digest: digest.into(),
        }
    }

    /// The event announcing `view`.
    pub fn for_view(view: &super::view::CompactionView) -> Self {
        Self::new(
            view.session_ref.clone(),
            view.view_version,
            view.covers_through.ordinal,
            view.digest.clone(),
        )
    }

    /// The bus message id, `<session_ref>:<view_version>`.
    pub fn message_id(&self) -> String {
        format!("{}:{}", self.session_ref, self.view_version)
    }

    pub fn to_headers(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (headers::SESSION_REF.into(), self.session_ref.clone()),
            (headers::VIEW_VERSION.into(), self.view_version.to_string()),
            (
                headers::COVERS_THROUGH.into(),
                self.covers_through.to_string(),
            ),
            (headers::DIGEST.into(), self.digest.clone()),
        ])
    }

    pub fn from_headers(map: &BTreeMap<String, String>) -> Result<Self, NoticeProblem> {
        let ready = Self {
            session_ref: required(map, headers::SESSION_REF)?.to_owned(),
            view_version: decimal(required(map, headers::VIEW_VERSION)?, headers::VIEW_VERSION)?,
            covers_through: decimal(
                required(map, headers::COVERS_THROUGH)?,
                headers::COVERS_THROUGH,
            )?,
            digest: required(map, headers::DIGEST)?.to_owned(),
        };
        if !is_session_ref(&ready.session_ref) {
            return Err(NoticeProblem::Malformed(headers::SESSION_REF));
        }
        if ready.view_version == 0 {
            return Err(NoticeProblem::Malformed(headers::VIEW_VERSION));
        }
        if !is_sha256_hex(&ready.digest) {
            return Err(NoticeProblem::Malformed(headers::DIGEST));
        }
        Ok(ready)
    }
}
