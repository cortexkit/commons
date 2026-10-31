//! Error codes a runner answers with, carried as the `code` of an `ERROR`
//! frame's body (`{code, message, detail?}`), and the reasons a runner
//! writes into a tool result or a run's terminal.
//!
//! Codes are open strings on the wire: a consumer that meets a code it does
//! not know treats it as a terminal refusal of that one request. Only the
//! codes [`is_retryable`] names are retried with the same request; every
//! other refusal is an answer, and retrying it unchanged meets the same
//! refusal.
//!
//! This crate carries no error-body type of its own. The helpers here take
//! the body's `code` and `detail` as found in whatever frame type the
//! caller's transport decodes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Named runner-side run error classes. Classes remain open strings.
pub mod run_error_classes {
    /// A required refresh could not start the session host with its new plan.
    pub const HOST_RELAUNCH_FAILED: &str = "host_relaunch_failed";
}

/// A request field is malformed, unknown, or combined with a field it
/// excludes. `detail.field` names it.
pub const INVALID_PARAMS: &str = "invalid_params";

/// An output-reference id or cursor is unknown on the route's session.
/// Foreign-session ids are deliberately indistinguishable from missing ones.
pub const UNKNOWN_REF: &str = "unknown_ref";

pub fn is_unknown_ref(code: &str) -> bool {
    code == UNKNOWN_REF
}

macro_rules! attachment_codes {
    ($($name:ident, $predicate:ident, $value:literal;)+) => {
        $(pub const $name: &str = $value;
        pub fn $predicate(code: &str) -> bool { code == $name })+
    };
}

attachment_codes! {
    UNKNOWN_SESSION, is_unknown_session, "unknown_session";
    UNKNOWN_UPLOAD, is_unknown_upload, "unknown_upload";
    UPLOAD_NOT_COMMITTED, is_upload_not_committed, "upload_not_committed";
    UPLOAD_GONE, is_upload_gone, "upload_gone";
    UPLOAD_CHUNK_TOO_LARGE, is_upload_chunk_too_large, "upload_chunk_too_large";
    UPLOAD_OFFSET_MISMATCH, is_upload_offset_mismatch, "upload_offset_mismatch";
    UPLOAD_SIZE_MISMATCH, is_upload_size_mismatch, "upload_size_mismatch";
    UPLOAD_SHA256_MISMATCH, is_upload_sha256_mismatch, "upload_sha256_mismatch";
    MEDIA_UNSUPPORTED, is_media_unsupported, "media_unsupported";
    UNKNOWN_MEDIA, is_unknown_media, "unknown_media";
}

/// A contiguous append retry resumes at the runner's acknowledged byte count.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct UploadOffsetMismatchDetail {
    pub received: u64,
}

impl UploadOffsetMismatchDetail {
    pub fn new(received: u64) -> Self {
        Self { received }
    }
    pub fn received(&self) -> u64 {
        self.received
    }
}

/// Two providers declare the same command. Detail: `{name, providers}`.
pub const USER_COMMAND_COLLISION: &str = "user_command_collision";
/// The requested command was not made available at admission. Detail: `{command}`.
pub const UNKNOWN_USER_COMMAND: &str = "unknown_user_command";
/// The session holds no invocation under that call key.
pub const UNKNOWN_INVOCATION: &str = "unknown_invocation";
/// The session's plan declares `interactive: false`.
pub const NO_PERSON: &str = "no_person";
/// The runner does not declare the `user_commands` group.
pub const USER_COMMANDS_UNSUPPORTED: &str = "user_commands_unsupported";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserCommandCollisionDetail {
    pub name: String,
    pub providers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UnknownUserCommandDetail {
    pub command: String,
}

/// A session input-token cap stopped a tool call before dispatch. Not a
/// retryable request refusal; the run ends in the state of the same name.
pub const BUDGET_EXHAUSTED: &str = "budget_exhausted";

/// A read's `lineage_id` is not the session's current lineage. The caller
/// re-reads from the tail; it never retries the same cursor.
pub const LINEAGE_CHANGED: &str = "lineage_changed";

/// A read would serve a message whose body, rebuilt from the host's store,
/// no longer matches the digest it was first served with (CONTRACT §4.2.1).
/// The runner serves no changed bytes under the lineage. It serves that
/// message again only under a new lineage, so the reader re-reads from the
/// tail. `detail` is [`ServedBytesChangedDetail`].
pub const SERVED_BYTES_CHANGED: &str = "served_bytes_changed";

/// The detail of a `served_bytes_changed` refusal: the first message whose
/// rebuilt body differs. No content or digest is carried.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ServedBytesChangedDetail {
    pub mid: String,
}

impl ServedBytesChangedDetail {
    pub fn new(mid: impl Into<String>) -> Self {
        Self { mid: mid.into() }
    }
}

/// A read's `after_mid` names no message in the lineage the read named, or
/// a `session.history` request's `from_mid` or `before_mid` names no
/// message of the session's history.
pub const UNKNOWN_MID: &str = "unknown_mid";

/// A range read, an after-message read or a `session.resolve` names a
/// position below the current lineage's first ordinal: history before the
/// session's latest cut, which `session.read` and `session.resolve` never
/// serve. `detail` is [`BeforeLineageStartDetail`]. Terminal: the reader
/// reads from `first_ordinal`, or reads the earlier history with
/// `session.history` on a runner that declares `host_history`.
pub const BEFORE_LINEAGE_START: &str = "before_lineage_start";

