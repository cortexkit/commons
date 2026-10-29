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

/// Network access requested by a caller or actually granted by a runner.
///
/// A job granted outbound access may have had outside effects. A caller must
/// not automatically rerun it after an unknown outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum Network {
    /// The job may open outbound connections to the internet. This never means
    /// access to the caller's machine or local network; the runner must enforce
    /// that isolation.
    Outbound,
    /// A network mode this version does not recognise. A runner receiving it in
    /// a request must refuse the job before start with
    /// [`RefusalReason::NetworkUnsupported`], not downgrade or upgrade access.
    Unknown(String),
}

impl From<String> for Network {
    fn from(network: String) -> Self {
        match network.as_str() {
            "outbound" => Self::Outbound,
            _ => Self::Unknown(network),
        }
    }
}

impl From<Network> for String {
    fn from(network: Network) -> Self {
        match network {
            Network::Outbound => "outbound".into(),
            Network::Unknown(network) => network,
        }
    }
}

/// The operating system requested for a run or used by the runner.
///
/// An absent request means Linux. Runners must refuse unknown requested platforms
/// before start rather than silently running the command elsewhere.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum Platform {
    /// Run on Linux.
    Linux,
    /// Run on Windows.
    Windows,
    /// Run on macOS.
    Macos,
    /// A platform string this version does not recognise.
    Unknown(String),
}

impl From<String> for Platform {
    fn from(platform: String) -> Self {
        match platform.as_str() {
            "linux" => Self::Linux,
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            _ => Self::Unknown(platform),
        }
    }
}

impl From<Platform> for String {
    fn from(platform: Platform) -> Self {
        match platform {
            Platform::Linux => "linux".into(),
            Platform::Windows => "windows".into(),
            Platform::Macos => "macos".into(),
            Platform::Unknown(platform) => platform,
        }
    }
}

/// `exec.run` parameters; executor-owned IDs and snapshot metadata are absent.
/// Both timeout and queue_wait_limit_s are measured in seconds.
///
/// Use [`Self::new`] and the `with_*` setters so future optional fields remain
/// additive for callers.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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
    /// Requested network access. Absent means offline. The runner enforces the
    /// isolation and reports the access actually granted in [`Accepted::network`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    /// Requested execution platform. Absent means Linux; the runner reports the
    /// platform it actually uses in [`Accepted::platform`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    /// Paths relative to `cwd` to return with the stream. An empty list disables
    /// returned-file collection and its `return_result` record (or, with
    /// `return_chunked`, its `return_chunk` and `return_manifest` records).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub return_paths: Vec<String>,
    /// Total returned-file byte cap. `None` selects [`RETURN_MAX_BYTES_CAP`], or
    /// [`RETURN_CHUNKED_MAX_BYTES_CAP`] when `return_chunked` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_max_bytes: Option<u64>,
    /// Return the requested files as `return_chunk` records followed by one
    /// `return_manifest`, instead of one `return_result`. Set it only for a
    /// runner that advertises [`RETURN_CHUNKED_CAPABILITY`]. It has no effect
    /// when `return_paths` is empty. Omitted on the wire when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub return_chunked: bool,
    /// Opaque caller-chosen idempotency key: 16..=128 bytes of ASCII 0x21..=0x7e.
    /// Send only to an executor advertising [`SUBMISSION_KEY_CAPABILITY`]; older
    /// executors ignore unknown fields and could start a second job on resubmit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_key: Option<String>,
    /// Resume an existing keyed job with records whose seq is strictly greater
    /// than this value. Requires `submission_key`; absent means replay from the
    /// beginning. Send only to an executor advertising [`SUBMISSION_KEY_CAPABILITY`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_from_seq: Option<u64>,
    /// Ask for display-only progress before acceptance. Send `true` only to a
    /// runner advertising [`PROGRESS_CAPABILITY`]. Omitted when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub progress: bool,
}

#[derive(Deserialize)]
struct RunRequestInput {
    workspace_key: String,
    repository_root: String,
    cwd: String,
    command: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    weight_hint: Option<u32>,
    timeout: Option<u64>,
    queue_wait_limit_s: Option<u64>,
    #[serde(default)]
    siblings: Vec<String>,
    network: Option<Network>,
    platform: Option<Platform>,
    #[serde(default)]
    return_paths: Vec<String>,
    return_max_bytes: Option<u64>,
    #[serde(default)]
    return_chunked: bool,
    submission_key: Option<String>,
    resume_from_seq: Option<u64>,
    #[serde(default)]
    progress: bool,
}

impl<'de> Deserialize<'de> for RunRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let input = RunRequestInput::deserialize(deserializer)?;
        if let Some(key) = &input.submission_key {
            if !(16..=128).contains(&key.len())
                || !key.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            {
                return Err(serde::de::Error::custom(
                    "submission_key must be 16..=128 bytes of ASCII 0x21..=0x7e",
                ));
            }
        } else if input.resume_from_seq.is_some() {
            return Err(serde::de::Error::custom(
                "resume_from_seq requires submission_key",
            ));
        }
        Ok(Self {
            workspace_key: input.workspace_key,
            repository_root: input.repository_root,
            cwd: input.cwd,
            command: input.command,
            env: input.env,
            weight_hint: input.weight_hint,
            timeout: input.timeout,
            queue_wait_limit_s: input.queue_wait_limit_s,
            siblings: input.siblings,
            network: input.network,
            platform: input.platform,
            return_paths: input.return_paths,
            return_max_bytes: input.return_max_bytes,
            return_chunked: input.return_chunked,
            submission_key: input.submission_key,
            resume_from_seq: input.resume_from_seq,
            progress: input.progress,
        })
    }
}

