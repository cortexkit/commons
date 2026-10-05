use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
pub use uuid::Uuid;

/// Raw bytes, not text: an output chunk can split a UTF-8 character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BytePayload(pub Vec<u8>);

impl Serialize for BytePayload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for BytePayload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD
            .decode(encoded)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

/// `exec.run` parameters; executor-owned IDs and snapshot metadata are absent.
/// Both timeout and queue_wait_limit_s are measured in seconds.
///
/// Use [`Self::new`] and the `with_*` setters so future optional fields remain
/// additive for callers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunRequest {
    pub workspace_key: String,
    pub repository_root: String,
    pub cwd: String,
    pub command: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_hint: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_wait_limit_s: Option<u64>,
    #[serde(default)]
    pub siblings: Vec<String>,
}

impl RunRequest {
    /// A request with empty env/siblings and executor-default scheduling limits.
    pub fn new(
        workspace_key: impl Into<String>,
        repository_root: impl Into<String>,
        cwd: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            workspace_key: workspace_key.into(),
            repository_root: repository_root.into(),
            cwd: cwd.into(),
            command: command.into(),
            env: BTreeMap::new(),
            weight_hint: None,
            timeout: None,
            queue_wait_limit_s: None,
            siblings: Vec::new(),
        }
    }

    /// Set caller environment overrides; filtering belongs to the executor.
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Set the scheduling weight hint.
    pub fn with_weight_hint(mut self, weight_hint: u32) -> Self {
        self.weight_hint = Some(weight_hint);
        self
    }

    /// Set the command timeout in seconds.
    pub fn with_timeout(mut self, timeout: u64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the queue wait limit in seconds, including an explicit zero.
    pub fn with_queue_wait_limit_s(mut self, queue_wait_limit_s: u64) -> Self {
        self.queue_wait_limit_s = Some(queue_wait_limit_s);
        self
    }

    /// Set sibling repositories by their canonical absolute paths.
    pub fn with_siblings(mut self, siblings: Vec<String>) -> Self {
        self.siblings = siblings;
        self
    }
}

/// `workspace.prepare` parameters, without runner snapshot/transfer metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrepareRequest {
    pub workspace_key: String,
    pub repository_root: String,
    pub base_commit: String,
    #[serde(default)]
    pub siblings: Vec<String>,
}

impl PrepareRequest {
    /// Prepare the named workspace with no sibling repositories.
    pub fn new(
        workspace_key: impl Into<String>,
        repository_root: impl Into<String>,
        base_commit: impl Into<String>,
    ) -> Self {
        Self {
            workspace_key: workspace_key.into(),
            repository_root: repository_root.into(),
            base_commit: base_commit.into(),
            siblings: Vec::new(),
        }
    }

    /// Set sibling repositories by their canonical absolute paths.
    pub fn with_siblings(mut self, siblings: Vec<String>) -> Self {
        self.siblings = siblings;
        self
    }
}

/// Why the executor refused a command before starting it. Every reason,
/// including an unrecognised one, means the command did not start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum RefusalReason {
    Unreachable,
    RunnerVersionMismatch,
    SnapshotFailed,
    WorkspaceKeyRejected,
    RunnerFull,
    QueueWaitExceeded,
    TransferInterrupted,
    TreeHashMismatch,
    BundleRejected,
    WorkspaceSetupFailed,
    /// A refusal tag this version does not recognise. The refused command
    /// still did not start.
    Unknown(String),
}

impl From<String> for RefusalReason {
    fn from(reason: String) -> Self {
        match reason.as_str() {
            "unreachable" => Self::Unreachable,
            "runner_version_mismatch" => Self::RunnerVersionMismatch,
            "snapshot_failed" => Self::SnapshotFailed,
            "workspace_key_rejected" => Self::WorkspaceKeyRejected,
            "runner_full" => Self::RunnerFull,
            "queue_wait_exceeded" => Self::QueueWaitExceeded,
            "transfer_interrupted" => Self::TransferInterrupted,
            "tree_hash_mismatch" => Self::TreeHashMismatch,
            "bundle_rejected" => Self::BundleRejected,
            "workspace_setup_failed" => Self::WorkspaceSetupFailed,
            _ => Self::Unknown(reason),
        }
    }
}

