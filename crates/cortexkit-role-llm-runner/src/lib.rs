//! Wire contract of the `llm-runner/v1` role.
//!
//! An LLM runner is a module that runs model sessions: it calls the session's
//! providers (compaction, step transforms, tools) and serves the reads and
//! prompts it declares. It claims the role by listing [`PROVIDES`] in its
//! manifest's `capabilities.provides` and answers every op in
//! [`REQUIRED_OPS`]. Everything else is a declared capability group
//! ([`capabilities`]): a runner serves a group only if `role.describe` lists
//! it, and serves every op of a group it lists.
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document: it
//! marks every item pinned or open, and defines the role's terms
//! (admission, baseline, rungs, hooks, scopes). The types here are the wire
//! shapes it describes. Shapes a consumer decodes are lenient (unknown fields ignored,
//! enumerations decoded as open strings); request shapes whose missing field
//! would silently change the question are strict.
//!
//! The session an op acts on is the session the route is bound to; no
//! request in this crate names its session.

#![forbid(unsafe_code)]

pub mod adopt;
pub mod attachments;
pub mod baseline;
pub mod compaction;
pub mod content_digest;
pub mod delivery_order;
pub mod describe;
pub mod directives;
pub mod display;
pub mod errors;
pub mod history;
pub mod live;
pub mod output_refs;
pub mod points;
pub mod read;
pub mod retract;
pub mod run;
pub mod send;
pub mod send_status;
pub mod serializer_profile;
pub mod session_change;
pub mod session_ref;
pub mod structured_output;
pub mod subscribe;
pub mod tags;
pub mod tool_calls;
pub mod usage;
pub mod user_commands;
#[cfg(any(test, feature = "test-vectors"))]
pub mod vectors;
pub mod wire;

pub use run::{
    COMPACTION_WAIT_REPORT_AFTER_MS, RUN_REASON_COMPACTION_WAIT, RUN_REASON_CONTEXT_CEILING_WAIT,
};

pub use session_ref::session_ref;

/// The role name as `role.describe` reports it.
pub const ROLE: &str = "llm-runner";

/// The only major this crate defines.
pub const VERSION: &str = "v1";

/// The string a runner lists in its manifest's `capabilities.provides`.
pub const PROVIDES: &str = "llm-runner/v1";

/// The largest encoded frame the transport delivers, in bytes, its 21-byte
/// header included: the limit a `reply_too_large` refusal names
/// (`errors::ReplyTooLargeDetail::limit`). A reply whose complete encoded
/// frame is longer is refused, never cut short.
///
/// It equals `subc_protocol::MAX_DELIVERED_FRAME_LEN`, the subconscious
/// daemon's limit on a frame it delivers. This crate states the number
/// itself rather than depending on `subc-protocol`, so the two must be kept
/// equal by hand.
pub const TRANSPORT_FRAME_LIMIT: usize = 4_194_304;