// serde's `skip_serializing_if` needs a function taking a reference.
fn is_false(value: &bool) -> bool {
    !*value
}

impl RunRequest {
    /// An offline request with empty env/siblings and executor-default scheduling limits.
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
            network: None,
            platform: None,
            return_paths: Vec::new(),
            return_max_bytes: None,
            return_chunked: false,
            submission_key: None,
            resume_from_seq: None,
            progress: false,
        }
    }

    /// Set caller environment overrides; filtering belongs to the executor.
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Set an idempotency key, only after checking [`SUBMISSION_KEY_CAPABILITY`].
    pub fn with_submission_key(mut self, submission_key: impl Into<String>) -> Self {
        self.submission_key = Some(submission_key.into());
        self
    }

    /// Caller-chosen idempotency key, if supplied.
    pub fn submission_key(&self) -> Option<&str> {
        self.submission_key.as_deref()
    }

    /// Resume after this sequence number; requires a key and the executor's
    /// [`SUBMISSION_KEY_CAPABILITY`] advertisement.
    pub fn with_resume_from_seq(mut self, resume_from_seq: u64) -> Self {
        self.resume_from_seq = Some(resume_from_seq);
        self
    }

    /// Exclusive replay cursor, or `None` to replay from the beginning.
    pub fn resume_from_seq(&self) -> Option<u64> {
        self.resume_from_seq
    }

    /// Ask for pre-acceptance progress only after checking [`PROGRESS_CAPABILITY`].
    pub fn with_progress(mut self, progress: bool) -> Self {
        self.progress = progress;
        self
    }

    /// Whether display-only pre-acceptance progress is requested.
    pub fn progress(&self) -> bool {
        self.progress
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

    /// Request network access; the runner must acknowledge what it granted.
    pub fn with_network(mut self, network: Network) -> Self {
        self.network = Some(network);
        self
    }

    /// Requested network access, or `None` for an offline job.
    pub fn network(&self) -> Option<&Network> {
        self.network.as_ref()
    }

    /// Request a platform; the runner must acknowledge the platform it uses.
    pub fn with_platform(mut self, platform: Platform) -> Self {
        self.platform = Some(platform);
        self
    }

    /// Requested execution platform, or `None` for the default Linux platform.
    pub fn platform(&self) -> Option<&Platform> {
        self.platform.as_ref()
    }

    /// Request files relative to `cwd` in a `return_result` stream record.
    pub fn with_return_paths(mut self, return_paths: Vec<String>) -> Self {
        self.return_paths = return_paths;
        self
    }

    /// Return the requested cwd-relative paths; an empty slice disables file collection.
    pub fn return_paths(&self) -> &[String] {
        &self.return_paths
    }

    /// Set the total returned-file byte cap. Without `return_chunked`, the
    /// runner must reject values above [`RETURN_MAX_BYTES_CAP`]; with it, values
    /// above [`RETURN_CHUNKED_MAX_BYTES_CAP`].
    pub fn with_return_max_bytes(mut self, return_max_bytes: u64) -> Self {
        self.return_max_bytes = Some(return_max_bytes);
        self
    }

    /// Read the explicit total byte cap, or `None` for the default cap.
    pub fn return_max_bytes(&self) -> Option<u64> {
        self.return_max_bytes
    }

    /// Ask for chunked returned files. Only send `true` to a runner that
    /// advertises [`RETURN_CHUNKED_CAPABILITY`].
    pub fn with_return_chunked(mut self, return_chunked: bool) -> Self {
        self.return_chunked = return_chunked;
        self
    }

    /// Whether returned files are requested as chunks plus a manifest.
    pub fn return_chunked(&self) -> bool {
        self.return_chunked
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
    /// Not transient: the runner does not support the requested network mode.
    /// The job was refused before start, without downgrading or upgrading access.
    NetworkUnsupported,
    /// The runner does not support the requested platform. The job was refused
    /// before start; `refusal_detail` carries the requested platform wire name.
    PlatformUnsupported,
    /// An admission that had not started was lost during runner recovery. The
    /// caller may resubmit fresh under its own time budget. The tag alone does
    /// not prove the job never started; that guarantee comes only from the
    /// refused-before-start contract.
    LostToRestart,
    /// The operation or its parameters are not permitted. The caller should
    /// correct or avoid the request rather than retry it unchanged.
    /// `refusal_detail` carries a specific human-only code, such as
    /// `prepare_unsupported` or `direct_timeout_refused`.
    RequestRejected,
    /// The executor could not authenticate the job to its runner: credentials
    /// were unavailable, a grant was refused, the agent or signing failed, or
    /// SSH configuration was invalid. The job's caller cannot fix this, and a
    /// retry is not expected to help. `refusal_detail` carries a safe human-only
    /// fault code, never credential material.
    RunnerAuthenticationFailed,
    /// Final refusal, sent only for a request with `submission_key`: no runner
    /// ever accepted the earlier submission and this resubmit was refused too.
    /// Nothing ran under the key; the caller may resubmit under a new key.
    SubmissionNeverAccepted,
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
            "network_unsupported" => Self::NetworkUnsupported,
            "platform_unsupported" => Self::PlatformUnsupported,
            "lost_to_restart" => Self::LostToRestart,
            "request_rejected" => Self::RequestRejected,
            "runner_authentication_failed" => Self::RunnerAuthenticationFailed,
            "submission_never_accepted" => Self::SubmissionNeverAccepted,
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
            RefusalReason::NetworkUnsupported => "network_unsupported".into(),
            RefusalReason::PlatformUnsupported => "platform_unsupported".into(),
            RefusalReason::LostToRestart => "lost_to_restart".into(),
            RefusalReason::RequestRejected => "request_rejected".into(),
            RefusalReason::RunnerAuthenticationFailed => "runner_authentication_failed".into(),
            RefusalReason::SubmissionNeverAccepted => "submission_never_accepted".into(),
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
    /// The full unsigned 32-bit value reported by `GetExitCodeProcess`. Normal
    /// exits carry their status; crashes carry their NTSTATUS, not a signed
    /// Linux-style exit code. Use [`Self::ntstatus_failure`] to identify crashes.
    ///
    /// Older callers decode this tag as [`Outcome::Unknown`] and must grade it
    /// as `outcome_unknown`, never as proof that the command did not run.
    WindowsExit {
        code: u32,
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
    WindowsExit { code: u32 },
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
            Self::WindowsExit { code } => KnownOutcome::WindowsExit { code: *code },
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
                        | "windows_exit"
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
            KnownOutcome::WindowsExit { code } => Self::WindowsExit { code },
            KnownOutcome::Signal { signal } => Self::Signal { signal },
            KnownOutcome::Cancelled => Self::Cancelled,
            KnownOutcome::RefusedBeforeStart { reason } => Self::RefusedBeforeStart { reason },
            KnownOutcome::OutcomeUnknown => Self::OutcomeUnknown,
            KnownOutcome::HistoryExpired => Self::HistoryExpired,
        })
    }
}

