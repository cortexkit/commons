//! `compaction.setup`: called once per lineage, before the lineage's first
//! model call; a runner may call a cut's Setup as soon as it prepares the
//! view that carries the cut. A cut, a revert or a recovery that
//! starts a new lineage calls it again, and the provider keeps its state for
//! the session across those calls. The daemon-attested runner principal on the
//! incoming route supplies its bare module id, which together with Setup's
//! `harness` and `session` keys provider state and later reverse calls. `project_root` is route metadata only.
//!
//! Setup returns the initial CompactionMessage and, optionally, the ceiling
//! policy. The runner renders `initial` until it applies the session's first
//! view. While Setup's `view_due` is pending it never renders `initial`, and
//! applies that published view first. A provider with no head returns an
//! empty replacement. The
//! runner records the answer before the lineage's first model call, and
//! Setup does not run again within the lineage while the runner holds that
//! record; a runner that no longer
//! holds it (after a restore of its store, say) calls Setup again before its
//! next model call.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Setup permission required before a runner prunes an enrolled session.
pub const ACCEPTS_PRUNED_REASONING: &str = "pruned_reasoning";

/// Setup view feature permitting runner-rendered truncate edits.
pub const VIEW_FEATURE_TRUNCATE: &str = "truncate";
/// Setup view feature permitting ordinary tags on counted `bytes` edits.
pub const VIEW_FEATURE_TAG_BYTES_EDITS: &str = "tag_bytes_edits";

pub use cortexkit_role_llm_runner::archive_prunes_reasoning::ReasoningPruned;
pub use cortexkit_role_llm_runner::tags::SetupCheckpoint;

use crate::{
    ceiling::CeilingPolicy,
    errors::RefuseCode,
    lineage::{DescendsFrom, MessageRef},
    message::CompactionMessage,
};

/// The `compaction.setup` request. Decoded leniently.
///
/// Non-exhaustive so a later optional member is additive: decode one, or
/// build one with [`SetupRequest::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SetupRequest {
    /// The runner's opaque name for the session. If the incoming route binds
    /// a non-empty session that differs, the provider may refuse Setup with
    /// `setup_identity_mismatch`; a shared route has an empty bind session.
    /// `project_root` is not carried here and comes only from the route's bind
    /// metadata, never the provider's per-session state key.
    pub session: String,
    /// The caller-chosen harness in the session identity. The daemon does not
    /// attest this string, and a shared project route may carry a different
    /// bind harness. Required; a request without it does not decode.
    pub harness: String,
    /// This request's id; the answer names it.
    pub request_id: String,
    /// The session's lineage, absent when nothing has been written yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// Optional inherited prefix: the session's first lineage continues
    /// another lineage of the same conversation through an inclusive cut.
    /// Sent only to a provider whose `role.describe` declares
    /// `setup_descends_from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descends_from: Option<DescendsFrom>,
    /// The compaction item's preset, verbatim from the plan. Absent means
    /// the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The compaction item's params, verbatim from the plan.
    #[serde(default)]
    pub params: Map<String, Value>,
    /// The session's composition, verbatim from the plan; opaque.
    pub composition: Map<String, Value>,
    /// The model of the session's first step.
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_limit: Option<u64>,
    /// The newest message already written, if any (an imported session, or
    /// the first user message when the runner writes it before Setup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<MessageRef>,
    /// The runner's clock, in milliseconds since the Unix epoch.
    pub now: u64,
    /// The view `apply` marks the runner honors for this session
    /// (`view::apply_marks`), each listed once. A provider puts on a view
    /// only a mark listed here, and ignores names it does not know. Omitted
    /// when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub view_apply: Vec<String>,
    /// Opt-in view features the runner supports (`truncate`,
    /// `tag_bytes_edits`). Providers ignore tokens they do not know; decoding
    /// keeps them as sent. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub view_features: Vec<String>,
    /// The longest this runner can hold a step for a rewrite ride's answer
    /// (cause `ttl_expired`, `model_change`, `account_change` or
    /// `surface_change`), in milliseconds, from 1 to
    /// [`crate::ride::MAX_RIDE_WAIT_MS`]: its
    /// host's real limit. Absent means the runner does not honour a
    /// provider-set ride wait, and the provider sends no `ride_wait_ms`.
    /// Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ride_wait_max_ms: Option<u64>,
    /// The longest the runner holds each model step for a pending due view,
    /// in milliseconds from that step's start; at least 1. Absent means
    /// the provider must not send `view_due`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_due_max_ms: Option<u64>,
    /// The session's prune state, exactly as `llm-runner/v1`'s
    /// `session.head` reports it, on every Setup a runner declaring
    /// `archive_prunes_reasoning` calls for a session it has pruned. Absent
    /// when the session was never pruned. A provider that accepted pruned
    /// reasoning reconciles its held copies here when the generation
    /// differs from the one it last reconciled (CONTRACT.md §4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_pruned: Option<ReasoningPruned>,
}