/// Names of the ops a runner serves.
pub mod ops {
    pub const SESSION_OUTPUT_REFS: &str = "session.output_refs";
    pub const SESSION_OUTPUT_REF_READ: &str = "session.output_ref_read";
    pub const SESSION_UPLOAD_BEGIN: &str = "session.upload_begin";
    pub const SESSION_UPLOAD_APPEND: &str = "session.upload_append";
    pub const SESSION_UPLOAD_STATUS: &str = "session.upload_status";
    pub const SESSION_UPLOAD_COMMIT: &str = "session.upload_commit";
    pub const SESSION_MEDIA_READ: &str = "session.media_read";
    /// Discovery before use: the role's majors, ops and capabilities.
    /// Required.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// The session's current baseline, answered only to the session's owner.
    /// Required.
    pub const SESSION_BASELINE: &str = "session.baseline";
    /// Invoke a command made available at admission; only the owner may invoke it.
    pub const SESSION_USER_COMMAND: &str = "session.user_command";
    /// Read person-only command state; admitted only from the session's owner.
    pub const SESSION_USER_COMMAND_STATUS: &str = "session.user_command_status";
    /// A page of the transcript. In `transcript_reads`, and in every group
    /// that adds fields to a page.
    pub const SESSION_READ: &str = "session.read";
    /// The transcript's head metadata, without message bodies. In
    /// `transcript_reads`.
    pub const SESSION_HEAD: &str = "session.head";
    /// The session's whole surviving history, paged by opaque cursors,
    /// including what lies before the current lineage. On demand only. In
    /// `host_history`.
    pub const SESSION_HISTORY: &str = "session.history";
    /// A run's terminal state and final message. In `run_ops`.
    pub const RUN_RESULT: &str = "run.result";
    /// A run's current state without reading its transcript. In `run_status`.
    pub const RUN_STATUS: &str = "run.status";
    /// Stop an active or paused run. In `run_cancel`.
    pub const RUN_CANCEL: &str = "run.cancel";
    /// Withdraw a submission that has not started. In `retract`.
    pub const SESSION_RETRACT: &str = "session.retract";
    /// The session's event stream, attachable at a page's `head`. In
    /// `streaming`.
    pub const SESSION_SUBSCRIBE: &str = "session.subscribe";
    /// A prompt from the session's owner, with a delivery mode. In `steer`,
    /// `queue` and `interrupt`.
    pub const SESSION_SEND: &str = "session.send";
    /// A mid-session change: fetch a new plan's items and hold them as the
    /// pending change, with its policy and generation. In `session_change`.
    pub const SESSION_REFRESH: &str = "session.refresh";
    /// A new policy for the pending change. In `session_change`.
    pub const SESSION_REFRESH_POLICY: &str = "session.refresh_policy";
    /// Drop the frozen prefix and apply any pending change on the next step,
    /// whatever its policy. In `session_change`.
    pub const SESSION_FLUSH_PREFIX: &str = "session.flush_prefix";
    /// Make the caller the owner of a session the host started with a plan
    /// already applied, without a prompt: no run starts and nothing is
    /// written to the transcript. In `adopt`.
    pub const SESSION_ADOPT: &str = "session.adopt";
    /// The exact stored content of the message part a runner tag names, or a
    /// character range of it. In `tags`.
    pub const SESSION_RESOLVE: &str = "session.resolve";
    /// Every tool call a run made, in order, without message bodies. In
    /// `run_tool_calls`.
    pub const RUN_TOOL_CALLS: &str = "run.tool_calls";
    /// What a re-send of a `send_id` would answer, from the runner's durable
    /// record of the send, without its body. In `send_status`.
    pub const SEND_STATUS: &str = "send.status";
}

/// Ops every `llm-runner/v1` module must list in `role.describe`. A consumer
/// refuses a module missing any of them, by name, before routing anything.
pub const REQUIRED_OPS: &[&str] = &[ops::ROLE_DESCRIBE, ops::SESSION_BASELINE];

/// The provider manifest capability required to admit preset-less system text.
/// Read from the daemon catalog, not from a provider's crate version.
pub const SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY: &str = "system-text-default/v1";

/// Capability names: the module-level groups `role.describe` declares, and
/// the session-level capabilities a session's admission reply or baseline
/// declares.
pub mod capabilities {
    use crate::ops;

    /// Immutable output references captured against the writing request.
    /// Requires tags and applies only where the frozen plan renders tags.
    pub const OUTPUT_REFS: &str = "output_refs";

    /// Session-scoped uploads, upload references on sends and media by digest.
    pub const ATTACHMENTS: &str = "attachments";