/// The detail of a `before_lineage_start` refusal: the current lineage's
/// first ordinal, as `session.head` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct BeforeLineageStartDetail {
    pub first_ordinal: u64,
}

impl BeforeLineageStartDetail {
    pub fn new(first_ordinal: u64) -> Self {
        Self { first_ordinal }
    }
}

/// A `session.history` cursor no longer names a place in the session's
/// history: a revert or a rewrite removed or changed the entry it was
/// minted next to, or an entry older than it, or the cursor was minted for
/// another session. Never answered with an empty page instead. Terminal:
/// the reader pages again from the newest page or from a `mid` it holds. A
/// cursor that does not parse is `invalid_params` naming `cursor` instead.
pub const CURSOR_EXPIRED: &str = "cursor_expired";

/// A `session.history` `from_ordinal` or `before_ordinal` names an ordinal
/// the runner never assigned, or one a revert removed. Terminal: the reader
/// seeks to an ordinal it read, or pages by cursor.
pub const UNKNOWN_ORDINAL: &str = "unknown_ordinal";

/// The complete encoded `session.read` reply exceeds the transport frame
/// limit. Terminal: retrying with a smaller `max_bytes` cannot shrink a
/// single message or atomic model-view group. `detail` is
/// [`ReplyTooLargeDetail`].
pub const REPLY_TOO_LARGE: &str = "reply_too_large";

/// Safe identity and size details for a `reply_too_large` refusal. The
/// subject names a message or a model-view group without carrying its content.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReplyTooLargeDetail {
    /// Length of the complete encoded reply frame, including its header.
    pub bytes: u64,
    /// Maximum encoded frame length accepted by the transport:
    /// [`crate::TRANSPORT_FRAME_LIMIT`].
    pub limit: u64,
    pub subject: ReplyTooLargeSubject,
}

impl ReplyTooLargeDetail {
    pub fn new(bytes: u64, limit: u64, subject: ReplyTooLargeSubject) -> Self {
        Self {
            bytes,
            limit,
            subject,
        }
    }
}

/// The indivisible item that made a `session.read` reply too large.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReplyTooLargeSubject {
    /// A raw transcript message, identified by its session and ordinal.
    Message { session: String, ordinal: u64 },
    /// An atomic model-view group, identified by the existing source of its
    /// anchored entry.
    ModelGroup { source: crate::read::EntrySource },
    /// A `run.result` or `run.status` answer for this run, made too large by
    /// its structured-output `value`, which is never cut short.
    Run { run_id: String },
    /// One entry of a run's `run.tool_calls` list, identified by its run and
    /// the model's id for the call (with its `call_key` when it has one,
    /// since a model id need not be unique), made too large by its `args`,
    /// which are never cut short.
    ToolCall {
        run_id: String,
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_key: Option<String>,
    },
}

/// `run.result` or `run.tool_calls` names a run the session does not have.
pub const UNKNOWN_RUN: &str = "unknown_run";

/// A `session.resolve` names a well-formed tag that no part of the session's
/// current lineage carries: one no part has shown in this lineage, or one
/// that named a part only under a lineage the session has since left.
/// Numbers may repeat across lineages, but resolve answers only the current
/// one, so within it the same request meets the same refusal: terminal. A
/// malformed tag is `invalid_params` naming `tag` instead.
pub const UNKNOWN_TAG: &str = "unknown_tag";

/// A `session.send` or `session.adopt` for a session the runner knows was
/// forked or cloned from a managed session (one whose plan names a
/// compaction provider). Such a session is never enrolled; the person uses
/// undo instead. `detail` is [`ForkUnsupportedDetail`]. Terminal, writing
/// nothing. A runner refuses only forks it can know about: a fork that
/// carries nothing of the runner's (say, of a session that never compacted)
/// is an ordinary session.
pub const FORK_UNSUPPORTED: &str = "fork_unsupported";

/// The detail of a `fork_unsupported` refusal: the session it was forked
/// from, when the runner knows it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ForkUnsupportedDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

impl ForkUnsupportedDetail {
    pub fn new(parent: Option<String>) -> Self {
        Self { parent }
    }
}

/// A template send names, in a `part_tag` slot, a part the request that
/// would carry it does not show (a compaction dropped it from the view, or
/// a revert removed it). The send has no text, so it is never delivered.
/// `detail` is [`TemplatePartGoneDetail`]. Terminal: the sender recomposes
/// with a part it can see. A runner refuses a send at admission with it
/// when the part is already gone; a queued send whose part goes before it
/// is delivered ends in the terminal send state of the same name
/// (`crate::send::states::TEMPLATE_PART_GONE`), as an expired one ends
/// `expired`.
pub const TEMPLATE_PART_GONE: &str = "template_part_gone";

/// The detail of a `template_part_gone` refusal: the template slot whose
/// part is gone.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct TemplatePartGoneDetail {
    pub slot: String,
}

impl TemplatePartGoneDetail {
    pub fn new(slot: impl Into<String>) -> Self {
        Self { slot: slot.into() }
    }
}

/// A `send.status` names a `send_id` the session has no record of for the
/// caller. Terminal: `send.status` writes nothing, so repeating the same
/// request cannot create the record and meets the same refusal. A malformed
/// request is `invalid_params` instead.
pub const UNKNOWN_SEND: &str = "unknown_send";

