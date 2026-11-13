//! Immutable rendering inputs carried by a plan generation.

use serde::{de::Error, Deserialize, Deserializer, Serialize};

/// Published schema, byte-for-byte identical to the packaged JSON file.
pub const SYSTEM_TEXT_FACTS_SCHEMA: &str = include_str!("../system-text-facts.schema.json");

/// Facts a system text may vary on. There is deliberately no variant, effort,
/// time, count or session state: a render changes only with a plan generation.
/// Lists use UTF-8 byte order, with unique, nonempty strings. Constructors
/// normalize lists; decoding refuses a noncanonical list rather than fixing it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SystemTextFacts {
    harness: SystemTextHarness,
    #[serde(deserialize_with = "decode_names")]
    idle_start: Vec<String>,
    model: SystemTextModel,
    #[serde(deserialize_with = "decode_names")]
    tools: Vec<String>,
}

/// The selected model's provider and id, not its variant or effort.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SystemTextModel {
    pub id: String,
    pub provider: String,
}

impl SystemTextModel {
    pub fn new(provider: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
        }
    }
}

/// A harness id and only the host-fact names explicitly declared for it by
/// the runner. Other runner capabilities never imply host facts.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SystemTextHarness {
    #[serde(deserialize_with = "decode_names")]
    facts: Vec<String>,
    pub id: String,
}

impl SystemTextHarness {
    pub fn new(id: impl Into<String>, facts: Vec<String>) -> Result<Self, &'static str> {
        Ok(Self {
            facts: normalize(facts)?,
            id: id.into(),
        })
    }

    pub fn facts(&self) -> &[String] {
        &self.facts
    }
}

impl SystemTextFacts {
    /// `tools` is every served tool's catalog name; `idle_start` is every
    /// sender module allowed to start a turn. Empty lists are valid.
    pub fn new(
        model: SystemTextModel,
        harness: SystemTextHarness,
        tools: Vec<String>,
        idle_start: Vec<String>,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            harness,
            idle_start: normalize(idle_start)?,
            model,
            tools: normalize(tools)?,
        })
    }

    pub fn model(&self) -> &SystemTextModel {
        &self.model
    }
    pub fn harness(&self) -> &SystemTextHarness {
        &self.harness
    }
    pub fn tools(&self) -> &[String] {
        &self.tools
    }
    pub fn idle_start(&self) -> &[String] {
        &self.idle_start
    }

    /// Sorted object keys and canonical lists, with no insignificant whitespace.
    pub fn canonical_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_jcs::to_vec(self)
    }
}

fn normalize(mut names: Vec<String>) -> Result<Vec<String>, &'static str> {
    if names.iter().any(String::is_empty) {
        return Err("fact names must not be empty");
    }
    names.sort();
    names.dedup();
    Ok(names)
}

fn decode_names<'de, D: Deserializer<'de>>(decoder: D) -> Result<Vec<String>, D::Error> {
    let names = Vec::<String>::deserialize(decoder)?;
    if names.iter().any(String::is_empty)
        || names
            .windows(2)
            .any(|pair| pair[0].as_bytes() >= pair[1].as_bytes())
    {
        return Err(D::Error::custom(
            "fact names must be nonempty, sorted by UTF-8 bytes and unique",
        ));
    }
    Ok(names)
}

/// A present optional facts member must be an object, never JSON null.
pub fn decode_optional<'de, D: Deserializer<'de>>(
    decoder: D,
) -> Result<Option<SystemTextFacts>, D::Error> {
    SystemTextFacts::deserialize(decoder).map(Some)
}