impl From<RefusalReason> for String {
    fn from(reason: RefusalReason) -> Self {
        match reason {
            RefusalReason::Unreachable => "unreachable".into(),
            RefusalReason::RunnerVersionMismatch => "runner_version_mismatch".into(),
            RefusalReason::SnapshotFailed => "snapshot_failed".into(),
            RefusalReason::WorkspaceKeyRejected => "workspace_key_rejected".into(),
            RefusalReason::RunnerFull => "runner_full".into(),
            RefusalReason::QueueWaitExceeded => "queue_wait_exceeded".into(),
            RefusalReason::TransferInterrupted => "transfer_interrupted".into(),
            RefusalReason::TreeHashMismatch => "tree_hash_mismatch".into(),
            RefusalReason::BundleRejected => "bundle_rejected".into(),
            RefusalReason::WorkspaceSetupFailed => "workspace_setup_failed".into(),
            RefusalReason::Unknown(reason) => reason,
        }
    }
}

/// A command's terminal outcome. Only `refused_before_start` proves that the
/// submitted command did not start; expired or unknown history does not.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    Exit {
        code: i32,
    },
    Signal {
        signal: i32,
    },
    Cancelled,
    RefusedBeforeStart {
        reason: RefusalReason,
    },
    OutcomeUnknown,
    HistoryExpired,
    /// An unrecognised terminal outcome tag. Callers grade it exactly like
    /// `outcome_unknown`: never assume the command did not run, and never
    /// re-run it locally. Only the tag is retained; unknown fields are ignored.
    Unknown {
        kind: String,
    },
}

/// Known tags are decoded separately so a malformed known outcome cannot fall
/// through into the unknown-tag arm.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KnownOutcome {
    Exit { code: i32 },
    Signal { signal: i32 },
    Cancelled,
    RefusedBeforeStart { reason: RefusalReason },
    OutcomeUnknown,
    HistoryExpired,
}

#[derive(Deserialize)]
struct OutcomeTag {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OutcomeInput {
    Known(KnownOutcome),
    Tag(OutcomeTag),
}

impl Serialize for Outcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let known = match self {
            Self::Exit { code } => KnownOutcome::Exit { code: *code },
            Self::Signal { signal } => KnownOutcome::Signal { signal: *signal },
            Self::Cancelled => KnownOutcome::Cancelled,
            Self::RefusedBeforeStart { reason } => KnownOutcome::RefusedBeforeStart {
                reason: reason.clone(),
            },
            Self::OutcomeUnknown => KnownOutcome::OutcomeUnknown,
            Self::HistoryExpired => KnownOutcome::HistoryExpired,
            Self::Unknown { kind } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("type", kind)?;
                return map.end();
            }
        };
        known.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Outcome {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let known = match OutcomeInput::deserialize(deserializer)? {
            OutcomeInput::Known(known) => known,
            OutcomeInput::Tag(OutcomeTag { kind }) => {
                if matches!(
                    kind.as_str(),
                    "exit"
                        | "signal"
                        | "cancelled"
                        | "refused_before_start"
                        | "outcome_unknown"
                        | "history_expired"
                ) {
                    return Err(serde::de::Error::custom(format!(
                        "malformed fields for known outcome tag {kind}"
                    )));
                }
                return Ok(Self::Unknown { kind });
            }
        };
        Ok(match known {
            KnownOutcome::Exit { code } => Self::Exit { code },
            KnownOutcome::Signal { signal } => Self::Signal { signal },
            KnownOutcome::Cancelled => Self::Cancelled,
            KnownOutcome::RefusedBeforeStart { reason } => Self::RefusedBeforeStart { reason },
            KnownOutcome::OutcomeUnknown => Self::OutcomeUnknown,
            KnownOutcome::HistoryExpired => Self::HistoryExpired,
        })
    }
}