/// The session's context is at or over its ceiling, so the runner will not
/// send the request. It refuses a `session.send` that would start or join a
/// run, writing nothing; a run already in progress that reaches the ceiling
/// at a step boundary ends in the run state of the same name instead
/// (`crate::run::states::CONTEXT_CEILING`). `detail` is
/// [`ContextCeilingDetail`].
///
/// It is not a model or provider failure: a caller that routes between
/// models must not charge backoff to the model or mark it unhealthy for it.
/// A provider's own overflow refusal is different: that run ends `error`
/// with the provider code `context_overflow`.
pub const CONTEXT_CEILING: &str = "context_ceiling";

/// Why the runner stopped at the ceiling: the `cause` of a
/// [`ContextCeilingDetail`]. Decoded as an open string; a consumer treats a
/// cause it does not know as a ceiling stop all the same.
pub mod context_ceiling_causes {
    /// The runner waited the compaction provider's bound at the ceiling and
    /// no view came.
    pub const VIEW_TIMEOUT: &str = "view_timeout";
    /// The session has no compaction provider (none at setup), or its
    /// provider is unreachable at the ceiling.
    pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";
    /// The request cannot fit the window even with the current view applied.
    pub const OVER_WINDOW: &str = "over_window";
    /// Every cause this role documents.
    pub const ALL: &[&str] = &[VIEW_TIMEOUT, PROVIDER_UNAVAILABLE, OVER_WINDOW];
}

/// The detail of a `context_ceiling` refusal, and of a run that ended in
/// the `context_ceiling` state: sizes in tokens, and why the runner stopped.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ContextCeilingDetail {
    /// The model's context window, in tokens.
    pub window: u64,
    /// The runner's estimate of the request's size, in tokens, when it
    /// stopped.
    pub size: u64,
    /// One of [`context_ceiling_causes`]; a cause this crate does not name
    /// is kept as the string received.
    pub cause: String,
}

impl ContextCeilingDetail {
    pub fn new(window: u64, size: u64, cause: impl Into<String>) -> Self {
        Self {
            window,
            size,
            cause: cause.into(),
        }
    }
}

/// The session's transcript has expired. Terminal, never retried. The
/// detail is [`ExpiredDetail`], not an empty never-written-session reply.
pub const EXPIRED: &str = "expired";

/// Detail of the named `expired` answer to a read of an expired session.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ExpiredDetail {
    /// Wall-clock expiry in milliseconds since the Unix epoch, not deletion time.
    pub expired_at_ms: u64,
}

impl ExpiredDetail {
    pub fn new(expired_at_ms: u64) -> Self {
        Self { expired_at_ms }
    }
}

/// Detail of the `invalid_params` refusal a runner returns when the `retention` field of a
/// `session.send` request is out of range.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RetentionInvalidDetail {
    pub field: String,
    /// Present for an above-maximum request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_seconds: Option<u64>,
}

impl RetentionInvalidDetail {
    pub fn new() -> Self {
        Self {
            field: "retention".into(),
            max_seconds: None,
        }
    }
    pub fn with_max_seconds(mut self, max_seconds: u64) -> Self {
        self.max_seconds = Some(max_seconds);
        self
    }
}

impl Default for RetentionInvalidDetail {
    fn default() -> Self {
        Self::new()
    }
}

/// A send reused a `send_id` with a different payload or delivery mode, or
/// a `session.adopt` reused one with another plan. `detail.field` names the
/// first field that differs, for example `delivery` or `plan`.
pub const SEND_ID_REUSE: &str = "send_id_reuse";

/// A retry of a durably retracted send. Nothing is written and it never starts.
pub const SEND_RETRACTED: &str = "send_retracted";

/// A send whose delivery mode (`queue` when absent) the runner does not
/// declare as a capability group. `detail.delivery` names the mode. Nothing
/// was written, and the send is never delivered in another mode instead.
pub const DELIVERY_UNSUPPORTED: &str = "delivery_unsupported";

/// A send into a session whose last run is paused, other than the paused
/// run's own retry or an interrupt. Nothing was written.
pub const RUN_PAUSED: &str = "run_paused";

/// The caller is not the identity allowed to drive the session: a send, a
/// `session.baseline` request or a `session.adopt`, from anyone but the
/// session's owner.
pub const SCOPE_OWNER_MISMATCH: &str = "scope_owner_mismatch";

/// A scoped send into a session recorded without a scope that cannot adopt
/// the stamped one. `detail` is `no_recorded_principal` or
/// `principal_mismatch {recorded}`.
pub const SCOPE_ADOPTION_REFUSED: &str = "scope_adoption_refused";

/// A send under a scope on a daemon that lacks scopes. Nothing was written.
pub const SCOPE_UNSUPPORTED: &str = "scope_unsupported";

/// Admission could not fetch a required plan item within its deadline.
/// `detail.provider` names it. Nothing was written. Retryable.
pub const FETCH_UNAVAILABLE: &str = "fetch_unavailable";

/// A preset-less system-text item's provider lacks `system-text-default/v1`.
/// `detail.provider` names the module id. Terminal, never timed; nothing is
/// written or forwarded.
pub const SYSTEM_TEXT_PRESET_REQUIRED: &str = "system_text_preset_required";

