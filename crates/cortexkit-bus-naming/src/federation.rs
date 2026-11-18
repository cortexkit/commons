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
pub const FEDOUT_STREAM: &str = "FEDOUT";
pub const FEDOUT_SUBJECT: &str = "fedout.>";
pub const IDLOG_SUBJECT: &str = "ck.fleet.idlog";
pub const RESIDENCE_BUCKET: &str = "residence";
pub const RESIDENCE_STREAM: &str = "KV_residence";
pub const CUTS_BUCKET: &str = "cuts";
pub const CUTS_STREAM: &str = "OBJ_cuts";

const HUB_MEMORY_MODULE: &str = "magic-context";
const HUB_ROOMS_MODULE: &str = "prefrontal";
const HUB_ROOMS_MODULE_LEGACY: &str = "prefrontal-core";
const HUB_IDLOG_MODULE: &str = "entorhinal";
const HUB_CUTS_MODULE: &str = "engram";

/// Canonical module IDs with direct hub access. Use [`is_hub_module`] to also check the legacy ID.
pub const HUB_MODULES: [&str; 4] = [
    HUB_MEMORY_MODULE,
    HUB_ROOMS_MODULE,
    HUB_IDLOG_MODULE,
    HUB_CUTS_MODULE,
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
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
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

pub fn hub_inbox_stream_name(box_id: &BoxId) -> String {
    format!("HUB_INBOX_{box_id}")
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

pub fn hub_inbox_stream(box_id: &BoxId) -> HubStreamSpec {
    let subjects = FEDERATED_FAMILIES
        .iter()
        .map(|family| format!("ck.{box_id}.{family}.>"))
        .collect();
    let mut stream = StreamSpec::new(
        StreamKind::HubInbox,
        hub_inbox_stream_name(box_id),
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
    let mut stream = StreamSpec::new(
        StreamKind::KvResidence,
        RESIDENCE_STREAM.to_owned(),
        vec![format!("$KV.{RESIDENCE_BUCKET}.>")],
    );
    stream.max_msgs_per_subject = Some(1);
    stream.max_message_size = Some(16 * 1024);
    stream.max_bytes = 64 * MIB;
    stream.discard = DiscardPolicy::New;
    let mut backing_stream = HubStreamSpec::new(stream);
    // Provisioning must agree with async-nats 0.50.0's KV creation settings.
    backing_stream.allow_rollup_hdrs = true;
    backing_stream.allow_direct = true;
    backing_stream.deny_delete = true;
    backing_stream.replicas = Some(match hub {
        HubKind::SelfHosted => 1,
        HubKind::Hosted => 3,
    });
    KvBucketSpec::new(RESIDENCE_BUCKET.to_owned(), 1, backing_stream)
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
pub fn hub_streams(box_id: &BoxId, hub: HubKind) -> Vec<HubStreamSpec> {
    vec![
        hub_inbox_stream(box_id),
        hub_idlog_stream(),
        hub_rooms_stream(),
        hub_memory_stream(),
        residence_bucket(hub).backing_stream,
        cuts_object_store(),
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

pub fn leaf_user_grant(box_id: &BoxId) -> LeafUserGrant {
    let mut publish: Vec<String> = FEDERATED_FAMILIES
        .iter()
        .map(|family| format!("ck.*.{family}.>"))
        .collect();
    publish.extend(["$JS.FC.>", "$JS.ACK.>", "_INBOX.>"].map(str::to_owned));
    let inbox = hub_inbox_stream_name(box_id);
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
    ] {
        for suffix in [
            format!("STREAM.INFO.{stream}"),
            format!("STREAM.MSG.GET.{stream}"),
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
    use crate::limits::subject_filters_overlap;

    use super::*;

    const SAMPLE_BOX: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn federation_hub_module_grants_do_not_overlap() {
        let box_id = BoxId::parse(SAMPLE_BOX).unwrap();
        let grants = HUB_MODULES.map(|module| (module, hub_module_grant(&box_id, module).unwrap()));
        for (i, (module, grant)) in grants.iter().enumerate() {
            for subject in grant.publish.iter().chain(&grant.subscribe) {
                for (other, foreign) in &grants[i + 1..] {
                    for foreign_subject in foreign.publish.iter().chain(&foreign.subscribe) {
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
