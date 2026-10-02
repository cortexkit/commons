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
//! request in this crate names its session, except `compaction.ready` (in the
//! `compaction` group), which arrives on a module-level route.

#![forbid(unsafe_code)]

pub mod baseline;
pub mod compaction;
pub mod describe;
pub mod errors;
pub mod points;
pub mod read;
pub mod run;
pub mod send;
pub mod subscribe;

/// The role name as `role.describe` reports it.
pub const ROLE: &str = "llm-runner";

/// The only major this crate defines.
pub const VERSION: &str = "v1";

/// The string a runner lists in its manifest's `capabilities.provides`.
pub const PROVIDES: &str = "llm-runner/v1";

/// Names of the ops a runner serves.
pub mod ops {
    /// Discovery before use: the role's majors, ops and capabilities.
    /// Required.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// The session's current baseline, answered only to the session's owner.
    /// Required.
    pub const SESSION_BASELINE: &str = "session.baseline";
    /// A compaction provider's signal that a step it asked the runner to
    /// hold may be asked about again. In `compaction`: the one op of the
    /// compaction interface the runner serves rather than calls.
    pub const COMPACTION_READY: &str = "compaction.ready";
    /// A page of the transcript. In `transcript_reads`, and in every group
    /// that adds fields to a page.
    pub const SESSION_READ: &str = "session.read";
    /// The transcript's head metadata, without message bodies. In
    /// `transcript_reads`.
    pub const SESSION_HEAD: &str = "session.head";
    /// A run's terminal state and final message. In `run_ops`.
    pub const RUN_RESULT: &str = "run.result";
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
}

/// Ops every `llm-runner/v1` module must list in `role.describe`. A consumer
/// refuses a module missing any of them, by name, before routing anything.
pub const REQUIRED_OPS: &[&str] = &[ops::ROLE_DESCRIBE, ops::SESSION_BASELINE];

/// Capability names: the module-level groups `role.describe` declares, and
/// the session-level capabilities a session's admission reply or baseline
/// declares.
pub mod capabilities {
    use crate::ops;

    /// Tail, range and after-message reads, head metadata and `lineage_id`
    /// on every page.
    pub const TRANSCRIPT_READS: &str = "transcript_reads";
    /// `run.result` and per-message run attribution.
    pub const RUN_OPS: &str = "run_ops";
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
    /// The compaction interface: the runner calls the session's compaction
    /// provider (Setup, the per-step status, the request fence, durable
    /// `WAIT` and its timeout, `REFUSE`) and serves `compaction.ready`. A
    /// runner that hosts no sessions of its own declares none of it.
    pub const COMPACTION: &str = "compaction";
    /// Mid-session changes from the session's owner: `session.refresh`,
    /// `session.refresh_policy` and `session.flush_prefix`. A runner without
    /// it gets no mid-session changes; its owner applies them at the next
    /// session instead.
    pub const SESSION_CHANGE: &str = "session_change";
    /// Hook phases run in their fixed order for every session. Module-level
    /// on a runner where that holds for every session; otherwise it is a
    /// session-level capability.
    pub const ORDERED_HOOK_PHASES: &str = "ordered_hook_phases";

