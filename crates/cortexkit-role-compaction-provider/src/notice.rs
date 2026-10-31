//! The two ck-bus events of this role (CONTRACT.md §5, §6).
//!
//! Both are claim checks: the broker carries a subject, an id, a digest and
//! string headers, never message or view content. An [`AdvanceNotice`] wakes
//! the provider, which then reads the new messages with `session.read`; a
//! [`ViewReady`] tells the runner which view to fetch with `view.get`.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::digest::{finish, frame, is_sha256_hex, parse_decimal};

pub use cortexkit_role_llm_runner::archive_prunes_reasoning::ReasoningPruned;

/// Event name of the runner's advance notice, under the runner's module id.
pub const ADVANCED_EVENT: &str = "compaction.advanced";
/// Event name of the provider's view-ready event, under the provider's
/// module id.
pub const VIEW_READY_EVENT: &str = "compaction.view_ready";
/// The event part of an advance notice's bus message id
/// (`<session_ref>:advanced:<seq>`).
pub const ADVANCED_ID_PART: &str = "advanced";
/// The event part of a view-ready event's bus message id
/// (`<session_ref>:view_ready:<view_version>`).
pub const VIEW_READY_ID_PART: &str = "view_ready";
/// Both events are at version 1: [`event_subject`] ends their subjects in `.v1`.
pub const EVENT_VERSION: u32 = 1;

/// Upper bound on a notice's headers: the sum over every header this contract
/// defines of the name's and the value's length in bytes, the way the bus
/// counts a message's header size. The per-value bounds below keep every
/// well-formed notice under it (at most 3,432 bytes); a runner with more
/// events than fit in `events` splits them across consecutive notices.
pub const MAX_NOTICE_HEADER_BYTES: usize = 4096;
/// Upper bound on one header value, in bytes.
pub const MAX_HEADER_VALUE_BYTES: usize = 1024;
/// Upper bound on the number of tokens in `events`.
pub const MAX_EVENTS: usize = 32;
/// Upper bound on one event token, in bytes.
pub const MAX_EVENT_TOKEN_BYTES: usize = 128;
/// Upper bound on a `cut_id`, in bytes.
pub const MAX_CUT_ID_BYTES: usize = 64;
/// Upper bound on the word that ends a `view_refused`, `view_deferred` or
/// `view_companion` token, in bytes.
pub const MAX_REFUSAL_REASON_BYTES: usize = 32;
/// Upper bound on a lineage id in a header, in bytes.
pub const MAX_LINEAGE_ID_BYTES: usize = 256;
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
    /// The session's current lineage, the one `through_ordinal` counts in.
    /// 1 to 256 bytes of printable ASCII without space; it may contain `:`.
    pub const LINEAGE_ID: &str = "lineage_id";
    /// Optional, on a notice that moves the session to a new lineage:
    /// `<lineage_id>:<through_ordinal>:<through_digest>` naming the old
    /// lineage, the last ordinal whose entry is byte-identical in both, and
    /// that entry's content digest. Parsed from the right, so the old lineage
    /// id may itself contain `:` ([`super::NoticeDescent`]).
    pub const DESCENDS_FROM: &str = "descends_from";
    /// The notice's per-session sequence number, from 1, one higher for every
    /// notice of the session.
    pub const SEQ: &str = "seq";
    /// The newest ordinal written in the session's current lineage when the
    /// notice was recorded.
    pub const THROUGH_ORDINAL: &str = "through_ordinal";
    /// [`crate::digest::content_digest`] of the message at `through_ordinal`.
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
    /// The current lineage's first ordinal (`llm-runner/v1`'s
    /// `session.head.first_ordinal`); optional.
    pub const FIRST_ORDINAL: &str = "first_ordinal";
    /// The session's prune state, `<generation>:<through_ordinal>:<at_ms>`,
    /// exactly as `llm-runner/v1`'s `session.head.reasoning_pruned` reports
    /// it when the notice was recorded; `generation` is at least 1. Optional:
    /// only a runner declaring `archive_prunes_reasoning` sends it, and only
    /// for a session it has pruned ([`super::ReasoningPruned`]).
    pub const REASONING_PRUNED: &str = "reasoning_pruned";
    /// Optional pricing hint for editing already-sent history on the current
    /// route: `suffix` or `full_resend`. Never a correctness signal.
    pub const HISTORY_EDIT_COST: &str = "history_edit_cost";
    /// A view's version.
    pub const VIEW_VERSION: &str = "view_version";
    /// The last ordinal a view covers.
    pub const COVERS_THROUGH: &str = "covers_through";
    /// A view's digest ([`crate::view::CompactionView::compute_digest`]).
    pub const DIGEST: &str = "digest";

    /// The headers of an advance notice, in ascending byte order: the order
    /// the notice digest frames them in.
    pub const NOTICE: &[&str] = &[
        APPLIED_VIEW_VERSION,
        DESCENDS_FROM,
        EST_TOKENS,
        EVENTS,
        FIRST_ORDINAL,
        HISTORY_EDIT_COST,
        LAST_USAGE,
        LINEAGE_ID,
        MODEL,
        REASONING_PRUNED,
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
/// view's cut (CONTRACT.md §9).
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
    /// A truncate mode is unknown or does not apply exactly to the stored part.
    pub const TRUNCATE_KIND: &str = "truncate_kind";
    /// Applying the view would separate a tool call from its result.
    pub const TOOL_PAIR_SPLIT: &str = "tool_pair_split";
    /// Applying the view would break the current model provider's role order.
    pub const ROLE_ORDER: &str = "role_order";
    /// A part edit names a part key the message does not have.
    pub const UNKNOWN_PART: &str = "unknown_part";
    /// The current model provider's rules forbid applying the view now, and
    /// the runner can neither defer it nor apply it with companion edits.
    pub const PROVIDER_CONSTRAINT: &str = "provider_constraint";
    /// The view carries an `apply` mark the runner did not list in Setup's
    /// `view_apply` for the session.
    pub const UNSUPPORTED_APPLY: &str = "unsupported_apply";
    /// The view's ranges do not start at the current lineage's first
    /// ordinal (`llm-runner/v1`'s `session.head.first_ordinal`), or, on a
    /// lineage whose first ordinal is above 0, do not start with a replace
    /// range: a view tiles only the lineage, and everything before it rides
    /// in the view's head.
    pub const RANGES_START: &str = "ranges_start";
    /// A replace range carries content the runner's archive pruning removed
    /// from a message the range stands for (`llm-runner/v1` §9.14.1). The
    /// refusal is permanent for that view. The provider reads `session.head`,
    /// reconciles its held copies as for a new prune generation, and only
    /// then builds another view.
    pub const PRUNED_REASONING_RESTORED: &str =
        cortexkit_role_llm_runner::archive_prunes_reasoning::PRUNED_REASONING_RESTORED;
}

