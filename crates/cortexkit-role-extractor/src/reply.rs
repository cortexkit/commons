//! `extract.run` replies and the provider's answer objects: only what is
//! known before the answer side is measured.
//!
//! An answer keeps the provider's own text for every member this crate
//! names ([`NumberText`], serde_json's `RawValue`), and keeps every other
//! member as it came, so a number never passes through binary floating
//! point and a member the provider adds passes through. Which members each
//! type's answer carries is partly unmeasured (`CONTRACT.md` §5, §12).

use indexmap::IndexMap;
use serde::{de::Error as _, ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{value::RawValue, Number, Value};

pub use cortexkit_model_batch::number::NumberText;
use cortexkit_model_batch::{absent::add_reported, number::same_text};

use crate::{errors::ItemError, field::FieldType};

/// The answer members this crate names, in the order it encodes them
/// after `type`.
pub const ANSWER_MEMBERS: &[&str] = &[
    "value",
    "guess",
    "p_null",
    "confidence",
    "span",
    "spans",
    "mean",
    "median",
    "p10",
    "p90",
    "p_below",
    "p_above",
    "quantiles",
    "steps",
];

/// One answer, the provider's answer object as returned. It always has
/// `type`; `value` is `null` when the input does not determine it
/// ("insufficient", not "absent"), and `p_null`, when present, is the
/// provider's probability of that.
///
/// Every member [`ANSWER_MEMBERS`] names is kept as the provider's own
/// text. A member this crate does not name is kept in [`Answer::extra`],
/// in the order it came, and an unknown `type` keeps its string. Encoding
/// writes `type`, the named members in [`ANSWER_MEMBERS`] order, then the
/// others.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Answer {
    pub kind: FieldType,
    pub value: Option<NumberText>,
    pub guess: Option<NumberText>,
    pub p_null: Option<NumberText>,
    pub confidence: Option<NumberText>,
    pub span: Option<NumberText>,
    pub spans: Option<NumberText>,
    pub mean: Option<NumberText>,
    pub median: Option<NumberText>,
    pub p10: Option<NumberText>,
    pub p90: Option<NumberText>,
    pub p_below: Option<NumberText>,
    pub p_above: Option<NumberText>,
    pub quantiles: Option<NumberText>,
    pub steps: Option<NumberText>,
    /// Members this crate does not name, as given.
    pub extra: IndexMap<String, NumberText>,
}

impl Answer {
    /// An answer of `kind` with no members yet.
    pub fn new(kind: FieldType) -> Self {
        Self {
            kind,
            value: None,
            guess: None,
            p_null: None,
            confidence: None,
            span: None,
            spans: None,
            mean: None,
            median: None,
            p10: None,
            p90: None,
            p_below: None,
            p_above: None,
            quantiles: None,
            steps: None,
            extra: IndexMap::new(),
        }
    }

    fn slot(&mut self, name: &str) -> Option<&mut Option<NumberText>> {
        Some(match name {
            "value" => &mut self.value,
            "guess" => &mut self.guess,
            "p_null" => &mut self.p_null,
            "confidence" => &mut self.confidence,
            "span" => &mut self.span,
            "spans" => &mut self.spans,
            "mean" => &mut self.mean,
            "median" => &mut self.median,
            "p10" => &mut self.p10,
            "p90" => &mut self.p90,
            "p_below" => &mut self.p_below,
            "p_above" => &mut self.p_above,
            "quantiles" => &mut self.quantiles,
            "steps" => &mut self.steps,
            _ => return None,
        })
    }

    /// The named member `name`, as the provider's text.
    pub fn member(&self, name: &str) -> Option<&RawValue> {
        let raw = match name {
            "value" => &self.value,
            "guess" => &self.guess,
            "p_null" => &self.p_null,
            "confidence" => &self.confidence,
            "span" => &self.span,
            "spans" => &self.spans,
            "mean" => &self.mean,
            "median" => &self.median,
            "p10" => &self.p10,
            "p90" => &self.p90,
            "p_below" => &self.p_below,
            "p_above" => &self.p_above,
            "quantiles" => &self.quantiles,
            "steps" => &self.steps,
            other => return self.extra.get(other).map(|raw| &**raw),
        };
        raw.as_deref()
    }

    /// Set the member `name` to `raw`, a named member or an extra one.
    pub fn with_member(mut self, name: &str, raw: NumberText) -> Self {
        match self.slot(name) {
            Some(slot) => *slot = Some(raw),
            None => {
                self.extra.insert(name.to_owned(), raw);
            }
        }
        self
    }
}

impl PartialEq for Answer {
    /// The same type and every member with the same text.
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && ANSWER_MEMBERS
                .iter()
                .all(|name| same_text(self.member(name), other.member(name)))
            && self.extra.len() == other.extra.len()
            && self
                .extra
                .iter()
                .zip(&other.extra)
                .all(|((a, x), (b, y))| a == b && x.get() == y.get())
    }
}