impl Outcome {
    /// Whether this Windows exit code is an NTSTATUS failure.
    ///
    /// Callers should display such codes in hexadecimal as a crash, rather
    /// than as an ordinary process exit status. Other outcome variants return
    /// `false`.
    pub fn ntstatus_failure(&self) -> bool {
        matches!(self, Self::WindowsExit { code } if code & 0xC000_0000 == 0xC000_0000)
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
/// Maximum requested returned-file bytes, also used when `return_max_bytes` is
/// absent. Applies to requests without `return_chunked`.
pub const RETURN_MAX_BYTES_CAP: u64 = 262_144;
/// Capability a runner advertises when it supports `RunRequest.return_chunked`.
pub const RETURN_CHUNKED_CAPABILITY: &str = "exec-remote-return/v2";
/// Capability an executor advertises for idempotent keyed submissions. A caller
/// sends `RunRequest.submission_key` or `RunRequest.resume_from_seq` only after
/// checking this advertisement: older executors silently ignore those fields.
pub const SUBMISSION_KEY_CAPABILITY: &str = "exec-remote-submission-key/v1";
/// Capability a runner advertises when it can send opt-in progress before
/// acceptance. A caller sends `RunRequest.progress: true` only after checking it.
pub const PROGRESS_CAPABILITY: &str = "exec-remote-progress/v1";
/// Maximum requested returned-file bytes with `return_chunked`, also used when
/// `return_max_bytes` is absent from such a request.
pub const RETURN_CHUNKED_MAX_BYTES_CAP: u64 = 10_485_760;
/// Maximum decoded bytes in one `return_chunk` record. Every chunk carries at
/// least one byte.
pub const RETURN_CHUNK_MAX_BYTES: usize = 524_288;

/// Host or pool that ran a hosted job and its optional billing measurement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostedUsage {
    /// Host or pool identifier, such as `ovh-1` or `boat:<sandbox_id>`.
    pub placement: String,
    /// Billed virtual-CPU milliseconds, absent while billing is pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billed_vcpu_ms: Option<u64>,
}

impl HostedUsage {
    /// Hosted placement with billing still pending.
    pub fn new(placement: impl Into<String>) -> Self {
        Self {
            placement: placement.into(),
            billed_vcpu_ms: None,
        }
    }

    /// Set the final billed virtual-CPU duration.
    pub fn with_billed_vcpu_ms(mut self, billed_vcpu_ms: u64) -> Self {
        self.billed_vcpu_ms = Some(billed_vcpu_ms);
        self
    }

    /// Read the host or pool placement.
    pub fn placement(&self) -> &str {
        &self.placement
    }

