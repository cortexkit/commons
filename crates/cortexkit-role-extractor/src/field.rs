//! Fields: the typed schema of an `extract.run` batch, forwarded to the
//! provider under its own key names.
//!
//! A field is `{type, instructions?, ...options}`. The nine types and the
//! options each takes are the provider's (Cloudflare Workers AI Clairic);
//! `CONTRACT.md` §4 holds the table. The role refuses an option outside the
//! table, or on a type the table doesn't list for it, so a misspelt option
//! is never silently dropped.

use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::errors::Refusal;

/// Field type names.
pub mod types {
    pub const TEXT: &str = "text";
    pub const NUMBER: &str = "number";
    pub const DATE: &str = "date";
    pub const TIME: &str = "time";
    pub const DATETIME: &str = "datetime";
    pub const DURATION: &str = "duration";
    pub const LIST: &str = "list";
    pub const ESTIMATE: &str = "estimate";
    pub const FORECAST: &str = "forecast";
    /// Every type, in the contract's order.
    pub const ALL: &[&str] = &[
        TEXT, NUMBER, DATE, TIME, DATETIME, DURATION, LIST, ESTIMATE, FORECAST,
    ];
}

/// Option names, as the provider spells them.
pub mod options {
    pub const INTEGER: &str = "integer";
    pub const DECIMALS: &str = "decimals";
    pub const UNIT: &str = "unit";
    pub const ITEMS: &str = "items";
    pub const MIN: &str = "min";
    pub const MAX: &str = "max";
    pub const STEP: &str = "step";
    pub const SCALE: &str = "scale";
    pub const NULL_THRESHOLD: &str = "null_threshold";
    pub const HORIZON: &str = "horizon";
    pub const INTERVAL: &str = "interval";
}

/// Longest field id, in bytes.
pub const FIELD_ID_MAX_LEN: usize = 64;
/// Most decimals a `decimals` option asks for.
pub const DECIMALS_MAX: u64 = 6;
/// Fewest and most steps a forecast's `horizon` asks for.
pub const HORIZON_MIN: u64 = 1;
pub const HORIZON_MAX: u64 = 64;
/// A duration's `unit` values.
pub const DURATION_UNITS: &[&str] = &[
    "seconds", "minutes", "hours", "days", "weeks", "months", "years",
];
/// A list's `items` values.
pub const LIST_ITEM_TYPES: &[&str] = &[types::TEXT, types::NUMBER, types::DATE];
/// `scale` values.
pub const SCALES: &[&str] = &["linear", "log"];
/// A forecast's `interval` values.
pub const INTERVALS: &[&str] = &[
    "step", "minute", "hour", "day", "week", "month", "quarter", "year",
];

/// A field type. Decodes open: a value other than [`types`] is kept in
/// [`FieldType::Other`]. The open variant is for decoding a reply, where a
/// provider's new type passes through as given; in a request,
/// [`check_field`] refuses it `invalid_params` naming `fields.<id>.type`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FieldType {
    Text,
    Number,
    Date,
    Time,
    Datetime,
    Duration,
    List,
    Estimate,
    Forecast,
    Other(String),
}

impl FieldType {
    pub fn parse(name: &str) -> Self {
        match name {
            types::TEXT => Self::Text,
            types::NUMBER => Self::Number,
            types::DATE => Self::Date,
            types::TIME => Self::Time,
            types::DATETIME => Self::Datetime,
            types::DURATION => Self::Duration,
            types::LIST => Self::List,
            types::ESTIMATE => Self::Estimate,
            types::FORECAST => Self::Forecast,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Text => types::TEXT,
            Self::Number => types::NUMBER,
            Self::Date => types::DATE,
            Self::Time => types::TIME,
            Self::Datetime => types::DATETIME,
            Self::Duration => types::DURATION,
            Self::List => types::LIST,
            Self::Estimate => types::ESTIMATE,
            Self::Forecast => types::FORECAST,
            Self::Other(other) => other,
        }
    }

    /// Whether this is one of the nine types the role names.
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for FieldType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FieldType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(deserializer)?))
    }
}