/// Reasons a runner gives in `view_deferred`. Open, like the refusal
/// reasons.
pub mod defer_reasons {
    /// Applying the view now would change bytes before signed reasoning the
    /// model provider requires unchanged; the runner waits for a boundary
    /// where that no longer holds.
    pub const SIGNED_REASONING: &str = "signed_reasoning";
}

/// Edits a runner names in `view_companion`. A companion edit is a change the
/// current model provider requires the runner to make to the request along
/// with the view (for example, removing signed reasoning the view would
/// invalidate); the runner makes that change and no other.
pub mod companion_edits {
    /// Remove the signed reasoning produced after the view's first changed
    /// byte.
    pub const STRIP_SIGNED_REASONING: &str = "strip_signed_reasoning";
}

/// One token of a notice's `events` header.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// `model_change`: the session's model changed.
    ModelChange,
    /// `account_change:<account>`: the session's provider account changed.
    /// The argument is an opaque, stable prompt-cache label, never a secret
    /// or personal identifier (CONTRACT.md §8).
    AccountChange(String),
    /// `surface_change`: the frozen system text or tool surface changed.
    SurfaceChange,
    /// `ttl_expired`: the prompt cache expired. Declared only by a runner that
    /// knows the cache lifetime it actually sent (CONTRACT.md §5).
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
    /// `view_deferred:<view_version>:<reason>`: the runner holds this view
    /// until a step boundary where applying it is safe for the current model
    /// provider, at the latest the first step of the session's next run, and
    /// keeps rendering its current one meanwhile. A view marked `apply:
    /// "next_step"` is deferred only when no companion edit makes it
    /// acceptable to the model provider at that step. `reason` is one of
    /// [`defer_reasons`] or another word of the same form.
    ViewDeferred { view_version: u64, reason: String },
    /// `view_companion:<view_version>:<edit>`: the runner applied this view
    /// together with a companion edit the model provider requires. `edit` is
    /// one of [`companion_edits`] or another word of the same form.
    ViewCompanion { view_version: u64, edit: String },
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
            Self::AccountChange(account) => format!("account_change:{account}"),
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
            Self::ViewDeferred {
                view_version,
                reason,
            } => format!("view_deferred:{view_version}:{reason}"),
            Self::ViewCompanion { view_version, edit } => {
                format!("view_companion:{view_version}:{edit}")
            }
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
            "account_change" if crate::ride::is_account(rest) => {
                Ok(Self::AccountChange(rest.to_owned()))
            }
            "account_change" => Err(malformed()),
            "surface_change" => Ok(Self::SurfaceChange),
            "ttl_expired" => Ok(Self::TtlExpired),
            "revert_below" => parse_decimal(rest)
                .map(Self::RevertBelow)
                .ok_or_else(malformed),
            "view_refused" | "view_deferred" | "view_companion" => {
                let (view_version, word) = rest.split_once(':').ok_or_else(malformed)?;
                let view_version = version(view_version)?;
                let word_ok = !word.is_empty()
                    && word.len() <= MAX_REFUSAL_REASON_BYTES
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
                if !word_ok {
                    return Err(malformed());
                }
                let word = word.to_owned();
                Ok(match name {
                    "view_refused" => Self::ViewRefused {
                        view_version,
                        reason: word,
                    },
                    "view_deferred" => Self::ViewDeferred {
                        view_version,
                        reason: word,
                    },
                    _ => Self::ViewCompanion {
                        view_version,
                        edit: word,
                    },
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

/// Whether `value` can be announced as a lineage id in a header: 1 to
/// [`MAX_LINEAGE_ID_BYTES`] bytes of printable ASCII without space.
pub fn is_header_lineage_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LINEAGE_ID_BYTES
        && value.bytes().all(|b| b.is_ascii_graphic())
}

/// The fork descent a notice declares in its `descends_from` header: the old
/// lineage, the last ordinal whose entry is byte-identical in both lineages,
/// and that entry's content digest ([`crate::digest::content_digest`]).
///
/// The first notice of a lineage a cut started carries one naming the
/// previous lineage at the cut: `through_ordinal` is the cut's
/// `through_ordinal`, one below the new lineage's first ordinal, so a
/// provider that keeps its state reads the new lineage from its first
/// ordinal and nothing before it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NoticeDescent {
    pub lineage_id: String,
    pub through_ordinal: u64,
    pub through_digest: String,
}

impl NoticeDescent {
    pub fn new(
        lineage_id: impl Into<String>,
        through_ordinal: u64,
        through_digest: impl Into<String>,
    ) -> Self {
        Self {
            lineage_id: lineage_id.into(),
            through_ordinal,
            through_digest: through_digest.into(),
        }
    }

    /// The header value, `<lineage_id>:<through_ordinal>:<through_digest>`.
    pub fn encode(&self) -> String {
        format!(
            "{}:{}:{}",
            self.lineage_id, self.through_ordinal, self.through_digest
        )
    }

    /// Parse a header value from the right: the digest after the last `:`,
    /// the ordinal before it, and everything before that as the lineage id,
    /// which may itself contain `:`.
    pub fn parse(value: &str) -> Option<Self> {
        let (rest, digest) = value.rsplit_once(':')?;
        let (lineage_id, ordinal) = rest.rsplit_once(':')?;
        let descent = Self::new(lineage_id, parse_decimal(ordinal)?, digest);
        descent.is_well_formed().then_some(descent)
    }

    fn is_well_formed(&self) -> bool {
        is_header_lineage_id(&self.lineage_id) && is_sha256_hex(&self.through_digest)
    }
}

/// What a provider keeps when a notice moves the session to a new lineage
/// (CONTRACT.md §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescentOutcome {
    /// Keep the state derived from entries through this ordinal (tags,
    /// queues, folds, coverage) and read the new lineage after it.
    KeepThrough(u64),
    /// Discard and re-read the whole new lineage: the resync of CONTRACT.md
    /// §5.
    Resync,
}

