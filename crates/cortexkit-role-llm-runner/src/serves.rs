//! On-request provider and model lists (CONTRACT.md §2.1).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_HARNESSES: usize = 16;
const MAX_ENTRIES: usize = 1_024;
const MAX_MODELS: usize = 4_096;

/// A snapshot of the runner's current lists. A missing list is not ready;
/// an explicitly empty one serves nothing.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Serves {
    pub generation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harnesses: Option<BTreeMap<String, Vec<ServesEntry>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<ServesEntry>>,
}

impl Serves {
    pub fn new(generation: impl Into<String>) -> Self {
        Self {
            generation: generation.into(),
            harnesses: None,
            entries: None,
        }
    }

    #[must_use]
    pub fn with_harnesses(mut self, harnesses: BTreeMap<String, Vec<ServesEntry>>) -> Self {
        self.harnesses = Some(harnesses);
        self
    }

    #[must_use]
    pub fn with_entries(mut self, entries: Vec<ServesEntry>) -> Self {
        self.entries = Some(entries);
        self
    }

    /// `None` means not ready; `Some(&[])` is an explicit empty list. Pass no
    /// harness only for a runner that declares none.
    pub fn list_for(&self, harness: Option<&str>) -> Option<&[ServesEntry]> {
        match harness {
            Some(harness) => self.harnesses.as_ref()?.get(harness).map(Vec::as_slice),
            None => self.entries.as_deref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ServesEntry {
    pub provider: String,
    pub models: ServesModels,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub except: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<ServesLogin>,
}

impl ServesEntry {
    pub fn new(provider: impl Into<String>, models: ServesModels) -> Self {
        Self {
            provider: provider.into(),
            models,
            except: None,
            login: None,
        }
    }

    #[must_use]
    pub fn with_except(mut self, except: Vec<String>) -> Self {
        self.except = Some(except);
        self
    }

    #[must_use]
    pub fn with_login(mut self, login: ServesLogin) -> Self {
        self.login = Some(login);
        self
    }

    /// Test membership using the catalog model set already resolved for this
    /// provider. Neither a login nor the operator's settings are checked here.
    pub fn includes(&self, model: &str, catalog_models: &BTreeSet<String>) -> bool {
        match &self.models {
            ServesModels::All => {
                catalog_models.contains(model)
                    && !self.except.iter().flatten().any(|id| id == model)
            }
            ServesModels::List(ids) => ids.iter().any(|id| id == model),
        }
    }
}

/// Check placement rule 1 against a resolved, validated list. `None` means
/// the list does not place the route. `Some(true)` means it places the route
/// without a login; `Some(false)` means it places the route but requires a login.
pub fn placement(
    list: &[ServesEntry],
    provider: &str,
    model: &str,
    catalog_models: &BTreeSet<String>,
) -> Option<bool> {
    let entry = list.iter().find(|entry| entry.provider == provider)?;
    entry
        .includes(model, catalog_models)
        .then_some(entry.login == Some(ServesLogin::None))
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServesModels {
    All,
    #[serde(untagged)]
    List(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServesLogin {
    None,
}

/// The contract's refusal code and the path of the offending field.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServesError {
    pub code: &'static str,
    pub field: String,
}

impl std::fmt::Display for ServesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.code, self.field)
    }
}

impl std::error::Error for ServesError {}

impl ServesError {
    fn new(code: &'static str, field: impl Into<String>) -> Self {
        Self {
            code,
            field: field.into(),
        }
    }
}

/// Check the list shape against the routing declaration, including bounds and
/// uniqueness. Use [`decode_serves`] when starting with a wire value.
pub fn check_serves(
    serves: &Serves,
    declared_harnesses: Option<&[String]>,
) -> Result<(), ServesError> {
    check_envelope(
        serves.harnesses.is_some(),
        serves.entries.is_some(),
        declared_harnesses,
    )?;
    if !(1..=128).contains(&serves.generation.len())
        || !serves
            .generation
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(ServesError::new(
            "serves_invalid_generation",
            "serves.generation",
        ));
    }
    if let Some(harnesses) = &serves.harnesses {
        check_bound(harnesses.len(), MAX_HARNESSES, "serves.harnesses")?;
        for (name, entries) in harnesses {
            let field = format!("serves.harnesses.{name}");
            if !declared_harnesses.is_some_and(|names| names.contains(name)) {
                return Err(ServesError::new("serves_unknown_harness", field));
            }
            check_entries(entries, &field)?;
        }
    }
    if let Some(entries) = &serves.entries {
        check_entries(entries, "serves.entries")?;
    }
    Ok(())
}

fn check_envelope(
    harnesses: bool,
    entries: bool,
    declared: Option<&[String]>,
) -> Result<(), ServesError> {
    let has_harnesses = declared.is_some_and(|names| !names.is_empty());
    let error = match (harnesses, entries) {
        (true, true) => ("serves_both", "serves"),
        (true, false) if !has_harnesses => ("serves_wrong_envelope", "serves.harnesses"),
        (false, true) if has_harnesses => ("serves_wrong_envelope", "serves.entries"),
        _ => return Ok(()),
    };
    Err(ServesError::new(error.0, error.1))
}

fn check_bound(len: usize, max: usize, field: &str) -> Result<(), ServesError> {
    if len > max {
        return Err(ServesError::new("serves_too_large", field));
    }
    Ok(())
}

fn check_entries(entries: &[ServesEntry], field: &str) -> Result<(), ServesError> {
    check_bound(entries.len(), MAX_ENTRIES, field)?;
    let mut providers = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("{field}[{index}]");
        let mut bytes = entry.provider.bytes();
        if entry.provider.len() > 64
            || !bytes.next().is_some_and(|first| first.is_ascii_lowercase())
            || !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        {
            return Err(ServesError::new(
                "serves_invalid_provider",
                format!("{path}.provider"),
            ));
        }
        if !providers.insert(&entry.provider) {
            return Err(ServesError::new(
                "serves_duplicate_provider",
                format!("{path}.provider"),
            ));
        }
        if let ServesModels::List(ids) = &entry.models {
            check_ids(ids, &format!("{path}.models"))?;
        }
        if let Some(ids) = &entry.except {
            let field = format!("{path}.except");
            if entry.models != ServesModels::All {
                return Err(ServesError::new("serves_except_without_all", field));
            }
            check_ids(ids, &field)?;
        }
    }
    Ok(())
}