/// The provider module id whose system-text item still requires a preset.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SystemTextPresetRequiredDetail {
    pub provider: String,
}

impl SystemTextPresetRequiredDetail {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
        }
    }
}

/// Admission found fetched items that disagree with the plan's composition.
/// `detail.differences` names them. Nothing was written. The starter
/// preflights again and retries once with a new plan, never the same one.
pub const PLAN_STALE: &str = "plan_stale";

/// Admission found tools with the same model-facing name.
/// `detail` is [`ToolNameCollisionDetail`]. Nothing was written.
pub const TOOL_NAME_COLLISION: &str = "tool_name_collision";

/// A later send carried a plan that differs from the session's frozen plan,
/// or a `session.adopt` carried a plan other than the one the runner applied
/// when the host started the session. `detail` is [`PlanDriftDetail`].
/// Nothing was applied. Terminal: never retried with the same request.
pub const PLAN_DRIFT: &str = "plan_drift";

/// The identities of the frozen and sent plans in a `plan_drift` refusal.
/// Identities only, never plan content.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PlanDriftDetail {
    /// Absent when the session's first episode had no plan, or, on
    /// `session.adopt`, when the runner applied no plan at host start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<String>,
    pub sent: String,
}

impl PlanDriftDetail {
    pub fn new(frozen: Option<String>, sent: impl Into<String>) -> Self {
        Self {
            frozen,
            sent: sent.into(),
        }
    }
}

/// The model-facing name and every provider that offered it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolNameCollisionDetail {
    pub name: String,
    pub providers: Vec<String>,
}

/// A steered or queued send whose PreUser hook was unavailable under
/// `on_unavailable: refuse`. Nothing was written.
pub const PRE_USER_UNAVAILABLE: &str = "pre_user_unavailable";

/// The runner could not answer now for a reason that may clear by itself
/// (a provider or store briefly unavailable). Retryable.
pub const TRANSIENT: &str = "transient";

/// The scope's owner has not yet synced the scope after a daemon restart.
/// Retryable, with backoff, within the caller's hold bound.
pub const SCOPE_NOT_SYNCED: &str = "scope_not_synced";

/// A `session.refresh` or `session.refresh_policy` names a policy whose rung
/// the session does not support for a surface the change touches. Refused
/// when the policy is set; nothing was written. `detail` is
/// `session_change::RungUnsupportedDetail`.
pub const RUNG_UNSUPPORTED: &str = "rung_unsupported";

/// A `session.refresh_policy` names a generation that is no longer the
/// pending one, because the runner has since accepted a later refresh.
/// Nothing was written. `detail` is
/// `session_change::PendingSupersededDetail`.
pub const PENDING_SUPERSEDED: &str = "pending_superseded";

/// A `session.refresh_policy` names a generation a prefix rebuild or an
/// append has already applied, so there is no pending change left for the
/// policy to govern. Nothing was written. `detail` is
/// `session_change::AlreadyAppliedDetail`.
pub const ALREADY_APPLIED: &str = "already_applied";

/// A `session.refresh` names a generation at or below the highest one the
/// runner accepted, and is not an identical repeat of an accepted refresh
/// (the same generation, plan identity and policy). `detail` is
/// `session_change::GenerationStaleDetail`. Nothing was written.
pub const GENERATION_STALE: &str = "generation_stale";

/// A `session.refresh` plan names a tool outside the tool set the host fixed
/// when it started (OpenCode registers each tool's name, description and
/// input schema then). A tool is inside only with an identical definition, so
/// a changed description or schema under a kept name is outside too.
/// `detail` is `session_change::ToolOutsideHostSetDetail`. Terminal; nothing
/// was written.
pub const TOOL_OUTSIDE_HOST_SET: &str = "tool_outside_host_set";

/// The refusals only the `session_change` group's ops answer. A refresh can
/// also meet admission's `fetch_unavailable`, `plan_stale` and
/// `tool_name_collision`, and a `send_id` reused with another body meets
/// `send_id_reuse`.
pub const SESSION_CHANGE_CODES: &[&str] = &[
    RUNG_UNSUPPORTED,
    PENDING_SUPERSEDED,
    ALREADY_APPLIED,
    GENERATION_STALE,
    TOOL_OUTSIDE_HOST_SET,
];

/// A first send or adopt omitted a required session policy. Nothing was written.
pub const RETENTION_REQUIRED: &str = "retention_required";
/// A later policy differs from the starter's frozen policy; detail carries `frozen`.
pub const RETENTION_CONFLICT: &str = "retention_conflict";
/// Release is pending. Retryable after deletion, when a send starts a new incarnation.
pub const SESSION_RELEASED: &str = "session_released";
/// Single-session backup is unavailable; detail carries `reason`.
pub const BACKUP_UNSUPPORTED: &str = "backup_unsupported";

