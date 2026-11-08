//! `role.describe`: discovery before use.
//!
//! A consumer that finds a module claiming `llm-runner/v1` calls
//! `role.describe`, refuses the module by name if a required op is missing,
//! and checks that every capability it needs is declared before using it.
//! The answer describes the runner build, never a session: session-level
//! capabilities come from the admission reply or `session.baseline`, and the
//! answer says which (`session_capabilities_from`). It depends only on the
//! module build, so a consumer may cache it for as long as it talks to the
//! same module incarnation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{capabilities, PROVIDES, REQUIRED_OPS};

/// Steer delivery receipt policy as declared in `role.describe`.
pub mod steer_receipt {
    /// On a guaranteed runner, a durably accepted steer is delivered:
    /// `delivered` may be absent, and must never be `pending` or `unknown`.
    /// The default when `steer_receipt` is absent.
    pub const GUARANTEED: &str = "guaranteed";
    /// On a confirm runner, an absent `delivered` means `pending`, never
    /// delivered.
    pub const CONFIRM: &str = "confirm";
}

/// Named discovery refusals carried by [`DescribeProblem::Undecodable`].
pub mod problem_names {
    pub const RESTART_SURVIVAL_WITHOUT_INTERRUPT_REASON: &str =
        "restart_survival_without_interrupt_reason";
    pub const RESTART_SURVIVAL_WITHOUT_LIVE_EVENTS: &str = "restart_survival_without_live_events";
    pub const STOP_ALL_WITHOUT_RUN_CANCEL_BY_INTERRUPT: &str =
        "stop_all_without_run_cancel_by_interrupt";
    pub const CALL_YIELD_WITHOUT_PLANS: &str = "call_yield_without_plans";
    pub const CALL_YIELD_WITHOUT_DELIVERY_ORDER: &str = "call_yield_without_delivery_order";
    pub const CALL_YIELD_WITHOUT_NOTICES: &str = "call_yield_without_notices";
    pub const CALL_YIELD_WITHOUT_MESSAGE_MARKS: &str = "call_yield_without_message_marks";
    pub const NOTICE_YIELD_WITHOUT_CALL_YIELD: &str = "notice_yield_without_call_yield";
    pub const FORK_IDLE_S_MISSING: &str = "fork_idle_s_missing";
    pub const FORK_IDLE_S_UNDECLARED: &str = "fork_idle_s_undeclared";
    pub const FORK_IDLE_S_INVALID: &str = "fork_idle_s_invalid";
    pub const ACCOUNT_ROUTE_WITHOUT_ROUTED_ACCOUNTS: &str = "account_route_without_routed_accounts";
    pub const ADOPT_OPEN_WITHOUT_ADOPT: &str = "adopt_open_without_adopt";
    pub const ADOPT_OPEN_WITHOUT_PLANS: &str = "adopt_open_without_plans";
    pub const MESSAGE_MARKS_WITHOUT_READ_DISPLAY_FACTS: &str =
        "message_marks_without_read_display_facts";
    pub const MESSAGE_MARKS_WITHOUT_HISTORY_DISPLAY: &str = "message_marks_without_history_display";
    pub const NOTICE_KEEP_WITHOUT_MESSAGE_MARKS: &str = "notice_keep_without_message_marks";
    pub const TAG_READ_WITHOUT_MESSAGE_SENDERS: &str = "tag_read_without_message_senders";
    pub const TAG_READ_WITHOUT_MESSAGE_MARKS: &str = "tag_read_without_message_marks";
    pub const TAG_READ_WITHOUT_CLIENT_MESSAGE_ID: &str = "tag_read_without_client_message_id";
    pub const TAG_READ_WITHOUT_READ_DISPLAY_FACTS: &str = "tag_read_without_read_display_facts";
    pub const NOTICES_WITHOUT_READ_DISPLAY_FACTS: &str = "notices_without_read_display_facts";
    pub const NOTICES_WITHOUT_HISTORY_DISPLAY: &str = "notices_without_history_display";
    pub const NOTICES_WITHOUT_LIVE_EVENTS: &str = "notices_without_live_events";
    #[deprecated(
        since = "0.8.11",
        note = "renamed to NOTICES_WITHOUT_READ_DISPLAY_FACTS"
    )]
    pub const RESULT_RIDES_WITHOUT_READ_DISPLAY_FACTS: &str = NOTICES_WITHOUT_READ_DISPLAY_FACTS;
    #[deprecated(since = "0.8.11", note = "renamed to NOTICES_WITHOUT_HISTORY_DISPLAY")]
    pub const RESULT_RIDES_WITHOUT_HISTORY_DISPLAY: &str = NOTICES_WITHOUT_HISTORY_DISPLAY;
    #[deprecated(since = "0.8.11", note = "renamed to NOTICES_WITHOUT_LIVE_EVENTS")]
    pub const RESULT_RIDES_WITHOUT_LIVE_EVENTS: &str = NOTICES_WITHOUT_LIVE_EVENTS;
    pub const SENDER_ATTRIBUTION_WITHOUT_DELIVERY_ORDER: &str =
        "sender_attribution_without_delivery_order";
    pub const SENDER_ATTRIBUTION_WITHOUT_READ_DISPLAY_FACTS: &str =
        "sender_attribution_without_read_display_facts";
    pub const SENDER_ATTRIBUTION_WITHOUT_HISTORY_DISPLAY: &str =
        "sender_attribution_without_history_display";
    pub const HEAD_SELECTION_WITHOUT_PLANS: &str = "head_selection_without_plans";
    pub const MEDIA_ONLY_SEND_WITHOUT_ATTACHMENTS: &str = "media_only_send_without_attachments";
    pub const MEDIA_REFS_WITHOUT_ATTACHMENTS: &str = "media_refs_without_attachments";
    pub const MEDIA_REFS_WITHOUT_TRANSCRIPT_READS: &str = "media_refs_without_transcript_reads";
    pub const PLAN_NOTES_WITHOUT_SESSION_CHANGE: &str = "plan_notes_without_session_change";
    pub const RUN_CANCEL_BY_INTERRUPT_WITHOUT_RUN_CANCEL: &str =
        "run_cancel_by_interrupt_without_run_cancel";
    pub const RUN_CANCEL_BY_INTERRUPT_WITHOUT_DELIVERY_ORDER: &str =
        "run_cancel_by_interrupt_without_delivery_order";
    pub const RETRACT_OWN_SEND_WITHOUT_RETRACT: &str = "retract_own_send_without_retract";
    /// `step_hooks` names the same hook more than once.
    pub const DUPLICATE_STEP_HOOK: &str = "duplicate_step_hook";
    pub const REFRESH_RELAUNCH_WITHOUT_SESSION_CHANGE: &str =
        "refresh_relaunch_without_session_change";
    pub const MISSING_SESSION_RETENTION: &str = "missing_session_retention";
    pub const INVALID_SESSION_RETENTION: &str = "invalid_session_retention";
    pub const RUN_CANCEL_UNDECLARED: &str = "run_cancel_undeclared";
    pub const EMPTY_RUN_CANCEL_AT: &str = "empty_run_cancel_at";
    pub const MISSING_RUN_CANCEL_NOW: &str = "missing_run_cancel_now";
    pub const DUPLICATE_RUN_CANCEL_AT: &str = "duplicate_run_cancel_at";
    pub const DELIVERY_ORDER_MODES_UNDECLARED: &str = "delivery_order_modes_undeclared";
    pub const EMPTY_DELIVERY_ORDER_MODES: &str = "empty_delivery_order_modes";
    /// No longer produced by [`super::check_describe`] since 0.8.30;
    /// readers ignore unknown delivery-order mode names.
    pub const UNKNOWN_DELIVERY_ORDER_MODE: &str = "unknown_delivery_order_mode";
    pub const DUPLICATE_DELIVERY_ORDER_MODE: &str = "duplicate_delivery_order_mode";
    pub const DELIVERY_ORDER_STEER_UNDECLARED: &str = "delivery_order_steer_undeclared";
    pub const DELIVERY_ORDER_IDLE_START_UNDECLARED: &str = "delivery_order_idle_start_undeclared";
    /// The answer declares `run_cause` without `live_events`, whose
    /// `run_started` event carries the run's cause.
    pub const RUN_CAUSE_WITHOUT_LIVE_EVENTS: &str = "run_cause_without_live_events";
    /// The answer declares provisional display ids without live events.
    pub const LIVE_IDS_WITHOUT_LIVE_EVENTS: &str = "live_ids_without_live_events";
    pub const MESSAGE_SENDERS_WITHOUT_HISTORY_DISPLAY: &str =
        "message_senders_without_history_display";
    pub const MESSAGE_SENDERS_WITHOUT_LIVE_EVENTS: &str = "message_senders_without_live_events";
    pub const SEND_IDS_WITHOUT_MESSAGE_SENDERS: &str = "send_ids_without_message_senders";
    pub const SEND_IDS_WITHOUT_READ_DISPLAY_FACTS: &str = "send_ids_without_read_display_facts";
    pub const STEP_EVENTS_MISSING: &str = "step_events_missing";
    pub const STEP_EVENTS_UNDECLARED: &str = "step_events_undeclared";
    pub const INVALID_STEP_EVENTS: &str = "invalid_step_events";
    pub const ROUTED_ACCOUNTS_MISSING: &str = "routed_accounts_missing";
    pub const ROUTED_ACCOUNTS_UNDECLARED: &str = "routed_accounts_undeclared";
    pub const INVALID_ROUTED_ACCOUNTS: &str = "invalid_routed_accounts";
    pub const ROUTED_ACCOUNTS_EMPTY: &str = "routed_accounts_empty";
    pub const ROUTED_ACCOUNTS_TOO_MANY: &str = "routed_accounts_too_many";
    pub const ROUTED_ACCOUNTS_INVALID_AUTH_METHOD: &str = "routed_accounts_invalid_auth_method";
    pub const ROUTED_ACCOUNTS_DUPLICATE: &str = "routed_accounts_duplicate";
    /// The answer declares `call_identity` with neither `history_display` nor
    /// `live_events`, the two surfaces whose tool-call entries it extends.
    pub const CALL_IDENTITY_WITHOUT_DISPLAY: &str = "call_identity_without_display";
    /// The answer declares `submit_streamed_siblings` without `structured_output`,
    /// whose `submit_tool` mode the group extends.
    pub const SUBMIT_STREAMED_SIBLINGS_WITHOUT_STRUCTURED_OUTPUT: &str =
        "submit_streamed_siblings_without_structured_output";
    /// The answer declares `structured_output_per_session` without
    /// `structured_output`, whose admission policy it narrows.
    pub const STRUCTURED_OUTPUT_PER_SESSION_WITHOUT_STRUCTURED_OUTPUT: &str =
        "structured_output_per_session_without_structured_output";
}

