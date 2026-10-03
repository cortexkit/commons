//! A reference in-process classifier module, used ONLY to test the suite.
//!
//! Conformance runs against real modules. This fake exists so the suite's
//! own tests can show that each check passes a module that keeps the
//! contract and fails one that breaks it. The property it models:
//!
//! > A module that admits a batch after validating it completely, records
//! > its body identity and spend ceiling (tightened by any lower re-send),
//! > answers its items in order one provider call at a time, records each
//! > item's outcome, stops the call at an auth failure, a missing model or a
//! > rate limit (refusing the whole call when the first provider call meets
//! > a 401, 403 or 404, and never storing the first two), and on a re-send
//! > replays stored answers and permanent errors, retries transient errors,
//! > and stops any item whose call would cross the effective ceiling.
//!
//! Its provider is a stand-in that counts calls per item and answers with
//! the suite's script. Everything it knows reaches the suite only as
//! replies on a route.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use cortexkit_role_classifier_conformance::{
    state_key,
    wire::{
        describe::{ImageSupport, Limits, ModelEntry, Price, RoleDescribe},
        errors::{self, ItemError, Refusal},
        identity::BatchIdentity,
        ops,
        provider::item_code_for_status,
        question::{Criteria, QuestionType, SCORE_MAX_LEVELS},
        reply::{Answer, Usage},
        request::{
            check_catalog_row, check_request, check_request_bytes, effective_ceiling,
            parse_request, ClassifyRequest, CLASSIFIER_KIND,
        },
        IndexMap,
    },
    ClassifierRoute, ClassifierSubject, Reply, RouteFailure, Scripted, ServedModel,
};
use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Number, Value};
use subc_protocol::ErrorBody;

pub const PRICED: &str = "fake/priced";
pub const TEXT_ONLY: &str = "fake/text";
pub const CHAT: &str = "fake/chat";
const EGRESS: &str = "stand-in.invalid";
const MAX_ITEMS: u64 = 20;
const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const PRICED_CONTEXT: u64 = 8192;
const TEXT_CONTEXT: u64 = 4096;

/// Deliberate contract breaks: each field makes the fake violate one rule a
/// check must catch.
#[derive(Clone, Copy, Debug, Default)]
pub struct Defects {
    /// role.describe states a context of 1 token for the priced model
    /// instead of the catalog's.
    pub describe_wrong_context_tokens: bool,
    /// role.describe leaves out the text-only model.
    pub describe_omits_model: bool,
    /// A second call for a batch in flight runs alongside the first instead
    /// of being refused batch_in_progress.
    pub no_single_flight: bool,
    /// A second call for a batch in flight waits for the first to end
    /// instead of being refused batch_in_progress.
    pub wait_for_in_flight: bool,
    /// A re-send asks the provider again for items that already have an
    /// answer.
    pub reask_answered_on_retry: bool,
    /// A re-send retries stored permanent errors too.
    pub retry_stored_permanent: bool,
    /// A re-send replays stored transient errors instead of retrying them.
    pub replay_stored_transient: bool,
    /// cost_usd covers only the items this call answered, so a replay
    /// reports 0.
    pub cost_per_call: bool,
    /// The ceiling is the re-send's max_cost_usd alone, so a higher value
    /// raises it.
    pub ceiling_from_latest_call: bool,
    /// The ceiling is the first admission's alone, so a lower value on a
    /// re-send does not tighten it.
    pub ceiling_from_first_call_only: bool,
    /// A re-send with a different body is answered as a replay instead of
    /// refused batch_id_reuse.
    pub reuse_accepted: bool,
    /// The body identity hashes each item's raw text rather than its
    /// canonical JSON, so key order matters.
    pub identity_from_raw_text: bool,
    /// The score-level bound is checked only after the items were sent.
    pub score_levels_checked_after_dispatch: bool,
    /// A null state is forwarded rather than refused.
    pub null_state_forwarded: bool,
    /// The reply lists items sorted by their state, not in request order.
    pub items_sorted_by_state: bool,
    /// Answers are decoded and re-encoded through f64, so number spellings
    /// change.
    pub numbers_through_f64: bool,
    /// An item the provider rejects with a 4xx fails the whole batch.
    pub fail_whole_batch_on_4xx: bool,
    /// Usage the provider did not report is written as 0.
    pub usage_zero_filled: bool,
    /// After an item meets a 401 or 403, the call goes on sending the
    /// items after it.
    pub continue_after_auth_failure: bool,
    /// `auth_failed` is stored as a permanent error, so a re-send replays
    /// it instead of asking the provider again.
    pub store_auth_failure: bool,
    /// A 401 or 403 on the call's first provider call becomes a per-item
    /// error instead of refusing the call at admission.
    pub first_call_auth_failure_not_refused: bool,
    /// `model_unavailable` is stored as a permanent error, so a re-send
    /// replays it instead of asking the provider again.
    pub store_model_unavailable: bool,
    /// After an item ends rate_limited, the call goes on sending the items
    /// after it.
    pub continue_after_rate_limit: bool,
    /// A request question of an unknown type is forwarded as if it were a
    /// noul question instead of being refused.
    pub unknown_question_type_forwarded: bool,
    /// A re-send's lower ceiling applies to that call only: the batch keeps
    /// the ceiling recorded at first admission.
    pub tightened_ceiling_not_recorded: bool,
}

