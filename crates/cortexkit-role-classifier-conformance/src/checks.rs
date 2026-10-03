//! The checks, each run against the live module through its route.

use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use cortexkit_role_classifier::{
    describe::{check_describe, ModelEntry, RoleDescribe},
    errors::{self, ErrorClass, Refusal},
    ops,
    reply::ClassifyReply,
    IndexMap, PROVIDES,
};
use futures_util::future::{join, select, Either};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};

use crate::{
    route::{ClassifierRoute, Reply},
    subject::{state_key, ClassifierSubject, Scripted, MAX_POLLS},
};

/// How a check that did not fail ended.
pub enum Ending {
    Passed,
    /// The module does not do what the check asks about.
    NotApplicable(String),
}

type Check = Result<Ending, String>;

/// A one-pixel PNG, as a `data:` URL.
const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// The largest request the suite builds to exceed `max_request_bytes`.
const OVERSIZE_CAP: u64 = 64 * 1024 * 1024;

/// The spend, in USD, the ceiling checks make the first item cost, and the
/// ceiling it crosses.
const ITEM_SPEND_USD: f64 = 2.0;
const CEILING_USD: f64 = 1.0;
const LOOSE_CEILING_USD: f64 = 1000.0;

/// The question every check asks unless it needs another.
fn noul_question() -> Value {
    json!({"q": {"type": "noul", "instructions": "Is this item flagged?"}})
}

/// A stand-in provider response answering the noul question `q` with
/// `noul`, spelled as given, and `usage` as given JSON text when present.
fn noul_body(noul: &str, usage: Option<&str>) -> String {
    let usage = usage.map_or(String::new(), |usage| format!(",\"usage\":{usage}"));
    format!(
        "{{\"model\":\"stand-in\",\"answers\":{{\"q\":{{\"type\":\"noul\",\"noul\":{noul}}}}}{usage}}}"
    )
}

fn string(text: &str) -> String {
    serde_json::to_string(text).expect("a string encodes")
}

/// `classify.run` params with the given item objects.
fn params(
    batch_id: &str,
    model: &str,
    questions: &Value,
    items: Vec<Value>,
    max_cost_usd: Option<f64>,
) -> String {
    let mut params = json!({
        "batch_id": batch_id,
        "model": model,
        "questions": questions,
        "items": items,
    });
    if let Some(max) = max_cost_usd {
        params["max_cost_usd"] = json!(max);
    }
    params.to_string()
}

fn items(states: &[&str]) -> Vec<Value> {
    states.iter().map(|state| json!({"state": state})).collect()
}

/// The number leaves of a JSON text, by path, with their exact spelling.
fn number_leaves(raw: &str, path: &str, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') {
        let map: IndexMap<String, Box<RawValue>> =
            serde_json::from_str(raw).map_err(|e| format!("{path}: {e}"))?;
        for (key, value) in map {
            number_leaves(value.get(), &format!("{path}.{key}"), out)?;
        }
    } else if trimmed.starts_with('[') {
        let list: Vec<Box<RawValue>> =
            serde_json::from_str(raw).map_err(|e| format!("{path}: {e}"))?;
        for (i, value) in list.iter().enumerate() {
            number_leaves(value.get(), &format!("{path}[{i}]"), out)?;
        }
    } else if trimmed.starts_with(|c: char| c == '-' || c.is_ascii_digit()) {
        out.push((path.to_owned(), trimmed.trim_end().to_owned()));
    }
    Ok(())
}

#[derive(Deserialize)]
struct RawReplyText {
    items: Vec<RawItemText>,
    #[serde(default)]
    cost_usd: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct RawItemText {
    #[serde(default)]
    answers: Option<Box<RawValue>>,
}

/// The suite's state for one run: the route, the decoded `role.describe`,
/// and the run's unique prefix for batch ids and item states.
pub struct Suite<'a, S: ClassifierSubject> {
    subject: &'a S,
    route: S::Route,
    describe: RoleDescribe,
    run: String,
    counter: AtomicUsize,
}

