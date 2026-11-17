//! Shared hub names and leaf permissions for the cross-computer message plane.

use std::fmt;
use std::str::FromStr;

use crate::{
    validate_token, DiscardPolicy, NamingError, StreamKind, StreamSpec, TokenKind, GIB, HOUR, MIB,
};

pub const HUB_DOMAIN: &str = "hub";
pub const HUB_API_PREFIX: &str = "$JS.hub.API";
pub const FEDERATED_FAMILIES: [&str; 4] = ["room", "wake", "peer", "sync"];
pub const HUB_IDLOG_STREAM: &str = "HUB_IDLOG";
pub const IDLOG_SUBJECT: &str = "ck.fleet.idlog";
pub const RESIDENCE_BUCKET: &str = "residence";
pub const RESIDENCE_STREAM: &str = "KV_residence";
pub const CUTS_BUCKET: &str = "cuts";
pub const CUTS_STREAM: &str = "OBJ_cuts";

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
    ]);
    for stream in [HUB_IDLOG_STREAM, RESIDENCE_STREAM, CUTS_STREAM] {
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
    // Releasing an object purges its chunks; the log and KV bucket get no purge.
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_BOX: &str = "0123456789abcdef0123456789abcdef";

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