/// One field: `{type, instructions?, ...options}`, its options in the
/// order the caller wrote them.
///
/// Non-exhaustive so later optional members are additive: use
/// [`Field::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Field {
    #[serde(rename = "type")]
    pub kind: FieldType,
    /// A non-empty string, at most the module's `max_instruction_bytes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Every other member, by the provider's option name.
    #[serde(flatten)]
    pub options: IndexMap<String, Value>,
}

impl Field {
    pub fn new(kind: FieldType) -> Self {
        Self {
            kind,
            instructions: None,
            options: IndexMap::new(),
        }
    }

    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    pub fn with_option(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.options.insert(name.into(), value.into());
        self
    }
}

/// Whether `id` is a valid field id: `^[A-Za-z_][A-Za-z0-9_-]{0,63}$`.
/// That is the provider's grammar without `.`, so a `detail.field` path
/// (`fields.<id>.type`) is never ambiguous, and every role id is a valid
/// provider id.
pub fn is_field_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= FIELD_ID_MAX_LEN
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

/// The types an option is allowed on, from the contract's §4 table, and
/// whether it is required on them. `None` for a name the table doesn't
/// list.
pub fn option_rule(name: &str, kind: &FieldType) -> Option<(bool, bool)> {
    use FieldType::*;
    let (allowed, required) = match name {
        options::INTEGER | options::UNIT => (matches!(kind, Number | Duration), false),
        options::DECIMALS => (matches!(kind, Number | Duration | List), false),
        options::ITEMS => (matches!(kind, List), true),
        options::MIN | options::MAX => (matches!(kind, Estimate | Forecast), true),
        options::STEP | options::SCALE => (matches!(kind, Estimate | Forecast), false),
        options::NULL_THRESHOLD => (kind.is_known(), false),
        options::HORIZON | options::INTERVAL => (matches!(kind, Forecast), true),
        _ => return None,
    };
    Some((allowed, allowed && required))
}

/// The options a type requires, in the table's order.
pub fn required_options(kind: &FieldType) -> &'static [&'static str] {
    match kind {
        FieldType::List => &[options::ITEMS],
        FieldType::Estimate => &[options::MIN, options::MAX],
        FieldType::Forecast => &[
            options::MIN,
            options::MAX,
            options::HORIZON,
            options::INTERVAL,
        ],
        _ => &[],
    }
}

fn integer_in(value: &Value, min: u64, max: u64) -> bool {
    value.as_u64().is_some_and(|n| (min..=max).contains(&n))
}

fn one_of(value: &Value, allowed: &[&str]) -> bool {
    value.as_str().is_some_and(|text| allowed.contains(&text))
}

/// Why `value` is not a valid `name` option on `kind`, or `None` when it
/// is. Called for options [`option_rule`] allows on `kind`.
fn option_value_problem(name: &str, kind: &FieldType, value: &Value) -> Option<String> {
    let ok = match name {
        options::INTEGER => value.is_boolean(),
        options::DECIMALS => integer_in(value, 0, DECIMALS_MAX),
        options::UNIT => match kind {
            FieldType::Duration => one_of(value, DURATION_UNITS),
            _ => value.as_str().is_some_and(|unit| !unit.is_empty()),
        },
        options::ITEMS => one_of(value, LIST_ITEM_TYPES),
        // Number bounds only: a date estimate (ISO-date bounds) is not yet
        // measured, so it is refused until it is (CONTRACT.md §12).
        options::MIN | options::MAX => value.is_number(),
        // A positive number of steps; `month` and `year` steps belong to
        // date estimates, refused for the same reason.
        options::STEP => value.as_f64().is_some_and(|step| step > 0.0),
        options::SCALE => one_of(value, SCALES),
        options::NULL_THRESHOLD => value
            .as_f64()
            .is_some_and(|threshold| threshold > 0.0 && threshold <= 1.0),
        options::HORIZON => integer_in(value, HORIZON_MIN, HORIZON_MAX),
        options::INTERVAL => one_of(value, INTERVALS),
        _ => true,
    };
    if ok {
        return None;
    }
    Some(match name {
        options::INTEGER => "integer is true or false".into(),
        options::DECIMALS => format!("decimals is an integer from 0 to {DECIMALS_MAX}"),
        options::UNIT if *kind == FieldType::Duration => {
            format!("a duration's unit is one of {}", DURATION_UNITS.join(", "))
        }
        options::UNIT => "unit is a non-empty string".into(),
        options::ITEMS => format!("items is one of {}", LIST_ITEM_TYPES.join(", ")),
        options::MIN | options::MAX => format!("{name} is a number"),
        options::STEP => "step is a positive number".into(),
        options::SCALE => format!("scale is one of {}", SCALES.join(", ")),
        options::NULL_THRESHOLD => "null_threshold is a number above 0, at most 1".into(),
        options::HORIZON => format!("horizon is an integer from {HORIZON_MIN} to {HORIZON_MAX}"),
        options::INTERVAL => format!("interval is one of {}", INTERVALS.join(", ")),
        _ => format!("{name} is not valid"),
    })
}