    /// Tail, range and after-message reads, head metadata and `lineage_id`
    /// on every page.
    pub const TRANSCRIPT_READS: &str = "transcript_reads";
    /// `session.history`: the session's surviving history from its first
    /// message, before the current lineage included, paged by opaque
    /// cursors and served on demand (an app's reader, raw recovery). A
    /// runner that is its own host declares it; a runner on a host may, and
    /// then serves history from the host's own storage.
    pub const HOST_HISTORY: &str = "host_history";
    /// `session.history` with `form: "display"`: each entry as a canonical
    /// row (`display::DisplayRow`), cut only by the caller's `limits`.
    pub const HISTORY_DISPLAY: &str = "history_display";
    /// The pinned live events on `session.subscribe` (`live::LiveEvent`):
    /// durable message, tool and run lifecycle events, display deltas with
    /// `mid` and `part`, `seq` on every frame, and the attach replay of the
    /// step in flight.
    pub const LIVE_EVENTS: &str = "live_events";
    /// `client_message_id` on `session.send`, kept beside the user message
    /// and shown on its display row. A caller never sends the member to a
    /// runner without the group, whose strict decoding may refuse it.
    pub const CLIENT_MESSAGE_ID: &str = "client_message_id";
    /// `send_id` on the live `run_started` event: the send that started the
    /// run, left out only for a turn the host started with no send. On a
    /// runner declaring it, an absent `send_id` means no send started the
    /// run; on any other runner it may only mean an older runner. Requires
    /// [`LIVE_EVENTS`], whose event it extends.
    pub const RUN_CAUSE: &str = "run_cause";
    /// `run.result` and per-message run attribution.
    pub const RUN_OPS: &str = "run_ops";
    /// Poll a run's current state, independent of transcript reads.
    pub const RUN_STATUS: &str = "run_status";
    /// Held tool calls on run status and status events; requires `run_status`.
    pub const HELD_CALLS: &str = "held_calls";
    /// Cancel an active or paused run.
    pub const RUN_CANCEL: &str = "run_cancel";
    /// Withdraw a queued submission or a pending steer on a confirm runner.
    pub const RETRACT: &str = "retract";
    /// Per tool call: the module it was dispatched to, its `call_key`, and
    /// whether it is still indeterminate.
    pub const DISPATCH_ATTRIBUTION: &str = "dispatch_attribution";
    /// `session.subscribe` with the snapshot handoff from a page's `head`.
    pub const STREAMING: &str = "streaming";
    /// `session.read` with `view: "model"`, the post-compaction view.
    pub const MODEL_VIEW: &str = "model_view";
    /// `session.send` with `delivery: "steer"` from the session's owner.
    pub const STEER: &str = "steer";
    /// `session.send` with `delivery: "queue"` from the session's owner.
    pub const QUEUE: &str = "queue";
    /// `session.send` with `delivery: "interrupt"` from the session's owner:
    /// cancel the running turn and start the message. A separate group from
    /// `steer` and `queue`, so a runner that cannot abort a model stream can
    /// still declare those two.
    pub const INTERRUPT: &str = "interrupt";
    /// Accept a fetch plan on a session's first `session.send`, freeze the
    /// fetched manifest, and report its composition digest and items through
    /// `session.baseline`. Identical later plans do not fetch again; different
    /// plans are refused to preserve the session's frozen manifest.
    pub const PLANS: &str = "plans";
    /// Admit preset-less system text only for a provider declaring
    /// `system-text-default/v1`, and forward the item unchanged.
    pub const SYSTEM_TEXT_DEFAULT: &str = "system_text_default";
    /// Hosting a compaction provider: the runner half of
    /// compaction-provider 0.3.0 §3 (Setup, advance notices, views applied at
    /// step boundaries, rides, the ceiling and native markers). Requires
    /// [`TAGS`], whose ops it lists: an answer declaring it without `tags`
    /// is malformed. The runner also implements the `context_ceiling`
    /// refusal and run state and the `context_ceiling_wait` status. A runner
    /// that hosts no sessions of its own declares none of it.
    pub const COMPACTION: &str = "compaction";
    /// Honouring a compaction provider's Setup checkpoint
    /// (`tags::SetupCheckpoint`, `{tag_checkpoint, cut}`): the runner
    /// anchors the checkpoint at its cut message when no in-effect marker
    /// carries a checkpoint for the session, and numbers from the session
    /// start with it, on a new lineage, once that message is gone. A
    /// provider refuses Setup for a session it moved from standalone Magic
    /// Context on a runner without it. Requires [`COMPACTION`]: an answer
    /// declaring it without `compaction` is malformed. Adds no op of its
    /// own.
    pub const TAG_CHECKPOINT: &str = "tag_checkpoint";
    /// Mid-session changes from the session's owner: `session.refresh`,
    /// `session.refresh_policy` and `session.flush_prefix`. A runner without
    /// it gets no mid-session changes; its owner applies them at the next
    /// session instead.
    pub const SESSION_CHANGE: &str = "session_change";
    /// Caller-chosen whole-session transcript retention, with limits in
    /// `role.describe.retention`.
    pub const RETENTION: &str = "retention";
    /// Durable, lower-only session input-token budgets on `session.send`.
    pub const INPUT_TOKEN_CAP: &str = "input_token_cap";
    /// Positive model-response limit in `stop_when.max_steps`.
    pub const MAX_STEPS: &str = "max_steps";
    /// Positive per-response output cap in `generation.max_output_tokens`.
    pub const MAX_OUTPUT_TOKENS: &str = "max_output_tokens";
    /// `tools: []` suppresses all tools, including plan tools and built-ins.
    pub const NO_TOOLS: &str = "no_tools";
    /// Stable trailing tags parsed from a completed run's stored final text.
    pub const TURN_DIRECTIVES: &str = "turn_directives";
    /// An interrupted run records why it was sealed (`reason`), durably. A run
    /// sealed before the runner adopted this group has none, and none is guessed.
    pub const INTERRUPT_REASON: &str = "interrupt_reason";
    /// Schema-shaped run output on `session.send` (`structured_output`),
    /// reported on `run.result` and `run.status`.
    pub const STRUCTURED_OUTPUT: &str = "structured_output";
    /// `session.adopt`: the owner takes over a session the host started
    /// with a plan already applied, before or during the person's own turns,
    /// without sending a prompt.
    pub const ADOPT: &str = "adopt";
    /// Runner-rendered tags: the `§N§` each counted part shows, computed
    /// from the transcript, on `session.read` entries and resolved in the
    /// current lineage by `session.resolve`.
    pub const TAGS: &str = "tags";
    /// `run.tool_calls`: a run's tool calls as a paged list, read without
    /// decoding message bodies.
    pub const RUN_TOOL_CALLS: &str = "run_tool_calls";
    /// Per-run token usage (`usage`) on `run.result`, and on `run.status`
    /// where the runner serves it. Adds no op of its own.
    pub const RUN_USAGE: &str = "run_usage";
    /// Durable, incrementally maintained session spend on `session.head`.
    pub const SESSION_USAGE: &str = "session_usage";
    /// The caller's effective grant on `session.head`; absence means no grant.
    pub const HEAD_SENDER: &str = "head_sender";
    /// `session.send` with `delivery_order`: an ordered list of delivery
    /// modes, plan-named senders and plan templates for system-authority
    /// text.
    pub const DELIVERY_ORDER: &str = "delivery_order";
    /// `send.status`: what a re-send of a `send_id` would answer, read
    /// without re-sending the send's body.
    pub const SEND_STATUS: &str = "send_status";
    /// Owner relay for user commands, whose output never enters model history.
    /// A host menu is optional; its replies must stay outside model history.
    pub const USER_COMMANDS: &str = "user_commands";
    /// Hook phases run in their fixed order for every session. Module-level
    /// on a runner where that holds for every session; otherwise it is a
    /// session-level capability.
    pub const ORDERED_HOOK_PHASES: &str = "ordered_hook_phases";