impl Serialize for Answer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("type", &self.kind)?;
        for name in ANSWER_MEMBERS {
            if let Some(raw) = self.member(name) {
                map.serialize_entry(name, raw)?;
            }
        }
        for (name, raw) in &self.extra {
            map.serialize_entry(name, raw)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Answer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let members = IndexMap::<String, NumberText>::deserialize(deserializer)?;
        let mut kind = None;
        let mut answer = Self::new(FieldType::Other(String::new()));
        for (name, raw) in members {
            if name == "type" {
                let text: String = serde_json::from_str(raw.get()).map_err(D::Error::custom)?;
                kind = Some(FieldType::parse(&text));
            } else {
                answer = answer.with_member(&name, raw);
            }
        }
        answer.kind = kind.ok_or_else(|| D::Error::missing_field("type"))?;
        Ok(answer)
    }
}

/// An item's token usage, as the provider reported it. A count it did not
/// report is absent, never zero. Members this crate does not name are kept.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passes: Option<u64>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

/// The batch's usage: `{input_tokens?, output_tokens?}`, each summed over
/// the items that reported it, absent when none did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct BatchUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

/// The batch's usage, summed over `items` (absent is not zero).
pub fn sum_usage<'a>(items: impl IntoIterator<Item = &'a ItemResult>) -> BatchUsage {
    items
        .into_iter()
        .filter_map(|item| item.usage.as_ref())
        .fold(BatchUsage::default(), |total, usage| BatchUsage {
            input_tokens: add_reported(total.input_tokens, usage.input_tokens),
            output_tokens: add_reported(total.output_tokens, usage.output_tokens),
        })
}

/// One item's outcome: `answers` if it succeeded, `error` if it failed,
/// never both.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ItemResult {
    /// The item's position in the request.
    pub index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<IndexMap<String, Answer>>,
    /// The provider's truncation flag, when the item succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// The provider's state text, when `options.state_text` asked for it and
    /// the item succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ItemError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl ItemResult {
    pub fn answered(index: u64, answers: IndexMap<String, Answer>) -> Self {
        Self {
            index,
            answers: Some(answers),
            truncated: None,
            state_text: None,
            error: None,
            usage: None,
        }
    }

    pub fn failed(index: u64, error: ItemError) -> Self {
        Self {
            index,
            answers: None,
            truncated: None,
            state_text: None,
            error: Some(error),
            usage: None,
        }
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }
}

/// The `extract.run` reply.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ExtractReply {
    pub batch_id: String,
    pub model: String,
    /// One per request item, in request order.
    pub items: Vec<ItemResult>,
    #[serde(default)]
    pub usage: BatchUsage,
    /// The batch's cumulative input-priced cost in USD, the same on every
    /// reply to the batch. Absent when the model is unpriced, or when an
    /// answered item did not report its input tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Number>,
}

impl ExtractReply {
    /// A reply whose usage is summed from `items`.
    pub fn new(
        batch_id: impl Into<String>,
        model: impl Into<String>,
        items: Vec<ItemResult>,
    ) -> Self {
        let usage = sum_usage(&items);
        Self {
            batch_id: batch_id.into(),
            model: model.into(),
            items,
            usage,
            cost_usd: None,
        }
    }

    pub fn with_cost_usd(mut self, cost_usd: Number) -> Self {
        self.cost_usd = Some(cost_usd);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_keep_number_text_and_unknown_members() {
        let text = r#"{"type":"estimate","value":12.50,"p_null":0.010,"mean":1E+2,"p10":-0,"rationale":{"why":1.0}}"#;
        let answer: Answer = serde_json::from_str(text).unwrap();
        assert_eq!(answer.kind, FieldType::Estimate);
        assert_eq!(answer.value.as_deref().map(RawValue::get), Some("12.50"));
        assert_eq!(answer.mean.as_deref().map(RawValue::get), Some("1E+2"));
        assert_eq!(answer.extra["rationale"].get(), r#"{"why":1.0}"#);
        assert_eq!(serde_json::to_string(&answer).unwrap(), text);
    }

    #[test]
    fn an_unknown_type_and_a_null_value_are_kept() {
        let text = r#"{"type":"sentiment","value":null,"p_null":0.9}"#;
        let answer: Answer = serde_json::from_str(text).unwrap();
        assert_eq!(answer.kind, FieldType::Other("sentiment".into()));
        assert_eq!(answer.value.as_deref().map(RawValue::get), Some("null"));
        assert_eq!(serde_json::to_string(&answer).unwrap(), text);
        assert!(serde_json::from_str::<Answer>(r#"{"value":1}"#).is_err());
    }

    #[test]
    fn unreported_usage_stays_absent() {
        let items = vec![
            ItemResult::answered(0, IndexMap::new()),
            ItemResult::answered(1, IndexMap::new()).with_usage(Usage {
                input_tokens: Some(5),
                ..Usage::default()
            }),
        ];
        let usage = sum_usage(&items);
        assert_eq!(usage.input_tokens, Some(5));
        assert_eq!(usage.output_tokens, None);
    }
}
