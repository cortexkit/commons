//! Coverage for contract ids following `- [pinned]`, with an explicit required subset.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_MAP: &str = include_str!("../rule-map.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RuleMap {
    pub version: u32,
    pub rules: Vec<Rule>,
}

impl RuleMap {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { version: 1, rules }
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub cases: Vec<String>,
    pub not_kit_checked: Option<Exemption>,
    pub bug_shaped: bool,
}

impl Rule {
    pub fn new(id: impl Into<String>, cases: Vec<String>) -> Self {
        Self {
            id: id.into(),
            cases,
            not_kit_checked: None,
            bug_shaped: false,
        }
    }
    pub fn with_not_kit_checked(mut self, reason: Exemption) -> Self {
        self.not_kit_checked = Some(reason);
        self.cases.clear();
        self
    }
    pub fn with_bug_shaped(mut self, value: bool) -> Self {
        self.bug_shaped = value;
        self
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Exemption {
    CallerSide,
    NotObservable,
    InformativeDependency,
}

pub fn core_family(id: &str) -> bool {
    [
        "media.identity.",
        "media.refusal.",
        "media.failure.",
        "media.describe.",
        "media.engine.",
        "media.input.",
        "media.objects.",
        "media.jobs.",
        "media.output.",
        "media.usage.",
        "media.feed.",
        "media.admission.",
    ]
    .iter()
    .any(|prefix| id.starts_with(prefix))
}

/// Validate all mapped ids and every pinned id selected by `required`.
/// Passing `|_| true` requires complete contract coverage.
pub fn validate_rule_map(
    contract: &str,
    map: &str,
    cases: &[&str],
    required: impl Fn(&str) -> bool,
) -> Result<RuleMap, String> {
    let map: RuleMap = serde_json::from_str(map).map_err(|e| e.to_string())?;
    if map.version != 1 {
        return Err("unknown rule-map version".into());
    }
    let mut pinned = BTreeSet::new();
    for line in contract
        .lines()
        .filter(|line| line.starts_with("- [pinned]"))
    {
        let id = line
            .strip_prefix("- [pinned] `")
            .and_then(|v| v.split_once("`:"))
            .map(|v| v.0)
            .ok_or("pinned rule missing marker-position id")?;
        if !id.starts_with("media.")
            || id.split('.').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
            })
        {
            return Err(format!("invalid pinned id {id}"));
        }
        if !pinned.insert(id.to_string()) {
            return Err(format!("duplicate pinned id {id}"));
        }
    }
    let exemptions = exemptions(contract);
    let mut mapped = BTreeSet::new();
    for rule in &map.rules {
        if !mapped.insert(rule.id.clone()) {
            return Err(format!("duplicate mapped id {}", rule.id));
        }
        if !pinned.contains(&rule.id) {
            return Err(format!("mapped id not pinned: {}", rule.id));
        }
        if rule.cases.is_empty() == rule.not_kit_checked.is_none() {
            return Err(format!("{} must have cases or not_kit_checked", rule.id));
        }
        if rule.not_kit_checked != exemptions.get(&rule.id).copied() {
            return Err(format!("{} exemption differs from CONTRACT", rule.id));
        }
        for case in &rule.cases {
            if !cases.contains(&case.as_str()) {
                return Err(format!("{} names unknown case {case}", rule.id));
            }
        }
    }
    for id in pinned.iter().filter(|id| required(id)) {
        if !mapped.contains(id) {
            return Err(format!("unmapped pinned id {id}"));
        }
    }
    for id in exemptions.keys() {
        if !pinned.contains(id) {
            return Err(format!("exempt id not pinned: {id}"));
        }
    }
    Ok(map)
}

fn exemptions(contract: &str) -> BTreeMap<String, Exemption> {
    let mut out = BTreeMap::new();
    if let Some((_, section)) =
        contract.split_once("## Rules not checked by the provider conformance kit")
    {
        let text = section.split_once(':').map_or(section, |(_, v)| v);
        let mut pending = Vec::new();
        for (i, token) in text.split('`').enumerate() {
            if i % 2 == 0 {
                continue;
            }
            if token.starts_with("media.") {
                pending.push(token.to_string());
            }
            let reason = match token {
                "caller_side" => Some(Exemption::CallerSide),
                "not_observable" => Some(Exemption::NotObservable),
                "informative_dependency" => Some(Exemption::InformativeDependency),
                _ => None,
            };
            if let Some(reason) = reason {
                for id in pending.drain(..) {
                    out.insert(id, reason);
                }
            }
        }
    }
    out
}
