use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "digest")]
use crate::{command::jcs_sha256_hex, commands_digest};
use crate::{
    facts::check_facts, require, text, validate_code, validate_commands, validate_version, Command,
    Fact, FactValue, ValidationError, UNITS,
};

/// Strict describe decoding. Relays can use `DescribeReply::decode_tolerant`
/// with the `digest` feature to retain readable commands in a verified reply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DescribeReply {
    pub commands: Vec<Command>,
    pub digest: String,
}
#[cfg(feature = "digest")]
impl DescribeReply {
    pub fn new(commands: Vec<Command>) -> Result<Self, ValidationError> {
        validate_commands(&commands)?;
        let digest = commands_digest(&commands)?;
        Ok(Self { commands, digest })
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        validate_commands(&self.commands)?;
        require(self.digest == commands_digest(&self.commands)?, "digest")
    }

    /// Verify the full raw array's digest before decoding its entries independently.
    /// This does not validate producer constraints on the readable commands.
    pub fn decode_tolerant(bytes: &[u8]) -> Result<TolerantDescribeReply, DescribeDecodeError> {
        let raw: RawDescribeReply =
            serde_json::from_slice(bytes).map_err(DescribeDecodeError::Json)?;
        if raw.digest != jcs_sha256_hex(&raw.commands).map_err(DescribeDecodeError::Json)? {
            return Err(DescribeDecodeError::DigestMismatch);
        }
        let mut result = TolerantDescribeReply {
            commands: Vec::new(),
            unreadable: Vec::new(),
            digest: raw.digest,
        };
        for (index, value) in raw.commands.into_iter().enumerate() {
            let id = value.get("id").and_then(Value::as_str).map(str::to_owned);
            match serde_json::from_value(value) {
                Ok(command) => result.commands.push(command),
                Err(error) => result.unreadable.push(UnreadableCommand {
                    index,
                    module: None,
                    id,
                    error: error.to_string(),
                }),
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ListedCommand {
    pub module: String,
    pub command: Command,
}
impl ListedCommand {
    pub fn new(module: impl Into<String>, command: Command) -> Self {
        Self {
            module: module.into(),
            command,
        }
    }
}

/// Strict list decoding. Apps should use [`read_list_reply`] so an unreadable
/// entry, such as a future subject kind, cannot hide the rest of the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ListReply {
    pub commands: Vec<ListedCommand>,
}
impl ListReply {
    pub fn new(commands: Vec<ListedCommand>) -> Self {
        Self { commands }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        let mut modules = std::collections::BTreeMap::<&str, Vec<Command>>::new();
        for entry in &self.commands {
            require(!entry.module.is_empty(), "commands.module")?;
            modules
                .entry(&entry.module)
                .or_default()
                .push(entry.command.clone());
        }
        for (module, commands) in modules {
            validate_commands(&commands).map_err(|e| e.at(module))?;
        }
        Ok(())
    }
}

/// An entry that could not be decoded, identified by its zero-based array index.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnreadableCommand {
    pub index: usize,
    pub module: Option<String>,
    pub id: Option<String>,
    pub error: String,
}

/// Decodable list entries and diagnostics for entries omitted during decoding.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TolerantListReply {
    pub commands: Vec<ListedCommand>,
    pub unreadable: Vec<UnreadableCommand>,
}

/// Decodable describe entries and diagnostics for entries omitted during decoding.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TolerantDescribeReply {
    pub commands: Vec<Command>,
    pub unreadable: Vec<UnreadableCommand>,
    pub digest: String,
}

/// A malformed description or a digest that does not match its original array.
#[cfg(feature = "digest")]
#[derive(Debug)]
#[non_exhaustive]
pub enum DescribeDecodeError {
    Json(serde_json::Error),
    DigestMismatch,
}

#[cfg(feature = "digest")]
impl std::fmt::Display for DescribeDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "invalid describe reply: {error}"),
            Self::DigestMismatch => write!(f, "describe reply digest mismatch"),
        }
    }
}

#[cfg(feature = "digest")]
impl std::error::Error for DescribeDecodeError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawListReply {
    commands: Vec<Value>,
}

#[cfg(feature = "digest")]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDescribeReply {
    commands: Vec<Value>,
    digest: String,
}