/// The `role.describe` answer: every major of the role the module serves,
/// each with its own ops and stability, plus the module-level capabilities.
///
/// Decoded leniently: unknown fields, majors, ops and capabilities are
/// ignored. [`check_describe`] checks strictly only what a consumer relies
/// on: the `llm-runner/v1` major, its required ops, the ops of every group
/// the answer declares, where session capabilities come from, and the
/// read byte cap when the answer declares `transcript_reads`.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`RoleDescribe::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RoleDescribe {
    /// Wall-clock idle lifetime in seconds, positive and present exactly with
    /// the `fork_answer` group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_idle_s: Option<u64>,
    pub majors: Vec<Major>,
    /// The module's own build identity, for display. Never a cache key.
    pub implementation_version: String,
    /// Module-level capabilities: the capability groups
    /// ([`capabilities::GROUPS`]) and `ordered_hook_phases` where it holds
    /// for every session.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Where session-level capabilities come from: `admission` or
    /// `baseline` ([`capabilities::source`]). Required by
    /// [`check_describe`]; optional in the type so an answer without it
    /// still decodes and is refused by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_capabilities_from: Option<String>,
    /// The `session.read` byte cap: the default a page stops at when the
    /// request names no `max_bytes`, and the largest `max_bytes` the runner
    /// honours. Required by [`check_describe`] when the answer declares
    /// `transcript_reads`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<MaxBytes>,
    /// Steer delivery receipt policy: `guaranteed` or `confirm`
    /// ([`steer_receipt`]). Absent means [`steer_receipt::GUARANTEED`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steer_receipt: Option<String>,
    /// List modes this runner can ever land. Absence uses the capability-group
    /// mapping for existing modes, never `turn_system_cleared`, which requires
    /// an explicit listing. Listed modes still need their capability groups.
    /// Readers ignore mode names they do not know.
    /// Validated by [`check_describe`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_order_modes: Option<Vec<String>>,
    /// Limits required when the `retention` capability group is declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<Retention>,
    /// Required limits when `session_retention` is declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_retention: Option<SessionRetention>,
    /// Supported cancellation points; absent means only `now` is supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_cancel: Option<RunCancel>,
    /// Publication granularity and durable-history join for `step_events`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_events: Option<crate::step_events::StepEvents>,
    /// Sign-in methods whose accounts the runner routes itself. Required with
    /// the `routed_accounts` capability; checked by [`check_routed_accounts`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routed_accounts: Option<Vec<String>>,
    /// Kinds this runner may prune in archived sessions at rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_prunes_reasoning: Option<crate::archive_prunes_reasoning::ArchivePrunesReasoning>,
    /// The step-transform hook call sites this runner runs for every session
    /// (CONTRACT.md §11.2), named as `step-transform-provider/v1` names them
    /// (see [`step_hooks`]). Absent or empty advertises no hook, so a plan
    /// composer leaves out every item that subscribes one. An unknown name is
    /// kept; a consumer that doesn't recognise it infers nothing from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_hooks: Option<Vec<String>>,
    /// Required with `service_tier`: non-standard levels supported per wire API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tiers: Option<crate::service_tier::ServiceTiers>,
}

/// Hook names a runner lists in [`RoleDescribe::step_hooks`], spelled as the
/// `step-transform-provider/v1` role spells them.
pub mod step_hooks {
    /// Runs on every user message and every steered or queued prompt.
    pub const PRE_USER: &str = "pre_user";
    /// Runs before each tool call executes.
    pub const PRE_TOOL: &str = "pre_tool";
    /// Runs on each tool call's result, before that result is written.
    pub const POST_TOOL: &str = "post_tool";
    /// Runs after an assistant message completes.
    pub const POST_ASSISTANT: &str = "post_assistant";
    /// Every hook name this release knows.
    pub const KNOWN: &[&str] = &[PRE_USER, PRE_TOOL, POST_TOOL, POST_ASSISTANT];
}

/// Limits for per-session storage policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SessionRetention {
    pub max_days: u32,
    pub max_unreleased_hold_ms: u64,
    pub required: bool,
    #[serde(default, skip_serializing_if = "is_zero_days")]
    pub min_hold_days: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deletion_waits_for_readers: bool,
}

fn is_zero_days(days: &u32) -> bool {
    *days == 0
}

impl SessionRetention {
    pub fn new(max_days: u32, max_unreleased_hold_ms: u64, required: bool) -> Self {
        Self {
            max_days,
            max_unreleased_hold_ms,
            required,
            min_hold_days: 0,
            deletion_waits_for_readers: false,
        }
    }

    pub fn with_min_hold_days(mut self, days: u32) -> Self {
        self.min_hold_days = days;
        self
    }

    pub fn with_deletion_waits_for_readers(mut self) -> Self {
        self.deletion_waits_for_readers = true;
        self
    }
}

/// Cancellation points advertised by a runner declaring `run_cancel`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunCancel {
    /// Plain strings rather than an enum, so a runner may list points this
    /// crate doesn't know yet. A consumer ignores any value it doesn't
    /// recognize and sends only points it knows and the runner lists.
    pub at: Vec<String>,
}

impl RunCancel {
    pub fn new(at: Vec<String>) -> Self {
        Self { at }
    }

    pub fn with_at(mut self, at: Vec<String>) -> Self {
        self.at = at;
        self
    }
}

/// Whole-session retention limits advertised by a runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Retention {
    /// Largest accepted `session.send.retention`, in seconds; must be positive.
    pub max_seconds: u64,
    /// Maximum delay from expiry to deletion of all runner-held content.
    pub delete_within_ms: u64,
}

impl Retention {
    pub fn new(max_seconds: u64, delete_within_ms: u64) -> Self {
        Self {
            max_seconds,
            delete_within_ms,
        }
    }
}

/// A runner's `session.read` byte cap, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct MaxBytes {
    /// The cap a page stops at when the request names no `max_bytes`.
    pub default: u64,
    /// The largest cap the runner honours; a larger request is capped to it.
    pub maximum: u64,
}

fn alpha() -> String {
    "alpha".to_owned()
}

/// One served major.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Major {
    /// The role and major as the manifest spells it, for example
    /// `llm-runner/v1`.
    pub version: String,
    pub ops: Vec<String>,
    /// `alpha`, `beta` or `stable`; see [`Major::stability`]. Stability is
    /// data, never part of a capability name. An answer that omits it is
    /// read as `alpha`, the level every major starts at.
    #[serde(default = "alpha")]
    pub stability: String,
}

/// A stability level. Unknown levels decode as [`Stability::Other`] rather
/// than failing the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stability {
    Alpha,
    Beta,
    Stable,
    Other(String),
}

impl RoleDescribe {
    pub fn with_fork_idle_s(mut self, seconds: u64) -> Self {
        self.fork_idle_s = Some(seconds);
        self
    }

    /// Declare the step-transform hook call sites this runner runs for every
    /// session (see [`step_hooks`]).
    pub fn with_step_hooks<I, S>(mut self, hooks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.step_hooks = Some(hooks.into_iter().map(Into::into).collect());
        self
    }

    /// Whether this runner advertises every hook in `hooks`, comparing names
    /// literally. It checks the advertisement, not that the runner executes
    /// the hooks. A plan composer adds a step item only when this holds for
    /// the hooks its frozen subscriptions name, and a runner admits a plan on
    /// the same condition. An empty `hooks` always passes, so a composer
    /// leaves out an item with no subscriptions by its own rule.
    pub fn serves_step_hooks<'a>(&self, hooks: impl IntoIterator<Item = &'a str>) -> bool {
        let declared = self.step_hooks.as_deref().unwrap_or_default();
        hooks
            .into_iter()
            .all(|hook| declared.iter().any(|name| name == hook))
    }

    pub fn with_archive_prunes_reasoning(
        mut self,
        settings: crate::archive_prunes_reasoning::ArchivePrunesReasoning,
    ) -> Self {
        self.archive_prunes_reasoning = Some(settings);
        self
    }
    /// An answer for the given majors and build identity, declaring no
    /// capability and no session-capability source.
    pub fn new(majors: Vec<Major>, implementation_version: impl Into<String>) -> Self {
        Self {
            majors,
            implementation_version: implementation_version.into(),
            ..Self::default()
        }
    }

    /// Set the module-level capabilities.
    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_service_tiers(mut self, tiers: crate::service_tier::ServiceTiers) -> Self {
        self.service_tiers = Some(tiers);
        self
    }

    /// State the `session.read` byte cap.
    pub fn with_max_bytes(mut self, default: u64, maximum: u64) -> Self {
        self.max_bytes = Some(MaxBytes { default, maximum });
        self
    }

    /// State the retention limits (the capability must also be listed).
    pub fn with_retention(mut self, max_seconds: u64, delete_within_ms: u64) -> Self {
        self.retention = Some(Retention::new(max_seconds, delete_within_ms));
        self
    }

    pub fn with_session_retention(mut self, limits: SessionRetention) -> Self {
        self.session_retention = Some(limits);
        self
    }

    /// State the cancellation points (the capability must also be listed).
    pub fn with_run_cancel(mut self, at: Vec<String>) -> Self {
        self.run_cancel = Some(RunCancel::new(at));
        self
    }

    /// State the step-event policy (the capability must also be listed).
    pub fn with_step_events(mut self, policy: crate::step_events::StepEvents) -> Self {
        self.step_events = Some(policy);
        self
    }

    /// List routed sign-in methods (the capability must also be listed).
    pub fn with_routed_accounts(mut self, methods: Vec<String>) -> Self {
        self.routed_accounts = Some(methods);
        self
    }

    /// Say where session-level capabilities come from.
    pub fn with_session_capabilities_from(mut self, source: impl Into<String>) -> Self {
        self.session_capabilities_from = Some(source.into());
        self
    }

    /// Say what steer delivery receipt policy the runner follows.
    pub fn with_steer_receipt(mut self, steer_receipt: impl Into<String>) -> Self {
        self.steer_receipt = Some(steer_receipt.into());
        self
    }

    /// Narrow the declared list modes (their capability groups must also be listed).
    pub fn with_delivery_order_modes(mut self, modes: Vec<String>) -> Self {
        self.delivery_order_modes = Some(modes);
        self
    }

    /// The steer delivery receipt policy, with absence read as [`steer_receipt::GUARANTEED`].
    pub fn steer_receipt(&self) -> &str {
        self.steer_receipt
            .as_deref()
            .unwrap_or(steer_receipt::GUARANTEED)
    }

    /// The entry for `version` (for example `llm-runner/v1`), if served.
    pub fn major(&self, version: &str) -> Option<&Major> {
        self.majors.iter().find(|major| major.version == version)
    }

    /// Whether the module-level capability `name` is declared.
    pub fn declares(&self, name: &str) -> bool {
        self.capabilities.iter().any(|declared| declared == name)
    }

    /// Whether this runner promises admission and unchanged forwarding of
    /// preset-less system text for providers declaring the default capability.
    pub fn system_text_default(&self) -> bool {
        self.declares(capabilities::SYSTEM_TEXT_DEFAULT)
    }

    /// The capabilities in `required` this answer does not declare, in the
    /// order given. A consumer refuses the module, naming them, when this is
    /// not empty.
    pub fn missing<'a>(&self, required: &[&'a str]) -> Vec<&'a str> {
        required
            .iter()
            .copied()
            .filter(|name| !self.declares(name))
            .collect()
    }
}

