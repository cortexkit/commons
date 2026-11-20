//! Shared hub names and permissions for the cross-computer message plane.

use std::fmt;
use std::str::FromStr;

use crate::{
    validate_token, DenyEntry, DiscardPolicy, NamingError, Operation, StreamKind, StreamSpec,
    TokenKind, GIB, HOUR, MIB,
};

pub const HUB_DOMAIN: &str = "hub";
pub const HUB_API_PREFIX: &str = "$JS.hub.API";
pub const FEDERATED_FAMILIES: [&str; 4] = ["room", "wake", "peer", "sync"];
pub const HUB_IDLOG_STREAM: &str = "HUB_IDLOG";
pub const HUB_ROOMS_STREAM: &str = "HUB_ROOMS";
pub const HUB_ROOMS_SUBJECT: &str = "ck.fleet.room.>";
pub const HUB_MEMORY_STREAM: &str = "HUB_MEMORY";
pub const HUB_MEMORY_SUBJECT: &str = "ck.fleet.memory.*.*";
pub const MEMORY_KIND_PROJECT: &str = "project";
pub const MEMORY_KIND_WORKSPACE: &str = "workspace";
pub const MEMORY_KIND_GLOBAL: &str = "global";
pub const MEMORY_KIND_OBSERVATION: &str = "observation";
pub const MEMORY_KIND_PRIMER: &str = "primer";
pub const MEMORY_KINDS: [&str; 5] = [
    MEMORY_KIND_PROJECT,
    MEMORY_KIND_WORKSPACE,
    MEMORY_KIND_GLOBAL,
    MEMORY_KIND_OBSERVATION,
    MEMORY_KIND_PRIMER,
];
/// ck-bus's one durable on the box account's FEDSEND stream: it reads each
/// outgoing hand-off frame once, seals it and stores it in FEDOUT.
pub const FEDSEND_DURABLE: &str = "c_ckbus_fedsend";
pub const FEDOUT_STREAM: &str = "FEDOUT";
pub const FEDOUT_SUBJECT: &str = "fedout.>";
pub const IDLOG_SUBJECT: &str = "ck.fleet.idlog";
pub const RESIDENCE_BUCKET: &str = "residence";
pub const RESIDENCE_STREAM: &str = "KV_residence";
pub const VAULT_SHARED_LOGINS_BUCKET: &str = "vault-shared-logins";
pub const VAULT_SHARED_LOGINS_STREAM: &str = "KV_vault-shared-logins";
pub const VAULT_MANIFEST_ACTIVE_BUCKET: &str = "vault-manifest-active";
pub const VAULT_MANIFEST_ACTIVE_STREAM: &str = "KV_vault-manifest-active";
pub const CUTS_BUCKET: &str = "cuts";
pub const CUTS_STREAM: &str = "OBJ_cuts";

const VAULT_SHARED_LOGINS_MAX_MESSAGE_SIZE: i32 = 256 * 1024;

const HUB_MEMORY_MODULE: &str = "magic-context";
const HUB_ROOMS_MODULE: &str = "prefrontal";
const HUB_ROOMS_MODULE_LEGACY: &str = "prefrontal-core";
const HUB_IDLOG_MODULE: &str = "entorhinal";
const HUB_CUTS_MODULE: &str = "engram";
const HUB_VAULT_MODULE: &str = "claustrum";
const HUB_MANIFEST_READER_MODULE: &str = "plexus";

/// Canonical module IDs with direct hub access. Use [`is_hub_module`] to also check the legacy ID.
/// The vault module uses [`VAULT_SHARED_LOGINS_STREAM`] and [`VAULT_MANIFEST_ACTIVE_STREAM`];
/// the manifest reader uses only [`VAULT_MANIFEST_ACTIVE_STREAM`].
pub const HUB_MODULES: [&str; 6] = [
    HUB_MEMORY_MODULE,
    HUB_ROOMS_MODULE,
    HUB_IDLOG_MODULE,
    HUB_CUTS_MODULE,
    HUB_VAULT_MODULE,
    HUB_MANIFEST_READER_MODULE,
];