/// Every refusal code this role defines.
pub const CODES: &[&str] = &[
    RETENTION_REQUIRED,
    RETENTION_CONFLICT,
    SESSION_RELEASED,
    BACKUP_UNSUPPORTED,
    UNKNOWN_REF,
    UNKNOWN_SESSION,
    UNKNOWN_UPLOAD,
    UPLOAD_NOT_COMMITTED,
    UPLOAD_GONE,
    UPLOAD_CHUNK_TOO_LARGE,
    UPLOAD_OFFSET_MISMATCH,
    UPLOAD_SIZE_MISMATCH,
    UPLOAD_SHA256_MISMATCH,
    MEDIA_UNSUPPORTED,
    UNKNOWN_MEDIA,
    USER_COMMAND_COLLISION,
    UNKNOWN_USER_COMMAND,
    UNKNOWN_INVOCATION,
    NO_PERSON,
    USER_COMMANDS_UNSUPPORTED,
    INVALID_PARAMS,
    LINEAGE_CHANGED,
    SERVED_BYTES_CHANGED,
    UNKNOWN_MID,
    BEFORE_LINEAGE_START,
    CURSOR_EXPIRED,
    UNKNOWN_ORDINAL,
    REPLY_TOO_LARGE,
    UNKNOWN_RUN,
    UNKNOWN_TAG,
    FORK_UNSUPPORTED,
    TEMPLATE_PART_GONE,
    UNKNOWN_SEND,
    CONTEXT_CEILING,
    EXPIRED,
    SEND_ID_REUSE,
    SEND_RETRACTED,
    DELIVERY_UNSUPPORTED,
    RUN_PAUSED,
    SCOPE_OWNER_MISMATCH,
    SCOPE_ADOPTION_REFUSED,
    SCOPE_UNSUPPORTED,
    FETCH_UNAVAILABLE,
    SYSTEM_TEXT_PRESET_REQUIRED,
    PLAN_STALE,
    TOOL_NAME_COLLISION,
    PLAN_DRIFT,
    PRE_USER_UNAVAILABLE,
    TRANSIENT,
    SCOPE_NOT_SYNCED,
    RUNG_UNSUPPORTED,
    PENDING_SUPERSEDED,
    ALREADY_APPLIED,
    GENERATION_STALE,
    TOOL_OUTSIDE_HOST_SET,
];

/// Whether a runner's refusal is retryable with the same request. Route
/// refusals the daemon itself classifies as retryable on open (module
/// reloading or warming, target unavailable) are the transport's to judge
/// and are not listed here.
pub fn is_retryable(code: &str) -> bool {
    matches!(
        code,
        TRANSIENT | FETCH_UNAVAILABLE | SCOPE_NOT_SYNCED | SESSION_RELEASED
    )
}

/// The field an `invalid_params` or `send_id_reuse` refusal names, or `None`
/// for any other code or a detail without `field`.
pub fn refused_field<'a>(code: &str, detail: Option<&'a Value>) -> Option<&'a str> {
    if code != INVALID_PARAMS && code != SEND_ID_REUSE {
        return None;
    }
    detail?.get("field")?.as_str()
}

/// Reasons a runner writes on a tool call's error result. Where the reason
/// sits inside a message body is the runner's message schema.
pub mod tool_result_reasons {
    /// The session input-token cap stopped the call before dispatch.
    pub const BUDGET_EXHAUSTED: &str = super::BUDGET_EXHAUSTED;
    /// The call shared a step with a `submit_result` call, which ended the
    /// run before any call of that step was dispatched.
    pub const SKIPPED_AFTER_SUBMIT: &str = "skipped_after_submit";
    /// The session's context reached its ceiling at a step boundary, which
    /// ended the run `context_ceiling` before the call was dispatched.
    pub const CONTEXT_CEILING: &str = super::CONTEXT_CEILING;
    /// The call had a durable dispatch intent and no result when the runner
    /// restarted: it may or may not have run. Never re-dispatched.
    pub const OUTCOME_UNKNOWN: &str = "outcome_unknown";
    /// A PreTool validate hook denied the call.
    pub const PRE_TOOL_DENIED: &str = "pre_tool_denied";
    /// A PreTool hook under `on_unavailable: refuse` was unavailable.
    pub const PRE_TOOL_UNAVAILABLE: &str = "pre_tool_unavailable";
    /// A human declined the call at the approve phase.
    pub const PRE_TOOL_DECLINED: &str = "pre_tool_declined";
    /// The approve-phase question expired under `on_expiry: deny`.
    pub const PRE_TOOL_EXPIRED: &str = "pre_tool_expired";
    /// The call executed, and a PostTool hook under `on_unavailable: refuse`
    /// was unavailable, so its output was withheld.
    pub const POST_TOOL_UNAVAILABLE: &str = "post_tool_unavailable";
    /// The call was never sent: its route closed under a drained scope.
    pub const ROUTE_DRAINED: &str = "route_drained";
}

/// `provider_code` values a runner writes on a run that ends `error`
/// because of a provider the session depends on.
pub mod provider_codes {
    /// Setup only: failed or timed out with no answer, so no initial view
    /// was recorded and no model call was made.
    pub const COMPACTION_UNAVAILABLE: &str = "compaction_unavailable";
    /// A PreUser hook under `on_unavailable: refuse` was unavailable for a
    /// user turn.
    pub const PRE_USER_UNAVAILABLE: &str = "pre_user_unavailable";