    /// Every capability group, with the ops a runner that declares the group
    /// must serve. A group adds no op of its own when it only adds fields to
    /// another group's reply, so it names the op it extends.
    pub const GROUPS: &[(&str, &[&str])] = &[
        (SYSTEM_TEXT_DEFAULT, &[ops::SESSION_SEND]),
        (
            OUTPUT_REFS,
            &[ops::SESSION_OUTPUT_REFS, ops::SESSION_OUTPUT_REF_READ],
        ),
        (
            ATTACHMENTS,
            &[
                ops::SESSION_UPLOAD_BEGIN,
                ops::SESSION_UPLOAD_APPEND,
                ops::SESSION_UPLOAD_STATUS,
                ops::SESSION_UPLOAD_COMMIT,
                ops::SESSION_MEDIA_READ,
                ops::SESSION_SEND,
                ops::SESSION_READ,
            ],
        ),
        (
            USER_COMMANDS,
            &[ops::SESSION_USER_COMMAND, ops::SESSION_USER_COMMAND_STATUS],
        ),
        (TRANSCRIPT_READS, &[ops::SESSION_READ, ops::SESSION_HEAD]),
        (HOST_HISTORY, &[ops::SESSION_HISTORY]),
        (HISTORY_DISPLAY, &[ops::SESSION_HISTORY]),
        (LIVE_EVENTS, &[ops::SESSION_SUBSCRIBE]),
        (CLIENT_MESSAGE_ID, &[ops::SESSION_SEND]),
        (RUN_CAUSE, &[ops::SESSION_SUBSCRIBE]),
        (RUN_OPS, &[ops::RUN_RESULT, ops::SESSION_READ]),
        (RUN_STATUS, &[ops::RUN_STATUS]),
        (HELD_CALLS, &[ops::RUN_STATUS]),
        (RUN_CANCEL, &[ops::RUN_CANCEL]),
        (RETRACT, &[ops::SESSION_RETRACT]),
        (DISPATCH_ATTRIBUTION, &[ops::SESSION_READ]),
        (STREAMING, &[ops::SESSION_READ, ops::SESSION_SUBSCRIBE]),
        (MODEL_VIEW, &[ops::SESSION_READ]),
        (STEER, &[ops::SESSION_SEND]),
        (QUEUE, &[ops::SESSION_SEND]),
        (INTERRUPT, &[ops::SESSION_SEND]),
        (PLANS, &[ops::SESSION_SEND]),
        (RETENTION, &[ops::SESSION_SEND]),
        (INPUT_TOKEN_CAP, &[ops::SESSION_SEND]),
        (MAX_STEPS, &[ops::SESSION_SEND]),
        (MAX_OUTPUT_TOKENS, &[ops::SESSION_SEND]),
        (NO_TOOLS, &[ops::SESSION_SEND]),
        (TURN_DIRECTIVES, &[ops::SESSION_SEND, ops::RUN_RESULT]),
        (INTERRUPT_REASON, &[ops::RUN_RESULT]),
        (STRUCTURED_OUTPUT, &[ops::SESSION_SEND]),
        (ADOPT, &[ops::SESSION_ADOPT]),
        (TAGS, &[ops::SESSION_READ, ops::SESSION_RESOLVE]),
        (RUN_TOOL_CALLS, &[ops::RUN_TOOL_CALLS]),
        (RUN_USAGE, &[ops::RUN_RESULT]),
        (SESSION_USAGE, &[ops::SESSION_HEAD]),
        (HEAD_SENDER, &[ops::SESSION_HEAD]),
        (DELIVERY_ORDER, &[ops::SESSION_SEND]),
        (SEND_STATUS, &[ops::SEND_STATUS]),
        (COMPACTION, &[ops::SESSION_READ, ops::SESSION_RESOLVE]),
        (TAG_CHECKPOINT, &[ops::SESSION_READ, ops::SESSION_RESOLVE]),
        (
            SESSION_CHANGE,
            &[
                ops::SESSION_REFRESH,
                ops::SESSION_REFRESH_POLICY,
                ops::SESSION_FLUSH_PREFIX,
            ],
        ),
    ];