/// Whether the caller's module ID is eligible for a hub credential.
/// `prefrontal-core` is accepted only during prefrontal's rename, until the old name retires.
pub fn is_hub_module(module_id: &str) -> bool {
    HUB_MODULES.contains(&module_id) || module_id == HUB_ROOMS_MODULE_LEGACY
}

/// A machine id in its canonical, unprefixed 32-character lowercase hexadecimal form.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BoxId(String);

/// The supplied machine id was not exactly 32 lowercase hexadecimal characters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidBoxId;

impl fmt::Display for InvalidBoxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("box id must be exactly 32 lowercase hexadecimal characters")
    }
}

impl std::error::Error for InvalidBoxId {}

impl BoxId {
    /// Refuses alternate spellings because the id is embedded in stream permissions.
    pub fn parse(value: &str) -> Result<Self, InvalidBoxId> {
        if !is_lowercase_hex(value, 32) {
            return Err(InvalidBoxId);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BoxId {
    type Err = InvalidBoxId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl fmt::Display for BoxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The first 16 lowercase hex characters of the SHA-256 pin of a hub's
/// Subject Public Key Info (SPKI).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HubName(String);

/// The supplied hub name or key pin had the wrong length or was not lowercase hexadecimal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidHubName;

impl fmt::Display for InvalidHubName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "expected a 16-character hub name or 64-character key pin in lowercase hexadecimal",
        )
    }
}

impl std::error::Error for InvalidHubName {}

impl HubName {
    /// Validates the entire key pin before taking its first 16 characters.
    pub fn from_key_pin(value: &str) -> Result<Self, InvalidHubName> {
        if !is_lowercase_hex(value, 64) {
            return Err(InvalidHubName);
        }
        Ok(Self(value[..16].to_owned()))
    }