/// The stand-in provider: scripts, call counts and held calls, shared by
/// the fake module and the subject.
#[derive(Default)]
pub struct StandIn {
    scripts: Mutex<BTreeMap<String, Scripted>>,
    calls: Mutex<BTreeMap<String, usize>>,
    hold_next: Mutex<BTreeSet<String>>,
    held: Mutex<BTreeSet<String>>,
}

impl StandIn {
    async fn call(&self, state: &Value) -> Scripted {
        let key = state_key(state);
        *self.calls.lock().unwrap().entry(key.clone()).or_default() += 1;
        let hold = self.hold_next.lock().unwrap().remove(&key);
        if hold {
            self.held.lock().unwrap().insert(key.clone());
            loop {
                let held = self.held.lock().unwrap().contains(&key);
                if !held {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
        self.scripts
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .unwrap_or(Scripted::status(500))
    }
}

/// One item's recorded outcome.
#[derive(Clone)]
enum Stored {
    Answered {
        answers: String,
        usage: Option<Usage>,
        cost: f64,
    },
    Failed(ItemError),
}

/// What a call's provider calls came to: each item's outcome as the reply
/// gives it, whether this call answered it, and each item's outcome as the
/// batch records it (an error that is never stored leaves what was there).
type CallOutcomes = (Vec<Stored>, Vec<bool>, Vec<Option<Stored>>);

struct Batch {
    identity: BatchIdentity,
    raw_items: Vec<String>,
    ceiling: Option<f64>,
    outcomes: Vec<Option<Stored>>,
    in_flight: bool,
}

#[derive(Serialize)]
struct ReplyItem {
    index: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    answers: Option<Box<RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ItemError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
}

#[derive(Serialize)]
struct ReplyText {
    batch_id: String,
    model: String,
    items: Vec<ReplyItem>,
    usage: Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<Number>,
}

#[derive(Deserialize)]
struct ProviderAnswer {
    answers: Box<RawValue>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct RawItems {
    items: Vec<Box<RawValue>>,
}

pub struct FakeModule {
    defects: Defects,
    priced: bool,
    stand_in: Arc<StandIn>,
    batches: Mutex<BTreeMap<String, Batch>>,
}

fn refused(refusal: Refusal) -> Reply {
    let detail = serde_json::to_value(&refusal.detail).unwrap();
    Reply::Error(ErrorBody::new(refusal.code, refusal.message).with_detail(detail))
}

impl FakeModule {
    fn catalog(&self) -> Vec<ModelEntry> {
        let mut priced = ModelEntry::new(PRICED, 8, MAX_ITEMS, PRICED_CONTEXT, EGRESS)
            .with_images(ImageSupport::new(2));
        if self.priced {
            priced = priced.with_price(
                Price::new()
                    .with_input_per_mtok_usd(Number::from_f64(2.0).unwrap())
                    .with_output_per_mtok_usd(Number::from_f64(10.0).unwrap()),
            );
        }
        vec![
            priced,
            ModelEntry::new(TEXT_ONLY, 8, MAX_ITEMS, TEXT_CONTEXT, EGRESS),
        ]
    }

    fn describe(&self) -> RoleDescribe {
        let mut models = self.catalog();
        if self.defects.describe_wrong_context_tokens {
            models[0].context_tokens = 1;
        }
        if self.defects.describe_omits_model {
            models.truncate(1);
        }
        RoleDescribe::new(Limits::new(MAX_ITEMS, MAX_REQUEST_BYTES), models)
    }

    fn kind(&self, model: &str) -> Option<&'static str> {
        match model {
            PRICED | TEXT_ONLY => Some(CLASSIFIER_KIND),
            CHAT => Some("chat"),
            _ => None,
        }
    }

    fn rates(&self, model: &ModelEntry) -> (f64, f64) {
        let rate = |rate: &Option<Number>| rate.as_ref().and_then(Number::as_f64).unwrap_or(0.0);
        model.price.as_ref().map_or((0.0, 0.0), |price| {
            (
                rate(&price.input_per_mtok_usd),
                rate(&price.output_per_mtok_usd),
            )
        })
    }

    /// The fake's input-token estimate: a quarter of the state's bytes.
    fn estimate(&self, state: &Value, model: &ModelEntry) -> f64 {
        let tokens = digest_len(state) as f64 / 4.0;
        tokens * self.rates(model).0 / 1e6
    }

    async fn classify(&self, text: &str) -> Reply {
        let describe = self.describe();
        if let Err(refusal) = check_request_bytes(text.len(), &describe.limits) {
            return refused(refusal);
        }
        let request = if self.defects.null_state_forwarded {
            match serde_json::from_str::<ClassifyRequest>(text) {
                Ok(request) => request,
                Err(e) => return refused(Refusal::invalid_params("params", e.to_string())),
            }
        } else {
            match parse_request(text) {
                Ok(request) => request,
                Err(refusal) => return refused(refusal),
            }
        };
        if let Err(refusal) = check_catalog_row(&request.model, self.kind(&request.model)) {
            return refused(refusal);
        }
        let catalog = self.catalog();
        let model = catalog
            .iter()
            .find(|entry| entry.model == request.model)
            .unwrap()
            .clone();
        let mut checked = request.clone();
        if self.defects.null_state_forwarded {
            for item in &mut checked.items {
                if item.state.is_null() {
                    item.state = Value::String("null".into());
                }
            }
        }
        if self.defects.unknown_question_type_forwarded {
            for question in checked.questions.values_mut() {
                if matches!(question.kind, QuestionType::Other(_)) {
                    question.kind = QuestionType::Noul;
                }
            }
        }
        let too_many_levels = request.questions.values().any(|question| {
            matches!(&question.criteria, Some(Criteria::Levels(levels)) if question.kind == QuestionType::Score && levels.len() > SCORE_MAX_LEVELS)
        });
        if self.defects.score_levels_checked_after_dispatch && too_many_levels {
            for item in &request.items {
                self.stand_in.call(&item.state).await;
            }
        }
        if let Err(refusal) = check_request(&checked, &model) {
            return refused(refusal);
        }
        let identity = BatchIdentity::of(&request).unwrap();
        let raw_items: Vec<String> = serde_json::from_str::<RawItems>(text)
            .unwrap()
            .items
            .iter()
            .map(|item| item.get().to_owned())
            .collect();
        let sent_ceiling = request.max_cost_usd.as_ref().and_then(Number::as_f64);
        if self.defects.wait_for_in_flight {
            loop {
                let busy = self
                    .batches
                    .lock()
                    .unwrap()
                    .get(&request.batch_id)
                    .is_some_and(|batch| batch.in_flight);
                if !busy {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
        let (ceiling, new_batch) = {
            let mut batches = self.batches.lock().unwrap();
            match batches.get_mut(&request.batch_id) {
                Some(batch) => {
                    if batch.in_flight && !self.defects.no_single_flight {
                        return refused(Refusal::batch_in_progress(50));
                    }
                    let differs = if self.defects.identity_from_raw_text {
                        (batch.identity.model != identity.model
                            || batch.identity.questions != identity.questions
                            || batch.raw_items != raw_items)
                            .then(|| "items".to_owned())
                    } else {
                        batch.identity.first_difference(&identity)
                    };
                    if let Some(field) = differs {
                        if !self.defects.reuse_accepted {
                            return refused(Refusal::batch_id_reuse(field));
                        }
                    }
                    batch.in_flight = true;
                    let working = if self.defects.ceiling_from_latest_call {
                        sent_ceiling
                    } else if self.defects.ceiling_from_first_call_only {
                        batch.ceiling
                    } else {
                        effective_ceiling(batch.ceiling, sent_ceiling)
                    };
                    // The tightened ceiling is recorded before anything is
                    // sent, so a later re-send cannot loosen it.
                    if !self.defects.tightened_ceiling_not_recorded
                        && !self.defects.ceiling_from_first_call_only
                    {
                        batch.ceiling = effective_ceiling(batch.ceiling, sent_ceiling);
                    }
                    (working, false)
                }
                None => {
                    if let Some(max) = sent_ceiling {
                        let estimate: f64 = request
                            .items
                            .iter()
                            .map(|item| self.estimate(&item.state, &model))
                            .sum();
                        if estimate > max {
                            return refused(Refusal::cost_exceeded(
                                Number::from_f64(estimate).unwrap(),
                                request.max_cost_usd.clone().unwrap(),
                            ));
                        }
                    }
                    batches.insert(
                        request.batch_id.clone(),
                        Batch {
                            identity,
                            raw_items,
                            ceiling: sent_ceiling,
                            outcomes: vec![None; request.items.len()],
                            in_flight: true,
                        },
                    );
                    (sent_ceiling, true)
                }
            }
        };
        let (outcomes, answered_now, record) =
            match self.answer_items(&request, &model, ceiling).await {
                Ok(answered) => answered,
                Err(refusal) => {
                    // Refused at the call's first provider call: nothing
                    // about the items is written, and a batch this call
                    // created is not recorded at all.
                    let mut batches = self.batches.lock().unwrap();
                    if new_batch {
                        batches.remove(&request.batch_id);
                    } else if let Some(batch) = batches.get_mut(&request.batch_id) {
                        batch.in_flight = false;
                    }
                    return refused(refusal);
                }
            };
        let reply = self.reply(&request, &model, &outcomes, &answered_now);
        let mut batches = self.batches.lock().unwrap();
        let batch = batches.get_mut(&request.batch_id).unwrap();
        batch.in_flight = false;
        batch.outcomes = record;
        reply
    }

    /// Every item's outcome as this call replies with it, which items this
    /// call answered, and every item's outcome as the batch records it. An
    /// error is a refusal at the call's first provider call.
    async fn answer_items(
        &self,
        request: &ClassifyRequest,
        model: &ModelEntry,
        ceiling: Option<f64>,
    ) -> Result<CallOutcomes, Refusal> {
        let stored = self.batches.lock().unwrap()[&request.batch_id]
            .outcomes
            .clone();
        let mut spend: f64 = stored
            .iter()
            .map(|outcome| match outcome {
                Some(Stored::Answered { cost, .. }) => *cost,
                _ => 0.0,
            })
            .sum();
        let (input_rate, output_rate) = self.rates(model);
        let mut outcomes = Vec::new();
        let mut answered_now = Vec::new();
        let mut record = Vec::new();
        // Set once an item ends with an error that stops the call: every
        // later item that would need a provider call gets this instead.
        let mut stopped: Option<ItemError> = None;
        let mut provider_calls = 0usize;
        for (item, stored) in request.items.iter().zip(stored) {
            let ask = match &stored {
                None => true,
                Some(Stored::Answered { .. }) => self.defects.reask_answered_on_retry,
                Some(Stored::Failed(error)) => {
                    if error.is_retried_on_resend() {
                        !self.defects.replay_stored_transient
                    } else {
                        self.defects.retry_stored_permanent
                    }
                }
            };
            if !ask {
                outcomes.push(stored.clone().unwrap());
                answered_now.push(false);
                record.push(stored);
                continue;
            }
            if let Some(stop) = &stopped {
                // Not sent, and not recorded: a re-send asks again.
                outcomes.push(Stored::Failed(stop.clone()));
                answered_now.push(false);
                record.push(stored);
                continue;
            }
            if let Some(ceiling) = ceiling {
                if spend + self.estimate(&item.state, model) > ceiling {
                    let outcome = Stored::Failed(ItemError::for_code(
                        errors::COST_EXCEEDED,
                        "answering this item would cross the batch's ceiling",
                    ));
                    outcomes.push(outcome.clone());
                    answered_now.push(false);
                    record.push(Some(outcome));
                    continue;
                }
            }
            provider_calls += 1;
            let outcome = match self.stand_in.call(&item.state).await {
                Scripted::Answer(body) => {
                    let answer: ProviderAnswer = serde_json::from_str(&body).unwrap();
                    let cost = answer.usage.map_or(0.0, |usage| {
                        usage.input_tokens.unwrap_or(0) as f64 * input_rate / 1e6
                            + usage.output_tokens.unwrap_or(0) as f64 * output_rate / 1e6
                    });
                    spend += cost;
                    Stored::Answered {
                        answers: answer.answers.get().to_owned(),
                        usage: answer.usage,
                        cost,
                    }
                }
                Scripted::Status {
                    status,
                    retry_after_ms,
                } => {
                    let code = item_code_for_status(status).unwrap_or(errors::PROVIDER_ERROR);
                    let mut error = ItemError::for_code(
                        code,
                        format!("the provider answered {status} after retries"),
                    );
                    if let Some(ms) = retry_after_ms {
                        error = error.with_retry_after_ms(ms);
                    }
                    Stored::Failed(error)
                }
            };
            let Stored::Failed(error) = &outcome else {
                answered_now.push(true);
                record.push(Some(outcome.clone()));
                outcomes.push(outcome);
                continue;
            };
            let code = error.code.as_str();
            if provider_calls == 1
                && !(code == errors::AUTH_FAILED
                    && self.defects.first_call_auth_failure_not_refused)
            {
                if code == errors::AUTH_FAILED {
                    return Err(Refusal::auth_failed());
                }
                if code == errors::MODEL_UNAVAILABLE {
                    return Err(Refusal::model_unavailable(&request.model));
                }
            }
            let stops = error.stops_the_call()
                && !(code == errors::AUTH_FAILED && self.defects.continue_after_auth_failure)
                && !(code == errors::RATE_LIMITED && self.defects.continue_after_rate_limit);
            if stops {
                stopped = Some(ItemError::for_code(
                    code,
                    format!("not sent: an earlier item in this call ended {code}"),
                ));
            }
            let kept = error.is_stored()
                || (code == errors::AUTH_FAILED && self.defects.store_auth_failure)
                || (code == errors::MODEL_UNAVAILABLE && self.defects.store_model_unavailable);
            answered_now.push(false);
            record.push(if kept { Some(outcome.clone()) } else { stored });
            outcomes.push(outcome);
        }
        Ok((outcomes, answered_now, record))
    }

    fn reply(
        &self,
        request: &ClassifyRequest,
        model: &ModelEntry,
        outcomes: &[Stored],
        answered_now: &[bool],
    ) -> Reply {
        if self.defects.fail_whole_batch_on_4xx
            && outcomes.iter().any(
                |outcome| matches!(outcome, Stored::Failed(error) if error.code == errors::INVALID_ITEM),
            )
        {
            return Reply::Error(ErrorBody::new(
                errors::PROVIDER_ERROR,
                "an item failed, so the batch failed",
            ));
        }
        let mut items: Vec<(String, ReplyItem)> = Vec::new();
        let mut cost = 0.0;
        for (index, (outcome, item)) in outcomes.iter().zip(&request.items).enumerate() {
            let reply_item = match outcome {
                Stored::Answered {
                    answers,
                    usage,
                    cost: item_cost,
                } => {
                    if !self.defects.cost_per_call || answered_now[index] {
                        cost += item_cost;
                    }
                    let answers = if self.defects.numbers_through_f64 {
                        let typed: IndexMap<String, Answer> =
                            serde_json::from_str(answers).unwrap();
                        serde_json::to_string(&typed).unwrap()
                    } else {
                        answers.clone()
                    };
                    let usage = if self.defects.usage_zero_filled {
                        Some(
                            Usage::new()
                                .with_input_tokens(usage.and_then(|u| u.input_tokens).unwrap_or(0))
                                .with_output_tokens(
                                    usage.and_then(|u| u.output_tokens).unwrap_or(0),
                                ),
                        )
                    } else {
                        *usage
                    };
                    ReplyItem {
                        index: index as u64,
                        answers: Some(RawValue::from_string(answers).unwrap()),
                        error: None,
                        usage,
                    }
                }
                Stored::Failed(error) => ReplyItem {
                    index: index as u64,
                    answers: None,
                    error: Some(error.clone()),
                    usage: None,
                },
            };
            items.push((state_key(&item.state), reply_item));
        }
        if self.defects.items_sorted_by_state {
            items.sort_by(|a, b| a.0.cmp(&b.0));
            for (index, (_, item)) in items.iter_mut().enumerate() {
                item.index = index as u64;
            }
        }
        let items: Vec<ReplyItem> = items.into_iter().map(|(_, item)| item).collect();
        let add = |total: Option<u64>, count: Option<u64>| match (total, count) {
            (Some(total), Some(count)) => Some(total + count),
            (total, None) => total,
            (None, count) => count,
        };
        let (input, output) = items.iter().filter_map(|item| item.usage).fold(
            (None, None),
            |(input, output), usage| {
                (
                    add(input, usage.input_tokens),
                    add(output, usage.output_tokens),
                )
            },
        );
        let mut usage = Usage::new();
        if let Some(input) = input {
            usage = usage.with_input_tokens(input);
        }
        if let Some(output) = output {
            usage = usage.with_output_tokens(output);
        }
        let reply = ReplyText {
            batch_id: request.batch_id.clone(),
            model: request.model.clone(),
            items,
            usage,
            cost_usd: model.is_priced().then(|| Number::from_f64(cost).unwrap()),
        };
        Reply::Response(serde_json::to_string(&reply).unwrap())
    }
}

/// The length of a state's canonical JSON, the fake's size measure.
fn digest_len(state: &Value) -> usize {
    serde_jcs::to_vec(state).map_or(0, |bytes| bytes.len())
}

#[derive(Clone)]
pub struct FakeRoute(Arc<FakeModule>);

#[async_trait]
impl ClassifierRoute for FakeRoute {
    async fn request(&self, method: &str, params: &str) -> Result<Reply, RouteFailure> {
        match method {
            ops::ROLE_DESCRIBE => Ok(Reply::Response(
                serde_json::to_string(&self.0.describe()).unwrap(),
            )),
            ops::CLASSIFY_RUN => Ok(self.0.classify(params).await),
            other => Ok(Reply::Error(ErrorBody::new(
                "unknown_method",
                format!("no op {other}"),
            ))),
        }
    }
}

/// The subject the suite's tests run against.
pub struct FakeSubject {
    module: Arc<FakeModule>,
    can_hold: bool,
}

impl FakeSubject {
    pub fn new(defects: Defects) -> Self {
        Self::build(defects, true, true)
    }

    /// A module whose catalog prices nothing.
    pub fn unpriced() -> Self {
        Self::build(Defects::default(), false, true)
    }

    /// A stand-in that cannot hold calls.
    pub fn without_hold() -> Self {
        Self::build(Defects::default(), true, false)
    }

    fn build(defects: Defects, priced: bool, can_hold: bool) -> Self {
        Self {
            module: Arc::new(FakeModule {
                defects,
                priced,
                stand_in: Arc::new(StandIn::default()),
                batches: Mutex::new(BTreeMap::new()),
            }),
            can_hold,
        }
    }
}

#[async_trait]
impl ClassifierSubject for FakeSubject {
    type Route = FakeRoute;

    async fn route(&self) -> Result<FakeRoute, String> {
        Ok(FakeRoute(self.module.clone()))
    }

    fn served_models(&self) -> Vec<ServedModel> {
        vec![
            ServedModel {
                model: PRICED.into(),
                priced: self.module.priced,
                images_max: Some(2),
                context_tokens: PRICED_CONTEXT,
                egress_host: EGRESS.into(),
            },
            ServedModel {
                model: TEXT_ONLY.into(),
                priced: false,
                images_max: None,
                context_tokens: TEXT_CONTEXT,
                egress_host: EGRESS.into(),
            },
        ]
    }

    fn non_classifier_model(&self) -> Option<String> {
        Some(CHAT.into())
    }

    async fn script(&self, key: &str, scripted: Scripted) {
        self.module
            .stand_in
            .scripts
            .lock()
            .unwrap()
            .insert(key.to_owned(), scripted);
    }

    async fn calls(&self, key: &str) -> usize {
        self.module
            .stand_in
            .calls
            .lock()
            .unwrap()
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    fn can_hold_calls(&self) -> bool {
        self.can_hold
    }

    async fn hold_next(&self, key: &str) {
        self.module
            .stand_in
            .hold_next
            .lock()
            .unwrap()
            .insert(key.to_owned());
    }

    async fn is_held(&self, key: &str) -> bool {
        self.module.stand_in.held.lock().unwrap().contains(key)
    }

    async fn release(&self, key: &str) {
        self.module.stand_in.held.lock().unwrap().remove(key);
    }

    async fn pause(&self) {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}