impl Major {
    pub fn new(version: impl Into<String>, ops: Vec<String>, stability: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            ops,
            stability: stability.into(),
        }
    }

    pub fn stability(&self) -> Stability {
        match self.stability.as_str() {
            "alpha" => Stability::Alpha,
            "beta" => Stability::Beta,
            "stable" => Stability::Stable,
            other => Stability::Other(other.to_owned()),
        }
    }

    pub fn serves(&self, op: &str) -> bool {
        self.ops.iter().any(|served| served == op)
    }
}

/// Validate 1–64 distinct routing auth-method tokens without a routing dependency.
/// Tokens use `^[a-z][a-z0-9._-]{0,63}$`. This checks shape only: a
/// token-shaped spelling missing from routing's `auth_method` table passes
/// here, but routing uses no method until its table names it.
pub fn check_routed_accounts(methods: &[String]) -> Result<(), &'static str> {
    if methods.is_empty() {
        return Err(problem_names::ROUTED_ACCOUNTS_EMPTY);
    }
    if methods.len() > 64 {
        return Err(problem_names::ROUTED_ACCOUNTS_TOO_MANY);
    }
    let mut seen = std::collections::BTreeSet::new();
    for method in methods {
        if !(1..=64).contains(&method.len())
            || !method.as_bytes()[0].is_ascii_lowercase()
            || !method.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
        {
            return Err(problem_names::ROUTED_ACCOUNTS_INVALID_AUTH_METHOD);
        }
        if !seen.insert(method.as_str()) {
            return Err(problem_names::ROUTED_ACCOUNTS_DUPLICATE);
        }
    }
    Ok(())
}

/// Why a `role.describe` answer does not describe a usable `llm-runner/v1`
/// module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescribeProblem {
    /// The answer is not a JSON object.
    NotAnObject,
    /// A required field is missing or has the wrong type, or optional
    /// cancellation metadata violates a named rule ([`problem_names`]).
    Undecodable(String),
    /// No entry in `majors` is `llm-runner/v1`.
    MissingMajor {
        found: Vec<String>,
    },
    /// The `llm-runner/v1` entry lacks a required op.
    MissingOp(&'static str),
    /// A declared capability group lacks one of its ops. A group is all or
    /// nothing, so a consumer cannot use a partly served group.
    GroupIncomplete {
        capability: &'static str,
        op: &'static str,
    },
    /// The answer does not say where session-level capabilities come from.
    MissingSessionCapabilitiesSource,
    /// The answer declares `transcript_reads` without stating its byte cap.
    MissingMaxBytes,
    /// The retention group has no limits, or cannot accept a positive value.
    MissingRetention,
    InvalidRetention,
}

