//! The typed render context of SPEC.md section 2. The engine adds no time,
//! counter or other ambient value, so the same inputs render the same bytes;
//! the module's own `module` value must be just as stable for that to hold.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use minijinja::value::{Enumerator, Object, ObjectRepr, Value as MinijinjaValue};
use serde::{Deserialize, Serialize};

use crate::{PersonaError, Value};

/// The names the engine itself puts in the template root. A caller-declared
/// root may not reuse one, or it would make that built-in unreachable.
pub(crate) const BUILT_IN_ROOTS: [&str; 9] = [
    "agent", "model", "harness", "tools", "mcp", "modules", "demand", "env", "module",
];

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
///
/// The roots a caller declares with [`Self::with_root`] are skipped by
/// `serde`: they are a run input, not part of the serialised shape, and a
/// deserialised context starts with none of them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
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
    /// Extra roots the caller declares, by name, next to the built-in roots.
    /// Empty unless [`Self::with_root`] added one; a name in here that is not
    /// allowed is refused at render.
    ///
    /// Declare roots with [`Self::with_root`] rather than filling this in by
    /// hand: only that builder notices a second declaration of a name.
    #[serde(skip)]
    pub roots: BTreeMap<String, Value>,
    /// The names [`Self::with_root`] was given more than once. Only that
    /// builder fills it in; a context written out by hand has no second
    /// declaration to report.
    #[serde(skip)]
    pub redeclared: BTreeSet<String>,
}

/// A context with empty roots, no grants and a `module` made by `M::default`.
impl<M: Default> Default for RenderContext<M> {
    fn default() -> Self {
        Self {
            agent: AgentContext {
                name: None,
                agent_id: None,
                project: None,
                workspace: None,
                role: String::new(),
            },
            model: ModelContext {
                provider: String::new(),
                id: String::new(),
                family: None,
                variant: String::new(),
            },
            harness: HarnessContext {
                id: String::new(),
                capabilities: BTreeMap::new(),
            },
            tools: GrantMap::default(),
            mcp: GrantMap::default(),
            modules: GrantMap::default(),
            demand: Demand { iq: 0, eq: 0 },
            env: EnvContext {
                os: OsName::Unknown,
                timezone: String::new(),
                locale: String::new(),
            },
            module: M::default(),
            roots: BTreeMap::new(),
            redeclared: BTreeSet::new(),
        }
    }
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
        if let Some(name) = self.redeclared.iter().next() {
            return invalid(
                &format!("root.{name}"),
                "is already declared; declare each root name once",
            );
        }
        for name in self.roots.keys() {
            if let Some(reason) = root_name_problem(name) {
                return invalid(&format!("root.{name}"), reason);
            }
        }
        Ok(())
    }

    /// Declare one more root that every template rendered with this context
    /// can read by `name`, with the given value.
    ///
    /// The name must be an identifier (`[A-Za-z_][A-Za-z0-9_]*`) that is not a
    /// built-in root (`agent`, `model`, `harness`, `tools`, `mcp`, `modules`,
    /// `demand`, `env`, `module`) and not already declared here; anything else
    /// is refused by name at [`LoadedRole::render`](crate::LoadedRole::render),
    /// before a template runs. A root the caller never declares stays
    /// undefined, so reading it is still a strict undefined error.
    ///
    /// A declared root is a value, not a file: it is rendered exactly as given
    /// and never changes any layer's digest or approval.
    #[must_use]
    pub fn with_root(mut self, name: &str, value: impl Into<Value>) -> Self {
        let name = name.to_string();
        if self.roots.insert(name.clone(), value.into()).is_some() {
            self.redeclared.insert(name);
        }
        self
    }

    /// The declared roots, by name.
    pub fn roots(&self) -> &BTreeMap<String, Value> {
        &self.roots
    }

    /// The template root value: the built-in roots, grant maps as objects that
    /// answer `false` for a missing name, then the caller's declared roots.
    pub(crate) fn to_value(&self) -> minijinja::Value {
        let grant = |g: &GrantMap| minijinja::Value::from_object(Grants(g.0.clone()));
        let mut root: Vec<(String, minijinja::Value)> = vec![
            (
                "agent".into(),
                minijinja::Value::from_serialize(&self.agent),
            ),
            (
                "model".into(),
                minijinja::Value::from_serialize(&self.model),
            ),
            (
                "harness".into(),
                minijinja::Value::from_serialize(&self.harness),
            ),
            ("tools".into(), grant(&self.tools)),
            ("mcp".into(), grant(&self.mcp)),
            ("modules".into(), grant(&self.modules)),
            (
                "demand".into(),
                minijinja::Value::from_serialize(self.demand),
            ),
            ("env".into(), minijinja::Value::from_serialize(&self.env)),
            (
                "module".into(),
                minijinja::Value::from_serialize(&self.module),
            ),
        ];
        root.extend(
            self.roots
                .iter()
                .map(|(name, value)| (name.clone(), value.to_minijinja())),
        );
        minijinja::Value::from_iter(root)
    }
}

/// Why `name` cannot be a caller-declared root, if it cannot. A name that is
/// not a template identifier could not be written as `name` or `name.field`
/// in a template, and a built-in name would hide the engine's own root. This
/// is the one place that decides both, so no caller can reach a built-in name
/// by another route.
fn root_name_problem(name: &str) -> Option<&'static str> {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
        Some(_) => {
            return Some("must start with an ASCII letter or `_`");
        }
        None => return Some("must not be empty"),
    }
    if !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Some("may contain only ASCII letters, digits and `_`");
    }
    if BUILT_IN_ROOTS.contains(&name) {
        return Some("is a built-in root and cannot be declared again");
    }
    None
}

#[derive(Debug)]
struct Grants(BTreeMap<String, bool>);

impl Object for Grants {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Map
    }

    fn get_value(self: &Arc<Self>, key: &MinijinjaValue) -> Option<MinijinjaValue> {
        let name = key.as_str()?;
        Some(MinijinjaValue::from(
            self.0.get(name).copied().unwrap_or(false),
        ))
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Values(
            self.0
                .keys()
                .map(|k| MinijinjaValue::from(k.as_str()))
                .collect(),
        )
    }
}