    /// Read billed virtual-CPU milliseconds, if billing has completed.
    pub fn billed_vcpu_ms(&self) -> Option<u64> {
        self.billed_vcpu_ms
    }
}

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
    /// True only when the producer has proof that no process of the job remains:
    /// the unit was confirmed exited, or a real exit status was read. False is
    /// omitted on the wire and means the same as an absent field: execution has
    /// not been proven to have ended, so the job may still be running. This says
    /// nothing about the exit code or success.
    #[serde(default, skip_serializing_if = "is_false")]
    pub execution_ended: bool,
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
    /// Hosted placement and billing. Build-server jobs do not report this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosted: Option<HostedUsage>,
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
            execution_ended: false,
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
            hosted: None,
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

    /// Record proof that no process of the job remains.
    pub fn with_execution_ended(mut self) -> Self {
        self.execution_ended = true;
        self
    }

    /// Read whether the producer has proved that no process of the job remains.
    pub fn execution_ended(&self) -> bool {
        self.execution_ended
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

    /// Report hosted placement and billing, if available.
    pub fn with_hosted(mut self, hosted: HostedUsage) -> Self {
        self.hosted = Some(hosted);
        self
    }

    /// Read hosted placement and billing, when reported.
    pub fn hosted(&self) -> Option<&HostedUsage> {
        self.hosted.as_ref()
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

/// A file returned from the job's cwd, with decoded bytes and their SHA-256.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReturnedFile {
    /// Path relative to the request's `cwd`.
    pub path: String,
    /// Lowercase hexadecimal SHA-256 of `data`.
    pub sha256: String,
    /// File bytes, encoded as standard padded base64 on the wire.
    pub data: BytePayload,
}

impl ReturnedFile {
    /// Create a returned file from its cwd-relative path, digest, and bytes.
    pub fn new(path: impl Into<String>, sha256: impl Into<String>, data: BytePayload) -> Self {
        Self {
            path: path.into(),
            sha256: sha256.into(),
            data,
        }
    }

    /// Read the cwd-relative path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Read the file digest.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Read the decoded file bytes.
    pub fn data(&self) -> &[u8] {
        &self.data.0
    }
}

/// Why requested files could not be returned. Unknown strings remain readable.
///
/// In chunked mode (`RunRequest.return_chunked`) each cause describes one path
/// in a [`ReturnManifestEntry`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum ReturnPathsFailureCause {
    /// The file would take the total past the cap. In chunked mode this is
    /// decided before any of the file's bytes are sent.
    OverCap,
    /// No such path.
    Missing,
    /// The path, or a symlink it resolves through, leads outside the job root.
    Escape,
    /// The path is not a regular file.
    NotRegular,
    /// The `return_result` record would exceed the runner's record size limit.
    /// Does not occur in chunked mode.
    FrameLimit,
    /// Chunked mode only: reading failed, or the file's bytes changed while it
    /// was read. Chunks may already have been sent for the path; the caller
    /// discards them. It is the only known cause that may follow chunks; a
    /// cause this version does not recognise may also follow them.
    ReadError,
    /// Chunked mode only: a replay after the runner stopped keeping the job's
    /// returned bytes. The entry keeps the file's original size.
    Expired,
    /// A cause this version does not recognise.
    Unknown(String),
}

impl From<String> for ReturnPathsFailureCause {
    fn from(cause: String) -> Self {
        match cause.as_str() {
            "over_cap" => Self::OverCap,
            "missing" => Self::Missing,
            "escape" => Self::Escape,
            "not_regular" => Self::NotRegular,
            "frame_limit" => Self::FrameLimit,
            "read_error" => Self::ReadError,
            "expired" => Self::Expired,
            _ => Self::Unknown(cause),
        }
    }
}

impl From<ReturnPathsFailureCause> for String {
    fn from(cause: ReturnPathsFailureCause) -> Self {
        match cause {
            ReturnPathsFailureCause::OverCap => "over_cap".into(),
            ReturnPathsFailureCause::Missing => "missing".into(),
            ReturnPathsFailureCause::Escape => "escape".into(),
            ReturnPathsFailureCause::NotRegular => "not_regular".into(),
            ReturnPathsFailureCause::FrameLimit => "frame_limit".into(),
            ReturnPathsFailureCause::ReadError => "read_error".into(),
            ReturnPathsFailureCause::Expired => "expired".into(),
            ReturnPathsFailureCause::Unknown(cause) => cause,
        }
    }
}

/// A failed attempt to collect the requested cwd-relative files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReturnPathsFailed {
    pub measured_bytes: u64,
    pub cap_bytes: u64,
    pub cause: ReturnPathsFailureCause,
}

impl ReturnPathsFailed {
    /// Describe a collection failure with its measured byte total, cap, and cause.
    pub fn new(measured_bytes: u64, cap_bytes: u64, cause: ReturnPathsFailureCause) -> Self {
        Self {
            measured_bytes,
            cap_bytes,
            cause,
        }
    }

    /// Read the measured byte total.
    pub fn measured_bytes(&self) -> u64 {
        self.measured_bytes
    }

    /// Read the effective total cap.
    pub fn cap_bytes(&self) -> u64 {
        self.cap_bytes
    }

    /// Read the open-ended collection failure cause.
    pub fn cause(&self) -> &ReturnPathsFailureCause {
        &self.cause
    }
}

/// Result of collecting requested files, emitted before the terminal record.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReturnResult {
    pub job_id: Uuid,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_files: Option<Vec<ReturnedFile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_paths_failed: Option<ReturnPathsFailed>,
}