    /// The ops a declared group requires, or `None` for a name that is not a
    /// group (a module-level capability such as `ordered_hook_phases`, or an
    /// unknown one).
    pub fn group_ops(name: &str) -> Option<&'static [&'static str]> {
        GROUPS
            .iter()
            .find(|(group, _)| *group == name)
            .map(|(_, ops)| *ops)
    }

    /// Session-level capabilities, keyed by name in an admission reply's or a
    /// baseline's `session_capabilities`. A capability is declared by the
    /// value `true`; any other value is not a declaration.
    pub mod session {
        /// The session can apply a mid-session tool or instruction change
        /// without rebuilding its prefix. Depends on the provider family and
        /// model, and is re-evaluated on a model switch.
        pub const MID_SESSION_APPENDS: &str = "mid_session_appends";
        /// Hook phases run in their fixed order for this session.
        pub const ORDERED_HOOK_PHASES: &str = super::ORDERED_HOOK_PHASES;
    }

    /// Where a runner's session-level capabilities come from, as
    /// `role.describe` declares it in `session_capabilities_from`.
    pub mod source {
        /// The reply to the send that admitted the session.
        pub const ADMISSION: &str = "admission";
        /// `session.baseline`, on a runner that has no admission reply.
        pub const BASELINE: &str = "baseline";
    }
}

/// The change-policy rungs a session may support, per surface, as a
/// baseline's `rungs` lists them. Ordered from cheapest to dearest to apply.
pub mod rungs {
    pub const ON_PREFIX_REBUILD: &str = "on_prefix_rebuild";
    pub const ON_SYSTEM_REBUILD: &str = "on_system_rebuild";
    pub const ON_HISTORY_REBUILD: &str = "on_history_rebuild";
    pub const IMMEDIATELY: &str = "immediately";
    pub const ALL: &[&str] = &[
        ON_PREFIX_REBUILD,
        ON_SYSTEM_REBUILD,
        ON_HISTORY_REBUILD,
        IMMEDIATELY,
    ];
}