/// Why the executor killed a running command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum Killed {
    Deadline,
    Cancel,
    /// The command was killed for a reason this version does not recognise.
    Unknown(String),
}

impl From<String> for Killed {
    fn from(reason: String) -> Self {
        match reason.as_str() {
            "deadline" => Self::Deadline,
            "cancel" => Self::Cancel,
            _ => Self::Unknown(reason),
        }
    }
}

impl From<Killed> for String {
    fn from(reason: Killed) -> Self {
        match reason {
            Killed::Deadline => "deadline".into(),
            Killed::Cancel => "cancel".into(),
            Killed::Unknown(reason) => reason,
        }
    }
}

/// Execution location; `None` is the wire string `"none"`, not a JSON null.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum Ran {
    Remote,
    None,
    /// An execution location this version does not recognise. Never treat it as
    /// `None`: the command may have run. Grade it like `outcome_unknown` for
    /// re-run decisions; never re-run it locally.
    Unknown(String),
}

impl From<String> for Ran {
    fn from(location: String) -> Self {
        match location.as_str() {
            "remote" => Self::Remote,
            "none" => Self::None,
            _ => Self::Unknown(location),
        }
    }
}

impl From<Ran> for String {
    fn from(location: Ran) -> Self {
        match location {
            Ran::Remote => "remote".into(),
            Ran::None => "none".into(),
            Ran::Unknown(location) => location,
        }
    }
}

/// The last record on an `exec.run` or `exec.attach` reply stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TerminalRecord {
    pub job_id: Uuid,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub killed: Option<Killed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipestatus: Option<Vec<i32>>,
    pub wall_ms: u64,
    pub queue_wait_ms: u64,
    pub bundle_bytes: u64,
    /// Null only for expired history; refusal and pre-dispatch cancellation use
    /// `Some(Ran::None)`. Always emitted, even when null.
    pub ran: Option<Ran>,
    /// Null before hashing or for expired history. Always emitted.
    pub tree_hash: Option<String>,
    /// Null unless the command ran remotely. An empty array means no changes,
    /// not unavailable history. Remote writes are never copied back.
    pub workspace_changes: Option<Vec<String>>,
}

impl TerminalRecord {
    /// Construct a terminal record with unavailable execution metadata.
    /// Executors must fill availability fields according to the outcome before
    /// sending it; these wire types do not enforce execution policy.
    pub fn new(
        job_id: Uuid,
        outcome: Outcome,
        wall_ms: u64,
        queue_wait_ms: u64,
        bundle_bytes: u64,
    ) -> Self {
        Self {
            job_id,
            outcome,
            killed: None,
            pipestatus: None,
            wall_ms,
            queue_wait_ms,
            bundle_bytes,
            ran: None,
            tree_hash: None,
            workspace_changes: None,
        }
    }

    /// Record why a command was killed.
    pub fn with_killed(mut self, killed: Killed) -> Self {
        self.killed = Some(killed);
        self
    }

    /// Record the pipeline exit statuses.
    pub fn with_pipestatus(mut self, pipestatus: Vec<i32>) -> Self {
        self.pipestatus = Some(pipestatus);
        self
    }

    /// Set known execution location (including the explicit before-start none).
    pub fn with_ran(mut self, ran: Ran) -> Self {
        self.ran = Some(ran);
        self
    }

    /// Record the snapshot tree hash.
    pub fn with_tree_hash(mut self, tree_hash: impl Into<String>) -> Self {
        self.tree_hash = Some(tree_hash.into());
        self
    }