/// Check one field. Refusals are `invalid_params`, checked in this order,
/// each naming its path:
///
/// 1. the id (`fields.<id>`);
/// 2. the type, before anything else, since the rest depends on it
///    (`fields.<id>.type`): one of the nine, and one of `served_types`, the
///    model's `field_types`;
/// 3. `instructions`, when present: non-empty, at most
///    `max_instruction_bytes` (`fields.<id>.instructions`);
/// 4. each option in the order written: a name the table lists for this
///    type, with a valid value (`fields.<id>.<option>`);
/// 5. each option the type requires, in the table's order
///    (`fields.<id>.<option>`);
/// 6. the options together: `min < max` (`fields.<id>.max`), `scale: log`
///    only with `min > 0` (`fields.<id>.scale`), and `decimals` on a list
///    only of `items: number` (`fields.<id>.decimals`).
pub fn check_field(
    id: &str,
    field: &Field,
    served_types: &[FieldType],
    max_instruction_bytes: u64,
) -> Result<(), Refusal> {
    let at = |member: &str| format!("fields.{id}.{member}");
    if !is_field_id(id) {
        return Err(Refusal::invalid_params(
            format!("fields.{id}"),
            "a field id is a letter or _, then up to 63 letters, digits, _ or -",
        ));
    }
    if !field.kind.is_known() {
        return Err(Refusal::invalid_params(
            at("type"),
            format!(
                "unknown field type {}; expected one of {}",
                field.kind,
                types::ALL.join(", ")
            ),
        ));
    }
    if !served_types.contains(&field.kind) {
        return Err(Refusal::invalid_params(
            at("type"),
            format!("this model does not take {} fields", field.kind),
        ));
    }
    if let Some(instructions) = &field.instructions {
        if instructions.is_empty() {
            return Err(Refusal::invalid_params(
                at("instructions"),
                "instructions, when present, is a non-empty string",
            ));
        }
        if instructions.len() as u64 > max_instruction_bytes {
            return Err(Refusal::over_limit(
                at("instructions"),
                max_instruction_bytes,
                format!(
                    "instructions is {} bytes; at most {max_instruction_bytes}",
                    instructions.len()
                ),
            ));
        }
    }
    for (name, value) in &field.options {
        match option_rule(name, &field.kind) {
            None => {
                return Err(Refusal::invalid_params(
                    at(name),
                    format!("{name} is not an option of any field type"),
                ))
            }
            Some((false, _)) => {
                return Err(Refusal::invalid_params(
                    at(name),
                    format!("{name} is not an option of a {} field", field.kind),
                ))
            }
            Some((true, _)) => {}
        }
        if let Some(problem) = option_value_problem(name, &field.kind, value) {
            return Err(Refusal::invalid_params(at(name), problem));
        }
    }
    for name in required_options(&field.kind) {
        if !field.options.contains_key(*name) {
            return Err(Refusal::invalid_params(
                at(name),
                format!("a {} field requires {name}", field.kind),
            ));
        }
    }
    let number = |name: &str| field.options.get(name).and_then(Value::as_f64);
    if let (Some(min), Some(max)) = (number(options::MIN), number(options::MAX)) {
        if min >= max {
            return Err(Refusal::invalid_params(
                at(options::MAX),
                "max is above min",
            ));
        }
        let log = field.options.get(options::SCALE).and_then(Value::as_str) == Some("log");
        if log && min <= 0.0 {
            return Err(Refusal::invalid_params(
                at(options::SCALE),
                "a log scale needs min above 0",
            ));
        }
    }
    if field.kind == FieldType::List
        && field.options.contains_key(options::DECIMALS)
        && field.options.get(options::ITEMS).and_then(Value::as_str) != Some(types::NUMBER)
    {
        return Err(Refusal::invalid_params(
            at(options::DECIMALS),
            "decimals on a list needs items: number",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn all() -> Vec<FieldType> {
        types::ALL.iter().map(|t| FieldType::parse(t)).collect()
    }

    fn field_of(value: Value) -> Field {
        serde_json::from_value(value).unwrap()
    }

    fn refused(value: Value) -> String {
        check_field("f", &field_of(value), &all(), 1000)
            .unwrap_err()
            .field()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn field_ids_are_the_provider_grammar_without_dots() {
        for good in ["a", "_x", "Total_2", "due-date", &"a".repeat(64)] {
            assert!(is_field_id(good), "{good}");
        }
        for bad in ["", "9x", "-x", "a.b", "a b", &"a".repeat(65)] {
            assert!(!is_field_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_type_is_checked_before_everything_else() {
        assert_eq!(
            refused(json!({"type": "rank", "bogus": 1})),
            "fields.f.type"
        );
        assert_eq!(
            refused(json!({"type": "rank", "instructions": ""})),
            "fields.f.type"
        );
        let text_only = [FieldType::Text];
        let refusal =
            check_field("f", &field_of(json!({"type": "number"})), &text_only, 10).unwrap_err();
        assert_eq!(refusal.field(), Some("fields.f.type"));
    }

    #[test]
    fn misspelt_and_misplaced_options_are_refused_by_name() {
        assert_eq!(
            refused(json!({"type": "number", "decimal": 2})),
            "fields.f.decimal"
        );
        assert_eq!(
            refused(json!({"type": "text", "integer": true})),
            "fields.f.integer"
        );
        assert!(check_field(
            "f",
            &field_of(json!({"type": "number", "integer": true, "unit": "EUR"})),
            &all(),
            10
        )
        .is_ok());
    }

    #[test]
    fn required_options_and_values_are_checked() {
        assert_eq!(refused(json!({"type": "list"})), "fields.f.items");
        assert_eq!(
            refused(json!({"type": "forecast", "min": 0, "max": 1, "interval": "day"})),
            "fields.f.horizon"
        );
        assert_eq!(
            refused(json!({"type": "number", "decimals": 7})),
            "fields.f.decimals"
        );
        assert_eq!(
            refused(json!({"type": "duration", "unit": "fortnights"})),
            "fields.f.unit"
        );
        assert_eq!(
            refused(json!({"type": "text", "null_threshold": 0})),
            "fields.f.null_threshold"
        );
        assert_eq!(
            refused(json!({"type": "estimate", "min": 5, "max": 5})),
            "fields.f.max"
        );
        assert_eq!(
            refused(json!({"type": "estimate", "min": 0, "max": 5, "scale": "log"})),
            "fields.f.scale"
        );
        // `json!` sorts keys, so `max` is the first option checked here.
        assert_eq!(
            refused(json!({"type": "estimate", "min": "2026-01-01", "max": "2026-12-31"})),
            "fields.f.max"
        );
        assert_eq!(
            refused(json!({"type": "list", "items": "text", "decimals": 2})),
            "fields.f.decimals"
        );
        assert!(check_field(
            "f",
            &field_of(json!({"type": "forecast", "min": 1, "max": 9, "scale": "log", "horizon": 12, "interval": "month", "step": 0.5, "null_threshold": 1})),
            &all(),
            10
        )
        .is_ok());
    }

    #[test]
    fn instructions_are_non_empty_and_bounded() {
        assert_eq!(
            refused(json!({"type": "text", "instructions": ""})),
            "fields.f.instructions"
        );
        let refusal = check_field(
            "f",
            &field_of(json!({"type": "text", "instructions": "abcdef"})),
            &all(),
            5,
        )
        .unwrap_err();
        assert_eq!(refusal.field(), Some("fields.f.instructions"));
        assert_eq!(refusal.detail.limit, Some(5));
    }

    #[test]
    fn options_keep_their_written_order() {
        let text = r#"{"type":"number","unit":"EUR","integer":false,"decimals":2}"#;
        let field: Field = serde_json::from_str(text).unwrap();
        assert_eq!(serde_json::to_string(&field).unwrap(), text);
    }
}