/// The provider's decision on a notice whose lineage differs from the one it
/// holds. `held_lineage` is that lineage; `recorded_digest_at(ordinal)` is the
/// content digest the provider recorded for the held lineage's entry at that
/// ordinal. State is kept only when the notice declares descent from the held
/// lineage and the recorded digest at the declared ordinal equals the
/// declared digest. A missing header, another old lineage, an ordinal the
/// provider never recorded or a different digest all resync.
pub fn descent_outcome(
    held_lineage: &str,
    recorded_digest_at: impl Fn(u64) -> Option<String>,
    descent: Option<&NoticeDescent>,
) -> DescentOutcome {
    let Some(descent) = descent else {
        return DescentOutcome::Resync;
    };
    let same_bytes = recorded_digest_at(descent.through_ordinal).as_deref()
        == Some(descent.through_digest.as_str());
    if descent.lineage_id == held_lineage && same_bytes {
        DescentOutcome::KeepThrough(descent.through_ordinal)
    } else {
        DescentOutcome::Resync
    }
}

/// The `reasoning_pruned` header value: `<generation>:<through_ordinal>:<at_ms>`.
pub fn encode_reasoning_pruned(state: &ReasoningPruned) -> String {
    format!(
        "{}:{}:{}",
        state.generation, state.through_ordinal, state.at_ms
    )
}

