use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{require, text, validate_code, SessionRef, ValidationError};

pub const MAX_COMMANDS: usize = 64;
pub const MAX_ARGS: usize = 8;
pub const EFFECTS: &[&str] = &[
    "read",
    "reversible",
    "ends",
    "narrow",
    "writes_files",
    "destroys",
    "widen",
    "context_change",
    "spend",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SubjectKind {
    Session,
    Agent,
    Computer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ArgKind {
    Text,
    Number,
    Choice,
    Toggle,
    Agent,
    Session,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

impl ChoiceOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Arg {
    pub key: String,
    pub kind: ArgKind,
    pub label: String,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub default: Option<Value>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub options: Option<Vec<ChoiceOption>>,
}

impl Arg {
    pub fn new(key: impl Into<String>, kind: ArgKind, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            kind,
            label: label.into(),
            default: None,
            options: None,
        }
    }

    pub fn check(&self) -> Result<(), ValidationError> {
        require(validate_code(&self.key), "key")?;
        require(text(&self.label, 40), "label")?;
        if self.kind == ArgKind::Choice {
            let options = self
                .options
                .as_ref()
                .ok_or_else(|| ValidationError::new("options"))?;
            require((1..=32).contains(&options.len()), "options")?;
            let mut values = BTreeSet::new();
            for (index, option) in options.iter().enumerate() {
                require(
                    text(&option.value, 64) && values.insert(&option.value),
                    &format!("options.{index}.value"),
                )?;
                require(text(&option.label, 40), &format!("options.{index}.label"))?;
            }
        } else {
            require(self.options.is_none(), "options")?;
        }
        if let Some(value) = &self.default {
            require(self.accepts(value), "default")?;
        }
        Ok(())
    }

    /// Check a value without coercion. Call `check` on the declaration first.
    pub fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            ArgKind::Text => value.as_str().is_some_and(|v| v.chars().count() <= 2000),
            ArgKind::Number => value.as_f64().is_some_and(f64::is_finite),
            ArgKind::Toggle => value.is_boolean(),
            ArgKind::Agent => value.as_str().is_some_and(|v| text(v, 128)),
            ArgKind::Session => {
                serde_json::from_value::<SessionRef>(value.clone()).is_ok_and(|v| v.check().is_ok())
            }
            ArgKind::Choice => value.as_str().is_some_and(|v| {
                self.options
                    .as_ref()
                    .is_some_and(|options| options.iter().any(|option| option.value == v))
            }),
        }
    }
}

/// Presence of `{}` distinguishes a runnable command from a read-only one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Run {}

impl Run {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Command {
    pub id: String,
    pub title: String,
    pub summary: String,
    /// A kind here; request subjects identify the actual thing.
    pub subject: SubjectKind,
    pub args: Vec<Arg>,
    /// Open on decode so a reader can withhold run for a future effect.
    pub effects: Vec<String>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub run: Option<Run>,
}

impl Command {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
        subject: SubjectKind,
        args: Vec<Arg>,
        effects: Vec<String>,
        run: Option<Run>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            summary: summary.into(),
            subject,
            args,
            effects,
            run,
        }
    }

    /// Producer validation: unknown effects are retained on decode but not sent.
    pub fn check(&self) -> Result<(), ValidationError> {
        require(validate_code(&self.id), "id")?;
        require(text(&self.title, 40), "title")?;
        require(text(&self.summary, 240), "summary")?;
        require(self.args.len() <= MAX_ARGS, "args")?;
        let mut keys = BTreeSet::new();
        for (index, arg) in self.args.iter().enumerate() {
            require(keys.insert(&arg.key), &format!("args.{index}.key"))?;
            arg.check().map_err(|e| e.at(&format!("args.{index}")))?;
        }
        let mut effects = BTreeSet::new();
        require(!self.effects.is_empty(), "effects")?;
        for effect in &self.effects {
            require(
                EFFECTS.contains(&effect.as_str()) && effects.insert(effect),
                "effects",
            )?;
        }
        let read = self.effects.iter().any(|e| e == "read");
        require(!read || self.effects.len() == 1, "effects")?;
        require(self.run.is_some() != read, "run")
    }

    /// Readers never offer run for an unknown consequence, even if `run` exists.
    pub fn offers_run(&self) -> bool {
        self.run.is_some()
            && !self.effects.is_empty()
            && self
                .effects
                .iter()
                .all(|e| e != "read" && EFFECTS.contains(&e.as_str()))
    }

    /// Missing arguments use declared defaults; an argument with no default is
    /// required. The returned map is suitable for the module's command handler.
    pub fn resolve_args(
        &self,
        args: &Map<String, Value>,
    ) -> Result<Map<String, Value>, ValidationError> {
        self.check()?;
        for key in args.keys() {
            require(
                self.args.iter().any(|arg| &arg.key == key),
                &format!("args.{key}"),
            )?;
        }
        let mut resolved = Map::new();
        for arg in &self.args {
            let value = args
                .get(&arg.key)
                .or(arg.default.as_ref())
                .ok_or_else(|| ValidationError::new(format!("args.{}", arg.key)))?;
            require(arg.accepts(value), &format!("args.{}", arg.key))?;
            resolved.insert(arg.key.clone(), value.clone());
        }
        Ok(resolved)
    }
}

pub fn validate_commands(commands: &[Command]) -> Result<(), ValidationError> {
    require(commands.len() <= MAX_COMMANDS, "commands")?;
    let mut ids = BTreeSet::new();
    for (index, command) in commands.iter().enumerate() {
        require(ids.insert(&command.id), &format!("commands.{index}.id"))?;
        command
            .check()
            .map_err(|e| e.at(&format!("commands.{index}")))?;
    }
    Ok(())
}

/// Lowercase SHA-256 of the RFC 8785 bytes of the array, not the reply envelope.
pub fn commands_digest(commands: &[Command]) -> Result<String, ValidationError> {
    let bytes = serde_jcs::to_vec(commands).map_err(|_| ValidationError::new("commands"))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
