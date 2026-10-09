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
    /// Transient: the runner is draining and is not accepting new work.
    RunnerDraining,
    /// Transient: the runner needs disk space freed before accepting new work.
    RunnerDiskFull,
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
            "runner_draining" => Self::RunnerDraining,
            "runner_disk_full" => Self::RunnerDiskFull,
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
            RefusalReason::RunnerDraining => "runner_draining".into(),
            RefusalReason::RunnerDiskFull => "runner_disk_full".into(),
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

/// Server-side Git state before and after a command; it is not copied back.
///
/// A reporting producer sends this even when the values are unchanged. Use
/// [`Self::changed`] to distinguish a reported change from a reported no-change.
/// Construct with [`Self::new`] and fill the available IDs with `with_*` setters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitStateChange {
    /// Commit ID before execution, or `None` for an unborn HEAD.
    pub head_before: Option<String>,
    /// Commit ID after execution, or `None` for an unborn HEAD.
    pub head_after: Option<String>,
    /// Symbolic ref before execution, or `None` when detached.
    pub ref_before: Option<String>,
    /// Symbolic ref after execution, or `None` when detached.
    pub ref_after: Option<String>,
    /// Index tree ID before execution, or `None` if no tree is available.
    pub index_tree_before: Option<String>,
    /// Index tree ID after execution, or `None` if no tree is available.
    pub index_tree_after: Option<String>,
    pub stash_count_before: u32,
    pub stash_count_after: u32,
}

impl GitStateChange {
    /// Record stash counts with unavailable commit, ref and index tree IDs.
    pub fn new(stash_count_before: u32, stash_count_after: u32) -> Self {
        Self {
            head_before: None,
            head_after: None,
            ref_before: None,
            ref_after: None,
            index_tree_before: None,
            index_tree_after: None,
            stash_count_before,
            stash_count_after,
        }
    }

    /// Record the commit ID before execution.
    pub fn with_head_before(mut self, head_before: impl Into<String>) -> Self {
        self.head_before = Some(head_before.into());
        self
    }

    /// Record the commit ID after execution.
    pub fn with_head_after(mut self, head_after: impl Into<String>) -> Self {
        self.head_after = Some(head_after.into());
        self
    }

    /// Record the symbolic ref before execution; leave unset when detached.
    pub fn with_ref_before(mut self, ref_before: impl Into<String>) -> Self {
        self.ref_before = Some(ref_before.into());
        self
    }

    /// Record the symbolic ref after execution; leave unset when detached.
    pub fn with_ref_after(mut self, ref_after: impl Into<String>) -> Self {
        self.ref_after = Some(ref_after.into());
        self
    }

    /// Record the index tree ID before execution.
    pub fn with_index_tree_before(mut self, index_tree_before: impl Into<String>) -> Self {
        self.index_tree_before = Some(index_tree_before.into());
        self
    }

    /// Record the index tree ID after execution.
    pub fn with_index_tree_after(mut self, index_tree_after: impl Into<String>) -> Self {
        self.index_tree_after = Some(index_tree_after.into());
        self
    }

    /// Whether any reported before/after pair differs, including availability.
    pub fn changed(&self) -> bool {
        self.head_before != self.head_after
            || self.ref_before != self.ref_after
            || self.index_tree_before != self.index_tree_after
            || self.stash_count_before != self.stash_count_after
    }
}

/// New untracked, non-ignored paths on the server, not copied back.
///
/// Producers should cap `paths` at 100 entries and set `truncated` when more
/// paths exist. The runner enforces the cap; this type does not. An empty list
/// with `truncated: false` reports that no new untracked paths were found.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct UntrackedFiles {
    pub paths: Vec<String>,
    /// Whether the list omits additional new untracked paths.
    pub truncated: bool,
}

impl UntrackedFiles {
    /// Record a complete list of new untracked paths, including an empty list.
    pub fn new(paths: Vec<String>) -> Self {
        Self {
            paths,
            truncated: false,
        }
    }

    /// Indicate that additional paths were omitted by the producer's cap.
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }
}

/// Server writes under ignored paths outside `target/`, `node_modules/`, and
/// `dist/`; these writes are not copied back.
///
/// `count` is the total, not the sample length. Producers should cap
/// `sample_paths` at 20 entries without capping `count`. The runner enforces the
/// cap and build-directory exclusions; this type does not. A zero count and
/// empty sample report that no such writes were found.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct IgnoredWrites {
    pub count: u64,
    pub sample_paths: Vec<String>,
}

