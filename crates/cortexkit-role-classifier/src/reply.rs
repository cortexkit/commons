//! `classify.run` replies and the provider's answer objects.
//!
//! After admission the batch never fails as a whole: every item gets
//! `answers` or `error`, at its position in the request.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::{errors::ItemError, question::QuestionType, request::ClassifyRequest};

/// One answer, the provider's answer object as returned. Which members are
/// present depends on the question type:
///
/// - `noul`: `{type, noul}`. `noul` is the probability that the answer is
///   yes, from 0 (no) to 1 (yes).
/// - `choice`: `{type, choice, probabilities, confidence}`.
/// - `score`: `{type, score, legend, probabilities, confidence}`. `score`
///   is fractional, from 0 to levels - 1, and can land between levels;
///   `legend` and `probabilities` are keyed by the level index as a string.
///
/// Numbers are [`Number`]s, so an integer stays an integer, and a member
/// the provider adds that this crate does not name is kept in
/// [`Answer::extra`]. A reply decodes open: an answer of a type this crate
/// does not name keeps its type string, and every member is kept as given.
/// The runner never rounds, rescales or renames them.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Answer {
    #[serde(rename = "type")]
    pub kind: QuestionType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noul: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend: Option<IndexMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<IndexMap<String, Number>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Number>,
    /// Members the provider returned that this crate does not name, kept
    /// in the order they came.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

impl Answer {
    fn empty(kind: QuestionType) -> Self {
        Self {
            kind,
            noul: None,
            choice: None,
            score: None,
            legend: None,
            probabilities: None,
            confidence: None,
            extra: IndexMap::new(),
        }
    }

    /// A `noul` answer.
    pub fn noul(noul: Number) -> Self {
        Self {
            noul: Some(noul),
            ..Self::empty(QuestionType::Noul)
        }
    }

    /// A `choice` answer.
    pub fn choice(
        choice: impl Into<String>,
        probabilities: IndexMap<String, Number>,
        confidence: Number,
    ) -> Self {
        Self {
            choice: Some(choice.into()),
            probabilities: Some(probabilities),
            confidence: Some(confidence),
            ..Self::empty(QuestionType::Choice)
        }
    }

    /// A `score` answer.
    pub fn score(
        score: Number,
        legend: IndexMap<String, Value>,
        probabilities: IndexMap<String, Number>,
        confidence: Number,
    ) -> Self {
        Self {
            score: Some(score),
            legend: Some(legend),
            probabilities: Some(probabilities),
            confidence: Some(confidence),
            ..Self::empty(QuestionType::Score)
        }
    }
}

/// Token usage. A count the provider did not report is absent, never zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

impl Usage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_input_tokens(mut self, tokens: u64) -> Self {
        self.input_tokens = Some(tokens);
        self
    }

    pub fn with_output_tokens(mut self, tokens: u64) -> Self {
        self.output_tokens = Some(tokens);
        self
    }

    /// Whether neither count was reported.
    pub fn is_empty(&self) -> bool {
        self.input_tokens.is_none() && self.output_tokens.is_none()
    }
}

/// The batch's usage: each count summed over the items that reported it,
/// and absent when none did.
pub fn sum_usage<'a>(items: impl IntoIterator<Item = &'a ItemResult>) -> Usage {
    let add = |total: Option<u64>, count: Option<u64>| match (total, count) {
        (Some(total), Some(count)) => Some(total + count),
        (total, None) => total,
        (None, count) => count,
    };
    items
        .into_iter()
        .filter_map(|item| item.usage)
        .fold(Usage::default(), |total, usage| Usage {
            input_tokens: add(total.input_tokens, usage.input_tokens),
            output_tokens: add(total.output_tokens, usage.output_tokens),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ItemError>,
    /// What the provider reported for this item; absent when it reported
    /// nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl ItemResult {
    pub fn answered(index: u64, answers: IndexMap<String, Answer>) -> Self {
        Self {
            index,
            answers: Some(answers),
            error: None,
            usage: None,
        }
    }

    pub fn failed(index: u64, error: ItemError) -> Self {
        Self {
            index,
            answers: None,
            error: Some(error),
            usage: None,
        }
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }
}

/// The `classify.run` reply.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ClassifyReply {
    pub batch_id: String,
    pub model: String,
    /// One per request item, in request order.
    pub items: Vec<ItemResult>,
    /// Summed over answered items; a count no item reported is absent.
    #[serde(default)]
    pub usage: Usage,
    /// The batch's cumulative cost in USD, the same on every reply to the
    /// batch, a replay included. Absent only when the model is unpriced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Number>,
}

impl ClassifyReply {
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

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }

    pub fn with_cost_usd(mut self, cost_usd: Number) -> Self {
        self.cost_usd = Some(cost_usd);
        self
    }
}