    /// Record changed paths from remote execution, including an empty list.
    pub fn with_workspace_changes(mut self, workspace_changes: Vec<String>) -> Self {
        self.workspace_changes = Some(workspace_changes);
        self
    }
}

/// The output file descriptor a chunk came from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum OutputStream {
    Stdout,
    Stderr,
    /// Only the stream label is unrecognised. Callers still deliver the chunk's
    /// bytes and sequence number.
    Unknown(String),
}

impl From<String> for OutputStream {
    fn from(stream: String) -> Self {
        match stream.as_str() {
            "stdout" => Self::Stdout,
            "stderr" => Self::Stderr,
            _ => Self::Unknown(stream),
        }
    }
}

impl From<OutputStream> for String {
    fn from(stream: OutputStream) -> Self {
        match stream {
            OutputStream::Stdout => "stdout".into(),
            OutputStream::Stderr => "stderr".into(),
            OutputStream::Unknown(stream) => stream,
        }
    }
}

/// A raw output chunk with its replay sequence number.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct Output {
    pub seq: u64,
    pub stream: OutputStream,
    pub bytes: BytePayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated_before_seq: Option<u64>,
}

impl Output {
    /// An output chunk with no replay truncation notice.
    pub fn new(seq: u64, stream: OutputStream, bytes: BytePayload) -> Self {
        Self {
            seq,
            stream,
            bytes,
            truncated_before_seq: None,
        }
    }

    /// Indicate the oldest sequence retained for replay.
    pub fn with_truncated_before_seq(mut self, truncated_before_seq: u64) -> Self {
        self.truncated_before_seq = Some(truncated_before_seq);
        self
    }
}

/// Durable acceptance of a run and its queue position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct Accepted {
    pub job_id: Uuid,
    pub queue_position: u32,
}

impl Accepted {
    pub fn new(job_id: Uuid, queue_position: u32) -> Self {
        Self {
            job_id,
            queue_position,
        }
    }
}

/// `workspace.prepare`'s outcome, not a runner transfer-control frame.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrepareOutcome {
    Prepared,
    RefusedBeforeStart {
        reason: RefusalReason,
    },
    /// An unrecognised prepare outcome. Never treat the workspace as prepared,
    /// and do not run against it on that basis. Only the tag is retained;
    /// unknown fields are ignored.
    Unknown {
        kind: String,
    },
}

/// Decode known tags separately so malformed known replies cannot become unknown.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KnownPrepareOutcome {
    Prepared,
    RefusedBeforeStart { reason: RefusalReason },
}

#[derive(Deserialize)]
struct PrepareOutcomeTag {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PrepareOutcomeInput {
    Known(KnownPrepareOutcome),
    Tag(PrepareOutcomeTag),
}

impl Serialize for PrepareOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let known = match self {
            Self::Prepared => KnownPrepareOutcome::Prepared,
            Self::RefusedBeforeStart { reason } => KnownPrepareOutcome::RefusedBeforeStart {
                reason: reason.clone(),
            },
            Self::Unknown { kind } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("type", kind)?;
                return map.end();
            }
        };
        known.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PrepareOutcome {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let known = match PrepareOutcomeInput::deserialize(deserializer)? {
            PrepareOutcomeInput::Known(known) => known,
            PrepareOutcomeInput::Tag(PrepareOutcomeTag { kind }) => {
                if matches!(kind.as_str(), "prepared" | "refused_before_start") {
                    return Err(serde::de::Error::custom(format!(
                        "malformed fields for known prepare outcome tag {kind}"
                    )));
                }
                return Ok(Self::Unknown { kind });
            }
        };
        Ok(match known {
            KnownPrepareOutcome::Prepared => Self::Prepared,
            KnownPrepareOutcome::RefusedBeforeStart { reason } => {
                Self::RefusedBeforeStart { reason }
            }
        })
    }
}

/// A `workspace.prepare` reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrepareReply {
    pub transfer_id: Uuid,
    pub outcome: PrepareOutcome,
}

