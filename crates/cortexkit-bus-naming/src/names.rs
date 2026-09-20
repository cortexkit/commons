use std::error::Error;
use std::fmt;

use crate::token::{validate_account_token, validate_token, NamingError, TokenKind};

/// All five stream names owned by one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamNames {
    pub room: String,
    pub wake: String,
    pub peer: String,
    pub effect: String,
    pub effect_dead: String,
}

impl StreamNames {
    pub fn all(&self) -> [&str; 5] {
        [
            &self.room,
            &self.wake,
            &self.peer,
            &self.effect,
            &self.effect_dead,
        ]
    }
}

/// The census KV bucket and its JetStream backing stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketNames {
    pub census: String,
    pub census_stream: String,
}

impl BucketNames {
    pub fn all(&self) -> [&str; 2] {
        [&self.census, &self.census_stream]
    }
}

/// Account-derived names. Derivation never normalizes input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountNames {
    account: String,
    account_upper: String,
    streams: StreamNames,
    buckets: BucketNames,
}

impl AccountNames {
    pub fn derive(account: &str) -> Result<Self, NamingError> {
        validate_account_token(account)?;
        let account_upper = account.to_ascii_uppercase();
        Ok(Self {
            account: account.to_owned(),
            streams: StreamNames {
                room: format!("CK_{account_upper}_ROOM"),
                wake: format!("CK_{account_upper}_WAKE"),
                peer: format!("CK_{account_upper}_PEER"),
                effect: format!("CK_{account_upper}_EFFECT"),
                effect_dead: format!("CK_{account_upper}_EFFECT_DEAD"),
            },
            buckets: BucketNames {
                census: format!("CK_{account_upper}_CENSUS"),
                census_stream: format!("KV_CK_{account_upper}_CENSUS"),
            },
            account_upper,
        })
    }

    pub fn from_roster_host_id(roster_host_id: &str) -> Result<Self, NamingError> {
        validate_token(TokenKind::RosterHostId, roster_host_id)?;
        Self::derive(&format!("box_{roster_host_id}"))
    }

    pub fn account(&self) -> &str {
        &self.account
    }

    pub fn account_upper(&self) -> &str {
        &self.account_upper
    }

    pub fn streams(&self) -> &StreamNames {
        &self.streams
    }

    pub fn buckets(&self) -> &BucketNames {
        &self.buckets
    }

    pub fn room_post(&self, room_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::RoomId, room_id)?;
        Ok(format!("ck.{}.room.{room_id}.post", self.account))
    }

    pub fn room_subscription(&self, room_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::RoomId, room_id)?;
        Ok(format!("ck.{}.room.{room_id}.>", self.account))
    }

    pub fn room_binding(&self) -> String {
        format!("ck.{}.room.*.post", self.account)
    }

    pub fn wake_fire(&self, agent_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::AgentId, agent_id)?;
        Ok(format!("ck.{}.wake.{agent_id}.fire", self.account))
    }

    pub fn wake_binding(&self) -> String {
        format!("ck.{}.wake.*.fire", self.account)
    }

    pub fn peer_delivery(&self, agent_id: &str, session_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::AgentId, agent_id)?;
        validate_token(TokenKind::SessionId, session_id)?;
        Ok(format!(
            "ck.{}.peer.{agent_id}.{session_id}.deliver",
            self.account
        ))
    }

    pub fn peer_filter(&self, agent_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::AgentId, agent_id)?;
        Ok(format!("ck.{}.peer.{agent_id}.*.deliver", self.account))
    }

    pub fn peer_subscription(&self, agent_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::AgentId, agent_id)?;
        Ok(format!("ck.{}.peer.{agent_id}.>", self.account))
    }

    pub fn peer_binding(&self) -> String {
        format!("ck.{}.peer.*.*.deliver", self.account)
    }

    pub fn effect_intent(&self, agent_id: &str, session_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::AgentId, agent_id)?;
        validate_token(TokenKind::SessionId, session_id)?;
        Ok(format!(
            "ck.{}.effect.{agent_id}.{session_id}.intent",
            self.account
        ))
    }

    pub fn effect_filter(&self, agent_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::AgentId, agent_id)?;
        Ok(format!("ck.{}.effect.{agent_id}.*.intent", self.account))
    }

    pub fn effect_publish_grant(&self, agent_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::AgentId, agent_id)?;
        Ok(format!("ck.{}.effect.{agent_id}.>", self.account))
    }

    pub fn effect_binding(&self) -> String {
        format!("ck.{}.effect.*.*.intent", self.account)
    }

    pub fn effect_dead(&self) -> String {
        format!("ck.{}.effect.dead", self.account)
    }

    pub fn sentinel_ping(&self) -> String {
        format!("ck.{}.sentinel.ping", self.account)
    }

    pub fn consumer_name(agent_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::AgentId, agent_id)?;
        Ok(format!("c_{agent_id}"))
    }

    pub fn process_record_name(
        module_id: &str,
        generation: u64,
        epoch: u64,
    ) -> Result<String, NamingError> {
        validate_token(TokenKind::ModuleId, module_id)?;
        Ok(format!("nats.{module_id}.g{generation}.e{epoch}"))
    }

    pub fn leaf_record_name(roster_host_id: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::RosterHostId, roster_host_id)?;
        Ok(format!("nats.leaf.{roster_host_id}"))
    }

    pub fn operator_record_name(&self) -> String {
        format!("nats.operator.{}", self.account)
    }

    pub fn account_record_name(&self) -> String {
        format!("nats.account.{}", self.account)
    }

    pub fn system_account_record_name(&self) -> String {
        format!("nats.sysaccount.{}", self.account)
    }
}