/// Decode `raw` and check it lists an `llm-runner/v1` major with every
/// required op, every op of each declared group, and a session-capability
/// source, plus any advertised cancellation points and delivery list modes. Every problem is returned,
/// not only the first. Unknown
/// capabilities and an unknown source value are tolerated.
pub fn check_describe(raw: &Value) -> Result<RoleDescribe, Vec<DescribeProblem>> {
    if !raw.is_object() {
        return Err(vec![DescribeProblem::NotAnObject]);
    }
    if raw
        .get("fork_idle_s")
        .is_some_and(|value| value.as_u64().is_none_or(|seconds| seconds == 0))
    {
        return Err(vec![DescribeProblem::Undecodable(
            problem_names::FORK_IDLE_S_INVALID.into(),
        )]);
    }
    if let Some(settings) = raw.get("archive_prunes_reasoning") {
        if serde_json::from_value::<crate::archive_prunes_reasoning::ArchivePrunesReasoning>(
            settings.clone(),
        )
        .map_or(true, |s| !s.valid())
        {
            return Err(vec![DescribeProblem::Undecodable(
                "archive_prunes_reasoning_invalid_kinds".into(),
            )]);
        }
    }
    if let Some(policy) = raw.get("step_events") {
        if serde_json::from_value::<crate::step_events::StepEvents>(policy.clone())
            .map_or(true, |policy| !policy.valid())
        {
            return Err(vec![DescribeProblem::Undecodable(
                problem_names::INVALID_STEP_EVENTS.into(),
            )]);
        }
    }
    if let Some(methods) = raw.get("routed_accounts").filter(|value| !value.is_null()) {
        if serde_json::from_value::<Vec<String>>(methods.clone()).is_err() {
            return Err(vec![DescribeProblem::Undecodable(
                problem_names::INVALID_ROUTED_ACCOUNTS.into(),
            )]);
        }
    }
    let describe: RoleDescribe = match serde_json::from_value(raw.clone()) {
        Ok(describe) => describe,
        Err(error) => return Err(vec![DescribeProblem::Undecodable(error.to_string())]),
    };
    let mut problems = Vec::new();
    match (
        describe.declares(capabilities::FORK_ANSWER),
        describe.fork_idle_s,
    ) {
        (true, None) => problems.push(DescribeProblem::Undecodable(
            problem_names::FORK_IDLE_S_MISSING.into(),
        )),
        (false, Some(_)) => problems.push(DescribeProblem::Undecodable(
            problem_names::FORK_IDLE_S_UNDECLARED.into(),
        )),
        _ => {}
    }
    match (
        describe.declares(capabilities::ROUTED_ACCOUNTS),
        &describe.routed_accounts,
    ) {
        (true, None) => problems.push(DescribeProblem::Undecodable(
            problem_names::ROUTED_ACCOUNTS_MISSING.into(),
        )),
        (false, Some(_)) => problems.push(DescribeProblem::Undecodable(
            problem_names::ROUTED_ACCOUNTS_UNDECLARED.into(),
        )),
        (_, Some(methods)) => {
            if let Err(reason) = check_routed_accounts(methods) {
                problems.push(DescribeProblem::Undecodable(reason.into()));
            }
        }
        _ => {}
    }
    if describe.declares(capabilities::SERVICE_TIER) && describe.service_tiers.is_none() {
        problems.push(DescribeProblem::Undecodable("service_tiers_missing".into()));
    }
    if let Some(tiers) = &describe.service_tiers {
        if let Err(reason) = crate::service_tier::check_service_tiers(tiers) {
            problems.push(DescribeProblem::Undecodable(format!(
                "service_tiers_{reason}"
            )));
        }
    }
    if let Some(hooks) = &describe.step_hooks {
        let mut seen = std::collections::BTreeSet::new();
        if hooks.iter().any(|hook| !seen.insert(hook.as_str())) {
            problems.push(DescribeProblem::Undecodable(
                problem_names::DUPLICATE_STEP_HOOK.into(),
            ));
        }
    }
    for (group, dependency, problem) in [
        (
            capabilities::NOTICE_YIELD,
            capabilities::CALL_YIELD,
            problem_names::NOTICE_YIELD_WITHOUT_CALL_YIELD,
        ),
        (
            capabilities::TAG_READ,
            capabilities::MESSAGE_SENDERS,
            problem_names::TAG_READ_WITHOUT_MESSAGE_SENDERS,
        ),
        (
            capabilities::TAG_READ,
            capabilities::MESSAGE_MARKS,
            problem_names::TAG_READ_WITHOUT_MESSAGE_MARKS,
        ),
        (
            capabilities::TAG_READ,
            capabilities::CLIENT_MESSAGE_ID,
            problem_names::TAG_READ_WITHOUT_CLIENT_MESSAGE_ID,
        ),
        (
            capabilities::TAG_READ,
            capabilities::READ_DISPLAY_FACTS,
            problem_names::TAG_READ_WITHOUT_READ_DISPLAY_FACTS,
        ),
        (
            capabilities::NOTICE_KEEP,
            capabilities::MESSAGE_MARKS,
            problem_names::NOTICE_KEEP_WITHOUT_MESSAGE_MARKS,
        ),
        (
            capabilities::ACCOUNT_ROUTE,
            capabilities::ROUTED_ACCOUNTS,
            problem_names::ACCOUNT_ROUTE_WITHOUT_ROUTED_ACCOUNTS,
        ),
        (
            capabilities::ADOPT_OPEN,
            capabilities::ADOPT,
            problem_names::ADOPT_OPEN_WITHOUT_ADOPT,
        ),
        (
            capabilities::ADOPT_OPEN,
            capabilities::PLANS,
            problem_names::ADOPT_OPEN_WITHOUT_PLANS,
        ),
        (
            capabilities::MESSAGE_MARKS,
            capabilities::READ_DISPLAY_FACTS,
            problem_names::MESSAGE_MARKS_WITHOUT_READ_DISPLAY_FACTS,
        ),
        (
            capabilities::MESSAGE_MARKS,
            capabilities::HISTORY_DISPLAY,
            problem_names::MESSAGE_MARKS_WITHOUT_HISTORY_DISPLAY,
        ),
        (
            capabilities::NOTICES,
            capabilities::READ_DISPLAY_FACTS,
            problem_names::NOTICES_WITHOUT_READ_DISPLAY_FACTS,
        ),
        (
            capabilities::NOTICES,
            capabilities::HISTORY_DISPLAY,
            problem_names::NOTICES_WITHOUT_HISTORY_DISPLAY,
        ),
        (
            capabilities::NOTICES,
            capabilities::LIVE_EVENTS,
            problem_names::NOTICES_WITHOUT_LIVE_EVENTS,
        ),
        (
            capabilities::CALL_YIELD,
            capabilities::PLANS,
            problem_names::CALL_YIELD_WITHOUT_PLANS,
        ),
        (
            capabilities::CALL_YIELD,
            capabilities::DELIVERY_ORDER,
            problem_names::CALL_YIELD_WITHOUT_DELIVERY_ORDER,
        ),
        (
            capabilities::CALL_YIELD,
            capabilities::NOTICES,
            problem_names::CALL_YIELD_WITHOUT_NOTICES,
        ),
        (
            capabilities::CALL_YIELD,
            capabilities::MESSAGE_MARKS,
            problem_names::CALL_YIELD_WITHOUT_MESSAGE_MARKS,
        ),
        (
            capabilities::STOP_ALL,
            capabilities::RUN_CANCEL_BY_INTERRUPT,
            problem_names::STOP_ALL_WITHOUT_RUN_CANCEL_BY_INTERRUPT,
        ),
        (
            capabilities::RESTART_SURVIVAL,
            capabilities::INTERRUPT_REASON,
            problem_names::RESTART_SURVIVAL_WITHOUT_INTERRUPT_REASON,
        ),
        (
            capabilities::RESTART_SURVIVAL,
            capabilities::LIVE_EVENTS,
            problem_names::RESTART_SURVIVAL_WITHOUT_LIVE_EVENTS,
        ),
        (
            capabilities::HEAD_SELECTION,
            capabilities::PLANS,
            problem_names::HEAD_SELECTION_WITHOUT_PLANS,
        ),
        (
            capabilities::MEDIA_ONLY_SEND,
            capabilities::ATTACHMENTS,
            problem_names::MEDIA_ONLY_SEND_WITHOUT_ATTACHMENTS,
        ),
        (
            capabilities::MEDIA_REFS,
            capabilities::ATTACHMENTS,
            problem_names::MEDIA_REFS_WITHOUT_ATTACHMENTS,
        ),
        (
            capabilities::MEDIA_REFS,
            capabilities::TRANSCRIPT_READS,
            problem_names::MEDIA_REFS_WITHOUT_TRANSCRIPT_READS,
        ),
        (
            capabilities::PLAN_NOTES,
            capabilities::SESSION_CHANGE,
            problem_names::PLAN_NOTES_WITHOUT_SESSION_CHANGE,
        ),
        (
            capabilities::RETRACT_OWN_SEND,
            capabilities::RETRACT,
            problem_names::RETRACT_OWN_SEND_WITHOUT_RETRACT,
        ),
        (
            capabilities::RUN_CANCEL_BY_INTERRUPT,
            capabilities::RUN_CANCEL,
            problem_names::RUN_CANCEL_BY_INTERRUPT_WITHOUT_RUN_CANCEL,
        ),
        (
            capabilities::RUN_CANCEL_BY_INTERRUPT,
            capabilities::DELIVERY_ORDER,
            problem_names::RUN_CANCEL_BY_INTERRUPT_WITHOUT_DELIVERY_ORDER,
        ),
        (
            capabilities::REFRESH_RELAUNCH,
            capabilities::SESSION_CHANGE,
            problem_names::REFRESH_RELAUNCH_WITHOUT_SESSION_CHANGE,
        ),
    ] {
        if describe.declares(group) && !describe.declares(dependency) {
            problems.push(DescribeProblem::Undecodable(problem.into()));
        }
    }
    match (
        describe.declares(capabilities::ARCHIVE_PRUNES_REASONING),
        &describe.archive_prunes_reasoning,
    ) {
        (true, None) => problems.push(DescribeProblem::Undecodable(
            "archive_prunes_reasoning_missing".into(),
        )),
        (false, Some(_)) => problems.push(DescribeProblem::Undecodable(
            "archive_prunes_reasoning_undeclared".into(),
        )),
        (_, Some(settings)) if !settings.valid() => problems.push(DescribeProblem::Undecodable(
            "archive_prunes_reasoning_invalid_kinds".into(),
        )),
        _ => {}
    }
    match (
        describe.declares(capabilities::STEP_EVENTS),
        describe.step_events.is_some(),
    ) {
        (true, false) => problems.push(DescribeProblem::Undecodable(
            problem_names::STEP_EVENTS_MISSING.into(),
        )),
        (false, true) => problems.push(DescribeProblem::Undecodable(
            problem_names::STEP_EVENTS_UNDECLARED.into(),
        )),
        _ => {}
    }
    if describe.declares(capabilities::HELD_CALLS) && !describe.declares(capabilities::RUN_STATUS) {
        problems.push(DescribeProblem::Undecodable(
            "held_calls requires run_status".into(),
        ));
    }
    if describe.declares(capabilities::SUBMIT_STREAMED_SIBLINGS)
        && !describe.declares(capabilities::STRUCTURED_OUTPUT)
    {
        problems.push(DescribeProblem::Undecodable(
            problem_names::SUBMIT_STREAMED_SIBLINGS_WITHOUT_STRUCTURED_OUTPUT.into(),
        ));
    }
    if describe.declares(capabilities::STRUCTURED_OUTPUT_PER_SESSION)
        && !describe.declares(capabilities::STRUCTURED_OUTPUT)
    {
        problems.push(DescribeProblem::Undecodable(
            problem_names::STRUCTURED_OUTPUT_PER_SESSION_WITHOUT_STRUCTURED_OUTPUT.into(),
        ));
    }
    match describe.major(PROVIDES) {
        None => problems.push(DescribeProblem::MissingMajor {
            found: describe.majors.iter().map(|m| m.version.clone()).collect(),
        }),
        Some(major) => {
            for op in REQUIRED_OPS {
                if !major.serves(op) {
                    problems.push(DescribeProblem::MissingOp(op));
                }
            }
            for (group, group_ops) in capabilities::GROUPS {
                if !describe.declares(group) {
                    continue;
                }
                for op in *group_ops {
                    if !major.serves(op) {
                        problems.push(DescribeProblem::GroupIncomplete {
                            capability: group,
                            op,
                        });
                    }
                }
            }
        }
    }
    if describe.declares(capabilities::TRANSCRIPT_READS) && describe.max_bytes.is_none() {
        problems.push(DescribeProblem::MissingMaxBytes);
    }
    if describe.declares(capabilities::RETENTION) {
        match describe.retention {
            None => problems.push(DescribeProblem::MissingRetention),
            Some(limits) if limits.max_seconds == 0 => {
                problems.push(DescribeProblem::InvalidRetention)
            }
            Some(_) => {}
        }
    }
    if describe.declares(capabilities::SESSION_RETENTION) {
        let problem = match describe.session_retention {
            None => Some(problem_names::MISSING_SESSION_RETENTION),
            Some(limits)
                if !(1..=3650).contains(&limits.max_days)
                    || limits.max_unreleased_hold_ms == 0
                    || limits.min_hold_days > limits.max_days =>
            {
                Some(problem_names::INVALID_SESSION_RETENTION)
            }
            Some(_) => None,
        };
        if let Some(problem) = problem {
            problems.push(DescribeProblem::Undecodable(problem.into()));
        }
    }
    if describe.session_capabilities_from.is_none() {
        problems.push(DescribeProblem::MissingSessionCapabilitiesSource);
    }
    if describe.declares(capabilities::RUN_CAUSE) && !describe.declares(capabilities::LIVE_EVENTS) {
        problems.push(DescribeProblem::Undecodable(
            problem_names::RUN_CAUSE_WITHOUT_LIVE_EVENTS.into(),
        ));
    }
    if describe.declares(capabilities::LIVE_IDS) && !describe.declares(capabilities::LIVE_EVENTS) {
        problems.push(DescribeProblem::Undecodable(
            problem_names::LIVE_IDS_WITHOUT_LIVE_EVENTS.into(),
        ));
    }
    if describe.declares(capabilities::CALL_IDENTITY)
        && !describe.declares(capabilities::HISTORY_DISPLAY)
        && !describe.declares(capabilities::LIVE_EVENTS)
    {
        problems.push(DescribeProblem::Undecodable(
            problem_names::CALL_IDENTITY_WITHOUT_DISPLAY.into(),
        ));
    }
    if describe.declares(capabilities::SEND_IDS) {
        for (dependency, problem) in [
            (
                capabilities::MESSAGE_SENDERS,
                problem_names::SEND_IDS_WITHOUT_MESSAGE_SENDERS,
            ),
            (
                capabilities::READ_DISPLAY_FACTS,
                problem_names::SEND_IDS_WITHOUT_READ_DISPLAY_FACTS,
            ),
        ] {
            if !describe.declares(dependency) {
                problems.push(DescribeProblem::Undecodable(problem.into()));
            }
        }
    }
    if describe.declares(capabilities::MESSAGE_SENDERS) {
        for (dependency, problem) in [
            (
                capabilities::HISTORY_DISPLAY,
                problem_names::MESSAGE_SENDERS_WITHOUT_HISTORY_DISPLAY,
            ),
            (
                capabilities::LIVE_EVENTS,
                problem_names::MESSAGE_SENDERS_WITHOUT_LIVE_EVENTS,
            ),
        ] {
            if !describe.declares(dependency) {
                problems.push(DescribeProblem::Undecodable(problem.into()));
            }
        }
    }
    if describe.declares(capabilities::SENDER_ATTRIBUTION) {
        for (dependency, problem) in [
            (
                capabilities::DELIVERY_ORDER,
                problem_names::SENDER_ATTRIBUTION_WITHOUT_DELIVERY_ORDER,
            ),
            (
                capabilities::READ_DISPLAY_FACTS,
                problem_names::SENDER_ATTRIBUTION_WITHOUT_READ_DISPLAY_FACTS,
            ),
            (
                capabilities::HISTORY_DISPLAY,
                problem_names::SENDER_ATTRIBUTION_WITHOUT_HISTORY_DISPLAY,
            ),
        ] {
            if !describe.declares(dependency) {
                problems.push(DescribeProblem::Undecodable(problem.into()));
            }
        }
    }
    if let Some(modes) = &describe.delivery_order_modes {
        use crate::delivery_order::modes as names;
        if !describe.declares(capabilities::DELIVERY_ORDER) {
            problems.push(DescribeProblem::Undecodable(
                problem_names::DELIVERY_ORDER_MODES_UNDECLARED.into(),
            ));
        }
        if modes.is_empty() {
            problems.push(DescribeProblem::Undecodable(
                problem_names::EMPTY_DELIVERY_ORDER_MODES.into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for mode in modes {
            let problem = match mode.as_str() {
                names::TOOL_RESULT | names::TURN_SYSTEM | names::TURN_SYSTEM_CLEARED => None,
                names::STEER if !describe.declares(capabilities::STEER) => {
                    Some(problem_names::DELIVERY_ORDER_STEER_UNDECLARED)
                }
                names::IDLE_START if !describe.declares(capabilities::QUEUE) => {
                    Some(problem_names::DELIVERY_ORDER_IDLE_START_UNDECLARED)
                }
                names::STEER | names::IDLE_START => None,
                _ => None,
            };
            if let Some(problem) = problem {
                problems.push(DescribeProblem::Undecodable(problem.into()));
            }
            if !seen.insert(mode) {
                problems.push(DescribeProblem::Undecodable(
                    problem_names::DUPLICATE_DELIVERY_ORDER_MODE.into(),
                ));
            }
        }
    }
    if let Some(cancel) = &describe.run_cancel {
        if !describe.declares(capabilities::RUN_CANCEL) {
            problems.push(DescribeProblem::Undecodable(
                problem_names::RUN_CANCEL_UNDECLARED.into(),
            ));
        }
        if cancel.at.is_empty() {
            problems.push(DescribeProblem::Undecodable(
                problem_names::EMPTY_RUN_CANCEL_AT.into(),
            ));
        }
        if !cancel.at.iter().any(|at| at == crate::run::cancel_at::NOW) {
            problems.push(DescribeProblem::Undecodable(
                problem_names::MISSING_RUN_CANCEL_NOW.into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for at in &cancel.at {
            if !seen.insert(at) {
                problems.push(DescribeProblem::Undecodable(
                    problem_names::DUPLICATE_RUN_CANCEL_AT.into(),
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(describe)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod message_senders_tests {
    use super::*;

    #[test]
    fn message_senders_requires_each_dependency_by_name() {
        for (dependencies, missing) in [
            (
                vec![],
                vec![
                    problem_names::MESSAGE_SENDERS_WITHOUT_HISTORY_DISPLAY,
                    problem_names::MESSAGE_SENDERS_WITHOUT_LIVE_EVENTS,
                ],
            ),
            (
                vec![capabilities::HISTORY_DISPLAY],
                vec![problem_names::MESSAGE_SENDERS_WITHOUT_LIVE_EVENTS],
            ),
            (
                vec![capabilities::LIVE_EVENTS],
                vec![problem_names::MESSAGE_SENDERS_WITHOUT_HISTORY_DISPLAY],
            ),
            (
                vec![capabilities::HISTORY_DISPLAY, capabilities::LIVE_EVENTS],
                vec![],
            ),
        ] {
            let describe = RoleDescribe::new(
                vec![Major::new(
                    crate::PROVIDES,
                    crate::REQUIRED_OPS
                        .iter()
                        .copied()
                        .chain([crate::ops::SESSION_HISTORY, crate::ops::SESSION_SUBSCRIBE])
                        .map(String::from)
                        .collect(),
                    "alpha",
                )],
                "0.7.47",
            )
            .with_capabilities(
                std::iter::once(capabilities::MESSAGE_SENDERS)
                    .chain(dependencies)
                    .map(String::from)
                    .collect(),
            )
            .with_session_capabilities_from("baseline");
            let result = check_describe(&serde_json::to_value(describe).unwrap());
            if missing.is_empty() {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    missing
                        .into_iter()
                        .map(|name| DescribeProblem::Undecodable(name.into()))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[cfg(test)]
mod live_ids_tests {
    use super::*;

    #[test]
    fn live_ids_requires_live_events_by_name() {
        let describe = RoleDescribe::new(
            vec![Major::new(
                crate::PROVIDES,
                crate::REQUIRED_OPS
                    .iter()
                    .copied()
                    .chain([crate::ops::SESSION_SUBSCRIBE])
                    .map(String::from)
                    .collect(),
                "alpha",
            )],
            "0.7.46",
        )
        .with_capabilities(vec![capabilities::LIVE_IDS.into()])
        .with_session_capabilities_from("baseline");
        let problems = check_describe(&serde_json::to_value(&describe).unwrap()).unwrap_err();
        assert_eq!(
            problems,
            vec![DescribeProblem::Undecodable(
                problem_names::LIVE_IDS_WITHOUT_LIVE_EVENTS.into()
            )]
        );
        let describe = describe.with_capabilities(vec![
            capabilities::LIVE_IDS.into(),
            capabilities::LIVE_EVENTS.into(),
        ]);
        assert!(check_describe(&serde_json::to_value(describe).unwrap()).is_ok());
    }
}

#[cfg(test)]
mod opt_in_dependency_tests {
    use super::*;

    #[test]
    fn opt_in_groups_require_their_dependencies() {
        for (group, dependencies) in [
            (
                capabilities::ADOPT_OPEN,
                vec![capabilities::ADOPT, capabilities::PLANS],
            ),
            (
                capabilities::MESSAGE_MARKS,
                vec![
                    capabilities::READ_DISPLAY_FACTS,
                    capabilities::HISTORY_DISPLAY,
                ],
            ),
            (
                capabilities::MEDIA_ONLY_SEND,
                vec![capabilities::ATTACHMENTS],
            ),
            (
                capabilities::MEDIA_REFS,
                vec![capabilities::ATTACHMENTS, capabilities::TRANSCRIPT_READS],
            ),
            (capabilities::PLAN_NOTES, vec![capabilities::SESSION_CHANGE]),
            (capabilities::RETRACT_OWN_SEND, vec![capabilities::RETRACT]),
            (
                capabilities::NOTICES,
                vec![
                    capabilities::READ_DISPLAY_FACTS,
                    capabilities::HISTORY_DISPLAY,
                    capabilities::LIVE_EVENTS,
                ],
            ),
            (
                capabilities::RUN_CANCEL_BY_INTERRUPT,
                vec![capabilities::RUN_CANCEL, capabilities::DELIVERY_ORDER],
            ),
            (
                capabilities::REFRESH_RELAUNCH,
                vec![capabilities::SESSION_CHANGE],
            ),
        ] {
            let mut raw = serde_json::json!({
                "majors": [{"version": crate::PROVIDES, "ops": crate::REQUIRED_OPS,
                            "stability": "stable"}],
                "implementation_version": "test",
                "capabilities": [group],
                "session_capabilities_from": "baseline"
            });
            let problems = check_describe(&raw).unwrap_err();
            for dependency in &dependencies {
                assert!(problems.contains(&DescribeProblem::Undecodable(format!(
                    "{group}_without_{dependency}"
                ))));
            }
            raw["capabilities"] = serde_json::json!(dependencies);
            raw["capabilities"]
                .as_array_mut()
                .unwrap()
                .push(group.into());
            let mut ops = crate::REQUIRED_OPS.to_vec();
            for dependency in dependencies {
                ops.extend_from_slice(capabilities::group_ops(dependency).unwrap());
            }
            ops.extend_from_slice(capabilities::group_ops(group).unwrap());
            raw["majors"][0]["ops"] = serde_json::json!(ops);
            if group == capabilities::MEDIA_REFS {
                raw["max_bytes"] = serde_json::json!({"default": 1024, "maximum": 4096});
            }
            assert!(check_describe(&raw).is_ok(), "{:?}", check_describe(&raw));
            if group == capabilities::REFRESH_RELAUNCH {
                let mut no_subscribe = raw.clone();
                no_subscribe["majors"][0]["ops"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|op| op != crate::ops::SESSION_SUBSCRIBE);
                assert!(check_describe(&no_subscribe).unwrap_err().contains(
                    &DescribeProblem::GroupIncomplete {
                        capability: capabilities::REFRESH_RELAUNCH,
                        op: crate::ops::SESSION_SUBSCRIBE
                    }
                ));
            }
            for dependency in raw["capabilities"].as_array().unwrap().clone() {
                if dependency == group {
                    continue;
                }
                let mut missing = raw.clone();
                missing["capabilities"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|c| c != &dependency);
                assert!(check_describe(&missing).unwrap_err().contains(
                    &DescribeProblem::Undecodable(format!(
                        "{group}_without_{}",
                        dependency.as_str().unwrap()
                    ))
                ));
            }
        }
    }
}

#[cfg(test)]
mod delivery_order_modes_tests {
    use super::*;
    use crate::delivery_order::Mode;
    use serde_json::json;

    fn vectors() -> Value {
        serde_json::from_str(crate::vectors::file("delivery-order-modes-describe.json").unwrap())
            .unwrap()
    }

    #[test]
    fn advertised_delivery_order_modes_narrow_group_mapping() {
        let vector = &vectors()["accept"];
        let describe = check_describe(&vector["describe"]).unwrap();
        for mode in serde_json::from_value::<Vec<Mode>>(vector["declared"].clone()).unwrap() {
            assert!(mode.is_declared_by(&describe), "{}", mode.as_str());
        }
        for mode in serde_json::from_value::<Vec<Mode>>(vector["undeclared"].clone()).unwrap() {
            assert!(!mode.is_declared_by(&describe), "{}", mode.as_str());
        }
        assert!(!describe.declares(capabilities::RUN_OPS));
        let mut absent = describe.clone();
        absent.delivery_order_modes = None;
        for mode in [
            Mode::ToolResult,
            Mode::TurnSystem,
            Mode::Steer,
            Mode::IdleStart,
        ] {
            assert_eq!(
                mode.is_declared_by(&absent),
                mode.is_declared(&absent.capabilities)
            );
            assert!(mode.is_declared_by(&absent));
        }
        let built = absent.with_delivery_order_modes(vec!["steer".into()]);
        assert_eq!(built, describe);
        assert_eq!(serde_json::to_value(built).unwrap(), vector["describe"]);
    }

    fn refused(case: &str) {
        let vector = &vectors()[case];
        assert_eq!(
            check_describe(&vector["describe"]),
            Err(vec![DescribeProblem::Undecodable(
                vector["problem"].as_str().unwrap().into()
            )]),
            "{case}",
        );
    }

    #[test]
    fn listed_steer_requires_steer_group() {
        refused("steer_without_group");
    }

    #[test]
    fn listed_idle_start_requires_queue_group() {
        refused("idle_start_without_group");
    }

    #[test]
    fn delivery_order_modes_require_delivery_order_group() {
        refused("without_delivery_order");
        let mut raw = vectors()["unknown"]["describe"].clone();
        raw["capabilities"] = json!(["steer", "queue"]);
        assert_eq!(
            check_describe(&raw),
            Err(vec![DescribeProblem::Undecodable(
                problem_names::DELIVERY_ORDER_MODES_UNDECLARED.into()
            )])
        );
    }

    #[test]
    fn delivery_order_modes_ignore_unknown_names() {
        let vector = &vectors()["unknown"];
        let mut raw = vector["describe"].clone();
        let describe = check_describe(&raw).unwrap();
        let modes = serde_json::from_value::<Vec<Mode>>(vector["undeclared"].clone()).unwrap();
        for mode in &modes {
            assert!(!mode.is_declared_by(&describe), "{}", mode.as_str());
        }

        raw["delivery_order_modes"]
            .as_array_mut()
            .unwrap()
            .push(json!("steer"));
        let describe = check_describe(&raw).unwrap();
        for mode in modes {
            assert_eq!(mode.is_declared_by(&describe), mode == Mode::Steer);
        }
    }

    #[test]
    fn delivery_order_modes_reject_duplicates() {
        refused("duplicate");
        let mut raw = vectors()["unknown"]["describe"].clone();
        raw["delivery_order_modes"] = json!(["interrupt", "interrupt"]);
        assert_eq!(
            check_describe(&raw),
            Err(vec![DescribeProblem::Undecodable(
                problem_names::DUPLICATE_DELIVERY_ORDER_MODE.into()
            )])
        );
    }

    #[test]
    fn delivery_order_modes_reject_empty_list() {
        refused("empty");
    }

    #[test]
    fn delivery_order_modes_reject_wrong_wire_types() {
        for invalid in [json!("steer"), json!([3]), json!({"steer": true})] {
            let mut raw = vectors()["accept"]["describe"].clone();
            raw["delivery_order_modes"] = invalid;
            assert!(matches!(check_describe(&raw), Err(problems)
                if matches!(problems.as_slice(), [DescribeProblem::Undecodable(_)])));
        }
    }

    #[test]
    fn listed_modes_never_add_missing_groups() {
        for (mode, group) in [
            (Mode::ToolResult, capabilities::DELIVERY_ORDER),
            (Mode::TurnSystem, capabilities::DELIVERY_ORDER),
            (Mode::Steer, capabilities::STEER),
            (Mode::IdleStart, capabilities::QUEUE),
        ] {
            let describe =
                RoleDescribe::default().with_delivery_order_modes(vec![mode.as_str().into()]);
            assert!(!mode.is_declared_by(&describe));
            let describe =
                describe.with_capabilities(vec![group.into(), capabilities::DELIVERY_ORDER.into()]);
            assert!(mode.is_declared_by(&describe));
        }
    }

    /// `is_declared_by` must hold the narrowing rule on its own, because a
    /// caller may hold a describe answer it never passed through
    /// `check_describe`: listing `steer` or `idle_start` with only the
    /// `delivery_order` group still leaves that mode undeclared.
    #[test]
    fn a_listed_mode_without_its_own_group_stays_undeclared() {
        for mode in [Mode::Steer, Mode::IdleStart] {
            let describe = RoleDescribe::default()
                .with_capabilities(vec![capabilities::DELIVERY_ORDER.into()])
                .with_delivery_order_modes(vec![mode.as_str().into()]);
            assert!(!mode.is_declared_by(&describe), "{}", mode.as_str());
        }
        let unlisted =
            RoleDescribe::default().with_capabilities(vec![capabilities::DELIVERY_ORDER.into()]);
        assert!(Mode::ToolResult.is_declared_by(&unlisted));
        assert!(Mode::TurnSystem.is_declared_by(&unlisted));
        assert!(!Mode::Steer.is_declared_by(&unlisted));
        assert!(!Mode::IdleStart.is_declared_by(&unlisted));
    }
}

#[cfg(test)]
mod tests {
    fn problem_name(problem: &DescribeProblem) -> &'static str {
        match problem {
            DescribeProblem::NotAnObject => "not_an_object",
            DescribeProblem::Undecodable(name) => match name.as_str() {
                problem_names::CALL_YIELD_WITHOUT_PLANS => problem_names::CALL_YIELD_WITHOUT_PLANS,
                problem_names::CALL_YIELD_WITHOUT_DELIVERY_ORDER => {
                    problem_names::CALL_YIELD_WITHOUT_DELIVERY_ORDER
                }
                problem_names::CALL_YIELD_WITHOUT_NOTICES => {
                    problem_names::CALL_YIELD_WITHOUT_NOTICES
                }
                problem_names::CALL_YIELD_WITHOUT_MESSAGE_MARKS => {
                    problem_names::CALL_YIELD_WITHOUT_MESSAGE_MARKS
                }
                problem_names::STOP_ALL_WITHOUT_RUN_CANCEL_BY_INTERRUPT => {
                    problem_names::STOP_ALL_WITHOUT_RUN_CANCEL_BY_INTERRUPT
                }
                problem_names::RUN_CANCEL_UNDECLARED => problem_names::RUN_CANCEL_UNDECLARED,
                problem_names::EMPTY_RUN_CANCEL_AT => problem_names::EMPTY_RUN_CANCEL_AT,
                problem_names::MISSING_RUN_CANCEL_NOW => problem_names::MISSING_RUN_CANCEL_NOW,
                problem_names::DUPLICATE_RUN_CANCEL_AT => problem_names::DUPLICATE_RUN_CANCEL_AT,
                problem_names::RUN_CAUSE_WITHOUT_LIVE_EVENTS => {
                    problem_names::RUN_CAUSE_WITHOUT_LIVE_EVENTS
                }
                _ => "undecodable",
            },
            DescribeProblem::MissingMajor { .. } => "missing_major",
            DescribeProblem::MissingOp(_) => "missing_op",
            DescribeProblem::GroupIncomplete { .. } => "group_incomplete",
            DescribeProblem::MissingSessionCapabilitiesSource => {
                "missing_session_capabilities_source"
            }
            DescribeProblem::MissingMaxBytes => "missing_max_bytes",
            DescribeProblem::MissingRetention => "missing_retention",
            DescribeProblem::InvalidRetention => "invalid_retention",
        }
    }
    use super::*;
    use crate::{capabilities::source, ops, vectors};

    #[test]
    fn head_sender_group_requires_session_head() {
        let mut major = Major::new(
            crate::PROVIDES,
            crate::REQUIRED_OPS.iter().map(|s| s.to_string()).collect(),
            "alpha",
        );
        let mut describe = RoleDescribe::new(vec![major.clone()], "0.6.2")
            .with_capabilities(vec![capabilities::HEAD_SENDER.into()])
            .with_session_capabilities_from("baseline");
        let problems = check_describe(&serde_json::to_value(&describe).unwrap()).unwrap_err();
        assert!(problems.iter().any(|problem| matches!(
            problem,
            DescribeProblem::GroupIncomplete { capability, op }
                if *capability == capabilities::HEAD_SENDER && *op == ops::SESSION_HEAD
        )));
        major.ops.push(ops::SESSION_HEAD.into());
        describe.majors = vec![major];
        assert!(check_describe(&serde_json::to_value(describe).unwrap()).is_ok());
    }

    #[test]
    fn new_management_groups_are_independent_and_never_partial() {
        for (group, op) in [
            (capabilities::RUN_STATUS, crate::ops::RUN_STATUS),
            (capabilities::RUN_CANCEL, crate::ops::RUN_CANCEL),
            (capabilities::RETRACT, crate::ops::SESSION_RETRACT),
        ] {
            let mut major = Major::new(
                crate::PROVIDES,
                crate::REQUIRED_OPS.iter().map(|s| s.to_string()).collect(),
                "alpha",
            );
            let describe = RoleDescribe::new(vec![major.clone()], "0.2.0")
                .with_capabilities(vec![group.into()])
                .with_session_capabilities_from("baseline");
            let problems = check_describe(&serde_json::to_value(&describe).unwrap()).unwrap_err();
            assert!(problems.iter().any(|problem| matches!(problem, DescribeProblem::GroupIncomplete { capability: g, op: missing } if *g == group && *missing == op)), "{group}: {problems:?}");
            major.ops.push(op.into());
            let describe = RoleDescribe::new(vec![major], "0.2.0")
                .with_capabilities(vec![group.into()])
                .with_session_capabilities_from("baseline");
            assert!(
                check_describe(&serde_json::to_value(&describe).unwrap()).is_ok(),
                "{group}"
            );
        }
    }

    #[test]
    fn run_cancel_discovery_validates_points_by_name() {
        let mut ops: Vec<String> = REQUIRED_OPS.iter().map(|op| (*op).into()).collect();
        ops.push(ops::RUN_CANCEL.into());
        let base = RoleDescribe::new(vec![Major::new(PROVIDES, ops, "alpha")], "1")
            .with_capabilities(vec![capabilities::RUN_CANCEL.into()])
            .with_session_capabilities_from(source::BASELINE);
        let check = |value: &RoleDescribe| check_describe(&serde_json::to_value(value).unwrap());
        assert_eq!(check(&base).unwrap().run_cancel, None);
        assert!(serde_json::to_value(&base)
            .unwrap()
            .get("run_cancel")
            .is_none());
        for (points, expected) in [
            (vec![], problem_names::EMPTY_RUN_CANCEL_AT),
            (vec!["step_boundary"], problem_names::MISSING_RUN_CANCEL_NOW),
            (vec!["now", "now"], problem_names::DUPLICATE_RUN_CANCEL_AT),
            (
                vec!["now", "future", "future"],
                problem_names::DUPLICATE_RUN_CANCEL_AT,
            ),
        ] {
            let described = base
                .clone()
                .with_run_cancel(points.into_iter().map(String::from).collect());
            let problems = check(&described).unwrap_err();
            assert!(
                problems.contains(&DescribeProblem::Undecodable(expected.into())),
                "{problems:?}"
            );
        }
        let described = base.clone().with_run_cancel(vec![
            "now".into(),
            "step_boundary".into(),
            "future".into(),
        ]);
        assert_eq!(check(&described).unwrap(), described);
        assert_eq!(
            check(&described.with_capabilities(vec![])),
            Err(vec![DescribeProblem::Undecodable(
                problem_names::RUN_CANCEL_UNDECLARED.into()
            )])
        );
        let decoded: RunCancel =
            serde_json::from_value(serde_json::json!({"at": ["now"], "future": true})).unwrap();
        assert_eq!(decoded, RunCancel::new(vec![]).with_at(vec!["now".into()]));
    }

    #[test]
    fn describe_vectors_check_as_recorded() {
        let file = vectors::load("role-describe.json");
        for case in vectors::cases(&file, "accepted") {
            let name = case["name"].as_str().unwrap();
            let describe = check_describe(&case["answer"])
                .unwrap_or_else(|problems| panic!("{name}: {problems:?}"));
            for capability in case["declares"].as_array().unwrap() {
                assert!(describe.declares(capability.as_str().unwrap()), "{name}");
            }
        }
        for case in vectors::cases(&file, "stability_defaults") {
            let name = case["name"].as_str().unwrap();
            let describe = check_describe(&case["answer"])
                .unwrap_or_else(|problems| panic!("{name}: {problems:?}"));
            let major = describe.major(PROVIDES).unwrap();
            assert_eq!(
                major.stability,
                case["stability"].as_str().unwrap(),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "canonical") {
            vectors::round_trip::<RoleDescribe>(case["name"].as_str().unwrap(), &case["answer"]);
        }
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            let problems = check_describe(&case["answer"]).expect_err(name);
            let names: Vec<&str> = problems.iter().map(problem_name).collect();
            let expected = case["problem"].as_str().unwrap();
            assert!(names.contains(&expected), "{name}: {names:?}");
        }
    }

    #[test]
    fn run_cancel_describe_vectors_round_trip_and_refuse_by_name() {
        let file = vectors::load("run-cancel-describe.json");
        for case in vectors::cases(&file, "canonical") {
            let name = case["name"].as_str().unwrap();
            let decoded: RoleDescribe = vectors::round_trip(name, &case["answer"]);
            assert_eq!(check_describe(&case["answer"]).unwrap(), decoded, "{name}");
        }
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            let expected = match case["problem"].as_str().unwrap() {
                problem_names::RUN_CANCEL_UNDECLARED => problem_names::RUN_CANCEL_UNDECLARED,
                problem_names::EMPTY_RUN_CANCEL_AT => problem_names::EMPTY_RUN_CANCEL_AT,
                problem_names::MISSING_RUN_CANCEL_NOW => problem_names::MISSING_RUN_CANCEL_NOW,
                problem_names::DUPLICATE_RUN_CANCEL_AT => problem_names::DUPLICATE_RUN_CANCEL_AT,
                other => panic!("{name}: unknown problem {other}"),
            };
            let problems = check_describe(&case["answer"]).expect_err(name);
            assert!(
                problems.contains(&DescribeProblem::Undecodable(expected.into())),
                "{name}: {problems:?}"
            );
        }
    }

    #[test]
    fn unknown_stability_extra_majors_ops_and_capabilities_are_tolerated() {
        let answer = serde_json::json!({
            "majors": [
                {"version": "llm-runner/v1", "stability": "experimental",
                 "ops": ["role.describe", "session.baseline", "vendor.extra"]},
                {"version": "llm-runner/v2", "stability": "alpha", "ops": ["role.describe"]}
            ],
            "implementation_version": "1.2.3",
            "capabilities": ["vendor_feature"],
            "session_capabilities_from": "somewhere_new",
            "future_field": {}
        });
        let describe = check_describe(&answer).unwrap();
        let v1 = describe.major(PROVIDES).unwrap();
        assert_eq!(v1.stability(), Stability::Other("experimental".into()));
        assert!(describe.declares("vendor_feature"));
        assert_eq!(
            describe.major("llm-runner/v2").unwrap().stability(),
            Stability::Alpha
        );
    }

    #[test]
    fn builders_produce_a_checkable_answer_and_name_missing_capabilities() {
        let describe = RoleDescribe::new(
            vec![Major::new(
                PROVIDES,
                vec![
                    ops::ROLE_DESCRIBE.into(),
                    ops::SESSION_BASELINE.into(),
                    ops::SESSION_SEND.into(),
                ],
                "alpha",
            )],
            "0.0.1",
        )
        .with_capabilities(vec![capabilities::STEER.into()])
        .with_session_capabilities_from(source::BASELINE);
        let raw = serde_json::to_value(&describe).unwrap();
        assert_eq!(check_describe(&raw).unwrap(), describe);
        assert_eq!(
            describe.missing(&[capabilities::STEER, capabilities::TRANSCRIPT_READS]),
            vec![capabilities::TRANSCRIPT_READS]
        );
    }

    #[test]
    fn steer_receipt_vectors_round_trip_and_defaults() {
        let file = vectors::load("role-describe.json");
        let mut seen_guaranteed = false;
        let mut seen_confirm = false;
        let mut seen_absent = false;
        for case in vectors::cases(&file, "canonical") {
            let name = case["name"].as_str().unwrap();
            let describe: RoleDescribe = vectors::round_trip(name, &case["answer"]);
            match describe.steer_receipt.as_deref() {
                Some("guaranteed") => seen_guaranteed = true,
                Some("confirm") => seen_confirm = true,
                None => seen_absent = true,
                _ => {}
            }
        }
        assert!(seen_guaranteed);
        assert!(seen_confirm);
        assert!(seen_absent);

        let default_describe = RoleDescribe::new(vec![], "1.0");
        assert_eq!(default_describe.steer_receipt, None);
        assert_eq!(default_describe.steer_receipt(), steer_receipt::GUARANTEED);

        let confirm_describe = default_describe
            .clone()
            .with_steer_receipt(steer_receipt::CONFIRM);
        assert_eq!(
            confirm_describe.steer_receipt.as_deref(),
            Some(steer_receipt::CONFIRM)
        );
        assert_eq!(confirm_describe.steer_receipt(), steer_receipt::CONFIRM);
    }
}

#[cfg(test)]
mod retention_tests {
    use super::{check_describe, DescribeProblem, Major, Retention, RoleDescribe};
    use crate::{capabilities, ops, PROVIDES, REQUIRED_OPS};

    #[test]
    fn retention_group_requires_positive_limits() {
        let mut ops: Vec<String> = REQUIRED_OPS.iter().map(|s| (*s).into()).collect();
        ops.push(ops::SESSION_SEND.into());
        let base = RoleDescribe::new(vec![Major::new(PROVIDES, ops, "alpha")], "1")
            .with_capabilities(vec![capabilities::RETENTION.into()])
            .with_session_capabilities_from("baseline");
        let check = |value: &RoleDescribe| check_describe(&serde_json::to_value(value).unwrap());
        assert_eq!(check(&base), Err(vec![DescribeProblem::MissingRetention]));
        assert_eq!(
            check(&base.clone().with_retention(0, 0)),
            Err(vec![DescribeProblem::InvalidRetention])
        );
        let described = check(&base.with_retention(u64::MAX, 0)).unwrap();
        assert_eq!(described.retention, Some(Retention::new(u64::MAX, 0)));
        let decoded: Retention =
            serde_json::from_str(r#"{"max_seconds":3,"delete_within_ms":100,"future":true}"#)
                .unwrap();
        assert_eq!(decoded, Retention::new(3, 100));
    }
}

#[cfg(test)]
mod input_cap_tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn input_token_cap_describe_group_requires_send_and_no_other_optional_group() {
        let file = vectors::load("input-token-cap-describe.json");
        let answer: RoleDescribe = vectors::round_trip("cap describe", &file["accept"]);
        assert_eq!(check_describe(&file["accept"]).unwrap(), answer);
        assert!(answer.declares(capabilities::INPUT_TOKEN_CAP));
        assert_eq!(
            check_describe(&file["refuse"]),
            Err(vec![DescribeProblem::GroupIncomplete {
                capability: capabilities::INPUT_TOKEN_CAP,
                op: crate::ops::SESSION_SEND
            }])
        );
    }
}

#[cfg(test)]
mod call_identity_tests {
    use super::*;
    use crate::vectors;

    /// `call_identity` is discovered beside either display surface, is
    /// refused by name when neither is declared, and a runner without it is usable.
    #[test]
    fn call_identity_rides_on_a_display_surface() {
        let file = vectors::load("call-identity.json");
        let describe = &file["describe"];
        for name in ["accept", "live_only"] {
            let answer: RoleDescribe = vectors::round_trip(name, &describe[name]);
            assert_eq!(check_describe(&describe[name]).unwrap(), answer, "{name}");
            assert!(answer.declares(capabilities::CALL_IDENTITY), "{name}");
        }
        let refused = &describe["without_display"];
        assert_eq!(
            refused["problem"],
            problem_names::CALL_IDENTITY_WITHOUT_DISPLAY
        );
        assert_eq!(
            check_describe(&refused["answer"]),
            Err(vec![DescribeProblem::Undecodable(
                problem_names::CALL_IDENTITY_WITHOUT_DISPLAY.into()
            )])
        );
        let without = check_describe(&describe["without"]).unwrap();
        assert!(!without.declares(capabilities::CALL_IDENTITY));
        assert_eq!(
            capabilities::group_ops(capabilities::CALL_IDENTITY),
            Some(&[][..])
        );
    }
}

#[cfg(test)]
mod adopt_tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn adopt_describe_group_requires_session_adopt_and_is_never_assumed() {
        let file = vectors::load("adopt-describe.json");
        let answer: RoleDescribe = vectors::round_trip("adopt describe", &file["accept"]);
        assert_eq!(check_describe(&file["accept"]).unwrap(), answer);
        assert!(answer.declares(capabilities::ADOPT));
        assert_eq!(
            check_describe(&file["refuse"]),
            Err(vec![DescribeProblem::GroupIncomplete {
                capability: capabilities::ADOPT,
                op: crate::ops::SESSION_ADOPT
            }])
        );
        // A runner without the group is a usable runner; a consumer that
        // needs adoption finds the group missing and never sends the op.
        let without = check_describe(&file["without"]).unwrap();
        assert!(!without.declares(capabilities::ADOPT));
        assert_eq!(
            without.missing(&[capabilities::ADOPT]),
            vec![capabilities::ADOPT]
        );
        assert!(!without
            .major(PROVIDES)
            .unwrap()
            .serves(crate::ops::SESSION_ADOPT));
    }
}

#[cfg(test)]
mod tool_call_group_tests {
    use super::*;
    use crate::vectors;

    /// A describe vector file pinning one optional group: its discovery, the
    /// refusal of a declaration missing `missing_op`, and a usable answer
    /// without the group.
    fn group_describe(file: &str, group: &'static str, missing_op: &'static str) {
        let file = vectors::load(file);
        let answer: RoleDescribe = vectors::round_trip(group, &file["accept"]);
        assert_eq!(check_describe(&file["accept"]).unwrap(), answer);
        assert!(answer.declares(group));
        assert_eq!(
            check_describe(&file["refuse"]),
            Err(vec![DescribeProblem::GroupIncomplete {
                capability: group,
                op: missing_op
            }])
        );
        let without = check_describe(&file["without"]).unwrap();
        assert!(!without.declares(group));
        assert_eq!(without.missing(&[group]), vec![group]);
    }

    #[test]
    fn delivery_order_describe_group_requires_session_send() {
        group_describe(
            "delivery-order-describe.json",
            capabilities::DELIVERY_ORDER,
            crate::ops::SESSION_SEND,
        );
    }

    #[test]
    fn run_tool_calls_describe_group_requires_its_op() {
        group_describe(
            "run-tool-calls-describe.json",
            capabilities::RUN_TOOL_CALLS,
            crate::ops::RUN_TOOL_CALLS,
        );
    }

    #[test]
    fn send_status_describe_group_requires_its_op() {
        group_describe(
            "send-status-describe.json",
            capabilities::SEND_STATUS,
            crate::ops::SEND_STATUS,
        );
    }
}

#[cfg(test)]
mod directive_group_tests {
    use super::*;

    #[test]
    fn directive_and_interrupt_reason_groups_require_only_their_ops() {
        for (group, required) in [
            (
                capabilities::TURN_DIRECTIVES,
                &[crate::ops::SESSION_SEND, crate::ops::RUN_RESULT][..],
            ),
            (
                capabilities::INTERRUPT_REASON,
                &[crate::ops::RUN_RESULT][..],
            ),
        ] {
            let mut ops: Vec<String> = crate::REQUIRED_OPS.iter().map(|op| (*op).into()).collect();
            ops.extend(required.iter().map(|op| (*op).into()));
            let describe =
                RoleDescribe::new(vec![Major::new(crate::PROVIDES, ops, "alpha")], "0.3.5")
                    .with_capabilities(vec![group.into()])
                    .with_session_capabilities_from("baseline");
            assert!(check_describe(&serde_json::to_value(&describe).unwrap()).is_ok());
            for op in required {
                let mut missing = describe.clone();
                missing.majors[0].ops.retain(|value| value != op);
                let problems = check_describe(&serde_json::to_value(missing).unwrap()).unwrap_err();
                assert!(problems.iter().any(|problem| matches!(problem, DescribeProblem::GroupIncomplete { capability, op: missing } if *capability == group && missing == op)));
            }
        }
    }
}

#[cfg(test)]
mod submit_streamed_siblings_tests {
    use super::*;

    #[test]
    fn check_describe_submit_streamed_siblings_requires_structured_output() {
        let mut ops: Vec<String> = crate::REQUIRED_OPS.iter().map(|op| (*op).into()).collect();
        ops.push(crate::ops::SESSION_SEND.into());
        let mut describe =
            RoleDescribe::new(vec![Major::new(crate::PROVIDES, ops, "alpha")], "0.7.16")
                .with_capabilities(vec![capabilities::SUBMIT_STREAMED_SIBLINGS.into()])
                .with_session_capabilities_from("baseline");
        assert_eq!(
            check_describe(&serde_json::to_value(&describe).unwrap()).unwrap_err(),
            vec![DescribeProblem::Undecodable(
                problem_names::SUBMIT_STREAMED_SIBLINGS_WITHOUT_STRUCTURED_OUTPUT.into()
            )]
        );
        describe
            .capabilities
            .push(capabilities::STRUCTURED_OUTPUT.into());
        assert!(check_describe(&serde_json::to_value(&describe).unwrap()).is_ok());
        assert_eq!(
            capabilities::group_ops(capabilities::SUBMIT_STREAMED_SIBLINGS),
            Some(&[crate::ops::SESSION_SEND][..])
        );
        let mut strict = describe.clone();
        strict
            .capabilities
            .retain(|group| group != capabilities::SUBMIT_STREAMED_SIBLINGS);
        assert!(check_describe(&serde_json::to_value(strict).unwrap()).is_ok());
        describe.majors[0]
            .ops
            .retain(|op| op != crate::ops::SESSION_SEND);
        assert!(check_describe(&serde_json::to_value(describe).unwrap())
            .unwrap_err()
            .contains(&DescribeProblem::GroupIncomplete {
                capability: capabilities::SUBMIT_STREAMED_SIBLINGS,
                op: crate::ops::SESSION_SEND,
            }));
    }
}

#[cfg(test)]
mod structured_output_per_session_tests {
    use super::*;

    #[test]
    fn check_describe_structured_output_per_session_requires_structured_output() {
        let describe = RoleDescribe::new(
            vec![Major::new(
                crate::PROVIDES,
                crate::REQUIRED_OPS
                    .iter()
                    .map(|op| (*op).into())
                    .chain([crate::ops::SESSION_SEND.into()])
                    .collect(),
                "alpha",
            )],
            "0.7.16",
        )
        .with_session_capabilities_from("baseline")
        .with_capabilities(vec![capabilities::STRUCTURED_OUTPUT_PER_SESSION.into()]);
        assert_eq!(
            check_describe(&serde_json::to_value(&describe).unwrap()).unwrap_err(),
            vec![DescribeProblem::Undecodable(
                problem_names::STRUCTURED_OUTPUT_PER_SESSION_WITHOUT_STRUCTURED_OUTPUT.into()
            )]
        );
        let paired = describe.with_capabilities(vec![
            capabilities::STRUCTURED_OUTPUT_PER_SESSION.into(),
            capabilities::STRUCTURED_OUTPUT.into(),
        ]);
        assert!(check_describe(&serde_json::to_value(paired).unwrap()).is_ok());
        assert_eq!(
            capabilities::group_ops(capabilities::STRUCTURED_OUTPUT_PER_SESSION),
            Some(&[crate::ops::SESSION_SEND][..])
        );
    }
}

#[cfg(test)]
mod step_hooks_tests {
    use super::*;
    use serde_json::json;

    fn raw(step_hooks: Option<Value>) -> Value {
        let mut raw = json!({
            "majors": [{"version": crate::PROVIDES, "ops": crate::REQUIRED_OPS,
                        "stability": "stable"}],
            "implementation_version": "test",
            "capabilities": [],
            "session_capabilities_from": "baseline"
        });
        if let Some(hooks) = step_hooks {
            raw["step_hooks"] = hooks;
        }
        raw
    }

    #[test]
    fn absent_step_hooks_serve_no_hook_and_remain_omitted() {
        let describe = check_describe(&raw(None)).unwrap();
        assert_eq!(describe.step_hooks, None);
        assert!(!describe.serves_step_hooks([step_hooks::PRE_USER]));
        assert!(describe.serves_step_hooks([]));
        assert!(serde_json::to_value(&describe)
            .unwrap()
            .get("step_hooks")
            .is_none());
    }

    #[test]
    fn listed_hooks_are_served_and_others_are_not() {
        let describe =
            check_describe(&raw(Some(json!([step_hooks::PRE_USER, "future_hook"])))).unwrap();
        assert!(describe.serves_step_hooks([step_hooks::PRE_USER]));
        assert!(!describe.serves_step_hooks([step_hooks::PRE_USER, step_hooks::PRE_TOOL]));
        assert_eq!(
            describe.step_hooks.as_deref(),
            Some(&["pre_user".to_owned(), "future_hook".to_owned()][..])
        );
        let built = RoleDescribe::new(Vec::new(), "test")
            .with_step_hooks(step_hooks::KNOWN.iter().copied());
        assert!(built.serves_step_hooks(step_hooks::KNOWN.iter().copied()));
    }

    #[test]
    fn empty_step_hooks_serve_no_hook() {
        let describe = check_describe(&raw(Some(json!([])))).unwrap();
        assert!(!describe.serves_step_hooks([step_hooks::PRE_USER]));
    }

    #[test]
    fn an_unknown_name_matches_only_itself() {
        let describe = check_describe(&raw(Some(json!(["future_hook"])))).unwrap();
        assert!(describe.serves_step_hooks(["future_hook"]));
        for hook in step_hooks::KNOWN {
            assert!(!describe.serves_step_hooks([*hook]));
        }
    }

    #[test]
    fn a_duplicate_step_hook_is_a_problem() {
        let problems = check_describe(&raw(Some(json!(["pre_user", "pre_user"])))).unwrap_err();
        assert!(problems.contains(&DescribeProblem::Undecodable(
            problem_names::DUPLICATE_STEP_HOOK.into()
        )));
    }

    #[test]
    fn a_non_array_step_hooks_does_not_decode() {
        assert!(check_describe(&raw(Some(json!("pre_user")))).is_err());
    }
}

#[cfg(test)]
mod fork_answer_tests {
    use super::*;
    use serde_json::json;

    fn describe() -> RoleDescribe {
        let ops = crate::REQUIRED_OPS
            .iter()
            .copied()
            .chain(
                capabilities::group_ops(capabilities::FORK_ANSWER)
                    .unwrap()
                    .iter()
                    .copied(),
            )
            .map(str::to_owned)
            .collect();
        RoleDescribe::new(vec![Major::new(crate::PROVIDES, ops, "alpha")], "test")
            .with_capabilities(vec![capabilities::FORK_ANSWER.into()])
            .with_session_capabilities_from("baseline")
            .with_fork_idle_s(30)
    }

    #[test]
    fn fork_answer_idle_fact_is_positive_and_present_exactly_with_group() {
        let mut raw = serde_json::to_value(describe()).unwrap();
        assert!(check_describe(&raw).is_ok());
        raw.as_object_mut().unwrap().remove("fork_idle_s");
        assert_eq!(
            check_describe(&raw).unwrap_err(),
            vec![DescribeProblem::Undecodable(
                problem_names::FORK_IDLE_S_MISSING.into()
            )]
        );
        raw["fork_idle_s"] = json!(30);
        raw["capabilities"] = json!([]);
        assert_eq!(
            check_describe(&raw).unwrap_err(),
            vec![DescribeProblem::Undecodable(
                problem_names::FORK_IDLE_S_UNDECLARED.into()
            )]
        );
        raw.as_object_mut().unwrap().remove("fork_idle_s");
        assert!(check_describe(&raw).is_ok());
        raw["capabilities"] = json!([capabilities::FORK_ANSWER]);
        for invalid in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!(true),
            json!("30"),
            json!(null),
            json!([]),
            json!({}),
        ] {
            raw["fork_idle_s"] = invalid.clone();
            assert_eq!(
                check_describe(&raw).unwrap_err(),
                vec![DescribeProblem::Undecodable(
                    problem_names::FORK_IDLE_S_INVALID.into()
                )],
                "{invalid}"
            );
        }
        for valid in [1, u64::MAX] {
            raw["fork_idle_s"] = json!(valid);
            assert_eq!(check_describe(&raw).unwrap().fork_idle_s, Some(valid));
        }
    }

    #[test]
    fn fork_answer_requires_all_three_management_ops() {
        let describe = describe();
        for missing in [
            crate::ops::SESSION_FORK_PREPARE,
            crate::ops::SESSION_FORK_ASK,
            crate::ops::SESSION_FORK_END,
        ] {
            let mut partial = describe.clone();
            partial.majors[0].ops.retain(|op| op != missing);
            assert_eq!(
                check_describe(&serde_json::to_value(partial).unwrap()).unwrap_err(),
                vec![DescribeProblem::GroupIncomplete {
                    capability: capabilities::FORK_ANSWER,
                    op: missing
                }]
            );
        }
    }
}

#[cfg(test)]
mod send_ids_tests {
    use super::*;

    #[test]
    fn send_ids_requires_both_prerequisites_without_changing_legacy_declarations() {
        let groups = [
            capabilities::HISTORY_DISPLAY,
            capabilities::LIVE_EVENTS,
            capabilities::MESSAGE_SENDERS,
            capabilities::READ_DISPLAY_FACTS,
            capabilities::SEND_IDS,
        ];
        let mut ops = crate::REQUIRED_OPS.to_vec();
        for group in groups {
            ops.extend_from_slice(capabilities::group_ops(group).unwrap());
        }
        let complete = serde_json::json!({
            "majors":[{"version":crate::PROVIDES,"ops":ops,"stability":"stable"}],
            "implementation_version":"test",
            "capabilities":groups,
            "session_capabilities_from":"baseline"
        });
        assert!(
            check_describe(&complete).is_ok(),
            "{:?}",
            check_describe(&complete)
        );
        for dependency in [
            capabilities::MESSAGE_SENDERS,
            capabilities::READ_DISPLAY_FACTS,
        ] {
            let mut missing = complete.clone();
            missing["capabilities"]
                .as_array_mut()
                .unwrap()
                .retain(|group| group != dependency);
            assert!(check_describe(&missing)
                .unwrap_err()
                .contains(&DescribeProblem::Undecodable(format!(
                    "send_ids_without_{dependency}"
                ))));
        }
        let mut legacy = complete;
        legacy["capabilities"]
            .as_array_mut()
            .unwrap()
            .retain(|group| group != capabilities::SEND_IDS);
        assert!(check_describe(&legacy).is_ok());
    }
}