impl SetupRequest {
    pub fn new(
        session: impl Into<String>,
        harness: impl Into<String>,
        request_id: impl Into<String>,
        composition: Map<String, Value>,
        model: impl Into<String>,
        now: u64,
    ) -> Self {
        Self {
            session: session.into(),
            harness: harness.into(),
            request_id: request_id.into(),
            lineage_id: None,
            descends_from: None,
            preset: None,
            params: Map::new(),
            composition,
            model: model.into(),
            variant: None,
            context_window: None,
            output_limit: None,
            newest: None,
            now,
            view_apply: Vec::new(),
            view_features: Vec::new(),
            ride_wait_max_ms: None,
            view_due_max_ms: None,
            reasoning_pruned: None,
        }
    }

    /// Set the plan item's preset and params.
    pub fn with_item(mut self, preset: Option<String>, params: Map<String, Value>) -> Self {
        self.preset = preset;
        self.params = params;
        self
    }

    pub fn with_lineage(mut self, lineage_id: impl Into<String>) -> Self {
        self.lineage_id = Some(lineage_id.into());
        self
    }

    pub fn with_descends_from(mut self, descends_from: DescendsFrom) -> Self {
        self.descends_from = Some(descends_from);
        self
    }

    pub fn with_model_details(
        mut self,
        variant: Option<String>,
        context_window: Option<u64>,
        output_limit: Option<u64>,
    ) -> Self {
        self.variant = variant;
        self.context_window = context_window;
        self.output_limit = output_limit;
        self
    }

    pub fn with_newest(mut self, newest: MessageRef) -> Self {
        self.newest = Some(newest);
        self
    }

    /// Set the view `apply` marks the runner honors for this session.
    pub fn with_view_apply(mut self, view_apply: Vec<String>) -> Self {
        self.view_apply = view_apply;
        self
    }

    pub fn with_view_features(mut self, features: Vec<String>) -> Self {
        self.view_features = features;
        self
    }

    pub fn view_features(&self) -> &[String] {
        &self.view_features
    }

    /// Declare the longest this runner can hold a step for a rewrite
    /// ride's answer.
    pub fn with_ride_wait_max_ms(mut self, cap: u64) -> Self {
        self.ride_wait_max_ms = Some(cap);
        self
    }

    /// Declare the longest the runner holds a step for Setup's due view.
    pub fn with_view_due_max_ms(mut self, cap: u64) -> Self {
        self.view_due_max_ms = Some(cap);
        self
    }

    pub fn view_due_max_ms(&self) -> Option<u64> {
        self.view_due_max_ms
    }

    /// Carry the session's prune state, as `session.head` reports it.
    pub fn with_reasoning_pruned(mut self, state: ReasoningPruned) -> Self {
        self.reasoning_pruned = Some(state);
        self
    }
}

/// The `ready` answer's member a runner names when it treats the answer as
/// a failed Setup over it ([`SetupAnswer::check_for`]): a `ride_wait_ms`
/// that is 0, above the `ride_wait_max_ms` its request carried, or present
/// when its request carried none.
pub const RIDE_WAIT_MS: &str = "ride_wait_ms";

/// The reason a runner records when a Setup answer carries `checkpoint` for
/// a request with `descends_from`: it treats the answer as a failed Setup
/// (no model call, run error `compaction_unavailable` with this `reason`),
/// because a lineage that descends from another continues that lineage's
/// numbering and never takes a checkpoint.
pub const CHECKPOINT_ON_DESCENT: &str = "checkpoint_on_descent";

/// The reason recorded when a pending due view does not apply: its fetch is
/// refused, its disposal is anything but `applied`, or the wait expires. The
/// run then ends `error` with `compaction_unavailable` and no model call.
pub const VIEW_DUE_UNAPPLIED: &str = "view_due_unapplied";