impl<'a, S> Suite<'a, S>
where
    S: ClassifierSubject,
    S::Route: ClassifierRoute,
{
    /// Open the route and read `role.describe`, which every check uses to
    /// pick its model and limits.
    pub async fn open(subject: &'a S) -> Result<Self, String> {
        let route = subject
            .route()
            .await
            .map_err(|e| format!("opening the module's route: {e}"))?;
        let text = match route.request(ops::ROLE_DESCRIBE, "{}").await {
            Ok(Reply::Response(text)) => text,
            Ok(Reply::Error(body)) => {
                return Err(format!(
                    "role.describe was refused {}: {}",
                    body.code, body.message
                ))
            }
            Err(failure) => return Err(format!("role.describe: {}", failure.message)),
        };
        let describe: RoleDescribe = serde_json::from_str(&text)
            .map_err(|e| format!("role.describe does not decode: {e}"))?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        Ok(Self {
            subject,
            route,
            describe,
            run: format!("ck{nanos:x}"),
            counter: AtomicUsize::new(0),
        })
    }

    pub async fn run(&self, name: &str) -> Check {
        match name {
            "describe_complete" => self.describe_complete().await,
            "concurrent_call_refused_batch_in_progress" => self.concurrent().await,
            "retry_calls_only_unanswered_items" => self.retry_only_unanswered().await,
            "stored_permanent_error_replayed_transient_retried" => self.stored_errors().await,
            "cost_usd_stable_on_replay" => self.cost_stable().await,
            "looser_max_cost_does_not_raise_ceiling" => self.ceiling(LOOSE_CEILING_USD).await,
            "tighter_max_cost_stops_crossing_items" => self.ceiling(CEILING_USD).await,
            "tightened_ceiling_recorded_across_resends" => self.ceiling_recorded().await,
            "reuse_refused_naming_field" => self.reuse().await,
            "reordered_object_state_is_replay" => self.reordered_state().await,
            "validation_refusals_reach_no_provider" => self.validation().await,
            "null_state_refused_naming_field" => self.null_state().await,
            "unknown_question_type_refused_naming_field" => self.unknown_type().await,
            "answers_in_request_order" => self.request_order().await,
            "provider_numbers_byte_for_byte" => self.numbers().await,
            "one_failing_item_does_not_fail_batch" => self.one_failing_item().await,
            "auth_failure_stops_the_call_unstored" => {
                self.stopping_status(401, errors::AUTH_FAILED, "auth").await
            }
            "auth_failure_on_first_call_refused" => self.auth_first_call().await,
            "model_unavailable_stops_the_call_unstored" => self.model_unavailable().await,
            "rate_limit_stops_the_rest_of_the_call" => {
                self.stopping_status(429, errors::RATE_LIMITED, "limit")
                    .await
            }
            "unreported_usage_stays_absent" => self.unreported_usage().await,
            other => Err(format!("the suite has no check named {other}")),
        }
    }

    // ---- helpers --------------------------------------------------------

    /// A name unique to this run. The counter is zero-padded, so names
    /// minted later sort later.
    fn mint(&self, label: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{}-{label}-{n:06}", self.run)
    }

    /// A unique item state and the stand-in key it is counted under.
    fn marker(&self, label: &str) -> (String, String) {
        let state = self.mint(label);
        let key = state_key(&Value::String(state.clone()));
        (state, key)
    }

    /// The first served model `role.describe` lists.
    fn main_model(&self) -> Result<&ModelEntry, String> {
        let served: BTreeSet<String> = self
            .subject
            .served_models()
            .into_iter()
            .map(|served| served.model)
            .collect();
        self.describe
            .models
            .iter()
            .find(|entry| served.contains(&entry.model))
            .ok_or_else(|| "role.describe lists none of the models the catalog serves".to_owned())
    }

    /// The first priced served model, if any.
    fn priced_model(&self) -> Option<&ModelEntry> {
        let served: BTreeSet<String> = self
            .subject
            .served_models()
            .into_iter()
            .map(|served| served.model)
            .collect();
        self.describe
            .models
            .iter()
            .find(|entry| served.contains(&entry.model) && entry.is_priced())
    }

    async fn send(&self, params: &str) -> Result<Reply, String> {
        self.route
            .request(ops::CLASSIFY_RUN, params)
            .await
            .map_err(|failure| format!("the route failed: {}", failure.message))
    }

    async fn script_answer(&self, key: &str, body: String) {
        self.subject.script(key, Scripted::Answer(body)).await;
    }

    async fn calls(&self, key: &str) -> usize {
        self.subject.calls(key).await
    }

    /// The reply of an admitted call, with its exact text.
    fn reply_of(what: &str, reply: Reply) -> Result<(String, ClassifyReply), String> {
        match reply {
            Reply::Response(text) => {
                let decoded: ClassifyReply = serde_json::from_str(&text)
                    .map_err(|e| format!("{what}: the reply does not decode: {e}: {text}"))?;
                Ok((text, decoded))
            }
            Reply::Error(body) => Err(format!("{what}: refused {}: {}", body.code, body.message)),
        }
    }

    /// The refusal of a call expected to be refused.
    fn refusal_of(what: &str, reply: Reply) -> Result<Refusal, String> {
        match reply {
            Reply::Error(body) => {
                let value = serde_json::to_value(&body).expect("an error body encodes");
                serde_json::from_value(value).map_err(|e| {
                    format!(
                        "{what}: refused {} without a decodable detail carrying its class: {e}",
                        body.code
                    )
                })
            }
            Reply::Response(text) => {
                Err(format!("{what}: expected a refusal, got a reply: {text}"))
            }
        }
    }

    /// `refusal` has `code`, the class the contract gives it, and `field`
    /// when one is expected.
    fn expect_refusal(
        what: &str,
        refusal: &Refusal,
        code: &str,
        field: Option<&str>,
    ) -> Result<(), String> {
        if refusal.code != code {
            return Err(format!(
                "{what}: refused {} ({}), expected {code}",
                refusal.code, refusal
            ));
        }
        if let Some(class) = errors::admission_class(code) {
            if refusal.detail.class != class {
                return Err(format!(
                    "{what}: {code} has class {}, expected {class}",
                    refusal.detail.class
                ));
            }
        }
        if let Some(field) = field {
            if refusal.field() != Some(field) {
                return Err(format!(
                    "{what}: {code} names field {:?}, expected {field}",
                    refusal.field()
                ));
            }
        }
        Ok(())
    }

    /// Item `i`'s answers as a value, or why it has none.
    fn answers(what: &str, reply: &ClassifyReply, i: usize) -> Result<Value, String> {
        let item = reply
            .items
            .get(i)
            .ok_or_else(|| format!("{what}: the reply has no item {i}"))?;
        match (&item.answers, &item.error) {
            (Some(answers), _) => Ok(serde_json::to_value(answers).expect("answers encode")),
            (None, Some(error)) => Err(format!(
                "{what}: item {i} failed {} ({}): {}",
                error.code, error.class, error.message
            )),
            (None, None) => Err(format!("{what}: item {i} has neither answers nor error")),
        }
    }

    async fn expect_calls(&self, what: &str, key: &str, expected: usize) -> Result<(), String> {
        let calls = self.calls(key).await;
        if calls == expected {
            Ok(())
        } else {
            Err(format!(
                "{what}: the stand-in was called {calls} times for the item, expected {expected}"
            ))
        }
    }

    /// Checks that the reply's item at index `i` is an error with `code`, and
    /// with the class the contract gives that code.
    fn expect_item_code(
        what: &str,
        reply: &ClassifyReply,
        i: usize,
        code: &str,
    ) -> Result<(), String> {
        let item = reply
            .items
            .get(i)
            .ok_or_else(|| format!("{what}: the reply has no item {i}"))?;
        let error = match (&item.answers, &item.error) {
            (None, Some(error)) => error,
            (Some(_), _) => return Err(format!("{what}: item {i} was answered, expected {code}")),
            (None, None) => return Err(format!("{what}: item {i} has neither answers nor error")),
        };
        if error.code != code {
            return Err(format!(
                "{what}: item {i} failed {}, expected {code}",
                error.code
            ));
        }
        if let Some(class) = errors::item_class(code) {
            if error.class != class {
                return Err(format!(
                    "{what}: item {i}'s {code} has class {}, expected {class}",
                    error.class
                ));
            }
        }
        Ok(())
    }

    // ---- checks ---------------------------------------------------------

    async fn describe_complete(&self) -> Check {
        let text = match self.route.request(ops::ROLE_DESCRIBE, "{}").await {
            Ok(Reply::Response(text)) => text,
            Ok(Reply::Error(body)) => return Err(format!("refused {}", body.code)),
            Err(failure) => return Err(failure.message),
        };
        let describe: RoleDescribe =
            serde_json::from_str(&text).map_err(|e| format!("does not decode: {e}"))?;
        check_describe(&describe).map_err(|problem| format!("check_describe: {problem}"))?;
        if describe.role != PROVIDES {
            return Err(format!("role is {}", describe.role));
        }
        let served = self.subject.served_models();
        let listed: BTreeSet<&str> = describe.models.iter().map(|m| m.model.as_str()).collect();
        let expected: BTreeSet<&str> = served.iter().map(|m| m.model.as_str()).collect();
        if listed != expected {
            return Err(format!(
                "role.describe lists {listed:?}; the catalog serves {expected:?}"
            ));
        }
        let raw: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for model in &served {
            let entry = describe.model(&model.model).expect("listed above");
            let name = &model.model;
            if entry.context_tokens != model.context_tokens {
                return Err(format!(
                    "{name}: context_tokens {}, the catalog says {}",
                    entry.context_tokens, model.context_tokens
                ));
            }
            if entry.images.map(|images| images.max) != model.images_max {
                return Err(format!(
                    "{name}: images {:?}, the catalog allows {:?}",
                    entry.images, model.images_max
                ));
            }
            if entry.is_priced() != model.priced {
                return Err(format!(
                    "{name}: price {:?}, the catalog {} it",
                    entry.price,
                    if model.priced {
                        "prices"
                    } else {
                        "does not price"
                    }
                ));
            }
            if entry.egress_host != model.egress_host {
                return Err(format!(
                    "{name}: egress_host {}, the module sends to {}",
                    entry.egress_host, model.egress_host
                ));
            }
            let stated = raw["models"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|m| m["model"] == json!(name))
                .is_some_and(|m| m.get("images").is_some());
            if !stated {
                return Err(format!(
                    "{name}: images is omitted, not stated (null for none)"
                ));
            }
        }
        Ok(Ending::Passed)
    }

    async fn concurrent(&self) -> Check {
        if !self.subject.can_hold_calls() {
            return Ok(Ending::NotApplicable(
                "the stand-in cannot hold a call, so no call can be caught holding its batch"
                    .into(),
            ));
        }
        let model = self.main_model()?;
        let (a, ka) = self.marker("concurrent-a");
        let (b, kb) = self.marker("concurrent-b");
        self.script_answer(&ka, noul_body("0.61", None)).await;
        self.script_answer(&kb, noul_body("0.62", None)).await;
        self.subject.hold_next(&ka).await;
        let batch = self.mint("concurrent");
        let body = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&a, &b]),
            None,
        );
        let first = self.send(&body);
        let second = async {
            let mut held = false;
            for _ in 0..MAX_POLLS {
                if self.subject.is_held(&ka).await {
                    held = true;
                    break;
                }
                self.subject.pause().await;
            }
            if !held {
                self.subject.release(&ka).await;
                return Err("the stand-in never held the first item's call".to_owned());
            }
            let timeout = async {
                for _ in 0..MAX_POLLS {
                    self.subject.pause().await;
                }
            };
            let outcome = match select(Box::pin(self.send(&body)), Box::pin(timeout)).await {
                Either::Left((reply, _)) => Some(reply),
                Either::Right(_) => None,
            };
            self.subject.release(&ka).await;
            Ok(outcome)
        };
        let (first, second) = join(first, second).await;
        let second = second?.ok_or(
            "the second call did not answer while the first held the batch: it waited instead \
             of being refused batch_in_progress",
        )?;
        let refusal = Self::refusal_of("the second call", second?)?;
        Self::expect_refusal("the second call", &refusal, errors::BATCH_IN_PROGRESS, None)?;
        if refusal.detail.retry_after_ms.is_none() {
            return Err("batch_in_progress carries no detail.retry_after_ms".into());
        }
        let (_, first) = Self::reply_of("the first call", first?)?;
        let first_a = Self::answers("the first call", &first, 0)?;
        let first_b = Self::answers("the first call", &first, 1)?;
        self.expect_calls("item 0, after both calls", &ka, 1)
            .await?;
        self.expect_calls("item 1, after both calls", &kb, 1)
            .await?;
        let (_, again) = Self::reply_of("the re-send", self.send(&body).await?)?;
        if Self::answers("the re-send", &again, 0)? != first_a
            || Self::answers("the re-send", &again, 1)? != first_b
        {
            return Err("the re-send's answers differ from the first call's".into());
        }
        self.expect_calls("item 0, after the re-send", &ka, 1)
            .await?;
        self.expect_calls("item 1, after the re-send", &kb, 1)
            .await?;
        Ok(Ending::Passed)
    }

    async fn retry_only_unanswered(&self) -> Check {
        let model = self.main_model()?;
        let (a, ka) = self.marker("retry-a");
        let (b, kb) = self.marker("retry-b");
        self.script_answer(&ka, noul_body("0.31", None)).await;
        self.subject.script(&kb, Scripted::status(503)).await;
        let batch = self.mint("retry");
        let body = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&a, &b]),
            None,
        );
        let (_, first) = Self::reply_of("the first call", self.send(&body).await?)?;
        let first_a = Self::answers("the first call", &first, 0)?;
        if first.items.get(1).is_none_or(|item| item.error.is_none()) {
            return Err("the first call: item 1 carries no error for the stand-in's 503".into());
        }
        self.expect_calls("item 0, first call", &ka, 1).await?;
        let failed_calls = self.calls(&kb).await;
        self.script_answer(&kb, noul_body("0.32", None)).await;
        let (_, retry) = Self::reply_of("the retry", self.send(&body).await?)?;
        if Self::answers("the retry", &retry, 0)? != first_a {
            return Err("the retry's answer for item 0 differs from the stored one".into());
        }
        self.expect_calls("item 0, answered before the retry", &ka, 1)
            .await?;
        Self::answers("the retry", &retry, 1)?;
        self.expect_calls("item 1, unanswered before the retry", &kb, failed_calls + 1)
            .await?;
        Ok(Ending::Passed)
    }

    async fn stored_errors(&self) -> Check {
        let model = self.main_model()?;
        let (p, kp) = self.marker("stored-permanent");
        let (t, kt) = self.marker("stored-transient");
        self.subject.script(&kp, Scripted::status(400)).await;
        self.subject.script(&kt, Scripted::status(503)).await;
        let batch = self.mint("stored");
        let body = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&p, &t]),
            None,
        );
        let (_, first) = Self::reply_of("the first call", self.send(&body).await?)?;
        let stored_permanent = first
            .items
            .first()
            .and_then(|item| item.error.clone())
            .ok_or("the first call: item 0 carries no error for the stand-in's 400")?;
        if stored_permanent.class.effective() != ErrorClass::Permanent {
            return Err(format!(
                "the stand-in's 400 became {} of class {}, not a permanent error",
                stored_permanent.code, stored_permanent.class
            ));
        }
        let stored_transient = first
            .items
            .get(1)
            .and_then(|item| item.error.clone())
            .ok_or("the first call: item 1 carries no error for the stand-in's 503")?;
        if !stored_transient.class.is_transient() {
            return Err(format!(
                "the stand-in's 503 became {} of class {}, not a transient error",
                stored_transient.code, stored_transient.class
            ));
        }
        let permanent_calls = self.calls(&kp).await;
        let transient_calls = self.calls(&kt).await;
        self.script_answer(&kp, noul_body("0.41", None)).await;
        self.script_answer(&kt, noul_body("0.42", None)).await;
        let (_, again) = Self::reply_of("the re-send", self.send(&body).await?)?;
        let item = again.items.first().ok_or("the re-send has no item 0")?;
        if item.error.as_ref() != Some(&stored_permanent) || item.answers.is_some() {
            return Err(format!(
                "the stored permanent error was not returned as stored: item 0 is {}",
                serde_json::to_string(item).expect("an item encodes")
            ));
        }
        self.expect_calls("item 0, stored permanent error", &kp, permanent_calls)
            .await
            .map_err(|e| format!("{e}: the stored permanent error was retried"))?;
        Self::answers("the re-send", &again, 1)
            .map_err(|e| format!("{e}: the stored transient error was not retried"))?;
        self.expect_calls("item 1, stored transient error", &kt, transient_calls + 1)
            .await?;
        Ok(Ending::Passed)
    }

    async fn cost_stable(&self) -> Check {
        let Some(model) = self.priced_model() else {
            return Ok(Ending::NotApplicable(
                "the module serves no priced model, so no reply carries cost_usd".into(),
            ));
        };
        let usage = Some(r#"{"input_tokens":1000,"output_tokens":100}"#);
        let (a, ka) = self.marker("cost-a");
        let (b, kb) = self.marker("cost-b");
        self.script_answer(&ka, noul_body("0.51", usage)).await;
        self.script_answer(&kb, noul_body("0.52", usage)).await;
        let batch = self.mint("cost");
        let body = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&a, &b]),
            None,
        );
        let (first_text, _) = Self::reply_of("the first call", self.send(&body).await?)?;
        let (replay_text, _) = Self::reply_of("the replay", self.send(&body).await?)?;
        let cost = |text: &str| -> Result<Option<String>, String> {
            let raw: RawReplyText = serde_json::from_str(text).map_err(|e| e.to_string())?;
            Ok(raw.cost_usd.map(|cost| cost.get().to_owned()))
        };
        let first_cost =
            cost(&first_text)?.ok_or("the first reply on a priced model has no cost_usd")?;
        let replay_cost =
            cost(&replay_text)?.ok_or("the replay on a priced model has no cost_usd")?;
        if first_cost != replay_cost {
            return Err(format!(
                "cost_usd is {first_cost} on the first reply and {replay_cost} on the replay"
            ));
        }
        self.expect_calls("item 0, after the replay", &ka, 1)
            .await?;
        self.expect_calls("item 1, after the replay", &kb, 1)
            .await?;
        Ok(Ending::Passed)
    }

    /// Usage that costs at least `usd` at `model`'s price, as JSON text.
    fn usage_costing(model: &ModelEntry, usd: f64) -> Option<String> {
        let price = model.price.as_ref()?;
        let rate = |rate: &Option<serde_json::Number>| {
            rate.as_ref()
                .and_then(serde_json::Number::as_f64)
                .filter(|rate| *rate > 0.0)
        };
        if let Some(input) = rate(&price.input_per_mtok_usd) {
            let tokens = (usd * 1e6 / input).ceil() as u64 + 1;
            return Some(format!("{{\"input_tokens\":{tokens}}}"));
        }
        rate(&price.output_per_mtok_usd).map(|output| {
            let tokens = (usd * 1e6 / output).ceil() as u64 + 1;
            format!("{{\"output_tokens\":{tokens}}}")
        })
    }

    /// One ceiling check. Item 0 fails transiently and item 1 is answered
    /// at a cost of [`ITEM_SPEND_USD`]; item 0 comes first so the first
    /// call sends it while spend is still under any ceiling, leaving a
    /// stored transient error the re-send must retry. The re-send carries
    /// `resend_ceiling`. With a first ceiling of [`CEILING_USD`] and
    /// a looser re-send, the recorded ceiling must hold; with a loose first
    /// ceiling and a re-send of [`CEILING_USD`], the tighter one must. In
    /// both, recorded spend is over the effective ceiling, so item 0 must
    /// get `cost_exceeded` and no call.
    async fn ceiling(&self, resend_ceiling: f64) -> Check {
        let Some(model) = self.priced_model() else {
            return Ok(Ending::NotApplicable(
                "the module serves no priced model, so nothing it answers has a cost to bound"
                    .into(),
            ));
        };
        let Some(usage) = Self::usage_costing(model, ITEM_SPEND_USD) else {
            return Ok(Ending::NotApplicable(format!(
                "{} has no non-zero price, so no usage crosses a ceiling",
                model.model
            )));
        };
        let first_ceiling = if resend_ceiling > CEILING_USD {
            CEILING_USD
        } else {
            LOOSE_CEILING_USD
        };
        let (b, kb) = self.marker("ceiling-0-transient");
        let (a, ka) = self.marker("ceiling-1-spends");
        self.script_answer(&ka, noul_body("0.71", Some(&usage)))
            .await;
        self.subject.script(&kb, Scripted::status(503)).await;
        let batch = self.mint("ceiling");
        let first_body = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&b, &a]),
            Some(first_ceiling),
        );
        let (_, first) = Self::reply_of(
            &format!("the first call, with max_cost_usd {first_ceiling}"),
            self.send(&first_body).await?,
        )?;
        Self::answers("the first call", &first, 1)?;
        if first.items.first().is_none_or(|item| item.error.is_none()) {
            return Err("the first call: item 0 carries no error for the stand-in's 503".into());
        }
        let item_calls = self.calls(&kb).await;
        self.script_answer(&kb, noul_body("0.72", None)).await;
        let resend = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&b, &a]),
            Some(resend_ceiling),
        );
        let what = format!(
            "a re-send with max_cost_usd {resend_ceiling} after spending {ITEM_SPEND_USD} under {first_ceiling}"
        );
        match self.send(&resend).await? {
            Reply::Error(body) if body.code == errors::COST_EXCEEDED => {}
            Reply::Error(body) => {
                return Err(format!("{what}: refused {}: {}", body.code, body.message))
            }
            Reply::Response(text) => {
                let (_, reply) = Self::reply_of(&what, Reply::Response(text))?;
                let item = reply.items.first().ok_or(format!("{what}: no item 0"))?;
                match (&item.answers, &item.error) {
                    (Some(_), _) => {
                        return Err(format!(
                            "{what}: item 0 was answered, so the ceiling was {}",
                            if resend_ceiling > first_ceiling {
                                "raised by the looser value"
                            } else {
                                "not tightened by the lower value"
                            }
                        ))
                    }
                    (None, Some(error)) if error.code == errors::COST_EXCEEDED => {}
                    (None, Some(error)) => {
                        return Err(format!(
                            "{what}: item 0 failed {}, expected cost_exceeded",
                            error.code
                        ))
                    }
                    (None, None) => return Err(format!("{what}: item 0 is empty")),
                }
            }
        }
        self.expect_calls(&format!("{what}, item 0"), &kb, item_calls)
            .await?;
        Ok(Ending::Passed)
    }

    async fn reuse(&self) -> Check {
        let model = self.main_model()?;
        let (a, ka) = self.marker("reuse-a");
        let (b, kb) = self.marker("reuse-b");
        let (c, kc) = self.marker("reuse-c");
        let (d, kd) = self.marker("reuse-d");
        for key in [&ka, &kb, &kc, &kd] {
            self.script_answer(key, noul_body("0.81", None)).await;
        }
        let batch = self.mint("reuse");
        let original = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&a, &b]),
            None,
        );
        Self::reply_of("the first call", self.send(&original).await?)?;
        let other_questions =
            json!({"q": {"type": "noul", "instructions": "Is this item flagged, really?"}});
        let mut variants = vec![
            (
                "a changed item 1",
                params(
                    &batch,
                    &model.model,
                    &noul_question(),
                    items(&[&a, &c]),
                    None,
                ),
                "items[1]",
            ),
            (
                "an added item",
                params(
                    &batch,
                    &model.model,
                    &noul_question(),
                    items(&[&a, &b, &d]),
                    None,
                ),
                "items",
            ),
            (
                "changed questions",
                params(
                    &batch,
                    &model.model,
                    &other_questions,
                    items(&[&a, &b]),
                    None,
                ),
                "questions",
            ),
        ];
        let served: BTreeSet<String> = self
            .subject
            .served_models()
            .into_iter()
            .map(|served| served.model)
            .collect();
        if let Some(other) = self
            .describe
            .models
            .iter()
            .find(|entry| entry.model != model.model && served.contains(&entry.model))
        {
            variants.push((
                "another model",
                params(
                    &batch,
                    &other.model,
                    &noul_question(),
                    items(&[&a, &b]),
                    None,
                ),
                "model",
            ));
        }
        for (what, body, field) in variants {
            let what = format!("a re-send with {what}");
            let refusal = Self::refusal_of(&what, self.send(&body).await?)?;
            Self::expect_refusal(&what, &refusal, errors::BATCH_ID_REUSE, Some(field))?;
        }
        self.expect_calls("the changed item", &kc, 0).await?;
        self.expect_calls("the added item", &kd, 0).await?;
        Self::reply_of(
            "the original body after the refused reuses",
            self.send(&original).await?,
        )?;
        self.expect_calls("item 0", &ka, 1).await?;
        self.expect_calls("item 1", &kb, 1).await?;
        Ok(Ending::Passed)
    }

    async fn reordered_state(&self) -> Check {
        let model = self.main_model()?;
        let marker = string(&self.mint("reordered"));
        let first_state = format!(
            "{{\"marker\":{marker},\"alpha\":1,\"nested\":{{\"y\":[1,2.5],\"x\":\"v\"}},\"big\":1e30}}"
        );
        let second_state = format!(
            "{{\"nested\":{{\"x\":\"v\",\"y\":[1,2.5]}},\"big\":1e30,\"marker\":{marker},\"alpha\":1}}"
        );
        let key = state_key(&serde_json::from_str(&first_state).expect("the state is JSON"));
        self.script_answer(&key, noul_body("0.91", None)).await;
        let batch = string(&self.mint("reordered"));
        let model_id = string(&model.model);
        let question = noul_question().to_string();
        let body = |state: &str| {
            format!(
                "{{\"batch_id\":{batch},\"model\":{model_id},\"questions\":{question},\"items\":[{{\"state\":{state}}}]}}"
            )
        };
        let (_, first) = Self::reply_of("the first call", self.send(&body(&first_state)).await?)?;
        let answers = Self::answers("the first call", &first, 0)?;
        self.expect_calls("the item, first call", &key, 1).await?;
        let reply = self.send(&body(&second_state)).await?;
        if let Reply::Error(error) = &reply {
            if error.code == errors::BATCH_ID_REUSE {
                return Err(format!(
                    "the re-send with the object state's keys reordered was refused \
                     batch_id_reuse ({}): key order changed the body identity",
                    error.message
                ));
            }
        }
        let (_, again) = Self::reply_of("the reordered re-send", reply)?;
        if Self::answers("the reordered re-send", &again, 0)? != answers {
            return Err("the reordered re-send's answers differ from the first call's".into());
        }
        self.expect_calls("the item, reordered re-send", &key, 1)
            .await?;
        Ok(Ending::Passed)
    }

    async fn validation(&self) -> Check {
        let model = self.main_model()?;
        struct Case {
            what: String,
            batch: String,
            body: String,
            code: &'static str,
            field: Option<String>,
            keys: Vec<String>,
            follow_up: bool,
            model: Option<String>,
        }
        let mut cases = Vec::new();
        let question = noul_question();
        let case = |what: &str,
                    body: &dyn Fn(&str, &str) -> String,
                    state: &str,
                    code: &'static str,
                    field: Option<&str>|
         -> Case {
            let batch = self.mint("invalid");
            let key = state_key(&Value::String(state.to_owned()));
            Case {
                what: what.to_owned(),
                body: body(&batch, state),
                batch,
                code,
                field: field.map(str::to_owned),
                keys: vec![key],
                follow_up: true,
                model: None,
            }
        };

        let (state, _) = self.marker("invalid-model");
        let unknown = format!("no-such-provider/{}", self.run);
        cases.push(case(
            "an unknown model",
            &|batch, state| params(batch, &unknown, &question, items(&[state]), None),
            &state,
            errors::MODEL_UNKNOWN,
            None,
        ));
        if let Some(chat) = self.subject.non_classifier_model() {
            let (state, _) = self.marker("invalid-kind");
            let mut refused = case(
                "a model whose row is not a classifier",
                &|batch, state| params(batch, &chat, &question, items(&[state]), None),
                &state,
                errors::MODEL_NOT_CLASSIFIER,
                None,
            );
            refused.model = Some(chat.clone());
            cases.push(refused);
        }
        if model.max_questions < 10_000 {
            let many: serde_json::Map<String, Value> = (0..=model.max_questions)
                .map(|i| (format!("q{i}"), question["q"].clone()))
                .collect();
            let many = Value::Object(many);
            let (state, _) = self.marker("invalid-questions");
            cases.push(case(
                "one question over max_questions",
                &|batch, state| params(batch, &model.model, &many, items(&[state]), None),
                &state,
                errors::INVALID_PARAMS,
                Some("questions"),
            ));
        }
        let bad_id = json!({"Bad": question["q"].clone()});
        let (state, _) = self.marker("invalid-id");
        cases.push(case(
            "an upper-case question id",
            &|batch, state| params(batch, &model.model, &bad_id, items(&[state]), None),
            &state,
            errors::INVALID_PARAMS,
            Some("questions.Bad"),
        ));
        let one_option = json!({"pick": {"type": "choice", "instructions": "Which one?", "criteria": {"only": "The only option"}}});
        let (state, _) = self.marker("invalid-choice");
        cases.push(case(
            "a choice with one option",
            &|batch, state| params(batch, &model.model, &one_option, items(&[state]), None),
            &state,
            errors::INVALID_PARAMS,
            Some("questions.pick.criteria"),
        ));
        let levels: Vec<String> = (0..11).map(|i| format!("level {i}")).collect();
        let eleven =
            json!({"level": {"type": "score", "instructions": "How much?", "criteria": levels}});
        let (state, _) = self.marker("invalid-score");
        cases.push(case(
            "a score with eleven levels",
            &|batch, state| params(batch, &model.model, &eleven, items(&[state]), None),
            &state,
            errors::INVALID_PARAMS,
            Some("questions.level.criteria"),
        ));
        let (state, _) = self.marker("invalid-number");
        cases.push(case(
            "a number state",
            &|batch, state| {
                let mut list = items(&[state]);
                list.push(json!({"state": 7}));
                params(batch, &model.model, &question, list, None)
            },
            &state,
            errors::INVALID_PARAMS,
            Some("items[1].state"),
        ));
        let served: BTreeSet<String> = self
            .subject
            .served_models()
            .into_iter()
            .map(|served| served.model)
            .collect();
        let image_model = self
            .describe
            .models
            .iter()
            .find(|entry| served.contains(&entry.model) && entry.images.is_none())
            .unwrap_or(model);
        let image_count = image_model.images.map_or(1, |images| images.max + 1);
        if image_count <= 64 {
            let (state, _) = self.marker("invalid-images");
            let images: Vec<&str> = (0..image_count).map(|_| PIXEL).collect();
            cases.push(case(
                "images beyond the model's support",
                &|batch, state| {
                    params(
                        batch,
                        &image_model.model,
                        &question,
                        vec![json!({"state": state, "images": images})],
                        None,
                    )
                },
                &state,
                errors::INVALID_PARAMS,
                Some("items[0].images"),
            ));
        }
        let limits = self.describe.limits;
        if model.max_items < 100_000 && (model.max_items + 1) * 128 < limits.max_request_bytes {
            let states: Vec<String> = (0..=model.max_items)
                .map(|i| self.mint(&format!("invalid-items-{i}")))
                .collect();
            let refs: Vec<&str> = states.iter().map(String::as_str).collect();
            let batch = self.mint("invalid");
            cases.push(Case {
                what: "one item over max_items".into(),
                body: params(&batch, &model.model, &question, items(&refs), None),
                batch,
                code: errors::INVALID_PARAMS,
                field: Some("items".into()),
                keys: states
                    .iter()
                    .map(|state| state_key(&Value::String(state.clone())))
                    .collect(),
                follow_up: true,
                model: None,
            });
        }
        if limits.max_request_bytes < OVERSIZE_CAP {
            let state = format!(
                "{}-{}",
                self.mint("invalid-size"),
                "x".repeat(limits.max_request_bytes as usize + 1)
            );
            let batch = self.mint("invalid");
            cases.push(Case {
                what: "a request over max_request_bytes".into(),
                body: params(&batch, &model.model, &question, items(&[&state]), None),
                batch,
                code: errors::INVALID_PARAMS,
                field: Some("params".into()),
                keys: vec![state_key(&Value::String(state))],
                follow_up: true,
                model: None,
            });
        }
        let (state, key) = self.marker("invalid-batch-id");
        cases.push(Case {
            what: "a batch_id with a tab".into(),
            batch: "bad\tid".into(),
            body: params("bad\tid", &model.model, &question, items(&[&state]), None),
            code: errors::INVALID_PARAMS,
            field: Some("batch_id".into()),
            keys: vec![key],
            follow_up: false,
            model: None,
        });

        for case in &cases {
            for key in &case.keys {
                self.script_answer(key, noul_body("0.5", None)).await;
            }
        }
        for case in cases {
            let what = case.what.as_str();
            let refusal = Self::refusal_of(what, self.send(&case.body).await?)?;
            Self::expect_refusal(what, &refusal, case.code, case.field.as_deref())?;
            if let Some(model) = &case.model {
                if refusal.detail.model.as_deref() != Some(model.as_str())
                    || refusal
                        .detail
                        .kind
                        .as_deref()
                        .is_none_or(|kind| kind == "classifier")
                {
                    return Err(format!(
                        "{what}: model_not_classifier names model {:?} and kind {:?}",
                        refusal.detail.model, refusal.detail.kind
                    ));
                }
            }
            for key in &case.keys {
                self.expect_calls(what, key, 0)
                    .await
                    .map_err(|e| format!("{e}: the refusal reached the provider"))?;
            }
            if case.follow_up {
                let (state, key) = self.marker("valid-after-refusal");
                self.script_answer(&key, noul_body("0.5", None)).await;
                let body = params(&case.batch, &model.model, &question, items(&[&state]), None);
                Self::reply_of(
                    &format!("a valid body with the batch_id refused for {what}"),
                    self.send(&body).await?,
                )
                .map_err(|e| format!("{e}: the refused request was recorded"))?;
            }
        }
        Ok(Ending::Passed)
    }

    async fn null_state(&self) -> Check {
        let model = self.main_model()?;
        let (a, ka) = self.marker("null-a");
        self.script_answer(&ka, noul_body("0.5", None)).await;
        let batch = self.mint("null");
        let body = params(
            &batch,
            &model.model,
            &noul_question(),
            vec![json!({"state": a}), json!({"state": null})],
            None,
        );
        let what = "an item with a null state";
        let refusal = Self::refusal_of(what, self.send(&body).await?)?;
        Self::expect_refusal(
            what,
            &refusal,
            errors::INVALID_PARAMS,
            Some("items[1].state"),
        )?;
        self.expect_calls("the valid item beside the null state", &ka, 0)
            .await?;
        let (state, key) = self.marker("null-follow-up");
        self.script_answer(&key, noul_body("0.5", None)).await;
        let valid = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&state]),
            None,
        );
        Self::reply_of(
            "a valid body with the refused batch_id",
            self.send(&valid).await?,
        )
        .map_err(|e| format!("{e}: the refused request was recorded"))?;
        Ok(Ending::Passed)
    }

    async fn request_order(&self) -> Check {
        let model = self.main_model()?;
        // Minted in reverse, so the states sort in the opposite order to
        // the request's: a module that orders by anything but position
        // shows up.
        let mut markers: Vec<(String, String)> =
            (0..5).map(|i| self.marker(&format!("order-{i}"))).collect();
        markers.reverse();
        for (i, (_, key)) in markers.iter().enumerate() {
            self.script_answer(key, noul_body(&format!("0.1{i}"), None))
                .await;
        }
        let states: Vec<&str> = markers.iter().map(|(state, _)| state.as_str()).collect();
        let batch = self.mint("order");
        let body = params(&batch, &model.model, &noul_question(), items(&states), None);
        let (_, reply) = Self::reply_of("the call", self.send(&body).await?)?;
        if reply.items.len() != states.len() {
            return Err(format!("{} items answer 5", reply.items.len()));
        }
        for (i, item) in reply.items.iter().enumerate() {
            if item.index != i as u64 {
                return Err(format!("position {i} holds index {}", item.index));
            }
            let answers = Self::answers("the call", &reply, i)?;
            let expected: f64 = format!("0.1{i}").parse().expect("a number");
            if answers["q"]["noul"].as_f64() != Some(expected) {
                return Err(format!(
                    "position {i} holds the answer {} where item {i}'s is {expected}",
                    answers["q"]["noul"]
                ));
            }
        }
        Ok(Ending::Passed)
    }

    async fn numbers(&self) -> Check {
        let model = self.main_model()?;
        let questions = json!({
            "n": {"type": "noul", "instructions": "Is it flagged?"},
            "c": {"type": "choice", "instructions": "Which?", "criteria": {"x": "Ex", "y": "Why"}},
            "s": {"type": "score", "instructions": "How much?", "criteria": ["low", "mid", "high"]},
        });
        let answers = r#"{"n":{"type":"noul","noul":0.1234567890123456789},"c":{"type":"choice","choice":"x","probabilities":{"x":0.70,"y":3e-1},"confidence":1},"s":{"type":"score","score":1.50,"legend":{"0":"low","1":"mid","2":"high"},"probabilities":{"0":0.0,"1":0.5000,"2":5E-1},"confidence":0.999999999999999999}}"#;
        let mut expected = Vec::new();
        number_leaves(answers, "answers", &mut expected)?;
        expected.sort();
        let (state, key) = self.marker("numbers");
        self.script_answer(
            &key,
            format!("{{\"model\":\"stand-in\",\"answers\":{answers}}}"),
        )
        .await;
        let batch = self.mint("numbers");
        let body = params(&batch, &model.model, &questions, items(&[&state]), None);
        for what in ["the first reply", "the replay"] {
            let (text, _) = Self::reply_of(what, self.send(&body).await?)?;
            let raw: RawReplyText = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let answered = raw
                .items
                .first()
                .and_then(|item| item.answers.as_ref())
                .ok_or(format!("{what}: item 0 has no answers"))?;
            let mut got = Vec::new();
            number_leaves(answered.get(), "answers", &mut got)?;
            got.sort();
            if got != expected {
                let changed: Vec<String> = expected
                    .iter()
                    .filter(|leaf| !got.contains(leaf))
                    .map(|(path, text)| {
                        let now = got
                            .iter()
                            .find(|(other, _)| other == path)
                            .map_or("absent", |(_, text)| text.as_str());
                        format!("{path}: {text} became {now}")
                    })
                    .collect();
                return Err(format!(
                    "{what}: provider numbers were rewritten: {}",
                    changed.join(", ")
                ));
            }
        }
        Ok(Ending::Passed)
    }

    async fn one_failing_item(&self) -> Check {
        let model = self.main_model()?;
        let (a, ka) = self.marker("failing-a");
        let (b, kb) = self.marker("failing-b");
        let (c, kc) = self.marker("failing-c");
        self.script_answer(&ka, noul_body("0.21", None)).await;
        self.subject.script(&kb, Scripted::status(400)).await;
        self.script_answer(&kc, noul_body("0.23", None)).await;
        let batch = self.mint("failing");
        let body = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&a, &b, &c]),
            None,
        );
        let (_, reply) = Self::reply_of(
            "a batch whose middle item the provider rejects",
            self.send(&body).await?,
        )?;
        Self::answers("item 0", &reply, 0)?;
        Self::answers("item 2", &reply, 2)?;
        let item = reply.items.get(1).ok_or("the reply has no item 1")?;
        match (&item.answers, &item.error) {
            (None, Some(error)) if error.class.effective() == ErrorClass::Permanent => {}
            (None, Some(error)) => {
                return Err(format!(
                    "item 1's 400 became {} of class {}, not a permanent error",
                    error.code, error.class
                ))
            }
            _ => return Err("item 1 carries answers for the provider's 400".into()),
        }
        self.expect_calls("item 2", &kc, 1).await?;
        Ok(Ending::Passed)
    }

    /// Four items; the stand-in answers item 0 and answers item 1 with
    /// `status`, whose `code` stops the call. Items 2 and 3 must not be
    /// sent and must carry `code`; item 0 keeps its answer. Once the
    /// stand-in accepts, a re-send must ask again for exactly items 1 to 3:
    /// the code left them unanswered, never stored as a permanent error.
    async fn stopping_status(&self, status: u16, code: &str, label: &str) -> Check {
        let model = self.main_model()?;
        let markers: Vec<(String, String)> = (0..4)
            .map(|i| self.marker(&format!("{label}-{i}")))
            .collect();
        let keys: Vec<&str> = markers.iter().map(|(_, key)| key.as_str()).collect();
        let states: Vec<&str> = markers.iter().map(|(state, _)| state.as_str()).collect();
        self.script_answer(keys[0], noul_body("0.61", None)).await;
        self.subject.script(keys[1], Scripted::status(status)).await;
        self.script_answer(keys[2], noul_body("0.63", None)).await;
        self.script_answer(keys[3], noul_body("0.64", None)).await;
        let batch = self.mint(label);
        let body = params(&batch, &model.model, &noul_question(), items(&states), None);
        let what = format!("a batch whose item 1 the stand-in answers {status}");
        let (_, first) = Self::reply_of(&what, self.send(&body).await?)?;
        let first_a = Self::answers(&what, &first, 0)?;
        for i in 1..4 {
            Self::expect_item_code(&what, &first, i, code)?;
        }
        self.expect_calls(&format!("item 0, before the {status}"), keys[0], 1)
            .await?;
        for (i, key) in keys.iter().enumerate().skip(2) {
            self.expect_calls(&format!("item {i}, after item 1 met {status}"), key, 0)
                .await
                .map_err(|e| format!("{e}: the call went on sending after {code}"))?;
        }
        let failed_calls = self.calls(keys[1]).await;
        self.script_answer(keys[1], noul_body("0.62", None)).await;
        let what = "the re-send after the stand-in accepts";
        let (_, again) = Self::reply_of(what, self.send(&body).await?)?;
        if Self::answers(what, &again, 0)? != first_a {
            return Err(format!(
                "{what}: item 0's answer differs from the stored one"
            ));
        }
        for i in 1..4 {
            Self::answers(what, &again, i)
                .map_err(|e| format!("{e}: the item {code} left unanswered was not asked again"))?;
        }
        self.expect_calls("item 0, answered before the stop", keys[0], 1)
            .await?;
        self.expect_calls("item 1, on the re-send", keys[1], failed_calls + 1)
            .await?;
        for (i, key) in keys.iter().enumerate().skip(2) {
            self.expect_calls(&format!("item {i}, on the re-send"), key, 1)
                .await?;
        }
        Ok(Ending::Passed)
    }

    /// Two items; the stand-in answers the first provider call with
    /// `status`. The whole call must be refused `code` at admission, item 1
    /// must not be sent, and nothing may be recorded: once the stand-in
    /// accepts, the same body is admitted and both items are asked.
    async fn first_call_refused(
        &self,
        status: u16,
        code: &str,
        label: &str,
    ) -> Result<Refusal, String> {
        let model = self.main_model()?;
        let (a, ka) = self.marker(&format!("{label}-0"));
        let (b, kb) = self.marker(&format!("{label}-1"));
        self.subject.script(&ka, Scripted::status(status)).await;
        self.script_answer(&kb, noul_body("0.66", None)).await;
        let batch = self.mint(label);
        let body = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&a, &b]),
            None,
        );
        let what = format!("a call whose first provider call the stand-in answers {status}");
        let refusal = Self::refusal_of(&what, self.send(&body).await?)?;
        Self::expect_refusal(&what, &refusal, code, None)?;
        self.expect_calls("item 1, after the refused first call", &kb, 0)
            .await?;
        let failed_calls = self.calls(&ka).await;
        self.script_answer(&ka, noul_body("0.65", None)).await;
        let what = "the re-send after the stand-in accepts";
        let (_, again) = Self::reply_of(what, self.send(&body).await?)
            .map_err(|e| format!("{e}: the refused call was recorded"))?;
        Self::answers(what, &again, 0)?;
        Self::answers(what, &again, 1)?;
        self.expect_calls("item 0, on the re-send", &ka, failed_calls + 1)
            .await?;
        self.expect_calls("item 1, on the re-send", &kb, 1).await?;
        Ok(refusal)
    }

    async fn auth_first_call(&self) -> Check {
        self.first_call_refused(401, errors::AUTH_FAILED, "auth-first")
            .await?;
        Ok(Ending::Passed)
    }

    async fn model_unavailable(&self) -> Check {
        self.stopping_status(404, errors::MODEL_UNAVAILABLE, "missing")
            .await?;
        let model = self.main_model()?;
        let refusal = self
            .first_call_refused(404, errors::MODEL_UNAVAILABLE, "missing-first")
            .await?;
        if refusal.detail.model.as_deref() != Some(model.model.as_str()) {
            return Err(format!(
                "model_unavailable names model {:?}, expected {}",
                refusal.detail.model, model.model
            ));
        }
        Ok(Ending::Passed)
    }

    async fn unknown_type(&self) -> Check {
        let model = self.main_model()?;
        let (a, ka) = self.marker("unknown-type");
        self.script_answer(&ka, noul_body("0.5", None)).await;
        let questions = json!({
            "q": {"type": "noul", "instructions": "Is this item flagged?"},
            "ranked": {"type": "rank", "instructions": "Rank this item among the others."},
        });
        let batch = self.mint("unknown-type");
        let body = params(&batch, &model.model, &questions, items(&[&a]), None);
        let what = "a request with a question of type rank";
        let refusal = Self::refusal_of(what, self.send(&body).await?)?;
        Self::expect_refusal(
            what,
            &refusal,
            errors::INVALID_PARAMS,
            Some("questions.ranked.type"),
        )?;
        self.expect_calls(what, &ka, 0)
            .await
            .map_err(|e| format!("{e}: the refusal reached the provider"))?;
        let (state, key) = self.marker("unknown-type-follow-up");
        self.script_answer(&key, noul_body("0.5", None)).await;
        let valid = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&state]),
            None,
        );
        Self::reply_of(
            "a valid body with the refused batch_id",
            self.send(&valid).await?,
        )
        .map_err(|e| format!("{e}: the refused request was recorded"))?;
        Ok(Ending::Passed)
    }

    /// High, then lower, then the original high again. Both items fail
    /// transiently on the first two calls, so nothing is spent and nothing
    /// is stored permanently. On the third, item 0 is answered at a cost of
    /// [`ITEM_SPEND_USD`], over the [`CEILING_USD`] the second call
    /// carried; if that lower ceiling was recorded, item 1 must get
    /// `cost_exceeded` and no call.
    async fn ceiling_recorded(&self) -> Check {
        let Some(model) = self.priced_model() else {
            return Ok(Ending::NotApplicable(
                "the module serves no priced model, so nothing it answers has a cost to bound"
                    .into(),
            ));
        };
        let Some(usage) = Self::usage_costing(model, ITEM_SPEND_USD) else {
            return Ok(Ending::NotApplicable(format!(
                "{} has no non-zero price, so no usage crosses a ceiling",
                model.model
            )));
        };
        let (a, ka) = self.marker("recorded-0-spends");
        let (x, kx) = self.marker("recorded-1-crosses");
        self.subject.script(&ka, Scripted::status(503)).await;
        self.subject.script(&kx, Scripted::status(503)).await;
        let batch = self.mint("recorded");
        let body = |max: f64| {
            params(
                &batch,
                &model.model,
                &noul_question(),
                items(&[&a, &x]),
                Some(max),
            )
        };
        for (n, max) in [(1, LOOSE_CEILING_USD), (2, CEILING_USD)] {
            let what = format!("call {n}, with max_cost_usd {max}");
            let (_, reply) = Self::reply_of(&what, self.send(&body(max)).await?)?;
            for i in 0..2 {
                if reply.items.get(i).is_none_or(|item| item.error.is_none()) {
                    return Err(format!(
                        "{what}: item {i} carries no error for the stand-in's 503"
                    ));
                }
            }
        }
        let crossing_calls = self.calls(&kx).await;
        self.script_answer(&ka, noul_body("0.73", Some(&usage)))
            .await;
        self.script_answer(&kx, noul_body("0.74", None)).await;
        let what = format!(
            "the third call, carrying the first call's max_cost_usd {LOOSE_CEILING_USD} after the second lowered it to {CEILING_USD}"
        );
        let (_, third) = Self::reply_of(&what, self.send(&body(LOOSE_CEILING_USD)).await?)?;
        Self::answers(&what, &third, 0)?;
        let item = third.items.get(1).ok_or(format!("{what}: no item 1"))?;
        match (&item.answers, &item.error) {
            (Some(_), _) => {
                return Err(format!(
                    "{what}: item 1 was answered, so the ceiling the second call lowered was loosened again"
                ))
            }
            (None, Some(error)) if error.code == errors::COST_EXCEEDED => {}
            (None, Some(error)) => {
                return Err(format!(
                    "{what}: item 1 failed {}, expected cost_exceeded",
                    error.code
                ))
            }
            (None, None) => return Err(format!("{what}: item 1 is empty")),
        }
        self.expect_calls(&format!("{what}, item 1"), &kx, crossing_calls)
            .await?;
        Ok(Ending::Passed)
    }

    async fn unreported_usage(&self) -> Check {
        let model = self.main_model()?;
        let (a, ka) = self.marker("usage-a");
        let (b, kb) = self.marker("usage-b");
        self.script_answer(&ka, noul_body("0.11", None)).await;
        self.script_answer(&kb, noul_body("0.12", Some(r#"{"input_tokens":7}"#)))
            .await;
        let batch = self.mint("usage");
        let body = params(
            &batch,
            &model.model,
            &noul_question(),
            items(&[&a, &b]),
            None,
        );
        let (text, _) = Self::reply_of("the call", self.send(&body).await?)?;
        let reply: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let counts = |usage: &Value| -> Vec<String> {
            ["input_tokens", "output_tokens"]
                .iter()
                .filter(|count| usage.get(**count).is_some())
                .map(|count| format!("{count}: {}", usage[*count]))
                .collect()
        };
        let first = &reply["items"][0]["usage"];
        if !counts(first).is_empty() {
            return Err(format!(
                "item 0's provider reported no usage, but its usage reads {}",
                counts(first).join(", ")
            ));
        }
        let second = &reply["items"][1]["usage"];
        if second["input_tokens"] != json!(7) || second.get("output_tokens").is_some() {
            return Err(format!(
                "item 1's provider reported input_tokens 7 only, but its usage reads {second}"
            ));
        }
        let batch_usage = &reply["usage"];
        if batch_usage["input_tokens"] != json!(7) || batch_usage.get("output_tokens").is_some() {
            return Err(format!(
                "the batch usage should sum input_tokens to 7 and leave output_tokens absent, \
                 but reads {batch_usage}"
            ));
        }
        Ok(Ending::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_leaves_keep_spelling() {
        let mut leaves = Vec::new();
        number_leaves(
            r#"{"a":0.50,"b":[1,2e-3],"c":"0.5","d":{"e":-0.0}}"#,
            "x",
            &mut leaves,
        )
        .unwrap();
        assert_eq!(
            leaves,
            [
                ("x.a".to_owned(), "0.50".to_owned()),
                ("x.b[0]".to_owned(), "1".to_owned()),
                ("x.b[1]".to_owned(), "2e-3".to_owned()),
                ("x.d.e".to_owned(), "-0.0".to_owned()),
            ]
        );
    }

    #[test]
    fn noul_body_is_a_provider_response() {
        let body = noul_body("0.5", Some(r#"{"input_tokens":3}"#));
        let decoded: cortexkit_role_classifier::provider::ProviderResponse =
            serde_json::from_str(&body).unwrap();
        assert_eq!(decoded.usage.unwrap().input_tokens, Some(3));
    }
}