impl PrepareReply {
    pub fn new(transfer_id: Uuid, outcome: PrepareOutcome) -> Self {
        Self {
            transfer_id,
            outcome,
        }
    }
}

/// A `workspace.drop` request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct DropRequest {
    pub workspace_key: String,
}

impl DropRequest {
    pub fn new(workspace_key: impl Into<String>) -> Self {
        Self {
            workspace_key: workspace_key.into(),
        }
    }
}

/// A `workspace.drop` reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct DropReply {
    pub dropped: bool,
    pub cancelled_jobs: Vec<Uuid>,
}

impl DropReply {
    pub fn new(dropped: bool, cancelled_jobs: Vec<Uuid>) -> Self {
        Self {
            dropped,
            cancelled_jobs,
        }
    }
}

/// `exec.attach` parameters; replies are [`StreamRecord`] values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttachRequest {
    pub job_id: Uuid,
    /// Inclusive replay cursor; the reconnect cursor is last forwarded seq + 1.
    pub from_seq: u64,
}

impl AttachRequest {
    pub fn new(job_id: Uuid, from_seq: u64) -> Self {
        Self { job_id, from_seq }
    }
}

/// An idempotent `exec.cancel` request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct CancelRequest {
    pub job_id: Uuid,
}

impl CancelRequest {
    pub fn new(job_id: Uuid) -> Self {
        Self { job_id }
    }
}

/// Acknowledges that the idempotent cancel (including an unknown-ID tombstone)
/// is durable. The job's terminal record arrives on its run/attach stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct CancelReply {
    pub job_id: Uuid,
}

impl CancelReply {
    pub fn new(job_id: Uuid) -> Self {
        Self { job_id }
    }
}

/// `exec.status` takes an empty arguments object.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatusRequest {}

impl StatusRequest {
    pub fn new() -> Self {
        Self {}
    }
}

/// The state of a repository's latest base rebuild.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum RebuildResult {
    Building,
    Ok,
    Failed,
    /// An unrecognised rebuild result. This status is informational only.
    Unknown(String),
}

impl From<String> for RebuildResult {
    fn from(result: String) -> Self {
        match result.as_str() {
            "building" => Self::Building,
            "ok" => Self::Ok,
            "failed" => Self::Failed,
            _ => Self::Unknown(result),
        }
    }
}

impl From<RebuildResult> for String {
    fn from(result: RebuildResult) -> Self {
        match result {
            RebuildResult::Building => "building".into(),
            RebuildResult::Ok => "ok".into(),
            RebuildResult::Failed => "failed".into(),
            RebuildResult::Unknown(result) => result,
        }
    }
}

/// A running command or base rebuild in an `exec.status` reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunningJob {
    pub job_id: Uuid,
    /// The job's workspace key. A base rebuild reports `base:` followed by the
    /// repository root instead of a workspace key.
    pub workspace_key: String,
    pub weight: u32,
}

impl RunningJob {
    pub fn new(job_id: Uuid, workspace_key: impl Into<String>, weight: u32) -> Self {
        Self {
            job_id,
            workspace_key: workspace_key.into(),
            weight,
        }
    }
}

/// Repository availability in an `exec.status` reply; nulls are not omitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct RepositoryStatus {
    pub repository_root: String,
    /// Null until the first generation has been published.
    pub warm_target_age_s: Option<u64>,
    pub published_commit: Option<String>,
    /// Null if no rebuild has been attempted yet.
    pub last_rebuild_result: Option<RebuildResult>,
}

impl RepositoryStatus {
    /// A cold repository with no published generation or attempted rebuild.
    pub fn new(repository_root: impl Into<String>) -> Self {
        Self {
            repository_root: repository_root.into(),
            warm_target_age_s: None,
            published_commit: None,
            last_rebuild_result: None,
        }
    }