#[derive(Deserialize)]
struct ReturnResultInput {
    job_id: Uuid,
    seq: u64,
    #[serde(default)]
    returned_files: Option<Vec<ReturnedFile>>,
    #[serde(default)]
    return_paths_failed: Option<ReturnPathsFailed>,
}

impl<'de> Deserialize<'de> for ReturnResult {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let input = ReturnResultInput::deserialize(deserializer)?;
        match (input.returned_files, input.return_paths_failed) {
            (Some(returned_files), None) => Ok(Self {
                job_id: input.job_id,
                seq: input.seq,
                returned_files: Some(returned_files),
                return_paths_failed: None,
            }),
            (None, Some(return_paths_failed)) => Ok(Self {
                job_id: input.job_id,
                seq: input.seq,
                returned_files: None,
                return_paths_failed: Some(return_paths_failed),
            }),
            _ => Err(serde::de::Error::custom(
                "return_result must contain exactly one of returned_files or return_paths_failed",
            )),
        }
    }
}

impl ReturnResult {
    /// A successful file collection, including an empty result list.
    pub fn with_files(job_id: Uuid, seq: u64, returned_files: Vec<ReturnedFile>) -> Self {
        Self {
            job_id,
            seq,
            returned_files: Some(returned_files),
            return_paths_failed: None,
        }
    }

    /// A failed file collection that leaves the command's exit outcome unchanged.
    pub fn failed(job_id: Uuid, seq: u64, failure: ReturnPathsFailed) -> Self {
        Self {
            job_id,
            seq,
            returned_files: None,
            return_paths_failed: Some(failure),
        }
    }

    /// Read the job identifier.
    pub fn job_id(&self) -> Uuid {
        self.job_id
    }

    /// Read the replay sequence number.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Read returned files when collection succeeded.
    pub fn returned_files(&self) -> Option<&[ReturnedFile]> {
        self.returned_files.as_deref()
    }

    /// Read the collection failure when collection did not succeed.
    pub fn return_paths_failed(&self) -> Option<&ReturnPathsFailed> {
        self.return_paths_failed.as_ref()
    }
}

/// A piece of one returned file, sent when the request set `return_chunked`.
///
/// Chunks come grouped by path in ascending `path_index`: a path's chunks are
/// never interleaved with another path's, and the stream never returns to an
/// earlier path. Each path's chunks are contiguous from offset 0. A zero-byte
/// file has no chunks. The bytes are not verified until the
/// [`ReturnManifest`] arrives; [`crate::ReturnAssembler`] applies these rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReturnChunk {
    pub job_id: Uuid,
    /// Replay sequence number for this stream record.
    pub seq: u64,
    /// Position of the file in the request's `return_paths`, from 0.
    pub path_index: u32,
    /// Byte offset of `data` within the file.
    pub offset: u64,
    /// 1 to [`RETURN_CHUNK_MAX_BYTES`] file bytes, encoded as standard padded
    /// base64 on the wire.
    pub data: BytePayload,
}

impl ReturnChunk {
    /// A chunk of the file at `path_index`, starting at `offset`.
    pub fn new(job_id: Uuid, seq: u64, path_index: u32, offset: u64, data: BytePayload) -> Self {
        Self {
            job_id,
            seq,
            path_index,
            offset,
            data,
        }
    }

    /// Read the job identifier.
    pub fn job_id(&self) -> Uuid {
        self.job_id
    }

    /// Read the replay sequence number.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Read the index into the request's `return_paths`.
    pub fn path_index(&self) -> u32 {
        self.path_index
    }

    /// Read the byte offset within the file.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Read the decoded chunk bytes.
    pub fn data(&self) -> &[u8] {
        &self.data.0
    }
}

/// What happened to one requested path in a [`ReturnManifest`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReturnManifestOutcome {
    /// The file was returned. Its received chunk bytes must total `size` and
    /// hash to `sha256` before the caller places the file.
    Returned {
        /// File size in bytes.
        size: u64,
        /// Lowercase hexadecimal SHA-256 of the file's bytes.
        sha256: String,
    },
    /// The file was not returned; any chunks received for it are discarded.
    Failed {
        cause: ReturnPathsFailureCause,
        /// The file's size when the runner knew it, for example for an
        /// over-cap or expired file.
        size: Option<u64>,
    },
}

/// One requested path's entry in a [`ReturnManifest`].
///
/// On the wire a returned entry is `{path, size, sha256, chunks?}` and a failed
/// entry is `{path, cause, size?, chunks?}`. Decoding refuses an entry with both
/// `sha256` and `cause`, with neither, or with `sha256` but no `size`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReturnManifestEntry {
    /// The requested path, echoed exactly as the caller sent it.
    pub path: String,
    pub outcome: ReturnManifestOutcome,
    /// Number of chunks originally sent for this path, including any sent before
    /// a read failure. Preserved on expired replay; omitted by older producers.
    /// A producer that supplies a count must supply it on every manifest entry.
    pub chunks: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct ReturnManifestEntryWire {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cause: Option<ReturnPathsFailureCause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chunks: Option<u32>,
}