/// Decode each list entry independently; malformed reply envelopes still fail.
/// This decodes wire shapes only and does not validate producer constraints.
pub fn read_list_reply(value: Value) -> Result<TolerantListReply, serde_json::Error> {
    let raw: RawListReply = serde_json::from_value(value)?;
    let mut result = TolerantListReply {
        commands: Vec::new(),
        unreadable: Vec::new(),
    };
    for (index, value) in raw.commands.into_iter().enumerate() {
        let module = value
            .get("module")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let id = value
            .get("command")
            .and_then(|command| command.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        match serde_json::from_value(value) {
            Ok(command) => result.commands.push(command),
            Err(error) => result.unreadable.push(UnreadableCommand {
                index,
                module,
                id,
                error: error.to_string(),
            }),
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Figure {
    pub label: String,
    pub value: f64,
    pub unit: String,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub was: Option<f64>,
}
impl Figure {
    pub fn new(label: impl Into<String>, value: f64, unit: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value,
            unit: unit.into(),
            was: None,
        }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        require(text(&self.label, 64), "label")?;
        require(self.value.is_finite(), "value")?;
        require(UNITS.contains(&self.unit.as_str()), "unit")?;
        require(self.was.is_none_or(f64::is_finite), "was")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TableRow {
    pub label: String,
    pub cells: Vec<FactValue>,
}
impl TableRow {
    pub fn new(label: impl Into<String>, cells: Vec<FactValue>) -> Self {
        Self {
            label: label.into(),
            cells,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<TableRow>,
}
impl Table {
    pub fn new(columns: Vec<String>, rows: Vec<TableRow>) -> Self {
        Self { columns, rows }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        require((1..=6).contains(&self.columns.len()), "columns")?;
        for (index, column) in self.columns.iter().enumerate() {
            require(text(column, 40), &format!("columns.{index}"))?;
        }
        require(self.rows.len() <= 32, "rows")?;
        for (index, row) in self.rows.iter().enumerate() {
            require(text(&row.label, 64), &format!("rows.{index}.label"))?;
            require(
                row.cells.len() == self.columns.len(),
                &format!("rows.{index}.cells"),
            )?;
            for (cell_index, cell) in row.cells.iter().enumerate() {
                cell.check()
                    .map_err(|e| e.at(&format!("rows.{index}.cells.{cell_index}")))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Preview {
    pub version: String,
    pub available: bool,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub reason: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub effect: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub facts: Option<Vec<Fact>>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub figure: Option<Figure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<Table>,
}
impl Preview {
    pub fn new(version: impl Into<String>, available: bool) -> Self {
        Self {
            version: version.into(),
            available,
            reason: None,
            effect: None,
            facts: None,
            figure: None,
            table: None,
        }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        require(validate_version(&self.version), "version")?;
        require(self.available || self.reason.is_some(), "reason")?;
        if let Some(reason) = &self.reason {
            require(validate_code(reason), "reason")?;
        }
        if let Some(effect) = &self.effect {
            require(
                text(effect, 120)
                    && !effect
                        .chars()
                        .any(|c| matches!(c, '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}')),
                "effect",
            )?;
        }
        check_facts(&self.facts)?;
        if let Some(figure) = &self.figure {
            figure.check().map_err(|e| e.at("figure"))?;
        }
        if let Some(table) = &self.table {
            table.check().map_err(|e| e.at("table"))?;
        }
        Ok(())
    }
}

pub mod outcomes {
    pub const APPLIED: &str = "applied";
    pub const SCHEDULED: &str = "scheduled";
    pub const PREVIEW_STALE: &str = "preview_stale";
    pub const DEVICE_PROOF_REQUIRED: &str = "device_proof_required";
    pub const APPROVAL_DECLINED: &str = "approval_declined";
    pub const APPROVAL_EXPIRED: &str = "approval_expired";
    pub const REQUEST_CONFLICT: &str = "request_conflict";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OpenKind {
    Ask,
    Artifact,
    WorkItem,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct OpenItem {
    pub kind: OpenKind,
    pub id: String,
}
impl OpenItem {
    pub fn new(kind: OpenKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RunReply {
    pub outcome: String,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub facts: Option<Vec<Fact>>,
    #[serde(
        default,
        deserialize_with = "crate::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub open: Option<OpenItem>,
}
impl RunReply {
    pub fn new(outcome: impl Into<String>) -> Self {
        Self {
            outcome: outcome.into(),
            facts: None,
            open: None,
        }
    }
    pub fn check(&self) -> Result<(), ValidationError> {
        require(validate_code(&self.outcome), "outcome")?;
        check_facts(&self.facts)?;
        if let Some(open) = &self.open {
            require(!open.id.is_empty(), "open.id")?;
        }
        Ok(())
    }
}