/// The Setup member named when a due view is invalid or was not requested.
pub const VIEW_DUE: &str = "view_due";

/// An already-published view that must be applied before any model call in
/// the lineage. Its digest is the view's own CONTRACT.md §6 digest.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ViewDue {
    /// A published version, at least 1, served by `view.get`.
    pub version: u64,
    /// Lowercase hexadecimal SHA-256 of the view.
    pub digest: String,
}

impl ViewDue {
    pub fn new(version: u64, digest: impl Into<String>) -> Self {
        Self {
            version,
            digest: digest.into(),
        }
    }
}

/// The `compaction.setup` answer. Strict on `answer`: a runner treats an
/// answer it cannot decode as a failed Setup.
///
/// The enum and both variants are non-exhaustive, so a later optional
/// member is not a breaking change. Build an answer with
/// [`SetupAnswer::ready`] or [`SetupAnswer::refuse`] and the `with_*`
/// setters, and read it with the accessors; outside this crate a `match`
/// keeps a `_` arm and a variant pattern keeps `..`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
#[non_exhaustive]
// Keep the public Ready fields unboxed: boxing the extended checkpoint
// would break callers of the existing public Rust API in a compatible release.
#[allow(clippy::large_enum_variant)]
pub enum SetupAnswer {
    /// The session's initial view and ceiling policy.
    #[non_exhaustive]
    Ready {
        request_id: String,
        /// The initial CompactionMessage. Its range is usually empty at the
        /// start of the lineage, `{from: f, to: f}` with `f` the lineage's
        /// first ordinal: the head goes before every message of the lineage,
        /// and on a lineage a cut started it carries what the cut folded. The runner renders it until it applies the
        /// session's first view, unless `view_due` is present: then it never
        /// renders `initial` while that view is due.
        initial: CompactionMessage,
        /// When the runner pauses for a view and, failing one, refuses the
        /// run (CONTRACT.md §8). Absent, the runner never pauses for the
        /// provider but still never sends over the window.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ceiling: Option<CeilingPolicy>,
        /// For a session moved from a standalone compaction tool: the
        /// permanent checkpoint at the first retained message `cut`.
        /// `message_count` and `head_cut` are sent together only to runners
        /// declaring `message_checkpoint`; they certify the message boundary
        /// separately from tag fallback (`llm-runner/v1` §9.6). Absent for
        /// a session that was never moved, and on descending Setup answers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint: Option<SetupCheckpoint>,
        /// The prompt cache's lifetime for Setup's model (CONTRACT.md
        /// §8.1). Omitted when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_ttl: Option<crate::cache_ttl::CacheTtl>,
        /// How long the runner holds a step for a rewrite ride's answer,
        /// in milliseconds: at least 1, and at most the `ride_wait_max_ms`
        /// of the request (CONTRACT.md §4, §8). Sent only when the request
        /// carried `ride_wait_max_ms`, and frozen with the session like
        /// `ceiling` (§12). Omitted when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ride_wait_ms: Option<u64>,
        /// An already-published view to apply before the first model call.
        /// Sent only when the request carried `view_due_max_ms`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view_due: Option<ViewDue>,
        /// Read transformations the provider accepts for this enrollment.
        /// Empty means no opt-in; unknown names grant no known permission.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        accepts: Vec<String>,
    },
    /// Setup failed. No model call is made, and the run ends `error` with
    /// the role's `code` and, when present, the provider's own
    /// `provider_code` beside it. The next send runs Setup again. A
    /// `retryable` member an older provider still sends is ignored: `code`
    /// fixes retryability.
    #[non_exhaustive]
    Refuse {
        request_id: String,
        code: RefuseCode,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_code: Option<String>,
    },
}

impl SetupAnswer {
    /// Opt a ready answer into read transformations. A refusal is unchanged.
    pub fn with_accepts(mut self, values: Vec<String>) -> Self {
        if let Self::Ready { accepts, .. } = &mut self {
            *accepts = values;
        }
        self
    }

    /// Accepted read transformations, or an empty slice for a refusal.
    pub fn accepts(&self) -> &[String] {
        match self {
            Self::Ready { accepts, .. } => accepts,
            Self::Refuse { .. } => &[],
        }
    }