/// Namespaces that are deliberately outside the account-token rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingExemption {
    InboxPrefix,
    JetStreamPrefix,
    KeyValuePrefix,
    SystemPrefix,
    DurableConsumer,
}

/// The exemption list is closed and deliberately exposed for exhaustive tests.
pub const CLOSED_NAMING_EXEMPTIONS: [NamingExemption; 5] = [
    NamingExemption::InboxPrefix,
    NamingExemption::JetStreamPrefix,
    NamingExemption::KeyValuePrefix,
    NamingExemption::SystemPrefix,
    NamingExemption::DurableConsumer,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingRule {
    CkSubject,
    StreamName,
    BucketName,
    Exempt(NamingExemption),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenancyNameError {
    name: String,
    rule: NamingRule,
}

impl TenancyNameError {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn rule(&self) -> NamingRule {
        self.rule
    }
}

impl fmt::Display for TenancyNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "name {:?} does not satisfy tenancy naming rule {:?}",
            self.name, self.rule
        )
    }
}

impl Error for TenancyNameError {}

/// Checks the account-token rule for one emitted name or a named exemption.
pub fn validate_tenancy_name(
    account: &AccountNames,
    name: &str,
    rule: NamingRule,
) -> Result<(), TenancyNameError> {
    let valid = match rule {
        NamingRule::CkSubject => name.starts_with(&format!("ck.{}.", account.account)),
        NamingRule::StreamName => name.starts_with(&format!("CK_{}_", account.account_upper)),
        NamingRule::BucketName => {
            name.starts_with(&format!("CK_{}_", account.account_upper))
                || name.starts_with(&format!("KV_CK_{}_", account.account_upper))
        }
        NamingRule::Exempt(NamingExemption::InboxPrefix) => name.starts_with("_INBOX."),
        NamingRule::Exempt(NamingExemption::JetStreamPrefix) => name.starts_with("$JS."),
        NamingRule::Exempt(NamingExemption::KeyValuePrefix) => name.starts_with("$KV."),
        NamingRule::Exempt(NamingExemption::SystemPrefix) => name.starts_with("$SYS."),
        NamingRule::Exempt(NamingExemption::DurableConsumer) => name.starts_with("c_"),
    };
    if valid {
        Ok(())
    } else {
        Err(TenancyNameError {
            name: name.to_owned(),
            rule,
        })
    }
}