    /// Parses the canonical short form used in stream names and permissions.
    pub fn parse(value: &str) -> Result<Self, InvalidHubName> {
        if !is_lowercase_hex(value, 16) {
            return Err(InvalidHubName);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for HubName {
    type Err = InvalidHubName;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl fmt::Display for HubName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn is_lowercase_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Names the per-machine inbox separately on each hub so purge subjects are hub-specific.
pub fn hub_inbox_stream_name(box_id: &BoxId, hub_name: &HubName) -> String {
    format!("HUB_INBOX_{box_id}_{hub_name}")
}

pub fn inbox_stream_name(box_id: &BoxId) -> String {
    format!("INBOX_{box_id}")
}

pub fn hub_room_subject(room_id: &str) -> Result<String, NamingError> {
    validate_token(TokenKind::RoomId, room_id)?;
    Ok(format!("ck.fleet.room.{room_id}"))
}

/// Address a shared memory on the hub rather than on an individual computer.
/// Project memories and primers use a project ID, workspace memories use a
/// workspace ID, and global memories and observations use `all`.
pub fn hub_memory_subject(kind: &str, owner: &str) -> Result<String, NamingError> {
    validate_token(TokenKind::MemoryKind, kind)?;
    validate_token(TokenKind::MemoryOwner, owner)?;
    if matches!(kind, MEMORY_KIND_GLOBAL | MEMORY_KIND_OBSERVATION) && owner != "all" {
        return Err(NamingError::new(
            TokenKind::MemoryOwner,
            owner,
            "global and observation memories require owner all",
        ));
    }
    Ok(format!("ck.fleet.memory.{kind}.{owner}"))
}

/// Sealed outbound frames live in the federation account, never the box account.
pub fn fedout_stream() -> StreamSpec {
    crate::limits::handoff_stream(
        StreamKind::Fedout,
        FEDOUT_STREAM.to_owned(),
        FEDOUT_SUBJECT.to_owned(),
    )
}

pub fn deliver_prefix(box_id: &BoxId) -> String {
    format!("ckdel.{box_id}")
}

/// Each module's inbox stays inside its own deliver-subject export.
pub fn module_deliver_prefix(box_id: &BoxId, module: &str) -> Result<String, NamingError> {
    validate_token(TokenKind::ModuleId, module)?;
    Ok(format!("{}.{module}", deliver_prefix(box_id)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HubKind {
    SelfHosted,
    Hosted,
}

/// A hub-only stream, with the settings absent from the local stream model.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HubStreamSpec {
    pub stream: StreamSpec,
    /// `None` leaves replication to the server default.
    pub replicas: Option<usize>,
    pub allow_rollup_hdrs: bool,
    pub allow_direct: bool,
    pub deny_delete: bool,
}

impl HubStreamSpec {
    pub fn new(stream: StreamSpec) -> Self {
        Self {
            stream,
            replicas: None,
            allow_rollup_hdrs: false,
            allow_direct: false,
            deny_delete: false,
        }
    }
}

/// The KV configuration and its backing stream describe the same bounds.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct KvBucketSpec {
    pub bucket: String,
    pub history: i64,
    pub backing_stream: HubStreamSpec,
}

impl KvBucketSpec {
    pub fn new(bucket: String, history: i64, backing_stream: HubStreamSpec) -> Self {
        Self {
            bucket,
            history,
            backing_stream,
        }
    }
}

pub fn hub_inbox_stream(box_id: &BoxId, hub_name: &HubName) -> HubStreamSpec {
    let subjects = FEDERATED_FAMILIES
        .iter()
        .map(|family| format!("ck.{box_id}.{family}.>"))
        .collect();
    let mut stream = StreamSpec::new(
        StreamKind::HubInbox,
        hub_inbox_stream_name(box_id, hub_name),
        subjects,
    );
    stream.max_age = HOUR * 24 * 7;
    stream.discard = DiscardPolicy::New;
    HubStreamSpec::new(stream)
}

pub fn hub_idlog_stream() -> HubStreamSpec {
    // A reader may resume from sequence 1, so neither age nor size may evict the log.
    // Discard new: when the account's storage quota is full, refuse the append and
    // keep every stored entry, rather than dropping the oldest one.
    let mut stream = StreamSpec::new(
        StreamKind::HubIdlog,
        HUB_IDLOG_STREAM.to_owned(),
        vec![IDLOG_SUBJECT.to_owned()],
    );
    stream.discard = DiscardPolicy::New;
    let mut spec = HubStreamSpec::new(stream);
    spec.allow_direct = true;
    spec
}

pub fn hub_rooms_stream() -> HubStreamSpec {
    let mut stream = StreamSpec::new(
        StreamKind::HubRooms,
        HUB_ROOMS_STREAM.to_owned(),
        vec![HUB_ROOMS_SUBJECT.to_owned()],
    );
    stream.max_msgs_per_subject = Some(100_000);
    stream.max_message_size = Some(256 * 1024);
    let mut spec = HubStreamSpec::new(stream);
    spec.allow_direct = true;
    spec
}

pub fn hub_memory_stream() -> HubStreamSpec {
    let mut stream = StreamSpec::new(
        StreamKind::HubMemory,
        HUB_MEMORY_STREAM.to_owned(),
        vec![HUB_MEMORY_SUBJECT.to_owned()],
    );
    // New computers replay from the start, so refuse new writes rather than evict history.
    stream.discard = DiscardPolicy::New;
    stream.max_bytes = GIB;
    stream.max_message_size = Some(64 * 1024);
    let mut spec = HubStreamSpec::new(stream);
    spec.allow_direct = true;
    spec
}

pub fn residence_bucket(hub: HubKind) -> KvBucketSpec {
    let mut spec = hub_kv_bucket(
        hub,
        StreamKind::KvResidence,
        RESIDENCE_BUCKET,
        RESIDENCE_STREAM,
        16 * 1024,
        64 * MIB,
    );
    // Residence uses KV purge markers, which require subject rollup.
    spec.backing_stream.allow_rollup_hdrs = true;
    spec
}

/// Stores sealed shared logins under `{home}.{member}`, both canonical [`BoxId`]s.
pub fn vault_shared_logins_bucket(hub: HubKind) -> KvBucketSpec {
    hub_kv_bucket(
        hub,
        StreamKind::KvVaultSharedLogins,
        VAULT_SHARED_LOGINS_BUCKET,
        VAULT_SHARED_LOGINS_STREAM,
        VAULT_SHARED_LOGINS_MAX_MESSAGE_SIZE,
        64 * MIB,
    )
}

/// Stores each computer's active vault manifest under its canonical [`BoxId`].
pub fn vault_manifest_active_bucket(hub: HubKind) -> KvBucketSpec {
    hub_kv_bucket(
        hub,
        StreamKind::KvVaultManifestActive,
        VAULT_MANIFEST_ACTIVE_BUCKET,
        VAULT_MANIFEST_ACTIVE_STREAM,
        16 * 1024,
        16 * MIB,
    )
}

fn hub_kv_bucket(
    hub: HubKind,
    kind: StreamKind,
    bucket: &str,
    stream: &str,
    max_message_size: i32,
    max_bytes: u64,
) -> KvBucketSpec {
    let mut stream = StreamSpec::new(kind, stream.to_owned(), vec![format!("$KV.{bucket}.>")]);
    stream.max_msgs_per_subject = Some(1);
    stream.max_message_size = Some(max_message_size);
    stream.max_bytes = max_bytes;
    stream.discard = DiscardPolicy::New;
    let mut backing_stream = HubStreamSpec::new(stream);
    // Leave rollup off by default: a stream-wide rollup would let a writer
    // remove records outside its own key prefix.
    backing_stream.allow_direct = true;
    backing_stream.deny_delete = true;
    backing_stream.replicas = Some(match hub {
        HubKind::SelfHosted => 1,
        HubKind::Hosted => 3,
    });
    KvBucketSpec::new(bucket.to_owned(), 1, backing_stream)
}

pub fn cuts_object_store() -> HubStreamSpec {
    let mut stream = StreamSpec::new(
        StreamKind::ObjCuts,
        CUTS_STREAM.to_owned(),
        vec![
            format!("$O.{CUTS_BUCKET}.C.>"),
            format!("$O.{CUTS_BUCKET}.M.>"),
        ],
    );
    stream.max_bytes = 4 * GIB;
    stream.max_age = HOUR * 24 * 7;
    stream.discard = DiscardPolicy::New;
    let mut spec = HubStreamSpec::new(stream);
    // Match async-nats 0.50.0's object-store creation settings: puts and deletes
    // replace their metadata using subject rollups.
    spec.allow_rollup_hdrs = true;
    spec.allow_direct = true;
    spec
}

/// Provision these only on a hub, never on the machine's local server.
pub fn hub_streams(box_id: &BoxId, hub_name: &HubName, hub: HubKind) -> Vec<HubStreamSpec> {
    vec![
        hub_inbox_stream(box_id, hub_name),
        hub_idlog_stream(),
        hub_rooms_stream(),
        hub_memory_stream(),
        residence_bucket(hub).backing_stream,
        cuts_object_store(),
        vault_shared_logins_bucket(hub).backing_stream,
        vault_manifest_active_bucket(hub).backing_stream,
    ]
}

/// Allow-only subjects, in a fixed order shared by every hub issuer.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LeafUserGrant {
    pub publish: Vec<String>,
    pub subscribe: Vec<String>,
}

impl LeafUserGrant {
    pub fn new(publish: Vec<String>, subscribe: Vec<String>) -> Self {
        Self { publish, subscribe }
    }
}

/// Grants inbox operations only on the issuing hub's per-machine stream.
pub fn leaf_user_grant(box_id: &BoxId, hub_name: &HubName) -> LeafUserGrant {
    let mut publish: Vec<String> = FEDERATED_FAMILIES
        .iter()
        .map(|family| format!("ck.*.{family}.>"))
        .collect();
    publish.extend(["$JS.FC.>", "$JS.ACK.>", "_INBOX.>"].map(str::to_owned));
    let inbox = hub_inbox_stream_name(box_id, hub_name);
    // Sources create an unnamed consumer on the bare subject, not its `.>` form.
    for suffix in [
        format!("CONSUMER.CREATE.{inbox}"),
        format!("CONSUMER.CREATE.{inbox}.>"),
        format!("CONSUMER.INFO.{inbox}.>"),
        format!("CONSUMER.DELETE.{inbox}.>"),
        format!("STREAM.INFO.{inbox}"),
        format!("STREAM.PURGE.{inbox}"),
    ] {
        publish.push(format!("{HUB_API_PREFIX}.{suffix}"));
    }
    publish.extend([
        format!("{HUB_API_PREFIX}.$KV.{RESIDENCE_BUCKET}.>"),
        // Vault writes must stay under the leaf's own box ID.
        format!("{HUB_API_PREFIX}.$KV.{VAULT_SHARED_LOGINS_BUCKET}.{box_id}.>"),
        format!("{HUB_API_PREFIX}.$KV.{VAULT_MANIFEST_ACTIVE_BUCKET}.{box_id}"),
        format!("$O.{CUTS_BUCKET}.>"),
        IDLOG_SUBJECT.to_owned(),
        HUB_ROOMS_SUBJECT.to_owned(),
        HUB_MEMORY_SUBJECT.to_owned(),
    ]);
    for stream in [
        HUB_IDLOG_STREAM,
        HUB_ROOMS_STREAM,
        HUB_MEMORY_STREAM,
        RESIDENCE_STREAM,
        CUTS_STREAM,
        VAULT_SHARED_LOGINS_STREAM,
        VAULT_MANIFEST_ACTIVE_STREAM,
    ] {
        publish.push(format!("{HUB_API_PREFIX}.STREAM.INFO.{stream}"));
        // The active manifest readers use direct gets, not STREAM.MSG.GET.
        if stream != VAULT_MANIFEST_ACTIVE_STREAM {
            publish.push(format!("{HUB_API_PREFIX}.STREAM.MSG.GET.{stream}"));
        }
        for suffix in [
            format!("DIRECT.GET.{stream}"),
            format!("DIRECT.GET.{stream}.>"),
            format!("CONSUMER.CREATE.{stream}"),
            format!("CONSUMER.CREATE.{stream}.>"),
            format!("CONSUMER.INFO.{stream}.>"),
            format!("CONSUMER.DELETE.{stream}.>"),
        ] {
            publish.push(format!("{HUB_API_PREFIX}.{suffix}"));
        }
    }
    // Releasing an object purges its chunks; the other shared streams get no purge.
    publish.push(format!("{HUB_API_PREFIX}.STREAM.PURGE.{CUTS_STREAM}"));
    LeafUserGrant::new(
        publish,
        vec![
            format!("ck.{box_id}.>"),
            "_INBOX.>".to_owned(),
            format!("{}.>", deliver_prefix(box_id)),
            "$JS.FC.>".to_owned(),
            "$JSC.R.>".to_owned(),
        ],
    )
}

/// Ordered permissions for a module's hub-only user in the federation account.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HubUserGrant {
    pub publish: Vec<String>,
    pub subscribe: Vec<String>,
    /// Explicit denies to attach alongside both allow lists.
    pub denies: Vec<DenyEntry>,
}

impl HubUserGrant {
    pub fn new(publish: Vec<String>, subscribe: Vec<String>, denies: Vec<DenyEntry>) -> Self {
        Self {
            publish,
            subscribe,
            denies,
        }
    }
}

/// Limit a module to its shared hub streams and its own delivery and reply subjects.
/// Use the module ID verified by the daemon, not one supplied in a credential request.
pub fn hub_module_grant(box_id: &BoxId, module_id: &str) -> Option<HubUserGrant> {
    let (streams, extra_publish): (&[&str], _) = match module_id {
        HUB_MEMORY_MODULE => (
            &[HUB_MEMORY_STREAM],
            vec![
                HUB_MEMORY_SUBJECT.to_owned(),
                format!("$JS.ACK.{HUB_MEMORY_STREAM}.>"),
                format!("$JS.FC.{HUB_MEMORY_STREAM}.*.*"),
            ],
        ),
        HUB_ROOMS_MODULE | HUB_ROOMS_MODULE_LEGACY => (
            &[HUB_ROOMS_STREAM],
            vec![
                HUB_ROOMS_SUBJECT.to_owned(),
                format!("$JS.ACK.{HUB_ROOMS_STREAM}.>"),
                format!("$JS.FC.{HUB_ROOMS_STREAM}.*.*"),
            ],
        ),
        HUB_IDLOG_MODULE => (
            &[HUB_IDLOG_STREAM, RESIDENCE_STREAM],
            vec![
                IDLOG_SUBJECT.to_owned(),
                format!("{HUB_API_PREFIX}.$KV.{RESIDENCE_BUCKET}.>"),
                // Read the residence epoch from the leader before sealed writes:
                // a direct get can return a stale epoch from a lagging replica.
                format!("{HUB_API_PREFIX}.STREAM.MSG.GET.{RESIDENCE_STREAM}"),
                format!("$JS.FC.{HUB_IDLOG_STREAM}.*.*"),
                format!("$JS.FC.{RESIDENCE_STREAM}.*.*"),
            ],
        ),
        HUB_CUTS_MODULE => (
            &[CUTS_STREAM],
            vec![
                format!("$O.{CUTS_BUCKET}.>"),
                format!("{HUB_API_PREFIX}.STREAM.MSG.GET.{CUTS_STREAM}"),
                format!("{HUB_API_PREFIX}.STREAM.PURGE.{CUTS_STREAM}"),
                format!("$JS.FC.{CUTS_STREAM}.*.*"),
            ],
        ),
        HUB_VAULT_MODULE => (
            &[VAULT_SHARED_LOGINS_STREAM, VAULT_MANIFEST_ACTIVE_STREAM],
            vec![
                format!("{HUB_API_PREFIX}.$KV.{VAULT_SHARED_LOGINS_BUCKET}.{box_id}.>"),
                format!("{HUB_API_PREFIX}.$KV.{VAULT_MANIFEST_ACTIVE_BUCKET}.{box_id}"),
                format!("{HUB_API_PREFIX}.STREAM.MSG.GET.{VAULT_SHARED_LOGINS_STREAM}"),
                format!("$JS.FC.{VAULT_SHARED_LOGINS_STREAM}.*.*"),
                format!("$JS.FC.{VAULT_MANIFEST_ACTIVE_STREAM}.*.*"),
            ],
        ),
        HUB_MANIFEST_READER_MODULE => (
            &[VAULT_MANIFEST_ACTIVE_STREAM],
            vec![format!("$JS.FC.{VAULT_MANIFEST_ACTIVE_STREAM}.*.*")],
        ),
        _ => return None,
    };
    let mut publish = Vec::new();
    for stream in streams {
        // `>` requires a trailing token, so direct gets and consumer creation need bare forms too.
        publish.extend([
            format!("{HUB_API_PREFIX}.STREAM.INFO.{stream}"),
            format!("{HUB_API_PREFIX}.DIRECT.GET.{stream}"),
            format!("{HUB_API_PREFIX}.DIRECT.GET.{stream}.>"),
            format!("{HUB_API_PREFIX}.CONSUMER.CREATE.{stream}"),
            format!("{HUB_API_PREFIX}.CONSUMER.CREATE.{stream}.>"),
            format!("{HUB_API_PREFIX}.CONSUMER.INFO.{stream}.>"),
            format!("{HUB_API_PREFIX}.CONSUMER.DELETE.{stream}.>"),
        ]);
    }
    publish.extend(extra_publish);

    let inbox_prefix = module_deliver_prefix(box_id, module_id).ok()?;
    let denies = [Operation::Publish, Operation::Subscribe]
        .into_iter()
        .flat_map(|operation| {
            ["fedlocal.>", FEDOUT_SUBJECT, "fedsend.>"]
                .map(|subject| DenyEntry::new(operation, subject.to_owned()))
        })
        .collect();
    Some(HubUserGrant::new(
        publish,
        vec![format!("{inbox_prefix}.>")],
        denies,
    ))
}

#[cfg(test)]
mod tests {
    use crate::limits::{subject_filter_is_subset, subject_filters_overlap};

    use super::*;

    const SAMPLE_BOX: &str = "0123456789abcdef0123456789abcdef";
    const OTHER_BOX: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn federation_leaf_grant_covers_every_hub_module_grant() {
        let hub_name = HubName::parse("0f1e2d3c4b5a6978").unwrap();
        for id in [SAMPLE_BOX, OTHER_BOX] {
            let box_id = BoxId::parse(id).unwrap();
            let leaf = leaf_user_grant(&box_id, &hub_name);
            for module in HUB_MODULES.into_iter().chain([HUB_ROOMS_MODULE_LEGACY]) {
                let grant = hub_module_grant(&box_id, module).unwrap();
                for (operation, module_allows, leaf_allows) in [
                    (Operation::Publish, &grant.publish, &leaf.publish),
                    (Operation::Subscribe, &grant.subscribe, &leaf.subscribe),
                ] {
                    for subject in module_allows {
                        assert!(
                            leaf_allows
                                .iter()
                                .any(|allow| subject_filter_is_subset(subject, allow).unwrap()),
                            "leaf for {box_id} does not allow {module} to {} {subject}",
                            operation.as_str(),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn federation_hub_module_grants_overlap_only_for_shared_manifest_reads() {
        // The two manifest readers share read APIs and flow control, not KV writes or inboxes.
        let shared = [
            "$JS.hub.API.STREAM.INFO.KV_vault-manifest-active",
            "$JS.hub.API.DIRECT.GET.KV_vault-manifest-active",
            "$JS.hub.API.DIRECT.GET.KV_vault-manifest-active.>",
            "$JS.hub.API.CONSUMER.CREATE.KV_vault-manifest-active",
            "$JS.hub.API.CONSUMER.CREATE.KV_vault-manifest-active.>",
            "$JS.hub.API.CONSUMER.INFO.KV_vault-manifest-active.>",
            "$JS.hub.API.CONSUMER.DELETE.KV_vault-manifest-active.>",
            "$JS.FC.KV_vault-manifest-active.*.*",
        ];
        let box_id = BoxId::parse(SAMPLE_BOX).unwrap();
        let grants = HUB_MODULES.map(|module| (module, hub_module_grant(&box_id, module).unwrap()));
        for (i, (module, grant)) in grants.iter().enumerate() {
            for subject in grant.publish.iter().chain(&grant.subscribe) {
                for (other, foreign) in &grants[i + 1..] {
                    for foreign_subject in foreign.publish.iter().chain(&foreign.subscribe) {
                        if [HUB_VAULT_MODULE, HUB_MANIFEST_READER_MODULE].contains(module)
                            && [HUB_VAULT_MODULE, HUB_MANIFEST_READER_MODULE].contains(other)
                            && subject == foreign_subject
                            && shared.contains(&subject.as_str())
                        {
                            continue;
                        }
                        assert!(
                            !subject_filters_overlap(subject, foreign_subject).unwrap(),
                            "{module}: {subject} overlaps {other}: {foreign_subject}"
                        );
                    }
                }
                for deny in &grant.denies {
                    assert!(
                        !subject_filters_overlap(subject, &deny.subject).unwrap(),
                        "{module}: {subject} overlaps denied subject {}",
                        deny.subject
                    );
                }
            }
        }
    }

    #[test]
    fn federation_vault_grant_can_put_or_delete_only_its_own_box_keys() {
        for (home, foreign) in [(SAMPLE_BOX, OTHER_BOX), (OTHER_BOX, SAMPLE_BOX)] {
            let box_id = BoxId::parse(home).unwrap();
            let grant = hub_module_grant(&box_id, HUB_VAULT_MODULE).unwrap();
            let may_publish = |subject: &str| {
                grant
                    .publish
                    .iter()
                    .any(|allow| subject_filters_overlap(allow, subject).unwrap())
            };
            // A put and a delete marker publish to the same key subject.
            let bucket = "$JS.hub.API.$KV.vault-shared-logins";
            let own_key = format!("{bucket}.{home}.{foreign}");
            assert!(may_publish(&own_key), "missing write to {own_key}");
            for subject in [
                format!("{bucket}.{foreign}.{home}"),
                format!("{bucket}.{home}"),
                "$KV.vault-shared-logins.>".to_owned(),
            ] {
                assert!(!may_publish(&subject), "unexpected write to {subject}");
            }
            assert!(may_publish(&format!(
                "$JS.hub.API.$KV.vault-manifest-active.{home}"
            )));
            for subject in [
                format!("$JS.hub.API.$KV.vault-manifest-active.{foreign}"),
                format!("$JS.hub.API.$KV.vault-manifest-active.{home}.extra"),
            ] {
                assert!(!may_publish(&subject), "unexpected write to {subject}");
            }
            assert!(!may_publish(
                "$JS.hub.API.STREAM.PURGE.KV_vault-shared-logins"
            ));
        }
    }

    #[test]
    fn federation_only_vault_modules_can_read_shared_logins() {
        let box_id = BoxId::parse(SAMPLE_BOX).unwrap();
        let key = format!("$KV.vault-shared-logins.{SAMPLE_BOX}.{OTHER_BOX}");
        let read_subjects = [
            "$JS.hub.API.STREAM.MSG.GET.KV_vault-shared-logins".to_owned(),
            "$JS.hub.API.DIRECT.GET.KV_vault-shared-logins".to_owned(),
            format!("$JS.hub.API.DIRECT.GET.KV_vault-shared-logins.{key}"),
            "$JS.hub.API.CONSUMER.CREATE.KV_vault-shared-logins".to_owned(),
            format!("$JS.hub.API.CONSUMER.CREATE.KV_vault-shared-logins.watch.{key}"),
        ];
        for module in HUB_MODULES.into_iter().chain([HUB_ROOMS_MODULE_LEGACY]) {
            let grant = hub_module_grant(&box_id, module).unwrap();
            for subject in &read_subjects {
                let allowed = grant
                    .publish
                    .iter()
                    .any(|allow| subject_filters_overlap(allow, subject).unwrap());
                assert_eq!(allowed, module == HUB_VAULT_MODULE, "{module}: {subject}");
            }
            if module != HUB_VAULT_MODULE {
                for allow in &grant.subscribe {
                    assert!(
                        !subject_filters_overlap(allow, "$KV.vault-shared-logins.>").unwrap(),
                        "{module}: unexpected shared-login subscription {allow}"
                    );
                }
            }
        }
    }

    #[test]
    fn federation_manifest_reader_grant_has_no_kv_writes() {
        let box_id = BoxId::parse(SAMPLE_BOX).unwrap();
        let grant = hub_module_grant(&box_id, HUB_MANIFEST_READER_MODULE).unwrap();
        for allow in grant.publish {
            for kv_writes in ["$KV.>", "$JS.hub.API.$KV.>"] {
                assert!(
                    !subject_filters_overlap(&allow, kv_writes).unwrap(),
                    "unexpected KV write permission: {allow}"
                );
            }
        }
    }

    #[test]
    fn box_id_accepts_only_canonical_lowercase_hex() {
        for value in [
            SAMPLE_BOX,
            "00000000000000000000000000000000",
            "ffffffffffffffffffffffffffffffff",
        ] {
            let parsed: BoxId = value.parse().unwrap();
            assert_eq!(parsed.as_str(), value);
            assert_eq!(parsed.to_string(), value);
        }

        for value in [
            "",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789Abcdef0123456789abcdef",
            "0x0123456789abcdef0123456789abcdef",
            "box:0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdeg",
            "0123456789abcdef0123456789abcde.",
            "0123456789abcdef0123456789abcde*",
            "0123456789abcdef0123456789abcde>",
            "0123456789abcdef0123456789abcde_",
            " 0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef\n",
            "é0123456789abcdef0123456789abcd",
            "éééééééééééééééé",
        ] {
            assert_eq!(BoxId::parse(value), Err(InvalidBoxId), "{value:?}");
        }
    }
}