    /// A `ready` answer with the session's initial CompactionMessage and no
    /// optional members.
    pub fn ready(request_id: impl Into<String>, initial: CompactionMessage) -> Self {
        Self::Ready {
            request_id: request_id.into(),
            initial,
            ceiling: None,
            checkpoint: None,
            cache_ttl: None,
            ride_wait_ms: None,
            view_due: None,
            accepts: Vec::new(),
        }
    }

    /// A `refuse` answer with no `provider_code`.
    pub fn refuse(
        request_id: impl Into<String>,
        code: RefuseCode,
        reason: impl Into<String>,
    ) -> Self {
        Self::Refuse {
            request_id: request_id.into(),
            code,
            reason: reason.into(),
            provider_code: None,
        }
    }

    /// A `ready` answer with `ceiling` set. A `refuse` has no ceiling and
    /// is returned unchanged.
    pub fn with_ceiling(mut self, policy: CeilingPolicy) -> Self {
        if let Self::Ready { ceiling, .. } = &mut self {
            *ceiling = Some(policy);
        }
        self
    }

    /// A `ready` answer with `checkpoint` set. A `refuse` has no checkpoint
    /// and is returned unchanged.
    pub fn with_checkpoint(mut self, value: SetupCheckpoint) -> Self {
        if let Self::Ready { checkpoint, .. } = &mut self {
            *checkpoint = Some(value);
        }
        self
    }

    /// A `ready` answer with `cache_ttl` set. A `refuse` has no cache
    /// lifetime and is returned unchanged.
    pub fn with_cache_ttl(mut self, entry: crate::cache_ttl::CacheTtl) -> Self {
        if let Self::Ready { cache_ttl, .. } = &mut self {
            *cache_ttl = Some(entry);
        }
        self
    }

    /// A `ready` answer with `ride_wait_ms` set. A `refuse` has no ride wait
    /// and is returned unchanged.
    pub fn with_ride_wait_ms(mut self, wait: u64) -> Self {
        if let Self::Ready { ride_wait_ms, .. } = &mut self {
            *ride_wait_ms = Some(wait);
        }
        self
    }

    /// Set the due view on a ready answer. A refusal is unchanged.
    pub fn with_view_due(mut self, due: ViewDue) -> Self {
        if let Self::Ready { view_due, .. } = &mut self {
            *view_due = Some(due);
        }
        self
    }

    pub fn view_due(&self) -> Option<&ViewDue> {
        match self {
            Self::Ready { view_due, .. } => view_due.as_ref(),
            Self::Refuse { .. } => None,
        }
    }