    /// The request exceeds the model's context window.
    pub const CONTEXT_OVERFLOW: &str = "context_overflow";
    /// The provider's output-token ceiling prevents a complete response.
    pub const OUTPUT_CEILING: &str = "output_ceiling";
    /// The provider's response deadline ended the request.
    pub const RESPONSE_DEADLINE: &str = "response_deadline";
    /// The account's usage limit is exhausted.
    pub const ACCOUNT_LIMIT_EXHAUSTED: &str = "account_limit_exhausted";
    /// The account's billing or spending allowance is exhausted.
    pub const BILLING_EXHAUSTED: &str = "billing_exhausted";
    /// The provider served an account other than the run's frozen account.
    pub const ACCOUNT_CHANGED: &str = "account_changed";
    /// A provider content-policy refusal.
    pub const CYBER_POLICY: &str = "cyber_policy";
    /// A provider content-policy refusal.
    pub const BIO_POLICY: &str = "bio_policy";
    /// A provider content-policy refusal.
    pub const MISALIGNMENT_POLICY_VIOLATION: &str = "misalignment_policy_violation";
    /// The normalized safety finish, a provider policy refusal.
    pub const CONTENT_FILTER: &str = "content_filter";
    /// Access terminated: the seat cannot run.
    pub const ACCESS_TERMINATED_ERROR: &str = "access_terminated_error";
    /// Read-only legacy alias. Runners must emit `billing_exhausted` instead.
    pub const INSUFFICIENT_QUOTA: &str = "insufficient_quota";

    /// Normalize recorded legacy codes for consumer decisions, preserving
    /// unknown codes verbatim. This does not rewrite stored run errors.
    pub fn normalize(code: &str) -> &str {
        if code == INSUFFICIENT_QUOTA {
            BILLING_EXHAUSTED
        } else {
            code
        }
    }