/// Check a reply against the request it answers: the same batch and model,
/// one item per request item in request order, each with exactly one of
/// `answers` and `error`, and every answered item answering exactly the
/// request's questions with answers of the asked type. The error names the
/// first problem.
pub fn check_reply(reply: &ClassifyReply, request: &ClassifyRequest) -> Result<(), String> {
    if reply.batch_id != request.batch_id {
        return Err(format!(
            "batch_id {} answers request {}",
            reply.batch_id, request.batch_id
        ));
    }
    if reply.model != request.model {
        return Err(format!(
            "model {} answers a request for {}",
            reply.model, request.model
        ));
    }
    if reply.items.len() != request.items.len() {
        return Err(format!(
            "{} items answer {} request items",
            reply.items.len(),
            request.items.len()
        ));
    }
    for (position, item) in reply.items.iter().enumerate() {
        if item.index != position as u64 {
            return Err(format!(
                "item at position {position} has index {}",
                item.index
            ));
        }
        match (&item.answers, &item.error) {
            (Some(answers), None) => {
                let asked: Vec<&String> = request.questions.keys().collect();
                let got: Vec<&String> = answers.keys().collect();
                let mut asked_sorted = asked.clone();
                let mut got_sorted = got.clone();
                asked_sorted.sort();
                got_sorted.sort();
                if asked_sorted != got_sorted {
                    return Err(format!(
                        "item {position} answers {got:?}, the request asked {asked:?}"
                    ));
                }
                for (id, answer) in answers {
                    let kind = &request.questions[id].kind;
                    if &answer.kind != kind {
                        return Err(format!(
                            "item {position} answers {id} as {}, asked as {kind}",
                            answer.kind
                        ));
                    }
                }
            }
            (None, Some(_)) => {}
            (Some(_), Some(_)) => {
                return Err(format!("item {position} has both answers and error"))
            }
            (None, None) => return Err(format!("item {position} has neither answers nor error")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{errors::ItemError, question::Question, request::Item};

    #[test]
    fn unreported_usage_stays_absent_in_the_sum() {
        let items = vec![
            ItemResult::answered(0, IndexMap::new()),
            ItemResult::answered(1, IndexMap::new()).with_usage(Usage::new().with_input_tokens(5)),
            ItemResult::failed(2, ItemError::for_code("invalid_item", "m")),
        ];
        let usage = sum_usage(&items);
        assert_eq!(usage.input_tokens, Some(5));
        assert_eq!(usage.output_tokens, None);
        assert!(sum_usage(&items[..1]).is_empty());
        let reply = ClassifyReply::new("b", "m", items[..1].to_vec());
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"batch_id":"b","model":"m","items":[{"index":0,"answers":{}}],"usage":{}}"#
        );
    }

    #[test]
    fn answers_keep_integers_order_and_unknown_members() {
        let text = r#"{"type":"choice","choice":"b","probabilities":{"b":1,"a":0.0},"confidence":0.5,"rationale":"x"}"#;
        let answer: Answer = serde_json::from_str(text).unwrap();
        assert_eq!(answer.extra["rationale"], json!("x"));
        assert_eq!(serde_json::to_string(&answer).unwrap(), text);
    }

    #[test]
    fn a_reply_out_of_order_or_with_both_members_is_refused() {
        let request = ClassifyRequest::new(
            "b",
            "m",
            [("q".to_owned(), Question::noul("i"))].into(),
            vec![Item::new("a"), Item::new("b")],
        );
        let answers: IndexMap<String, Answer> =
            [("q".to_owned(), Answer::noul(Number::from_f64(0.5).unwrap()))].into();
        let good = ClassifyReply::new(
            "b",
            "m",
            vec![
                ItemResult::answered(0, answers.clone()),
                ItemResult::failed(1, ItemError::for_code("rate_limited", "m")),
            ],
        );
        assert_eq!(check_reply(&good, &request), Ok(()));
        let mut swapped = good.clone();
        swapped.items.swap(0, 1);
        assert!(check_reply(&swapped, &request)
            .unwrap_err()
            .contains("index"));
        let mut both = good;
        both.items[1].answers = Some(answers);
        assert!(check_reply(&both, &request).unwrap_err().contains("both"));
    }
}