    pub fn with_warm_target_age_s(mut self, warm_target_age_s: u64) -> Self {
        self.warm_target_age_s = Some(warm_target_age_s);
        self
    }

    pub fn with_published_commit(mut self, published_commit: impl Into<String>) -> Self {
        self.published_commit = Some(published_commit.into());
        self
    }

    pub fn with_last_rebuild_result(mut self, last_rebuild_result: RebuildResult) -> Self {
        self.last_rebuild_result = Some(last_rebuild_result);
        self
    }
}

/// An `exec.status` reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatusReply {
    pub queue_depth: u32,
    pub running_jobs: Vec<RunningJob>,
    pub server_reachable: bool,
    pub repositories: Vec<RepositoryStatus>,
    pub rustc_version: String,
}

impl StatusReply {
    pub fn new(
        queue_depth: u32,
        running_jobs: Vec<RunningJob>,
        server_reachable: bool,
        repositories: Vec<RepositoryStatus>,
        rustc_version: impl Into<String>,
    ) -> Self {
        Self {
            queue_depth,
            running_jobs,
            server_reachable,
            repositories,
            rustc_version: rustc_version.into(),
        }
    }
}

/// StreamData payloads exposed to `exec.run` and `exec.attach` callers, without
/// SSH heartbeats or transfer-control frames. Fields live inline beside `type`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamRecord {
    Accepted(Accepted),
    Output(Output),
    Terminal(TerminalRecord),
    /// An unrecognised record. Skip it and keep reading, counting its sequence
    /// number toward the resume cursor when present. Never treat it as terminal.
    /// If the stream ends without a known terminal record, get the job's outcome
    /// from `exec.status` or `exec.attach` rather than waiting forever, and grade
    /// it outcome-unknown until one arrives. Only the tag and sequence number
    /// are retained; other unknown fields are ignored.
    Unknown {
        kind: String,
        seq: Option<u64>,
    },
}

/// Decode known tags separately so malformed known records cannot become unknown.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KnownStreamRecord {
    Accepted(Accepted),
    Output(Output),
    Terminal(TerminalRecord),
}

#[derive(Deserialize)]
struct StreamRecordTag {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, deserialize_with = "StreamRecordTag::deserialize_seq")]
    seq: Option<u64>,
}

impl StreamRecordTag {
    fn deserialize_seq<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        // An absent sequence is allowed, but a present one must be an integer,
        // not null, so callers can safely advance their replay cursor.
        u64::deserialize(deserializer).map(Some)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StreamRecordInput {
    Known(KnownStreamRecord),
    Tag(StreamRecordTag),
}

impl Serialize for StreamRecord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let known = match self {
            Self::Accepted(accepted) => KnownStreamRecord::Accepted(accepted.clone()),
            Self::Output(output) => KnownStreamRecord::Output(output.clone()),
            Self::Terminal(terminal) => KnownStreamRecord::Terminal(terminal.clone()),
            Self::Unknown { kind, seq } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1 + usize::from(seq.is_some())))?;
                map.serialize_entry("type", kind)?;
                if let Some(seq) = seq {
                    map.serialize_entry("seq", seq)?;
                }
                return map.end();
            }
        };
        known.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StreamRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let known = match StreamRecordInput::deserialize(deserializer)? {
            StreamRecordInput::Known(known) => known,
            StreamRecordInput::Tag(StreamRecordTag { kind, seq }) => {
                if matches!(kind.as_str(), "accepted" | "output" | "terminal") {
                    return Err(serde::de::Error::custom(format!(
                        "malformed fields for known stream record tag {kind}"
                    )));
                }
                return Ok(Self::Unknown { kind, seq });
            }
        };
        Ok(match known {
            KnownStreamRecord::Accepted(accepted) => Self::Accepted(accepted),
            KnownStreamRecord::Output(output) => Self::Output(output),
            KnownStreamRecord::Terminal(terminal) => Self::Terminal(terminal),
        })
    }
}