/// Parse a `reasoning_pruned` header value: three canonical decimals, the
/// first at least 1. `None` for anything else.
pub fn parse_reasoning_pruned(value: &str) -> Option<ReasoningPruned> {
    let mut parts = value.split(':').map(parse_decimal);
    let state = ReasoningPruned::new(parts.next()??, parts.next()??, parts.next()??);
    (parts.next().is_none() && state.valid()).then_some(state)
}

fn is_model(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_MODEL_BYTES
        && value.bytes().all(|b| b.is_ascii_graphic())
}

/// The cost of editing already-sent history on the next call's current route.
/// Unknown values survive a header round trip but supply no pricing hint.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryEditCost {
    /// Only the edited message and its suffix are re-costed; the prompt cache
    /// is keyed on prefix bytes.
    Suffix,
    /// An edit breaks a provider-side response chain and forces full input.
    FullResend,
    /// A future value, treated like an absent hint for pricing.
    Unknown(String),
}

impl HistoryEditCost {
    pub fn parse(value: &str) -> Self {
        match value {
            "suffix" => Self::Suffix,
            "full_resend" => Self::FullResend,
            other => Self::Unknown(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Suffix => "suffix",
            Self::FullResend => "full_resend",
            Self::Unknown(value) => value,
        }
    }
}

/// The runner's advance notice: `ck.{acct}.event.<runner_module_id>.compaction.advanced.v1`,
/// id [`AdvanceNotice::message_id`], digest [`AdvanceNotice::digest`], and
/// these fields as string headers ([`AdvanceNotice::to_headers`]).
///
/// It carries no message content. It only wakes the provider, which reads
/// what is new with `session.read` from its own cursor (CONTRACT.md §5).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdvanceNotice {
    pub session_ref: String,
    pub seq: u64,
    pub lineage_id: String,
    pub through_ordinal: u64,
    pub through_digest: String,
    pub events: Vec<Event>,
    pub est_tokens: u64,
    pub model: String,
    pub window: Option<u64>,
    pub last_usage: Option<LastUsage>,
    pub applied_view_version: Option<u64>,
    pub descends_from: Option<NoticeDescent>,
    /// The lineage's first ordinal, at most one above `through_ordinal`.
    pub first_ordinal: Option<u64>,
    /// The session's prune state when the notice was recorded, as
    /// `session.head` reports it. A convenience copy: `session.head` stays
    /// the authority.
    pub reasoning_pruned: Option<ReasoningPruned>,
    /// Raw pricing hint, including unknown values for digest-preserving round
    /// trips. Use [`Self::history_edit_cost`] to read only known hints.
    pub history_edit_cost: Option<HistoryEditCost>,
}

impl AdvanceNotice {
    pub fn new(
        session_ref: impl Into<String>,
        seq: u64,
        lineage_id: impl Into<String>,
        through_ordinal: u64,
        through_digest: impl Into<String>,
        est_tokens: u64,
        model: impl Into<String>,
    ) -> Self {
        Self {
            session_ref: session_ref.into(),
            seq,
            lineage_id: lineage_id.into(),
            through_ordinal,
            through_digest: through_digest.into(),
            events: Vec::new(),
            est_tokens,
            model: model.into(),
            window: None,
            last_usage: None,
            applied_view_version: None,
            descends_from: None,
            first_ordinal: None,
            reasoning_pruned: None,
            history_edit_cost: None,
        }
    }

    pub fn with_reasoning_pruned(mut self, state: ReasoningPruned) -> Self {
        self.reasoning_pruned = Some(state);
        self
    }

    pub fn with_history_edit_cost(mut self, cost: HistoryEditCost) -> Self {
        self.history_edit_cost = Some(cost);
        self
    }

    /// The next call's known pricing hint. Absent and unknown values both
    /// retain the provider's existing pricing behavior, never correctness.
    pub fn history_edit_cost(&self) -> Option<&HistoryEditCost> {
        self.history_edit_cost
            .as_ref()
            .filter(|cost| matches!(cost, HistoryEditCost::Suffix | HistoryEditCost::FullResend))
    }

