//! The typed render context of SPEC.md section 2. The engine adds no time,
//! counter or other ambient value, so the same inputs render the same bytes;
//! the module's own `module` value must be just as stable for that to hold.

use std::{collections::BTreeMap, sync::Arc};

use minijinja::value::{Enumerator, Object, ObjectRepr, Value};
use serde::{Deserialize, Serialize};

use crate::PersonaError;

/// The agent the persona is rendered for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentContext {
    /// The agent's name, if it has one.
    #[serde(default)]
    pub name: Option<String>,
    /// The agent's id, if it has one.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The project, if any.
    #[serde(default)]
    pub project: Option<String>,
    /// The workspace, if any.
    #[serde(default)]
    pub workspace: Option<String>,
    /// The role being rendered.
    pub role: String,
}

/// The model the text is for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelContext {
    /// The provider id.
    pub provider: String,
    /// The model id.
    pub id: String,
    /// The model family, if known.
    #[serde(default)]
    pub family: Option<String>,
    /// The model variant.
    pub variant: String,
}

/// The harness that will carry the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessContext {
    /// The harness id.
    pub id: String,
    /// Named capabilities. Unlike grant maps, a missing name is undefined.
    pub capabilities: BTreeMap<String, bool>,
}

/// A grant map `{id: bool}`. A template reading a name that is not in the
/// map gets `false`, never an undefined-value error.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GrantMap(pub BTreeMap<String, bool>);

/// The demand the role is rendered for, each 0-100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Demand {
    /// Reasoning demand.
    pub iq: u8,
    /// Social demand.
    pub eq: u8,
}

/// The operating system family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsName {
    /// macOS.
    Darwin,
    /// Linux.
    Linux,
    /// Windows.
    Windows,
    /// Anything else.
    Unknown,
}

/// The host environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvContext {
    /// The operating system family.
    pub os: OsName,
    /// An IANA time zone name, for example `Europe/Istanbul`.
    pub timezone: String,
    /// A BCP 47 language tag, for example `en-GB`.
    pub locale: String,
}

/// The whole render context. `M` is the one object the module types itself
/// and templates read as `module`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderContext<M> {
    /// See [`AgentContext`].
    pub agent: AgentContext,
    /// See [`ModelContext`].
    pub model: ModelContext,
    /// See [`HarnessContext`].
    pub harness: HarnessContext,
    /// Tool grants.
    pub tools: GrantMap,
    /// MCP server grants.
    pub mcp: GrantMap,
    /// Module grants.
    pub modules: GrantMap,
    /// See [`Demand`].
    pub demand: Demand,
    /// See [`EnvContext`].
    pub env: EnvContext,
    /// The module's own context.
    pub module: M,
}

impl<M: Serialize> RenderContext<M> {
    pub(crate) fn validate(&self) -> Result<(), PersonaError> {
        let invalid = |field: &str, reason: &str| {
            Err(PersonaError::InvalidContext {
                field: field.to_string(),
                reason: reason.to_string(),
            })
        };
        if self.agent.role.is_empty() {
            return invalid("agent.role", "must not be empty");
        }
        for (field, value) in [
            ("model.provider", &self.model.provider),
            ("model.id", &self.model.id),
            ("model.variant", &self.model.variant),
            ("harness.id", &self.harness.id),
        ] {
            if value.is_empty() {
                return invalid(field, "must not be empty");
            }
        }
        if self.demand.iq > 100 {
            return invalid("demand.iq", "must be 0-100");
        }
        if self.demand.eq > 100 {
            return invalid("demand.eq", "must be 0-100");
        }
        let tz_ok = !self.env.timezone.is_empty()
            && self
                .env
                .timezone
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'+'));
        if !tz_ok {
            return invalid("env.timezone", "must be an IANA time zone name");
        }
        let locale_ok = !self.env.locale.is_empty()
            && self.env.locale.split('-').all(|part| {
                !part.is_empty()
                    && part.len() <= 8
                    && part.bytes().all(|b| b.is_ascii_alphanumeric())
            });
        if !locale_ok {
            return invalid("env.locale", "must be a BCP 47 language tag");
        }
        Ok(())
    }

    /// The template root value: the nine keys, grant maps as objects that
    /// answer `false` for a missing name.
    pub(crate) fn to_value(&self) -> Value {
        let grant = |g: &GrantMap| Value::from_object(Grants(g.0.clone()));
        Value::from_iter([
            ("agent", Value::from_serialize(&self.agent)),
            ("model", Value::from_serialize(&self.model)),
            ("harness", Value::from_serialize(&self.harness)),
            ("tools", grant(&self.tools)),
            ("mcp", grant(&self.mcp)),
            ("modules", grant(&self.modules)),
            ("demand", Value::from_serialize(self.demand)),
            ("env", Value::from_serialize(&self.env)),
            ("module", Value::from_serialize(&self.module)),
        ])
    }
}

#[derive(Debug)]
struct Grants(BTreeMap<String, bool>);

impl Object for Grants {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Map
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        let name = key.as_str()?;
        Some(Value::from(self.0.get(name).copied().unwrap_or(false)))
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Values(self.0.keys().map(|k| Value::from(k.as_str())).collect())
    }
}