/// Decoding helpers for request fields whose VALUE is strict: an unknown
/// value is refused naming the field, rather than read as the default,
/// because the default would answer a different question.
pub(crate) mod strict {
    use serde::{de::DeserializeOwned, Deserialize, Deserializer};
    use serde_json::Value;

    /// Decode an optional field as `T`, reading JSON `null` as absent and
    /// refusing any other undecodable value with a message that names
    /// `field` and the `accepted` spellings.
    pub fn optional<'de, D, T>(
        deserializer: D,
        field: &str,
        accepted: &str,
    ) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let Some(value) = Option::<Value>::deserialize(deserializer)? else {
            return Ok(None);
        };
        serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| {
                serde::de::Error::custom(format!("{field} must be {accepted}; got {value}"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_group_names_only_role_ops() {
        let role_ops = [
            ops::SESSION_OUTPUT_REFS,
            ops::SESSION_OUTPUT_REF_READ,
            ops::SESSION_UPLOAD_BEGIN,
            ops::SESSION_UPLOAD_APPEND,
            ops::SESSION_UPLOAD_STATUS,
            ops::SESSION_UPLOAD_COMMIT,
            ops::SESSION_MEDIA_READ,
            ops::ROLE_DESCRIBE,
            ops::SESSION_BASELINE,
            ops::SESSION_USER_COMMAND,
            ops::SESSION_USER_COMMAND_STATUS,
            ops::SESSION_READ,
            ops::SESSION_HEAD,
            ops::SESSION_HISTORY,
            ops::RUN_RESULT,
            ops::RUN_STATUS,
            ops::RUN_CANCEL,
            ops::SESSION_RETRACT,
            ops::SESSION_SUBSCRIBE,
            ops::SESSION_SEND,
            ops::SESSION_REFRESH,
            ops::SESSION_REFRESH_POLICY,
            ops::SESSION_FLUSH_PREFIX,
            ops::SESSION_ADOPT,
            ops::SESSION_RESOLVE,
            ops::RUN_TOOL_CALLS,
            ops::SEND_STATUS,
        ];
        for (group, group_ops) in capabilities::GROUPS {
            assert!(!group_ops.is_empty(), "{group} names no op");
            for op in *group_ops {
                assert!(role_ops.contains(op), "{group} names {op}");
            }
        }
        assert_eq!(
            capabilities::group_ops(capabilities::STEER),
            Some(&[ops::SESSION_SEND][..])
        );
        assert_eq!(
            capabilities::group_ops(capabilities::ORDERED_HOOK_PHASES),
            None
        );
        assert_eq!(
            capabilities::group_ops(capabilities::COMPACTION),
            Some(&[ops::SESSION_READ, ops::SESSION_RESOLVE][..])
        );
        // A group's ops are never required of every runner: a runner that
        // declares no group serves only the required ops.
        for (group, group_ops) in capabilities::GROUPS {
            for op in *group_ops {
                assert!(!REQUIRED_OPS.contains(op), "{group}: {op} is required");
            }
        }
    }

    #[test]
    fn session_change_vectors_name_group_ops_and_refusals() {
        let file = vectors::load("session-change.json");
        let group = capabilities::group_ops(capabilities::SESSION_CHANGE).unwrap();
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            let op = case["op"].as_str().unwrap();
            assert!(group.contains(&op), "{name}: {op} is not in session_change");
            let code = case["refusal"]["code"].as_str().unwrap();
            assert!(errors::CODES.contains(&code), "{name}: {code}");
            assert_eq!(
                errors::is_retryable(code),
                case["retryable"].as_bool().unwrap(),
                "{name}"
            );
        }
        let declared: Vec<&str> = vectors::cases(&file, "refusals")
            .iter()
            .map(|case| case["refusal"]["code"].as_str().unwrap())
            .collect();
        for code in errors::SESSION_CHANGE_CODES.iter().chain(&[
            errors::FETCH_UNAVAILABLE,
            errors::PLAN_STALE,
            errors::TOOL_NAME_COLLISION,
        ]) {
            assert!(declared.contains(code), "{code} has no vector");
        }
    }

    #[test]
    fn the_transport_frame_limit_is_the_daemon_delivered_frame() {
        // The daemon's `subc_protocol::MAX_DELIVERED_FRAME_LEN`, 4 MiB.
        assert_eq!(TRANSPORT_FRAME_LIMIT, 4 * 1024 * 1024);
    }
}