    pub fn with_first_ordinal(mut self, first_ordinal: u64) -> Self {
        self.first_ordinal = Some(first_ordinal);
        self
    }

    pub fn with_descends_from(mut self, descent: NoticeDescent) -> Self {
        self.descends_from = Some(descent);
        self
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

    /// The bus message id, `<session_ref>:advanced:<seq>`. The event stream
    /// deduplicates ids across all its subjects, so the id names its event:
    /// without it a notice and a view-ready event with equal numbers would
    /// collide and the second would be dropped. A runner that republishes a
    /// notice after a crash reuses its id and headers, so the bus discards the
    /// duplicate.
    pub fn message_id(&self) -> String {
        format!("{}:{ADVANCED_ID_PART}:{}", self.session_ref, self.seq)
    }

    /// The headers, keyed by name. Optional headers are omitted when absent.
    pub fn to_headers(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert(headers::SESSION_REF.into(), self.session_ref.clone());
        map.insert(headers::SEQ.into(), self.seq.to_string());
        map.insert(headers::LINEAGE_ID.into(), self.lineage_id.clone());
        if let Some(descent) = &self.descends_from {
            map.insert(headers::DESCENDS_FROM.into(), descent.encode());
        }
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
        if let Some(first) = self.first_ordinal {
            map.insert(headers::FIRST_ORDINAL.into(), first.to_string());
        }
        if let Some(state) = &self.reasoning_pruned {
            map.insert(
                headers::REASONING_PRUNED.into(),
                encode_reasoning_pruned(state),
            );
        }
        if let Some(cost) = &self.history_edit_cost {
            map.insert(headers::HISTORY_EDIT_COST.into(), cost.as_str().into());
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
            lineage_id: required(map, headers::LINEAGE_ID)?.to_owned(),
            descends_from: optional(map, headers::DESCENDS_FROM)?
                .map(|value| {
                    NoticeDescent::parse(value)
                        .ok_or(NoticeProblem::Malformed(headers::DESCENDS_FROM))
                })
                .transpose()?,
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
            first_ordinal: optional(map, headers::FIRST_ORDINAL)?
                .map(|value| decimal(value, headers::FIRST_ORDINAL))
                .transpose()?,
            reasoning_pruned: optional(map, headers::REASONING_PRUNED)?
                .map(|value| {
                    parse_reasoning_pruned(value)
                        .ok_or(NoticeProblem::Malformed(headers::REASONING_PRUNED))
                })
                .transpose()?,
            history_edit_cost: optional(map, headers::HISTORY_EDIT_COST)?
                .map(HistoryEditCost::parse),
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
        if !is_header_lineage_id(&self.lineage_id) {
            return Err(NoticeProblem::Malformed(headers::LINEAGE_ID));
        }
        if let Some(descent) = &self.descends_from {
            // A descent names the lineage the session left, never its own.
            if !descent.is_well_formed() || descent.lineage_id == self.lineage_id {
                return Err(NoticeProblem::Malformed(headers::DESCENDS_FROM));
            }
        }
        if !is_sha256_hex(&self.through_digest) {
            return Err(NoticeProblem::Malformed(headers::THROUGH_DIGEST));
        }
        // A lineage starts at most one past its newest message (a cut that
        // retained nothing).
        if self
            .first_ordinal
            .is_some_and(|first| first > self.through_ordinal.saturating_add(1))
        {
            return Err(NoticeProblem::Malformed(headers::FIRST_ORDINAL));
        }
        if !is_model(&self.model) {
            return Err(NoticeProblem::Malformed(headers::MODEL));
        }
        if self.reasoning_pruned.is_some_and(|state| !state.valid()) {
            return Err(NoticeProblem::Malformed(headers::REASONING_PRUNED));
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
/// `<session_ref>:view_ready:<view_version>`, and as the bus digest the view's own
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
    pub fn for_view(view: &crate::view::CompactionView) -> Self {
        Self::new(
            view.session_ref.clone(),
            view.view_version,
            view.covers_through.ordinal,
            view.digest.clone(),
        )
    }

    /// The bus message id, `<session_ref>:view_ready:<view_version>`, which
    /// names its event for the reason given on [`AdvanceNotice::message_id`].
    pub fn message_id(&self) -> String {
        format!(
            "{}:{VIEW_READY_ID_PART}:{}",
            self.session_ref, self.view_version
        )
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