impl Serialize for ReturnManifestEntry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match &self.outcome {
            ReturnManifestOutcome::Returned { size, sha256 } => ReturnManifestEntryWire {
                path: self.path.clone(),
                size: Some(*size),
                sha256: Some(sha256.clone()),
                cause: None,
                chunks: self.chunks,
            },
            ReturnManifestOutcome::Failed { cause, size } => ReturnManifestEntryWire {
                path: self.path.clone(),
                size: *size,
                sha256: None,
                cause: Some(cause.clone()),
                chunks: self.chunks,
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ReturnManifestEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ReturnManifestEntryWire::deserialize(deserializer)?;
        let outcome = match (wire.sha256, wire.cause) {
            (Some(sha256), None) => {
                let Some(size) = wire.size else {
                    return Err(serde::de::Error::custom(
                        "a returned return_manifest entry must contain size with sha256",
                    ));
                };
                ReturnManifestOutcome::Returned { size, sha256 }
            }
            (None, Some(cause)) => ReturnManifestOutcome::Failed {
                cause,
                size: wire.size,
            },
            _ => {
                return Err(serde::de::Error::custom(
                    "a return_manifest entry must contain exactly one of sha256 or cause",
                ))
            }
        };
        Ok(Self {
            path: wire.path,
            outcome,
            chunks: wire.chunks,
        })
    }
}

impl ReturnManifestEntry {
    /// A returned file with its size and lowercase hexadecimal SHA-256.
    pub fn returned(path: impl Into<String>, size: u64, sha256: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            outcome: ReturnManifestOutcome::Returned {
                size,
                sha256: sha256.into(),
            },
            chunks: None,
        }
    }

    /// A path that was not returned, with its size unknown.
    pub fn failed(path: impl Into<String>, cause: ReturnPathsFailureCause) -> Self {
        Self {
            path: path.into(),
            outcome: ReturnManifestOutcome::Failed { cause, size: None },
            chunks: None,
        }
    }

    /// Set the file size: the returned size, or the known size of a failed
    /// file such as an over-cap or expired one.
    pub fn with_size(mut self, new_size: u64) -> Self {
        match &mut self.outcome {
            ReturnManifestOutcome::Returned { size, .. } => *size = new_size,
            ReturnManifestOutcome::Failed { size, .. } => *size = Some(new_size),
        }
        self
    }

    /// Set the number of chunks originally sent for this path. Set this on every
    /// entry, including zero for a path refused before its first byte.
    pub fn with_chunks(mut self, chunks: u32) -> Self {
        self.chunks = Some(chunks);
        self
    }

    /// Read the original chunk count, or `None` for an older producer.
    pub fn chunks(&self) -> Option<u32> {
        self.chunks
    }

    /// Read the requested path as echoed by the runner.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Read whether the path was returned or failed.
    pub fn outcome(&self) -> &ReturnManifestOutcome {
        &self.outcome
    }

    /// Read the file size: always present for a returned file, and present for
    /// a failed one only when the runner knew it.
    pub fn size(&self) -> Option<u64> {
        match &self.outcome {
            ReturnManifestOutcome::Returned { size, .. } => Some(*size),
            ReturnManifestOutcome::Failed { size, .. } => *size,
        }
    }

    /// Read the digest of a returned file, or `None` for a failed one.
    pub fn sha256(&self) -> Option<&str> {
        match &self.outcome {
            ReturnManifestOutcome::Returned { sha256, .. } => Some(sha256),
            ReturnManifestOutcome::Failed { .. } => None,
        }
    }

    /// Read the failure cause, or `None` for a returned file.
    pub fn cause(&self) -> Option<&ReturnPathsFailureCause> {
        match &self.outcome {
            ReturnManifestOutcome::Returned { .. } => None,
            ReturnManifestOutcome::Failed { cause, .. } => Some(cause),
        }
    }
}

/// The outcome of every requested path, sent once after the job's
/// `return_chunk` records and immediately before the terminal record when the
/// request set `return_chunked` with non-empty `return_paths`.
///
/// After the runner stops keeping the job's returned bytes, a replay sends no
/// chunks and this manifest at its original `seq`, with every returned entry
/// turned into a failed `expired` entry. A caller that set `return_chunked`
/// must then accept a gap in `seq` in exactly one place: between the last
/// record before the missing chunks and this manifest. Every other gap keeps
/// its usual meaning, and the resume cursor advances to this record's `seq`
/// as for any other record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReturnManifest {
    pub job_id: Uuid,
    /// Replay sequence number for this stream record.
    pub seq: u64,
    /// One entry per requested path, in request order.
    pub files: Vec<ReturnManifestEntry>,
    /// Sum of the returned entries' sizes.
    pub total_bytes: u64,
    /// The effective total byte cap.
    pub cap_bytes: u64,
}

impl ReturnManifest {
    /// A manifest with one entry per requested path, in request order.
    pub fn new(
        job_id: Uuid,
        seq: u64,
        files: Vec<ReturnManifestEntry>,
        total_bytes: u64,
        cap_bytes: u64,
    ) -> Self {
        Self {
            job_id,
            seq,
            files,
            total_bytes,
            cap_bytes,
        }
    }

    /// Read the job identifier.
    pub fn job_id(&self) -> Uuid {
        self.job_id
    }