fn check_ids(ids: &[String], field: &str) -> Result<(), ServesError> {
    check_bound(ids.len(), MAX_MODELS, field)?;
    let mut unique = BTreeSet::new();
    if ids.is_empty()
        || ids.iter().any(|id| {
            !(1..=256).contains(&id.len())
                || id.chars().any(|ch| ch.is_whitespace() || ch.is_control())
                || !unique.insert(id)
        })
    {
        return Err(ServesError::new("serves_invalid_models", field));
    }
    Ok(())
}

/// Decode the `serves` member, preserving the field path on wire-type errors,
/// and check the resulting list against the routing declaration.
pub fn decode_serves(
    raw: &Value,
    declared_harnesses: Option<&[String]>,
) -> Result<Serves, ServesError> {
    let harnesses = raw.get("harnesses");
    let entries = raw.get("entries");
    check_envelope(harnesses.is_some(), entries.is_some(), declared_harnesses)?;
    let mut serves = Serves::new(
        raw.get("generation")
            .and_then(Value::as_str)
            .ok_or_else(|| ServesError::new("serves_invalid_generation", "serves.generation"))?,
    );
    if let Some(harnesses) = harnesses {
        let harnesses = harnesses
            .as_object()
            .ok_or_else(|| ServesError::new("serves_invalid_models", "serves.harnesses"))?;
        check_bound(harnesses.len(), MAX_HARNESSES, "serves.harnesses")?;
        let mut lists = BTreeMap::new();
        for (name, entries) in harnesses {
            let field = format!("serves.harnesses.{name}");
            if !declared_harnesses.is_some_and(|names| names.contains(name)) {
                return Err(ServesError::new("serves_unknown_harness", field));
            }
            lists.insert(name.clone(), decode_entries(entries, &field)?);
        }
        serves.harnesses = Some(lists);
    }
    if let Some(entries) = entries {
        serves.entries = Some(decode_entries(entries, "serves.entries")?);
    }
    check_serves(&serves, declared_harnesses)?;
    Ok(serves)
}

fn decode_entries(raw: &Value, field: &str) -> Result<Vec<ServesEntry>, ServesError> {
    let entries = raw
        .as_array()
        .ok_or_else(|| ServesError::new("serves_invalid_models", field))?;
    check_bound(entries.len(), MAX_ENTRIES, field)?;
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let path = format!("{field}[{index}]");
            let provider = entry
                .get("provider")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ServesError::new("serves_invalid_provider", format!("{path}.provider"))
                })?;
            let models = match entry.get("models") {
                Some(Value::String(all)) if all == "all" => ServesModels::All,
                Some(raw) => ServesModels::List(decode_ids(raw, &format!("{path}.models"))?),
                None => {
                    return Err(ServesError::new(
                        "serves_invalid_models",
                        format!("{path}.models"),
                    ))
                }
            };
            let except = entry
                .get("except")
                .map(|raw| {
                    let field = format!("{path}.except");
                    if models != ServesModels::All {
                        return Err(ServesError::new("serves_except_without_all", field));
                    }
                    decode_ids(raw, &field)
                })
                .transpose()?;
            let login = match entry.get("login") {
                None => None,
                Some(Value::String(none)) if none == "none" => Some(ServesLogin::None),
                Some(_) => {
                    return Err(ServesError::new(
                        "serves_invalid_login",
                        format!("{path}.login"),
                    ))
                }
            };
            Ok(ServesEntry {
                provider: provider.into(),
                models,
                except,
                login,
            })
        })
        .collect()
}