    /// A `refuse` answer with the provider's own `provider_code` set. A
    /// `ready` answer has none and is returned unchanged.
    pub fn with_provider_code(mut self, value: impl Into<String>) -> Self {
        if let Self::Refuse { provider_code, .. } = &mut self {
            *provider_code = Some(value.into());
        }
        self
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::Ready { request_id, .. } | Self::Refuse { request_id, .. } => request_id,
        }
    }

    /// Whether this is a `ready` answer.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// The initial CompactionMessage of a `ready` answer.
    pub fn initial(&self) -> Option<&CompactionMessage> {
        match self {
            Self::Ready { initial, .. } => Some(initial),
            Self::Refuse { .. } => None,
        }
    }

    /// The ceiling policy a `ready` answer carries, if any.
    pub fn ceiling(&self) -> Option<&CeilingPolicy> {
        match self {
            Self::Ready { ceiling, .. } => ceiling.as_ref(),
            Self::Refuse { .. } => None,
        }
    }

    /// The prompt cache's lifetime a `ready` answer carries, if any.
    pub fn cache_ttl(&self) -> Option<&crate::cache_ttl::CacheTtl> {
        match self {
            Self::Ready { cache_ttl, .. } => cache_ttl.as_ref(),
            Self::Refuse { .. } => None,
        }
    }

    /// The ride wait a `ready` answer carries, if any.
    pub fn ride_wait_ms(&self) -> Option<u64> {
        match self {
            Self::Ready { ride_wait_ms, .. } => *ride_wait_ms,
            Self::Refuse { .. } => None,
        }
    }

    /// The role code of a `refuse` answer.
    pub fn refuse_code(&self) -> Option<&RefuseCode> {
        match self {
            Self::Refuse { code, .. } => Some(code),
            Self::Ready { .. } => None,
        }
    }

    /// The user-facing reason of a `refuse` answer.
    pub fn refuse_reason(&self) -> Option<&str> {
        match self {
            Self::Refuse { reason, .. } => Some(reason),
            Self::Ready { .. } => None,
        }
    }

    /// The provider's own finer code on a `refuse` answer, if any.
    pub fn provider_code(&self) -> Option<&str> {
        match self {
            Self::Refuse { provider_code, .. } => provider_code.as_deref(),
            Self::Ready { .. } => None,
        }
    }

    /// The Setup checkpoint a `ready` answer carries, if any.
    pub fn checkpoint(&self) -> Option<&SetupCheckpoint> {
        match self {
            Self::Ready { checkpoint, .. } => checkpoint.as_ref(),
            Self::Refuse { .. } => None,
        }
    }

    /// The runner's check of a `ready` answer against the request it
    /// answers. Each `Err` names what the runner records for a failed
    /// Setup (no model call, run error `compaction_unavailable`):
    /// [`CHECKPOINT_ON_DESCENT`] for a `checkpoint` on a Setup with
    /// `descends_from`, and [`RIDE_WAIT_MS`] for a `ride_wait_ms` that is 0,
    /// above the request's `ride_wait_max_ms`, or present though the
    /// request carried no `ride_wait_max_ms`. A runner never clamps the
    /// wait. [`VIEW_DUE`] names a due view without a positive request cap,
    /// with version 0, or without a lowercase hexadecimal SHA-256 digest.
    /// A `refuse` passes, as it ends the run on its own.
    pub fn check_for(&self, request: &SetupRequest) -> Result<(), &'static str> {
        if request.descends_from.is_some() && self.checkpoint().is_some() {
            return Err(CHECKPOINT_ON_DESCENT);
        }
        if let Some(wait) = self.ride_wait_ms() {
            // Above the role's own bound fails even under a larger cap: a ride
            // deadline over it is refused, so such a wait could never be honoured.
            if wait == 0
                || wait > crate::ride::MAX_RIDE_WAIT_MS
                || request.ride_wait_max_ms.is_none_or(|cap| wait > cap)
            {
                return Err(RIDE_WAIT_MS);
            }
        }
        if let Some(due) = self.view_due() {
            if request.view_due_max_ms.is_none_or(|cap| cap == 0)
                || due.version == 0
                || !crate::digest::is_sha256_hex(&due.digest)
            {
                return Err(VIEW_DUE);
            }
        }
        Ok(())
    }

    /// Whether a `refuse` may be retried; `None` for `ready`. Its
    /// retryability is fixed by the code: `true` means the runner may
    /// retry without the user acting.
    pub fn retryable(&self) -> Option<bool> {
        match self {
            Self::Refuse { code, .. } => Some(code.retryable()),
            Self::Ready { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn setup_accepts_pruned_reasoning_is_optional_and_open() {
        let raw: serde_json::Value =
            serde_json::from_str(crate::vectors::file("setup-accepts.json").unwrap()).unwrap();
        let answer: super::SetupAnswer = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(answer.accepts(), &[super::ACCEPTS_PRUNED_REASONING]);
        assert_eq!(serde_json::to_value(&answer).unwrap(), raw);
        let empty = answer.clone().with_accepts(vec![]);
        let encoded = serde_json::to_value(&empty).unwrap();
        assert!(encoded.get("accepts").is_none());
        assert!(serde_json::from_value::<super::SetupAnswer>(encoded)
            .unwrap()
            .accepts()
            .is_empty());
        assert_eq!(
            answer.with_accepts(vec!["future".into()]).accepts(),
            &["future"]
        );
    }
    use super::*;
    use crate::vectors;

    #[test]
    fn setup_view_due_round_trip_and_absence_preserve_wire_bytes() {
        let file = vectors::load("setup-view-due.json");
        for case in vectors::cases(&file, "cases") {
            let name = case["name"].as_str().unwrap();
            let request: SetupRequest = vectors::round_trip(name, &case["request"]);
            let answer: SetupAnswer = vectors::round_trip(name, &case["answer"]);
            assert_eq!(answer.check_for(&request), Ok(()), "{name}");
            if let Some(due) = answer.view_due() {
                assert_eq!(request.view_due_max_ms(), Some(1000));
                assert_eq!(due, &ViewDue::new(1, "a".repeat(64)));
                assert_eq!(
                    SetupAnswer::ready(answer.request_id(), answer.initial().unwrap().clone())
                        .with_view_due(due.clone()),
                    answer
                );
            } else {
                let request = SetupRequest::new("s", "test", "setup", Map::new(), "m", 1);
                assert_eq!(
                    serde_json::to_string(&request).unwrap(),
                    r#"{"session":"s","harness":"test","request_id":"setup","params":{},"composition":{},"model":"m","now":1}"#
                );
                assert_eq!(
                    serde_json::to_string(&answer).unwrap(),
                    r#"{"answer":"ready","request_id":"setup","initial":{"compaction_id":"c","version":0,"range":{"from":0,"to":0},"replacement":[]}}"#
                );
            }
        }
    }

    #[test]
    fn invalid_or_unrequested_view_due_fails_setup_by_name() {
        let request = SetupRequest::new("s", "test", "setup", Map::new(), "m", 1);
        let initial = CompactionMessage::new("c", 0, crate::message::Range::new(0, 0), vec![]);
        let answer = SetupAnswer::ready("setup", initial);
        let valid = ViewDue::new(1, "a".repeat(64));
        assert_eq!(
            answer
                .clone()
                .with_view_due(valid.clone())
                .check_for(&request),
            Err(VIEW_DUE)
        );
        let zero_cap = request.clone().with_view_due_max_ms(0);
        assert_eq!(
            answer
                .clone()
                .with_view_due(valid.clone())
                .check_for(&zero_cap),
            Err(VIEW_DUE)
        );
        let request = request.with_view_due_max_ms(1);
        assert_eq!(
            answer.clone().with_view_due(valid).check_for(&request),
            Ok(())
        );
        for due in [
            ViewDue::new(0, "a".repeat(64)),
            ViewDue::new(1, ""),
            ViewDue::new(1, "a".repeat(63)),
            ViewDue::new(1, "a".repeat(65)),
            ViewDue::new(1, "A".repeat(64)),
            ViewDue::new(1, "g".repeat(64)),
            ViewDue::new(1, "é".repeat(32)),
        ] {
            assert_eq!(
                answer.clone().with_view_due(due).check_for(&request),
                Err(VIEW_DUE)
            );
        }
        let refused = SetupAnswer::refuse("setup", RefuseCode::Misconfigured, "unavailable");
        assert_eq!(refused.clone().with_view_due(ViewDue::new(0, "")), refused);
        assert!(refused.view_due().is_none());
        assert_eq!(refused.check_for(&request), Ok(()));
    }

    /// The constructors, setters and accessors are the only way to build or
    /// read an answer outside this crate, so each vector answer must come
    /// back equal when rebuilt through them.
    #[test]
    fn the_builders_rebuild_every_vector_answer() {
        let setup = vectors::load("setup.json");
        let with_cache_ttl = vectors::load("async.json");
        let cases = vectors::cases(&setup, "answers")
            .iter()
            .chain(vectors::cases(
                &with_cache_ttl["cache_ttl"],
                "setup_answers",
            ));
        let mut cache_ttl_seen = false;
        let mut ride_wait_seen = false;
        for case in cases {
            let answer: SetupAnswer = serde_json::from_value(case["answer"].clone()).unwrap();
            cache_ttl_seen |= answer.cache_ttl().is_some();
            ride_wait_seen |= answer.ride_wait_ms().is_some();
            let rebuilt = if answer.is_ready() {
                let mut built =
                    SetupAnswer::ready(answer.request_id(), answer.initial().unwrap().clone());
                if let Some(policy) = answer.ceiling() {
                    built = built.with_ceiling(policy.clone());
                }
                if let Some(checkpoint) = answer.checkpoint() {
                    built = built.with_checkpoint(checkpoint.clone());
                }
                if let Some(entry) = answer.cache_ttl() {
                    built = built.with_cache_ttl(entry.clone());
                }
                if let Some(wait) = answer.ride_wait_ms() {
                    built = built.with_ride_wait_ms(wait);
                }
                built
            } else {
                let mut built = SetupAnswer::refuse(
                    answer.request_id(),
                    answer.refuse_code().unwrap().clone(),
                    answer.refuse_reason().unwrap(),
                );
                if let Some(code) = answer.provider_code() {
                    built = built.with_provider_code(code);
                }
                built
            };
            assert_eq!(rebuilt, answer, "{}", case["name"]);
        }
        assert!(cache_ttl_seen, "no vector answer carries cache_ttl");
        assert!(ride_wait_seen, "no vector answer carries ride_wait_ms");
    }

    /// A runner accepts a `ride_wait_ms` from 1 to the cap its request
    /// carried, and treats any other one as a failed Setup naming
    /// `ride_wait_ms`; a request that carries no cap accepts none.
    #[test]
    fn a_ride_wait_outside_the_runners_cap_is_a_failed_setup() {
        let file = vectors::load("setup.json");
        for case in vectors::cases(&file, "ride_wait") {
            let name = case["name"].as_str().unwrap();
            let request: SetupRequest = vectors::round_trip(name, &case["request"]);
            let answer: SetupAnswer = vectors::round_trip(name, &case["answer"]);
            let expected = case["failed_setup"].as_str();
            assert_eq!(answer.check_for(&request).err(), expected, "{name}");
        }
        let capped = SetupRequest::new("s", "claude-code", "setup", Map::new(), "m", 1)
            .with_ride_wait_max_ms(240_000);
        let initial = CompactionMessage::new("c", 1, crate::message::Range::new(0, 0), Vec::new());
        for (wait, outcome) in [
            (1, Ok(())),
            (240_000, Ok(())),
            (240_001, Err(RIDE_WAIT_MS)),
            (0, Err(RIDE_WAIT_MS)),
        ] {
            let answer = SetupAnswer::ready("setup", initial.clone()).with_ride_wait_ms(wait);
            assert_eq!(answer.check_for(&capped), outcome, "{wait}");
        }
        let over_bound = SetupRequest::new("s", "broca", "setup", Map::new(), "m", 1)
            .with_ride_wait_max_ms(crate::ride::MAX_RIDE_WAIT_MS + 100_000);
        let answer = SetupAnswer::ready("setup", initial.clone())
            .with_ride_wait_ms(crate::ride::MAX_RIDE_WAIT_MS + 1);
        assert_eq!(answer.check_for(&over_bound), Err(RIDE_WAIT_MS));
        let uncapped = SetupRequest::new("s", "opencode", "setup", Map::new(), "m", 1);
        let answer = SetupAnswer::ready("setup", initial.clone()).with_ride_wait_ms(1);
        assert_eq!(answer.check_for(&uncapped), Err(RIDE_WAIT_MS));
        assert_eq!(
            SetupAnswer::ready("setup", initial).check_for(&uncapped),
            Ok(())
        );
    }

    #[test]
    fn a_checkpoint_on_a_descending_setup_is_a_failed_setup() {
        let file = vectors::load("setup.json");
        let moved = vectors::cases(&file, "answers")
            .iter()
            .map(|case| serde_json::from_value::<SetupAnswer>(case["answer"].clone()).unwrap())
            .find(|answer| answer.checkpoint().is_some())
            .expect("a moved session's answer carries a checkpoint");
        let plain = SetupRequest::new("s", "opencode", "setup", Map::new(), "m", 1);
        assert_eq!(moved.check_for(&plain), Ok(()));
        let descending = plain
            .clone()
            .with_lineage("lin-2")
            .with_descends_from(DescendsFrom::new("lin-1", 1));
        assert_eq!(moved.check_for(&descending), Err(CHECKPOINT_ON_DESCENT));
        let refused: crate::errors::RefuseCode =
            serde_json::from_value(serde_json::json!(crate::errors::UNKNOWN_CUT)).unwrap();
        assert!(!refused.retryable());
    }

    #[test]
    fn setup_descent_round_trips_without_dropping_the_field() {
        let file = vectors::load("setup-descent.json");
        let case = &vectors::cases(&file, "requests")[1];
        let request =
            vectors::round_trip::<SetupRequest>(case["name"].as_str().unwrap(), &case["request"]);
        assert_eq!(request.descends_from, Some(DescendsFrom::new("parent", 2)));
        let built = SetupRequest::new("s", "opencode", "setup", Map::new(), "m", 1)
            .with_lineage("child")
            .with_descends_from(DescendsFrom::new("parent", 2));
        assert_eq!(built, request);
    }

    #[test]
    fn setup_descent_absence_and_lenient_decode_are_unchanged() {
        let file = vectors::load("setup-descent.json");
        let case = &vectors::cases(&file, "requests")[0];
        let request =
            vectors::round_trip::<SetupRequest>(case["name"].as_str().unwrap(), &case["request"]);
        assert_eq!(request.descends_from, None);
        let mut extended = case["request"].clone();
        extended["future_extension"] = serde_json::json!({"unknown": true});
        assert_eq!(
            serde_json::from_value::<SetupRequest>(extended).unwrap(),
            request
        );
        assert_eq!(
            SetupRequest::new("s", "opencode", "setup", Map::new(), "m", 1).descends_from,
            None
        );
    }

    #[test]
    fn setup_vectors_round_trip() {
        let file = vectors::load("setup.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<SetupRequest>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            let answer = vectors::round_trip::<SetupAnswer>(name, &case["answer"]);
            assert_eq!(answer.request_id(), case["answer"]["request_id"]);
            if let Some(retryable) = case.get("retryable") {
                assert_eq!(answer.retryable(), retryable.as_bool(), "{name}");
            }
            if let SetupAnswer::Ready { initial, .. } = &answer {
                assert_eq!(initial.check(), Ok(()), "{name}");
            }
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            let answer: SetupAnswer = serde_json::from_value(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(answer.retryable(), case["retryable"].as_bool(), "{name}");
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<SetupAnswer>(case["name"].as_str().unwrap(), &case["answer"]);
        }
    }

    /// A moved session's Setup answer carries its checkpoint, which round
    /// trips as the runner role's own type; an answer without one decodes
    /// as before.
    #[test]
    fn a_moved_session_carries_its_setup_checkpoint() {
        let file = vectors::load("setup.json");
        let moved = vectors::cases(&file, "answers")
            .iter()
            .find(|case| case["answer"].get("checkpoint").is_some())
            .expect("a vector for a moved session");
        let answer: SetupAnswer = vectors::round_trip("moved", &moved["answer"]);
        assert_eq!(
            answer.checkpoint(),
            Some(&SetupCheckpoint::new(340, "msg_boundary"))
        );
        let plain = vectors::cases(&file, "answers")
            .iter()
            .find(|case| {
                case["answer"]["answer"] == "ready" && case["answer"].get("checkpoint").is_none()
            })
            .unwrap();
        let answer: SetupAnswer = serde_json::from_value(plain["answer"].clone()).unwrap();
        assert_eq!(answer.checkpoint(), None);
    }

    #[test]
    fn builders_match_decoding() {
        let mut params = Map::new();
        params.insert("budget".into(), Value::from(3));
        let request = SetupRequest::new("s", "broca", "r0", Map::new(), "m", 1)
            .with_item(Some("head".into()), params);
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<SetupRequest>(encoded).unwrap(),
            request
        );
    }

    #[test]
    fn every_provider_refuse_answer_round_trips() {
        for code in crate::errors::refuse_codes::ALL {
            for provider_code in [None, Some("mc:historian_offline")] {
                let mut answer = serde_json::json!({
                    "answer": "refuse", "request_id": "r0", "code": code,
                    "reason": "cannot compact"
                });
                if let Some(own) = provider_code {
                    answer["provider_code"] = Value::from(own);
                }
                let setup = vectors::round_trip::<SetupAnswer>(code, &answer);
                let fixed = crate::errors::refuse_codes::retryable(code);
                assert_eq!(setup.retryable(), Some(fixed), "{code}");
            }
        }
    }

    /// An older provider still sends `retryable`. It decodes, the member is
    /// dropped, and it never decides retry: `code` alone does.
    #[test]
    fn a_stale_retryable_member_is_ignored() {
        for (code, sent, fixed) in [
            ("misconfigured", true, false),
            ("provider_busy", false, true),
            ("acme_own_code", true, false),
        ] {
            let answer = serde_json::json!({
                "answer": "refuse", "request_id": "r0", "code": code,
                "reason": "x", "retryable": sent
            });
            let setup: SetupAnswer = serde_json::from_value(answer.clone()).unwrap();
            assert_eq!(setup.retryable(), Some(fixed), "{code}");
            assert!(serde_json::to_value(&setup)
                .unwrap()
                .get("retryable")
                .is_none());
        }
    }
}