    /// Read the replay sequence number.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Read the per-path entries, in request order.
    pub fn files(&self) -> &[ReturnManifestEntry] {
        &self.files
    }

    /// Read the sum of the returned entries' sizes.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Read the effective total byte cap.
    pub fn cap_bytes(&self) -> u64 {
        self.cap_bytes
    }
}

/// A mapping between caller paths and the paths used by a runner.
///
/// A runner that rewrites paths sends `host_prefix` in the caller's form (for
/// example, `/`) and `guest_prefix` (for example, `C:\m`). A caller path P
/// maps to `guest_prefix` + P with `/` turned into `\`. When a job starts,
/// any path that cannot be mapped is refused. In output, only paths inside the
/// mapped folder are translated back; anything else is passed through
/// unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct PathMapping {
    pub host_prefix: String,
    pub guest_prefix: String,
}

impl PathMapping {
    /// Describe the caller and runner prefixes used to rewrite paths.
    pub fn new(host_prefix: impl Into<String>, guest_prefix: impl Into<String>) -> Self {
        Self {
            host_prefix: host_prefix.into(),
            guest_prefix: guest_prefix.into(),
        }
    }

    pub fn with_host_prefix(mut self, host_prefix: impl Into<String>) -> Self {
        self.host_prefix = host_prefix.into();
        self
    }

    pub fn with_guest_prefix(mut self, guest_prefix: impl Into<String>) -> Self {
        self.guest_prefix = guest_prefix.into();
        self
    }

    pub fn host_prefix(&self) -> &str {
        &self.host_prefix
    }

    pub fn guest_prefix(&self) -> &str {
        &self.guest_prefix
    }
}

/// The pre-acceptance phase currently being reported by the runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
#[serde(from = "String", into = "String")]
pub enum ProgressPhase {
    /// Uploading inputs.
    Upload,
    /// Waiting for a host with room.
    PlacementWait,
    /// Connecting to the selected host.
    Connect,
    /// An unrecognised phase. Show generic progress; this is display-only.
    Unknown(String),
}

impl From<String> for ProgressPhase {
    fn from(phase: String) -> Self {
        match phase.as_str() {
            "upload" => Self::Upload,
            "placement_wait" => Self::PlacementWait,
            "connect" => Self::Connect,
            _ => Self::Unknown(phase),
        }
    }
}

impl From<ProgressPhase> for String {
    fn from(phase: ProgressPhase) -> Self {
        match phase {
            ProgressPhase::Upload => "upload".into(),
            ProgressPhase::PlacementWait => "placement_wait".into(),
            ProgressPhase::Connect => "connect".into(),
            ProgressPhase::Unknown(phase) => phase,
        }
    }
}

/// A progress detail exceeded 120 UTF-8 bytes or contained a control character
/// or line break.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProgressDetailError;

impl std::fmt::Display for ProgressDetailError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "progress detail must be at most 120 bytes with no control characters or line breaks",
        )
    }
}

impl std::error::Error for ProgressDetailError {}

/// Display-only progress sent before [`Accepted`], never terminal or replayed.
///
/// The runner sends it only for an opted-in request with [`PROGRESS_CAPABILITY`],
/// at most once per 10 seconds and only after 2 seconds of waiting. It carries no
/// sequence number and never advances a resume cursor. Callers render detail as
/// plain text, never start, extend or reset timers from progress, and never retry
/// or cancel because of it. Producers must omit host names, network addresses,
/// tenant or account identifiers, and job contents from detail.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct Progress {
    phase: ProgressPhase,
    phase_elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Deserialize)]
struct ProgressInput {
    phase: ProgressPhase,
    phase_elapsed_ms: u64,
    detail: Option<String>,
}

impl<'de> Deserialize<'de> for Progress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let input = ProgressInput::deserialize(deserializer)?;
        Self::new(input.phase, input.phase_elapsed_ms, input.detail)
            .map_err(serde::de::Error::custom)
    }
}

impl Progress {
    /// Report the current phase and milliseconds since that phase started.
    ///
    /// Returns an error if detail exceeds 120 UTF-8 bytes or contains a control
    /// character or line break. Detail is plain text and must not contain host
    /// names, network addresses, tenant or account identifiers, or job contents.
    pub fn new(
        phase: ProgressPhase,
        phase_elapsed_ms: u64,
        detail: Option<String>,
    ) -> Result<Self, ProgressDetailError> {
        if detail.as_ref().is_some_and(|detail| {
            detail.len() > 120
                || detail.chars().any(|character| {
                    character.is_control() || matches!(character, '\u{2028}' | '\u{2029}')
                })
        }) {
            return Err(ProgressDetailError);
        }
        Ok(Self {
            phase,
            phase_elapsed_ms,
            detail,
        })
    }

    /// The current phase; show unknown phases generically.
    pub fn phase(&self) -> &ProgressPhase {
        &self.phase
    }

    /// Milliseconds since the current phase started, not since submission.
    pub fn phase_elapsed_ms(&self) -> u64 {
        self.phase_elapsed_ms
    }

    /// Optional one-line plain-text detail of at most 120 UTF-8 bytes.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
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
    /// Path prefixes used by the runner, absent when it does not rewrite paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_mapping: Option<PathMapping>,
}