fn decode_ids(raw: &Value, field: &str) -> Result<Vec<String>, ServesError> {
    let ids = raw
        .as_array()
        .ok_or_else(|| ServesError::new("serves_invalid_models", field))?;
    check_bound(ids.len(), MAX_MODELS, field)?;
    Vec::<String>::deserialize(raw).map_err(|_| ServesError::new("serves_invalid_models", field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{describe::check_describe, ops};
    use serde_json::json;

    #[test]
    fn serves_vectors_round_trip_and_refuse_by_field() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../test-vectors/llm-runner-v1/role-describe-serves.json"
        ))
        .unwrap();
        for case in vectors["accepted"].as_array().unwrap() {
            let declared: Option<Vec<String>> =
                serde_json::from_value(case["declared_harnesses"].clone()).unwrap();
            let answer = check_describe(&case["answer"]).unwrap();
            if let Some(raw) = case["answer"].get("serves") {
                assert_eq!(case["params"], ops::role_describe_serves_params());
                assert_eq!(
                    answer.serves,
                    Some(decode_serves(raw, declared.as_deref()).unwrap())
                );
            } else {
                assert_eq!(answer.serves, None);
                assert_eq!(serde_json::to_string(&answer).unwrap(), case["wire"]);
            }
            assert_eq!(serde_json::to_value(&answer).unwrap(), case["answer"]);
        }
        for case in vectors["refused"].as_array().unwrap() {
            let declared: Option<Vec<String>> =
                serde_json::from_value(case["declared_harnesses"].clone()).unwrap();
            let error = decode_serves(&case["serves"], declared.as_deref()).unwrap_err();
            assert_eq!(error.code, case["code"], "{}", case["name"]);
            assert_eq!(error.field, case["field"], "{}", case["name"]);
            if let Ok(typed) = Serves::deserialize(&case["serves"]) {
                assert_eq!(check_serves(&typed, declared.as_deref()), Err(error));
            }
        }
    }

    #[test]
    fn serves_bounds_and_model_grammar() {
        let ids: Vec<_> = (0..=MAX_MODELS).map(|i| format!("model/{i}")).collect();
        let entries: Vec<_> = (0..=MAX_ENTRIES)
            .map(|i| ServesEntry::new(format!("p{i}"), ServesModels::All))
            .collect();
        let snapshot = Serves::new("!".repeat(128)).with_entries(entries[..MAX_ENTRIES].to_vec());
        assert_eq!(check_serves(&snapshot, None), Ok(()));
        assert_eq!(
            check_serves(&snapshot.with_entries(entries), None),
            Err(ServesError::new("serves_too_large", "serves.entries"))
        );
        for field in ["models", "except"] {
            for count in [MAX_MODELS, MAX_MODELS + 1] {
                let mut raw =
                    json!({"generation":"g","entries":[{"provider":"example","models":"all"}]});
                raw["entries"][0][field] = json!(ids[..count]);
                let expected =
                    ServesError::new("serves_too_large", format!("serves.entries[0].{field}"));
                if count == MAX_MODELS {
                    assert!(decode_serves(&raw, None).is_ok());
                } else {
                    assert_eq!(decode_serves(&raw, None), Err(expected.clone()));
                    assert_eq!(
                        check_serves(&Serves::deserialize(raw).unwrap(), None),
                        Err(expected)
                    );
                }
            }
        }
        for id in [
            String::new(),
            "a\u{2003}b".into(),
            "a\t".into(),
            "a\u{009f}".into(),
            "é".repeat(129),
        ] {
            let entry = ServesEntry::new("example", ServesModels::List(vec![id]));
            assert_eq!(
                check_serves(&Serves::new("g").with_entries(vec![entry]), None),
                Err(ServesError::new(
                    "serves_invalid_models",
                    "serves.entries[0].models"
                ))
            );
        }
    }

    #[test]
    fn serves_placement_checks_exact_model_membership() {
        let catalog = BTreeSet::from(["cat/model".into(), "cat/old".into()]);
        let list = [
            ServesEntry::new("example", ServesModels::All).with_except(vec!["cat/old".into()]),
            ServesEntry::new("local", ServesModels::List(vec!["cat/model".into()]))
                .with_login(ServesLogin::None),
        ];
        assert_eq!(
            placement(&list, "example", "cat/model", &catalog),
            Some(false)
        );
        assert_eq!(placement(&list, "example", "cat/old", &catalog), None);
        assert_eq!(placement(&list, "example", "cat/unknown", &catalog), None);
        assert_eq!(placement(&list, "local", "cat/model", &catalog), Some(true));
        assert_eq!(
            placement(&list, "local", "cat/model/descendant", &catalog),
            None
        );
        assert_eq!(placement(&list, "other", "cat/model", &catalog), None);
    }
}