impl IgnoredWrites {
    /// Record the total number of ignored writes with an empty sample.
    pub fn new(count: u64) -> Self {
        Self {
            count,
            sample_paths: Vec::new(),
        }
    }

    /// Record a sample of ignored paths; the producer caps its length (see the type's doc).
    pub fn with_sample_paths(mut self, sample_paths: Vec<String>) -> Self {
        self.sample_paths = sample_paths;
        self
    }
}

/// Maximum UTF-8 byte length producers may send in a refusal detail.
///
/// Producers must never include secrets, tokens or credential material. This is
/// a producer obligation, not a decoder limit: longer received details are kept
/// intact, without truncation or rejection.
pub const REFUSAL_DETAIL_MAX_BYTES: usize = 1024;

/// The last record on an `exec.run` or `exec.attach` reply stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TerminalRecord {
    pub job_id: Uuid,
    pub outcome: Outcome,
    /// Human-readable refusal explanation for quoting verbatim, never parsing.
    /// Producers must keep it within [`REFUSAL_DETAIL_MAX_BYTES`] UTF-8 bytes and
    /// exclude secrets, tokens and credential material. Decoders preserve longer
    /// received values. `None` means no detail was reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal_detail: Option<String>,
    /// Millisecond hint that a refusal is transient. Ignore this field unless
    /// `outcome` is [`Outcome::RefusedBeforeStart`]; wait at most the smaller of
    /// this hint and the caller's remaining budget before retrying. `None` means
    /// no retry hint was reported, not proof that a refusal is permanent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
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
    /// Server-side Git state, not copied back. `None` means not reported, never
    /// no change. A reporting runner sends `Some` even for unchanged values;
    /// consumers can use [`GitStateChange::changed`] to check for differences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_state_changed: Option<GitStateChange>,
    /// New untracked, non-ignored server paths, not copied back. `None` means not
    /// reported, never no change; a reporting runner sends an empty report when
    /// no paths were found. See [`UntrackedFiles`] for the recommended cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untracked_files: Option<UntrackedFiles>,
    /// Ignored server writes outside known build directories, not copied back.
    /// `None` means not reported, never no change; a reporting runner sends a
    /// zero-count, empty-sample report when none were found. See [`IgnoredWrites`]
    /// for the recommended sample cap and build-directory exclusions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_writes: Option<IgnoredWrites>,
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
            refusal_detail: None,
            retry_after_ms: None,
            killed: None,
            pipestatus: None,
            wall_ms,
            queue_wait_ms,
            bundle_bytes,
            ran: None,
            tree_hash: None,
            workspace_changes: None,
            git_state_changed: None,
            untracked_files: None,
            ignored_writes: None,
        }
    }

    /// Set a verbatim refusal explanation. The producer must obey
    /// [`REFUSAL_DETAIL_MAX_BYTES`] and exclude secrets and credential material;
    /// this setter does not validate or truncate the detail.
    pub fn with_refusal_detail(mut self, refusal_detail: impl Into<String>) -> Self {
        self.refusal_detail = Some(refusal_detail.into());
        self
    }

    /// Read the verbatim refusal explanation, if reported.
    pub fn refusal_detail(&self) -> Option<&str> {
        self.refusal_detail.as_deref()
    }

    /// Set a transient-refusal retry hint in milliseconds. Callers must ignore
    /// it for other outcomes and cap their wait at their remaining budget.
    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    /// Read the retry hint, if reported. This does not check the outcome or cap
    /// the hint to the caller's remaining budget.
    pub fn retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
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

    /// Report server-side Git state, including unchanged before/after values.
    pub fn with_git_state_changed(mut self, git_state_changed: GitStateChange) -> Self {
        self.git_state_changed = Some(git_state_changed);
        self
    }

    /// Report new untracked server paths, including an empty report.
    pub fn with_untracked_files(mut self, untracked_files: UntrackedFiles) -> Self {
        self.untracked_files = Some(untracked_files);
        self
    }

    /// Report ignored server writes, including a zero-count report.
    pub fn with_ignored_writes(mut self, ignored_writes: IgnoredWrites) -> Self {
        self.ignored_writes = Some(ignored_writes);
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

/// The job has taken runner capacity and begun running.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct Started {
    /// Replay sequence number for this stream record.
    pub seq: u64,
    /// Time spent waiting for runner capacity, measured by the runner.
    pub queue_wait_ms: u64,
    /// Runner wall-clock Unix time in milliseconds; use for display only.
    pub started_at_ms: u64,
}

impl Started {
    /// Record the moment the job began running.
    pub fn new(seq: u64, queue_wait_ms: u64, started_at_ms: u64) -> Self {
        Self {
            seq,
            queue_wait_ms,
            started_at_ms,
        }
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn queue_wait_ms(&self) -> u64 {
        self.queue_wait_ms
    }

    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }
}

