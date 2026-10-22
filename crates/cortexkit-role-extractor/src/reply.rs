//! `extract.run` replies and the provider's answer objects.
//!
//! An answer keeps the provider's own text for every member this crate
//! names ([`NumberText`], serde_json's `RawValue`), and keeps every other
//! member as it came, so a number never passes through binary floating
//! point and a member the provider adds passes through. `CONTRACT.md` §5
//! states the members each type's answer carries, as Cloudflare's Clairic
//! returned them in a live probe of 2026-10-22.
//!
//! Spans are forwarded as the provider wrote them. They count Unicode
//! scalar values (Rust `char`s) of the state, end exclusive;
//! [`span_byte_range`] turns one into a byte range of the state for a
//! caller that wants to slice it.

use std::ops::Range;

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
    "unit",
    "guess",
    "p_null",
    "confidence",
    "confidences",
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
    /// A duration's unit, such as `"minutes"`.
    pub unit: Option<NumberText>,
    pub guess: Option<NumberText>,
    pub p_null: Option<NumberText>,
    pub confidence: Option<NumberText>,
    /// A number list's per-element confidences.
    pub confidences: Option<NumberText>,
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
            unit: None,
            guess: None,
            p_null: None,
            confidence: None,
            confidences: None,
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
            "unit" => &mut self.unit,
            "guess" => &mut self.guess,
            "p_null" => &mut self.p_null,
            "confidence" => &mut self.confidence,
            "confidences" => &mut self.confidences,
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
            "unit" => &self.unit,
            "guess" => &self.guess,
            "p_null" => &self.p_null,
            "confidence" => &self.confidence,
            "confidences" => &self.confidences,
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

/// The UTF-8 byte range of `state` that one provider span covers, for a
/// caller that wants to slice the state. The role itself never converts a
/// span: it forwards spans as the provider wrote them.
///
/// A span is `[start, end]`: offsets in Unicode scalar values (Rust
/// `char`s) of the state the provider read, `end` exclusive (`CONTRACT.md`
/// §5). Clairic's `state_text` is the caller's `state` byte for byte, so
/// `state` is the caller's own text. `None` when `span` is not an array of
/// two whole numbers, when `start` is past `end`, or when `end` is past the
/// state's last scalar value. Every scalar offset falls on a `char`
/// boundary, so a returned range always slices `state`.
pub fn span_byte_range(state: &str, span: &RawValue) -> Option<Range<usize>> {
    let [start, end]: [u64; 2] = serde_json::from_str(span.get()).ok()?;
    if start > end {
        return None;
    }
    // The byte offset of scalar `n`; the state's length for one past the
    // last scalar.
    let byte_at = |n: u64| -> Option<usize> {
        let n = usize::try_from(n).ok()?;
        state
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(state.len()))
            .nth(n)
    };
    Some(byte_at(start)?..byte_at(end)?)
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

    /// The state live probe 02 sent Clairic: a precomposed é, an
    /// emoji outside the Basic Multilingual Plane and four CJK characters
    /// before the tracking code.
    const PROBE_02_STATE: &str = "Note from the café 😀 東京支社 team: the tracking code is QK-58213, and the parcel shipped on Monday.";

    fn raw(text: &str) -> NumberText {
        RawValue::from_string(text.to_owned()).unwrap()
    }

    #[test]
    fn a_span_counts_scalar_values_with_an_exclusive_end() {
        // Clairic answered `span: [53,61]` for `QK-58213`: 65..73 in UTF-8
        // bytes, 54..62 in UTF-16 units.
        let range = span_byte_range(PROBE_02_STATE, &raw("[53,61]")).unwrap();
        assert_eq!(range, 65..73);
        assert_eq!(&PROBE_02_STATE[range], "QK-58213");
        let scalars = PROBE_02_STATE.chars().count() as u64;
        let whole = span_byte_range(PROBE_02_STATE, &raw(&format!("[0,{scalars}]"))).unwrap();
        assert_eq!(&PROBE_02_STATE[whole], PROBE_02_STATE);
        assert_eq!(
            span_byte_range(PROBE_02_STATE, &raw("[19,20]")).map(|r| &PROBE_02_STATE[r]),
            Some("😀")
        );
    }

    #[test]
    fn a_span_out_of_range_or_malformed_has_no_byte_range() {
        let scalars = PROBE_02_STATE.chars().count();
        for bad in [
            format!("[0,{}]", scalars + 1),
            "[61,53]".to_owned(),
            "[53.5,61]".to_owned(),
            "[-1,3]".to_owned(),
            "[53]".to_owned(),
            "null".to_owned(),
        ] {
            assert_eq!(span_byte_range(PROBE_02_STATE, &raw(&bad)), None, "{bad}");
        }
        assert_eq!(span_byte_range("", &raw("[0,0]")), Some(0..0));
    }

    #[test]
    fn a_duration_unit_and_list_confidences_are_named_members() {
        // Clairic's own answers, from probes 01 and 03.
        let duration = r#"{"type":"duration","p_null":5.235667686065426e-7,"unit":"minutes","guess":445,"confidence":0.9987765309792793,"value":445}"#;
        let answer: Answer = serde_json::from_str(duration).unwrap();
        assert_eq!(answer.kind, FieldType::Duration);
        assert_eq!(
            answer.unit.as_deref().map(RawValue::get),
            Some(r#""minutes""#)
        );
        assert!(answer.extra.is_empty());
        let list = r#"{"type":"list","value":[2.3,0.1,11.8,4.5,7.2],"confidences":[0.9986772934005607,0.999997615820007,0.9999840261592093,0.9999405170603807,0.9994769646306935],"confidence":0.9975896220014346}"#;
        let answer: Answer = serde_json::from_str(list).unwrap();
        assert_eq!(
            answer.confidences.as_deref().map(RawValue::get),
            Some("[0.9986772934005607,0.999997615820007,0.9999840261592093,0.9999405170603807,0.9994769646306935]")
        );
        assert!(answer.extra.is_empty());
        // Encoding writes the named members in `ANSWER_MEMBERS` order.
        assert_eq!(
            serde_json::to_string(&answer).unwrap(),
            r#"{"type":"list","value":[2.3,0.1,11.8,4.5,7.2],"confidence":0.9975896220014346,"confidences":[0.9986772934005607,0.999997615820007,0.9999840261592093,0.9999405170603807,0.9994769646306935]}"#
        );
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