impl Started {
    /// Record the moment the job began running.
    pub fn new(seq: u64, queue_wait_ms: u64, started_at_ms: u64) -> Self {
        Self {
            seq,
            queue_wait_ms,
            started_at_ms,
            path_mapping: None,
        }
    }

    pub fn with_path_mapping(mut self, path_mapping: PathMapping) -> Self {
        self.path_mapping = Some(path_mapping);
        self
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

    pub fn path_mapping(&self) -> Option<&PathMapping> {
        self.path_mapping.as_ref()
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
    /// Network access the runner actually granted. `None` means the runner
    /// predates this field, so the caller cannot assume access. If a caller asked
    /// for [`Network::Outbound`] and this is not `Some(Network::Outbound)`, the
    /// job runs offline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    /// Execution platform the runner will use. Runners that understand the
    /// request must fill this with the platform they will actually use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    /// True when attaching to the existing job named by `job_id`, not starting
    /// another job. Omitted when false. Older runners never send this field, so
    /// absence means "started, or a runner that doesn't support keys". Rely on
    /// it only after checking [`SUBMISSION_KEY_CAPABILITY`].
    #[serde(default, skip_serializing_if = "is_false")]
    pub attached: bool,
}

impl Accepted {
    pub fn new(job_id: Uuid, queue_position: u32) -> Self {
        Self {
            job_id,
            queue_position,
            env_not_forwarded: None,
            network: None,
            platform: None,
            attached: false,
        }
    }

    /// Report which caller environment variable names were not forwarded.
    pub fn with_env_not_forwarded(mut self, names: Vec<String>) -> Self {
        self.env_not_forwarded = Some(names);
        self
    }

    /// Report attachment to an existing keyed job rather than a new start.
    pub fn with_attached(mut self, attached: bool) -> Self {
        self.attached = attached;
        self
    }

    /// Whether this acceptance attaches to an existing keyed job. Only rely on
    /// the value after checking [`SUBMISSION_KEY_CAPABILITY`].
    pub fn attached(&self) -> bool {
        self.attached
    }

    /// Report the network access actually granted by the runner.
    pub fn with_network(mut self, network: Network) -> Self {
        self.network = Some(network);
        self
    }

    /// Network access actually granted, or `None` when not acknowledged.
    pub fn network(&self) -> Option<&Network> {
        self.network.as_ref()
    }

    /// Report the execution platform the runner will actually use.
    pub fn with_platform(mut self, platform: Platform) -> Self {
        self.platform = Some(platform);
        self
    }

    /// Execution platform the runner will use, or `None` when not acknowledged.
    pub fn platform(&self) -> Option<&Platform> {
        self.platform.as_ref()
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
    /// Opt-in display-only progress before acceptance. Never terminal, carries
    /// no sequence number, and is never replayed by `exec.attach`.
    Progress(Progress),
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
    /// Returned-file collection requested by the caller, immediately before terminal.
    ReturnResult(ReturnResult),
    /// A piece of a returned file, sent only when the request set
    /// `return_chunked`. Nonterminal.
    ReturnChunk(ReturnChunk),
    /// The per-path outcome of chunked returned files, immediately before
    /// terminal. Nonterminal.
    ReturnManifest(ReturnManifest),
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
    Progress(Progress),
    Accepted(Accepted),
    Started(Started),
    Output(Output),
    ReturnResult(ReturnResult),
    ReturnChunk(ReturnChunk),
    ReturnManifest(ReturnManifest),
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
            Self::Progress(progress) => KnownStreamRecord::Progress(progress.clone()),
            Self::Accepted(accepted) => KnownStreamRecord::Accepted(accepted.clone()),
            Self::Started(started) => KnownStreamRecord::Started(started.clone()),
            Self::Output(output) => KnownStreamRecord::Output(output.clone()),
            Self::ReturnResult(result) => KnownStreamRecord::ReturnResult(result.clone()),
            Self::ReturnChunk(chunk) => KnownStreamRecord::ReturnChunk(chunk.clone()),
            Self::ReturnManifest(manifest) => KnownStreamRecord::ReturnManifest(manifest.clone()),
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
                    "progress"
                        | "accepted"
                        | "started"
                        | "output"
                        | "return_result"
                        | "return_chunk"
                        | "return_manifest"
                        | "terminal"
                ) {
                    return Err(serde::de::Error::custom(format!(
                        "malformed fields for known stream record tag {kind}"
                    )));
                }
                return Ok(Self::Unknown { kind, seq });
            }
        };
        Ok(match known {
            KnownStreamRecord::Progress(progress) => Self::Progress(progress),
            KnownStreamRecord::Accepted(accepted) => Self::Accepted(accepted),
            KnownStreamRecord::Started(started) => Self::Started(started),
            KnownStreamRecord::Output(output) => Self::Output(output),
            KnownStreamRecord::ReturnResult(result) => Self::ReturnResult(result),
            KnownStreamRecord::ReturnChunk(chunk) => Self::ReturnChunk(chunk),
            KnownStreamRecord::ReturnManifest(manifest) => Self::ReturnManifest(manifest),
            KnownStreamRecord::Terminal(terminal) => Self::Terminal(terminal),
        })
    }
}