/// Durable acceptance of a run and its queue position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct Accepted {
    pub job_id: Uuid,
    pub queue_position: u32,
    /// Names of caller-supplied environment variables the runner did not
    /// forward to the command. Names only, never values. Absent when the runner
    /// reports nothing, including every runner older than this field, so an
    /// absent list means "not reported", not "everything was forwarded".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_not_forwarded: Option<Vec<String>>,
}

impl Accepted {
    pub fn new(job_id: Uuid, queue_position: u32) -> Self {
        Self {
            job_id,
            queue_position,
            env_not_forwarded: None,
        }
    }

    /// Report which caller environment variable names were not forwarded.
    pub fn with_env_not_forwarded(mut self, names: Vec<String>) -> Self {
        self.env_not_forwarded = Some(names);
        self
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
    /// Human-readable refusal explanation for quoting verbatim, never parsing.
    /// Producers must keep it within [`REFUSAL_DETAIL_MAX_BYTES`] UTF-8 bytes and
    /// exclude secrets, tokens and credential material. Decoders preserve longer
    /// received values. `None` means no detail was reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal_detail: Option<String>,
    /// Millisecond hint that a refusal is transient. Ignore this field unless
    /// `outcome` is [`PrepareOutcome::RefusedBeforeStart`]; wait at most the smaller
    /// of this hint and the caller's remaining budget before retrying. `None`
    /// means no hint was reported, not proof that a refusal is permanent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl PrepareReply {
    pub fn new(transfer_id: Uuid, outcome: PrepareOutcome) -> Self {
        Self {
            transfer_id,
            outcome,
            refusal_detail: None,
            retry_after_ms: None,
        }
    }

    /// Set a verbatim refusal explanation. The producer must obey
    /// [`REFUSAL_DETAIL_MAX_BYTES`] and exclude secrets and credential material;
    /// this setter does not validate or truncate the detail.
    pub fn with_refusal_detail(mut self, refusal_detail: impl Into<String>) -> Self {
        self.refusal_detail = Some(refusal_detail.into());
        self
    }

    /// Read the verbatim refusal explanation, if reported.
    pub fn refusal_detail(&self) -> Option<&str> {
        self.refusal_detail.as_deref()
    }

    /// Set a transient-refusal retry hint in milliseconds. Callers must ignore
    /// it for other outcomes and cap their wait at their remaining budget.
    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    /// Read the retry hint, if reported. This does not check the outcome or cap
    /// the hint to the caller's remaining budget.
    pub fn retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
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
// Keep Terminal inline: boxing it would break existing public variant construction.
#[allow(clippy::large_enum_variant)]
pub enum StreamRecord {
    Accepted(Accepted),
    /// Emitted once after acceptance when the job takes runner capacity, before
    /// its first output or terminal record. A job refused before starting never
    /// has this record. The runner's command timeout runs from this moment, not
    /// from acceptance. A caller can use `Started` to report that the job is now
    /// running instead of waiting for capacity. It must not start a timeout of
    /// its own from it. Older runners may omit this record, so callers retain
    /// their existing behavior when it is absent. This is progress, never a
    /// terminal record.
    Started(Started),
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
// Mirror the public inline Terminal variant without an extra allocation on decode.
#[allow(clippy::large_enum_variant)]
enum KnownStreamRecord {
    Accepted(Accepted),
    Started(Started),
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
// The known-record arm mirrors the public inline Terminal variant.
#[allow(clippy::large_enum_variant)]
enum StreamRecordInput {
    Known(KnownStreamRecord),
    Tag(StreamRecordTag),
}

impl Serialize for StreamRecord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let known = match self {
            Self::Accepted(accepted) => KnownStreamRecord::Accepted(accepted.clone()),
            Self::Started(started) => KnownStreamRecord::Started(started.clone()),
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
                if matches!(
                    kind.as_str(),
                    "accepted" | "started" | "output" | "terminal"
                ) {
                    return Err(serde::de::Error::custom(format!(
                        "malformed fields for known stream record tag {kind}"
                    )));
                }
                return Ok(Self::Unknown { kind, seq });
            }
        };
        Ok(match known {
            KnownStreamRecord::Accepted(accepted) => Self::Accepted(accepted),
            KnownStreamRecord::Started(started) => Self::Started(started),
            KnownStreamRecord::Output(output) => Self::Output(output),
            KnownStreamRecord::Terminal(terminal) => Self::Terminal(terminal),
        })
    }
}