    /// Every provider code this role names; providers may define others.
    pub const CODES: &[&str] = &[
        COMPACTION_UNAVAILABLE,
        PRE_USER_UNAVAILABLE,
        CONTEXT_OVERFLOW,
        OUTPUT_CEILING,
        RESPONSE_DEADLINE,
        ACCOUNT_LIMIT_EXHAUSTED,
        BILLING_EXHAUSTED,
        ACCOUNT_CHANGED,
        CYBER_POLICY,
        BIO_POLICY,
        MISALIGNMENT_POLICY_VIOLATION,
        CONTENT_FILTER,
        ACCESS_TERMINATED_ERROR,
        INSUFFICIENT_QUOTA,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn upstream_plan_drift_bytes_round_trip() {
        for name in [
            "refuse-later-send-with-different-plan.json",
            "refuse-later-plan-after-planless-first-episode.json",
        ] {
            let file = vectors::load(name);
            assert_eq!(file["expected"]["code"], PLAN_DRIFT);
            assert!(!is_retryable(PLAN_DRIFT));
            let detail: PlanDriftDetail = vectors::exact_object_round_trip(name, "\"detail\": ", 4);
            assert_eq!(
                detail.frozen.is_some(),
                name == "refuse-later-send-with-different-plan.json"
            );
        }
    }

    #[test]
    fn upstream_tool_name_collision_bytes_round_trip() {
        let name = "refuse-tool-name-collision.json";
        let file = vectors::load(name);
        assert_eq!(file["expected"]["code"], TOOL_NAME_COLLISION);
        let detail: ToolNameCollisionDetail =
            vectors::exact_object_round_trip(name, "\"detail\": ", 4);
        assert_eq!(detail.name, "web_search");
        assert_eq!(detail.providers, ["plexus", "prefrontal-core"]);
    }

    #[test]
    fn newly_pinned_provider_codes_and_legacy_alias_are_named() {
        for code in [
            "context_overflow",
            "output_ceiling",
            "response_deadline",
            "account_limit_exhausted",
            "billing_exhausted",
            "account_changed",
            "cyber_policy",
            "bio_policy",
            "misalignment_policy_violation",
            "content_filter",
            "access_terminated_error",
            "insufficient_quota",
        ] {
            assert!(provider_codes::CODES.contains(&code), "{code}");
        }
        assert_eq!(
            provider_codes::normalize("insufficient_quota"),
            "billing_exhausted"
        );
        assert_eq!(
            provider_codes::normalize("billing_exhausted"),
            "billing_exhausted"
        );
        assert_eq!(provider_codes::normalize("future-code"), "future-code");
        assert!(CODES.contains(&"send_retracted"));
        assert!(!is_retryable("send_retracted"));
    }

    #[test]
    fn reply_too_large_is_named_terminal_and_carries_only_safe_details() {
        assert!(CODES.contains(&REPLY_TOO_LARGE));
        assert!(!is_retryable(REPLY_TOO_LARGE));

        let file = vectors::load("reply-too-large.json");
        for case in vectors::cases(&file, "refusals") {
            let refusal = &case["refusal"];
            assert_eq!(refusal["code"], REPLY_TOO_LARGE);
            let detail: ReplyTooLargeDetail =
                serde_json::from_value(refusal["detail"].clone()).unwrap();
            assert_eq!(detail.limit, crate::TRANSPORT_FRAME_LIMIT as u64);
            assert!(detail.bytes > detail.limit);
            assert_eq!(
                serde_json::to_value(detail).unwrap(),
                refusal["detail"],
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn lineage_bound_and_cursor_refusals_are_named_and_terminal() {
        for code in [BEFORE_LINEAGE_START, CURSOR_EXPIRED, UNKNOWN_ORDINAL] {
            assert!(CODES.contains(&code), "{code}");
            assert!(!is_retryable(code), "{code}");
        }
        let file = vectors::load("errors.json");
        let case = vectors::cases(&file, "codes")
            .iter()
            .find(|case| case["code"] == BEFORE_LINEAGE_START)
            .unwrap();
        let detail: BeforeLineageStartDetail =
            serde_json::from_value(case["detail"].clone()).unwrap();
        assert_eq!(detail, BeforeLineageStartDetail::new(42));
        assert_eq!(serde_json::to_value(detail).unwrap(), case["detail"]);
    }

    #[test]
    fn retryability_matches_the_vectors() {
        let file = vectors::load("errors.json");
        let output_refs = vectors::load("output-refs.json");
        let system_text_default = vectors::load("system-text-default.json");
        let session_retention = vectors::load("session-retention.json");
        let cases: Vec<_> = vectors::cases(&file, "codes")
            .iter()
            .chain(vectors::cases(&output_refs, "codes"))
            .chain(vectors::cases(&system_text_default, "codes"))
            .chain(vectors::cases(&session_retention, "codes"))
            .collect();
        let listed: Vec<&str> = cases
            .iter()
            .map(|case| case["code"].as_str().unwrap())
            .collect();
        for code in CODES {
            assert!(listed.contains(code), "{code} has no vector");
        }
        for case in cases {
            let code = case["code"].as_str().unwrap();
            assert_eq!(
                is_retryable(code),
                case["retryable"].as_bool().unwrap(),
                "{code}"
            );
        }
    }

    #[test]
    fn compaction_unavailable_decodes_and_is_listed_as_a_provider_code() {
        let file = vectors::load("run-result.json");
        let case = vectors::cases(&file, "answers")
            .iter()
            .find(|case| case["name"] == "Setup unavailable with no answer")
            .unwrap();
        let result: crate::run::RunResult =
            vectors::round_trip("compaction unavailable", &case["answer"]);
        let error = result.error.unwrap();
        assert_eq!(error.provider_code, provider_codes::COMPACTION_UNAVAILABLE);
        assert!(provider_codes::CODES.contains(&"compaction_unavailable"));
        assert!(!CODES.contains(&"compaction_unavailable"));
    }

    #[test]
    fn refused_field_reads_only_field_naming_codes() {
        let detail = serde_json::json!({"field": "delivery"});
        assert_eq!(
            refused_field(SEND_ID_REUSE, Some(&detail)),
            Some("delivery")
        );
        assert_eq!(
            refused_field(INVALID_PARAMS, Some(&detail)),
            Some("delivery")
        );
        assert_eq!(refused_field(UNKNOWN_MID, Some(&detail)), None);
        assert_eq!(refused_field(INVALID_PARAMS, None), None);
    }
}

#[cfg(test)]
mod context_ceiling_tests {
    use super::*;
    use crate::{
        run::{RunResult, RunState, RunStatus},
        vectors,
    };

    #[test]
    fn context_ceiling_is_a_terminal_refusal_and_a_non_error_run_state() {
        assert!(CODES.contains(&CONTEXT_CEILING));
        assert!(!is_retryable(CONTEXT_CEILING));
        assert!(!provider_codes::CODES.contains(&CONTEXT_CEILING));
        assert_eq!(tool_result_reasons::CONTEXT_CEILING, CONTEXT_CEILING);
        let file = vectors::load("context-ceiling.json");
        let mut causes = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            assert_eq!(case["refusal"]["code"], CONTEXT_CEILING, "{name}");
            assert_eq!(case["writes"], "nothing", "{name}");
            let detail: ContextCeilingDetail =
                vectors::round_trip(name, &case["refusal"]["detail"]);
            causes.insert(detail.cause);
        }
        for cause in context_ceiling_causes::ALL {
            assert!(causes.contains(*cause), "no refusal vector for {cause}");
        }
        for case in vectors::cases(&file, "results") {
            let name = case["name"].as_str().unwrap();
            let (state, detail) = match case["kind"].as_str().unwrap() {
                "run.result" => {
                    let result: RunResult = vectors::round_trip(name, &case["answer"]);
                    assert!(result.error.is_none(), "{name}");
                    (result.run_state(), result.context_ceiling)
                }
                _ => {
                    let status: RunStatus = vectors::round_trip(name, &case["answer"]);
                    assert!(status.error.is_none(), "{name}");
                    (status.run_state(), status.context_ceiling)
                }
            };
            assert!(state.is_context_ceiling() && state.is_terminal(), "{name}");
            assert!(!state.is_budget_exhausted(), "{name}");
            assert!(detail.is_some(), "{name}");
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            let detail: ContextCeilingDetail = serde_json::from_value(case["detail"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(detail.cause, case["detail"]["cause"], "{name}");
        }
        // While held at the ceiling the run is active and waiting, with a
        // deadline, on run.status, session.head and the stream.
        let waiting = &file["waiting"];
        let status: RunStatus = vectors::round_trip("waiting status", &waiting["status"]);
        assert_eq!(status.run_state(), RunState::Active);
        assert_eq!(
            status.reason,
            Some(crate::run::RunReason::ContextCeilingWait)
        );
        assert_eq!(
            status,
            RunStatus::context_ceiling_wait(status.deadline_ms.expect("deadline_ms"))
        );
        let head: crate::read::HeadMeta = vectors::round_trip("waiting head", &waiting["head"]);
        let last = head.last_run_state.unwrap();
        assert_eq!(
            last.reason.as_deref(),
            Some(crate::RUN_REASON_CONTEXT_CEILING_WAIT)
        );
        assert!(!RunState::parse(&last.state).is_terminal());
        let event: crate::subscribe::SubscribeEvent =
            serde_json::from_value(waiting["event"].clone()).unwrap();
        assert_eq!(event.kind, crate::subscribe::kinds::CONTROL);
        assert_eq!(
            waiting["event"]["reason"],
            crate::RUN_REASON_CONTEXT_CEILING_WAIT
        );
        assert_eq!(
            waiting["event"]["deadline_ms"],
            waiting["status"]["deadline_ms"]
        );
        for end in vectors::cases(waiting, "ends") {
            let name = end["name"].as_str().unwrap();
            let status: RunStatus = vectors::round_trip(name, &end["status"]);
            assert!(status.deadline_ms.is_none(), "{name}");
            assert_ne!(
                status.reason,
                Some(crate::run::RunReason::ContextCeilingWait),
                "{name}"
            );
        }
        // A step held for a rewrite ride's answer reads the same way, under
        // its own reason, with the ride's deadline.
        let ride = &file["compaction_waiting"];
        let status: RunStatus = vectors::round_trip("ride status", &ride["status"]);
        let deadline = status.deadline_ms.expect("deadline_ms");
        assert_eq!(status, RunStatus::compaction_wait(deadline));
        assert_eq!(status.runner_wait_until(), Some(deadline));
        let head: crate::read::HeadMeta = vectors::round_trip("ride head", &ride["head"]);
        assert_eq!(
            head.last_run_state.unwrap().reason.as_deref(),
            Some(crate::RUN_REASON_COMPACTION_WAIT)
        );
        assert_eq!(ride["event"]["reason"], crate::RUN_REASON_COMPACTION_WAIT);
        assert_eq!(ride["event"]["deadline_ms"], ride["status"]["deadline_ms"]);
        for end in vectors::cases(ride, "ends") {
            let name = end["name"].as_str().unwrap();
            let status: RunStatus = vectors::round_trip(name, &end["status"]);
            assert_eq!(status.runner_wait_until(), None, "{name}");
        }
        // A provider's own overflow refusal stays a provider failure.
        let overflow: RunResult =
            vectors::round_trip("provider overflow", &file["provider_overflow"]["answer"]);
        assert_eq!(overflow.run_state(), RunState::Error);
        assert!(overflow.context_ceiling.is_none());
        assert_eq!(
            overflow.error.unwrap().provider_code,
            provider_codes::CONTEXT_OVERFLOW
        );
        assert_eq!(
            ContextCeilingDetail::new(1, 2, context_ceiling_causes::OVER_WINDOW),
            serde_json::from_value(serde_json::json!({"window":1,"size":2,"cause":"over_window"}))
                .unwrap()
        );
    }
}

#[cfg(test)]
mod tag_code_tests {
    use super::*;

    #[test]
    fn unknown_tag_is_named_and_terminal() {
        assert!(CODES.contains(&UNKNOWN_TAG));
        assert!(!is_retryable(UNKNOWN_TAG));
    }

    #[test]
    fn unknown_send_is_named_and_terminal() {
        assert!(CODES.contains(&UNKNOWN_SEND));
        assert!(!is_retryable(UNKNOWN_SEND));
    }

    #[test]
    fn fork_and_gone_part_refusals_are_named_terminal_and_carry_their_detail() {
        for code in [FORK_UNSUPPORTED, TEMPLATE_PART_GONE] {
            assert!(CODES.contains(&code), "{code}");
            assert!(!is_retryable(code), "{code}");
        }
        let file = crate::vectors::load("errors.json");
        let detail = |code: &str| {
            crate::vectors::cases(&file, "codes")
                .iter()
                .find(|case| case["code"] == code)
                .and_then(|case| case.get("detail").cloned())
                .unwrap_or_else(|| panic!("{code} has no detail vector"))
        };
        let fork: ForkUnsupportedDetail =
            crate::vectors::round_trip(FORK_UNSUPPORTED, &detail(FORK_UNSUPPORTED));
        assert_eq!(fork, ForkUnsupportedDetail::new(Some("ses_parent".into())));
        assert_eq!(
            serde_json::to_value(ForkUnsupportedDetail::new(None)).unwrap(),
            serde_json::json!({})
        );
        let gone: TemplatePartGoneDetail =
            crate::vectors::round_trip(TEMPLATE_PART_GONE, &detail(TEMPLATE_PART_GONE));
        assert_eq!(gone, TemplatePartGoneDetail::new("part"));
    }
}

#[cfg(test)]
mod retention_tests {
    use super::{is_retryable, ExpiredDetail, CODES, EXPIRED};

    #[test]
    fn expired_is_named_terminal_and_carries_expiry_time() {
        assert!(CODES.contains(&EXPIRED));
        assert!(!is_retryable(EXPIRED));
        let detail = ExpiredDetail::new(1234);
        assert_eq!(
            serde_json::to_value(&detail).unwrap(),
            serde_json::json!({"expired_at_ms":1234})
        );
        assert_eq!(
            serde_json::from_str::<ExpiredDetail>(r#"{"expired_at_ms":1234,"future":true}"#)
                .unwrap(),
            detail
        );
        assert!(serde_json::from_str::<ExpiredDetail>("{}").is_err());
    }
}
