//! Durable notice admission. Scope and principal are different: the principal
//! is authenticated by the carrier, never supplied in the request.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::errors::{Code, Refusal};

pub const MAX_NOTICE_BYTES: usize = 16 * 1024;
pub const MAX_LINKS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub scope_ref: String,
    pub epoch: u64,
}

impl Scope {
    pub fn new(scope_ref: impl Into<String>, epoch: u64) -> Self {
        Self {
            scope_ref: scope_ref.into(),
            epoch,
        }
    }
}

/// Strict on input. Attention strength is silent < ride < wake.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    Silent,
    Ride,
    Wake,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextContent {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub slots: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateContent {
    pub template: Template,
}

/// Exactly one of text and template; the inner closed objects reject both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(TextContent),
    Template(TemplateContent),
}

impl Content {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextContent { text: text.into() })
    }
    pub fn template(id: impl Into<String>, slots: Map<String, Value>) -> Self {
        Self::Template(TemplateContent {
            template: Template {
                id: id.into(),
                slots,
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub label: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Notice {
    pub idempotency_key: String,
    pub content: Content,
    pub attention: Attention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliver_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<Link>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Map<String, Value>>,
}

impl Notice {
    pub fn new(key: impl Into<String>, content: Content, attention: Attention) -> Self {
        Self {
            idempotency_key: key.into(),
            content,
            attention,
            target_agent: None,
            replace_key: None,
            deliver_after_ms: None,
            expires_at_ms: None,
            links: None,
            data: None,
        }
    }
    pub fn with_target_agent(mut self, agent: impl Into<String>) -> Self {
        self.target_agent = Some(agent.into());
        self
    }
    pub fn with_replace_key(mut self, key: impl Into<String>) -> Self {
        self.replace_key = Some(key.into());
        self
    }
    pub fn with_deliver_after_ms(mut self, time: u64) -> Self {
        self.deliver_after_ms = Some(time);
        self
    }
    pub fn with_expires_at_ms(mut self, time: u64) -> Self {
        self.expires_at_ms = Some(time);
        self
    }
    pub fn with_links(mut self, links: Vec<Link>) -> Self {
        self.links = Some(links);
        self
    }
    pub fn with_data(mut self, data: Map<String, Value>) -> Self {
        self.data = Some(data);
        self
    }
}

/// Size of the complete compact serde JSON notice in UTF-8, not just its text.
/// Apply to fresh posts only, after authenticated receipt lookup.
pub fn check_notice_len(notice: &Notice) -> Result<(), Refusal> {
    let bytes = serde_json::to_vec(notice)
        .map_err(|error| Refusal::new(Code::InvalidParams, error.to_string()))?
        .len();
    if bytes > MAX_NOTICE_BYTES {
        return Err(Refusal::new(
            Code::ContentTooLarge,
            "serialized notice exceeds 16 KiB",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PostRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    /// Only the reserved carrier's owner-notice path; not a delegated-source claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    pub notice: Notice,
}

impl PostRequest {
    pub fn new(scope: Scope, notice: Notice) -> Self {
        Self {
            scope: Some(scope),
            flow_id: None,
            notice,
        }
    }
    pub fn owner_notice(flow_id: impl Into<String>, notice: Notice) -> Self {
        Self {
            scope: None,
            flow_id: Some(flow_id.into()),
            notice,
        }
    }
    /// Structural checks only. Scope kind, served history, flow grants and
    /// template permissions require authenticated implementer metadata.
    pub fn validate(&self) -> Result<(), Refusal> {
        if self.scope.is_some() == self.flow_id.is_some() {
            return Err(Refusal::new(
                Code::InvalidParams,
                "exactly one of scope or flow_id is required",
            ));
        }
        if self.flow_id.is_some() && self.notice.target_agent.is_some() {
            return Err(Refusal::new(
                Code::InvalidParams,
                "owner notice cannot choose target_agent",
            ));
        }
        if self
            .notice
            .links
            .as_ref()
            .is_some_and(|links| links.len() > MAX_LINKS)
        {
            return Err(Refusal::new(Code::InvalidParams, "at most 8 links"));
        }
        check_notice_len(&self.notice)
    }
}

open_string!(PostState { Accepted => "accepted" });

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PostReply {
    pub state: PostState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed: Option<bool>,
}

impl PostReply {
    pub fn accepted(item_id: impl Into<String>) -> Self {
        Self {
            state: PostState::Accepted,
            item_id: Some(item_id.into()),
            replayed: None,
        }
    }
    pub fn new(state: PostState) -> Self {
        Self {
            state,
            item_id: None,
            replayed: None,
        }
    }
    pub fn with_item_id(mut self, id: impl Into<String>) -> Self {
        self.item_id = Some(id.into());
        self
    }
    pub fn with_replayed(mut self, replayed: bool) -> Self {
        self.replayed = Some(replayed);
        self
    }
}
