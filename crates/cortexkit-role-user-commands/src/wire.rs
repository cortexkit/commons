use serde::{Deserialize, Serialize};

#[cfg(feature = "digest")]
use crate::commands_digest;
use crate::{
    facts::check_facts, require, text, validate_code, validate_commands, validate_version, Command,
    Fact, FactValue, ValidationError, UNITS,
};

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