    /// Every capability group, with the ops a runner that declares the group
    /// must serve. A group adds no op of its own when it only adds fields to
    /// another group's reply, so it names the op it extends.
    pub const GROUPS: &[(&str, &[&str])] = &[
        (TRANSCRIPT_READS, &[ops::SESSION_READ, ops::SESSION_HEAD]),
        (RUN_OPS, &[ops::RUN_RESULT, ops::SESSION_READ]),
        (DISPATCH_ATTRIBUTION, &[ops::SESSION_READ]),
        (STREAMING, &[ops::SESSION_READ, ops::SESSION_SUBSCRIBE]),
        (MODEL_VIEW, &[ops::SESSION_READ]),
        (STEER, &[ops::SESSION_SEND]),
        (QUEUE, &[ops::SESSION_SEND]),
        (INTERRUPT, &[ops::SESSION_SEND]),
        (PLANS, &[ops::SESSION_SEND]),
        (COMPACTION, &[ops::COMPACTION_READY]),
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
pub(crate) mod vectors {
    //! The shared test vectors, read from the repository's `test-vectors`
    //! directory so the wire crate, a later conformance runner and any other
    //! implementation check the same bytes.

    use serde::{de::DeserializeOwned, Serialize};
    use serde_json::Value;

    /// Every vector file. A test checks this list against the directory, so
    /// a new file cannot land without a test reading it.
    pub const FILES: &[&str] = &[
        "admit-optional-text-refused.json",
        "admit-optional-text-timeout.json",
        "admit-optional-text-unknown-provider.json",
        "refuse-later-send-with-different-plan.json",
        "refuse-later-plan-after-planless-first-episode.json",
        "refuse-tool-name-collision.json",
        "baseline.json",
        "compaction-ready.json",
        "errors.json",
        "fence.json",
        "head.json",
        "plans.json",
        "read-pages.json",
        "read-requests.json",
        "role-describe.json",
        "run-result.json",
        "send.json",
        "session-change.json",
        "subscribe.json",
    ];

    pub fn dir() -> String {
        format!(
            "{}/../../test-vectors/llm-runner-v1",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    pub fn load(name: &str) -> Value {
        assert!(FILES.contains(&name), "{name} is not listed in FILES");
        let path = format!("{}/{name}", dir());
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// Re-encode a typed object inside an unchanged upstream fixture envelope.
    /// Compare actual bytes, including field order and indentation, rather than
    /// normalizing through `Value`, which would hide order and whitespace changes.
    pub fn exact_object_round_trip<T>(name: &str, marker: &str, indent: usize) -> T
    where
        T: DeserializeOwned + Serialize,
    {
        assert!(FILES.contains(&name));
        let bytes = std::fs::read_to_string(format!("{}/{name}", dir())).unwrap();
        let start = bytes.find(marker).unwrap() + marker.len();
        let mut stream = serde_json::Deserializer::from_str(&bytes[start..]).into_iter::<Value>();
        stream.next().unwrap().unwrap();
        let end = start + stream.byte_offset();
        let decoded: T = serde_json::from_str(&bytes[start..end]).unwrap();
        let encoded = serde_json::to_string_pretty(&decoded)
            .unwrap()
            .replace('\n', &format!("\n{}", " ".repeat(indent)));
        let mut rebuilt = bytes.clone();
        rebuilt.replace_range(start..end, &encoded);
        assert_eq!(rebuilt.as_bytes(), bytes.as_bytes(), "{name}: bytes differ");
        decoded
    }

    /// The array at `key`, panicking with the key's name if it is missing or
    /// not an array.
    pub fn cases<'a>(file: &'a Value, key: &str) -> &'a Vec<Value> {
        file[key]
            .as_array()
            .unwrap_or_else(|| panic!("vector key {key} is not an array"))
    }

    /// Decode `json` as `T`, then check that encoding it gives back exactly
    /// `json` and that decoding the encoding gives the same value. Canonical
    /// vectors carry no unknown fields, so a field the type drops or renames
    /// shows up here.
    pub fn round_trip<T>(name: &str, json: &Value) -> T
    where
        T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug,
    {
        let decoded: T = serde_json::from_value(json.clone())
            .unwrap_or_else(|e| panic!("{name}: does not decode: {e}"));
        let encoded = serde_json::to_value(&decoded).unwrap();
        assert_eq!(&encoded, json, "{name}: encoding differs from the vector");
        let again: T = serde_json::from_value(encoded).unwrap();
        assert_eq!(again, decoded, "{name}: second decode differs");
        decoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_vector_file_on_disk_is_listed() {
        let mut on_disk: Vec<String> = std::fs::read_dir(vectors::dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".json"))
            .collect();
        on_disk.sort();
        let mut listed: Vec<String> = vectors::FILES.iter().map(|s| s.to_string()).collect();
        listed.sort();
        assert_eq!(on_disk, listed);
    }

    #[test]
    fn every_group_names_only_role_ops() {
        let role_ops = [
            ops::ROLE_DESCRIBE,
            ops::SESSION_BASELINE,
            ops::COMPACTION_READY,
            ops::SESSION_READ,
            ops::SESSION_HEAD,
            ops::RUN_RESULT,
            ops::SESSION_SUBSCRIBE,
            ops::SESSION_SEND,
            ops::SESSION_REFRESH,
            ops::SESSION_REFRESH_POLICY,
            ops::SESSION_FLUSH_PREFIX,
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
            Some(&[ops::COMPACTION_READY][..])
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
            assert!(
                errors::SESSION_CHANGE_CODES.contains(&code),
                "{name}: {code}"
            );
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
        for code in errors::SESSION_CHANGE_CODES {
            assert!(declared.contains(code), "{code} has no vector");
        }
    }
}
