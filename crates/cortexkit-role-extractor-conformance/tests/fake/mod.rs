//! A reference in-process extractor module, used ONLY to test the suite.
//!
//! Conformance runs against real modules. This fake exists so the suite's
//! own tests can show that each check passes a module that keeps the
//! contract and fails one that breaks it. The property it models:
//!
//! > A module that admits a batch only after every admission check of the
//! > role crate (`request::admit`), records its body identity and refuses a
//! > re-send with another body `batch_id_reuse`, checks each item against
//! > the batch ceiling before its provider call, and sends each admitted
//! > item to the provider as the role maps it.
//!
//! Each `Defects` field breaks one rule. Most do it the way a real module
//! would get it wrong, by skipping one admission check: the fake drops any
//! refusal that check would make and admits the request anyway.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use cortexkit_role_extractor_conformance::{
    wire::{
        describe::{ImageLimits, Limits, ModelEntry, Price, RoleDescribe},
        errors::{ItemError, Refusal, PROVIDER_ERROR},
        field::{types, FieldType},
        identity::{digest, BatchIdentity},
        ops,
        provider::ProviderRequest,
        reply::{ExtractReply, ItemResult},
        request::{admit, parse_request, CatalogRow, ExtractRequest, REPLY_FRAME_BYTES},
        spend::{check_before_call, item_cost_input, usd_to_nanos, ModuleBounds, Rounding},
    },
    ExtractorRoute, ExtractorSubject, Reply, RouteFailure, ServedModel,
};
use serde_json::Number;
use subc_protocol::ErrorBody;

pub const SERVED: &str = "fake/clairic";
pub const PROVIDER_MODEL: &str = "@cf/fake/clairic";
pub const CHAT: &str = "fake/chat";
pub const UNTYPED: &str = "fake/untyped";
const EGRESS: &str = "stand-in.invalid";
const PASS_TOKENS: u64 = 8192;
const MAX_ITEMS: u64 = 20;
const MAX_FIELDS: u64 = 16;
const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_INSTRUCTION_BYTES: u64 = 512;
const MAX_ANSWER_BYTES: u64 = 65_536;
const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;
const BOUNDS: ModuleBounds = ModuleBounds {
    image_tokens_each: 1000,
    overhead_tokens: 200,
};

/// Deliberate contract breaks: each field makes the fake violate one rule a
/// check must catch.
#[derive(Clone, Copy, Debug, Default)]
pub struct Defects {
    /// role.describe states a pass budget of 1 token.
    pub describe_wrong_pass_tokens: bool,
    /// The batch id is not checked.
    pub batch_id_unchecked: bool,
    /// A model the catalog does not have is served anyway.
    pub unknown_model_served: bool,
    /// model_not_extractor leaves out the row's kind.
    pub non_extraction_kind_dropped: bool,
    /// A row with no kind is called model_unknown.
    pub kindless_row_called_unknown: bool,
    /// Field ids are not checked.
    pub field_ids_unchecked: bool,
    /// A field's options are checked before its type, so an unknown type
    /// with a bad option is refused naming the option.
    pub options_checked_before_type: bool,
    /// Options the table does not list for a type are dropped.
    pub unknown_options_dropped: bool,
    /// Required options are optional.
    pub required_options_optional: bool,
    /// Option values are not range-checked.
    pub option_values_unchecked: bool,
    /// instructions is neither checked for empty nor bounded.
    pub instructions_unchecked: bool,
    /// reference_date is not checked.
    pub reference_date_unchecked: bool,
    /// Unknown request, item and options members are dropped.
    pub unknown_members_dropped: bool,
    /// Field and item counts are not checked.
    pub counts_unchecked: bool,
    /// Images are not checked.
    pub images_unchecked: bool,
    /// The request size is not checked.
    pub request_size_unchecked: bool,
    /// The reply size is not checked.
    pub reply_size_unchecked: bool,
    /// The provider request carries both options as false, whatever the
    /// batch asked.
    pub options_not_forwarded: bool,
    /// A re-send with a different body is admitted.
    pub reuse_accepted: bool,
    /// Images are hashed as written, so two spellings are two batches.
    pub images_hashed_as_written: bool,
    /// The batch ceiling is not checked before a provider call.
    pub ceiling_ignored: bool,
}

impl Defects {
    /// Whether a module with these defects would never have made
    /// `refusal`, because it skips the check behind it.
    fn skips(&self, refusal: &Refusal) -> bool {
        let field = refusal.field().unwrap_or("");
        let message = refusal.message.as_str();
        let option = field.starts_with("fields.") && field.matches('.').count() == 2;
        (self.batch_id_unchecked && field == "batch_id")
            || (self.unknown_model_served && refusal.code == "model_unknown")
            || (self.field_ids_unchecked && message.starts_with("a field id is"))
            || (self.unknown_options_dropped && message.contains("is not an option of"))
            || (self.required_options_optional && message.contains(" field requires "))
            || (self.option_values_unchecked
                && option
                && !message.starts_with("a field id is")
                && !field.ends_with(".type")
                && !field.ends_with(".instructions")
                && !message.contains("is not an option of")
                && !message.contains(" field requires "))
            || (self.instructions_unchecked && field.ends_with(".instructions"))
            || (self.reference_date_unchecked && field.ends_with(".reference_date"))
            || (self.unknown_members_dropped
                && (message.contains("is not a member of")
                    || message.starts_with("an item has state")
                    || message.starts_with("options has")))
            || (self.counts_unchecked
                && (field == "fields" || field == "items")
                && refusal.detail.limit != Some(REPLY_FRAME_BYTES))
            || (self.images_unchecked && field.contains(".images"))
            || (self.request_size_unchecked && field == "params" && refusal.detail.limit.is_some())
            || (self.reply_size_unchecked && refusal.detail.limit == Some(REPLY_FRAME_BYTES))
    }
}

/// The stand-in provider: every request body it received, by state. It
/// answers every call 500.
#[derive(Default)]
pub struct StandIn {
    received: Mutex<BTreeMap<String, Vec<String>>>,
}

pub struct FakeModule {
    defects: Defects,
    stand_in: Arc<StandIn>,
    batches: Mutex<BTreeMap<String, BatchIdentity>>,
}

fn refused(refusal: Refusal) -> Reply {
    let detail = serde_json::to_value(&refusal.detail).expect("a detail encodes");
    Reply::Error(ErrorBody::new(refusal.code, refusal.message).with_detail(detail))
}

fn served_model() -> ModelEntry {
    ModelEntry::new(
        SERVED,
        MAX_ITEMS,
        MAX_FIELDS,
        types::ALL.iter().map(|t| FieldType::parse(t)).collect(),
        PASS_TOKENS,
        MAX_ANSWER_BYTES,
        EGRESS,
    )
    .with_images(ImageLimits::workers_ai(4))
    .with_price(
        Price::new()
            .with_input_per_mtok_usd(serde_json::from_str("0.11").unwrap())
            .with_output_per_mtok_usd(Number::from(0)),
    )
}

impl FakeModule {
    fn describe(&self) -> RoleDescribe {
        let mut model = served_model();
        if self.defects.describe_wrong_pass_tokens {
            model.pass_tokens = 1;
        }
        RoleDescribe::new(
            Limits::new(MAX_ITEMS, MAX_REQUEST_BYTES, MAX_INSTRUCTION_BYTES),
            vec![model],
        )
    }

    fn row(&self, model: &str) -> CatalogRow<'static> {
        match model {
            SERVED => CatalogRow::Kind("extraction"),
            CHAT => CatalogRow::Kind("chat"),
            UNTYPED if self.defects.kindless_row_called_unknown => CatalogRow::Absent,
            UNTYPED => CatalogRow::Kindless,
            _ => CatalogRow::Absent,
        }
    }

    /// The batch identity, or `None` for a request a skipped check let
    /// through with an image that does not decode.
    fn identity(&self, request: &ExtractRequest) -> Option<BatchIdentity> {
        let mut identity = BatchIdentity::of(request).ok()?;
        if self.defects.images_hashed_as_written {
            identity.items = request
                .items
                .iter()
                .map(|item| digest(item).expect("an item hashes"))
                .collect();
        }
        Some(identity)
    }

    async fn extract(&self, text: &str) -> Reply {
        let describe = self.describe();
        let mut request = match self.admit_unless_skipped(text, &describe) {
            Ok(Some(request)) => request,
            // A skipped check let through a request the module cannot read:
            // it answers with an empty reply and calls nothing.
            Ok(None) => {
                return Reply::Response(
                    r#"{"batch_id":"","model":"","items":[],"usage":{}}"#.into(),
                )
            }
            Err(refusal) => return refused(refusal),
        };
        if let Some(identity) = self.identity(&request) {
            let mut batches = self.batches.lock().unwrap();
            match batches.get(&request.batch_id) {
                Some(recorded) if !self.defects.reuse_accepted => {
                    if let Some(field) = recorded.first_difference(&identity) {
                        return refused(Refusal::batch_id_reuse(field));
                    }
                }
                Some(_) => {}
                None => {
                    batches.insert(request.batch_id.clone(), identity);
                }
            }
        }
        if self.defects.options_not_forwarded {
            request.options = None;
        }
        let model = describe.model(SERVED).cloned().unwrap_or_else(served_model);
        let rate = model
            .price
            .as_ref()
            .and_then(|price| price.input_per_mtok_usd.as_ref())
            .and_then(|usd| usd_to_nanos(usd, Rounding::Up))
            .map(|nanos| nanos as i64);
        let ceiling = request
            .max_cost_usd
            .as_ref()
            .and_then(|usd| usd_to_nanos(usd, Rounding::Down));
        let mut items = Vec::new();
        for (index, item) in request.items.iter().enumerate() {
            if !self.defects.ceiling_ignored {
                let input = item_cost_input(&request, index, &model, BOUNDS, None, None)
                    .expect("the item exists");
                if let Err(error) = check_before_call(&input, rate, 0, ceiling) {
                    items.push(ItemResult::failed(index as u64, error));
                    continue;
                }
            }
            let body = ProviderRequest::for_item(&request, index, PROVIDER_MODEL)
                .expect("the item exists");
            self.stand_in
                .received
                .lock()
                .unwrap()
                .entry(item.state.clone())
                .or_default()
                .push(serde_json::to_string(&body).expect("a provider request encodes"));
            items.push(ItemResult::failed(
                index as u64,
                ItemError::for_code(PROVIDER_ERROR, "the provider answered 500"),
            ));
        }
        let reply = ExtractReply::new(&request.batch_id, &request.model, items);
        Reply::Response(serde_json::to_string(&reply).expect("a reply encodes"))
    }

    /// `admit`, except that a refusal the defects skip lets the request
    /// through. `Ok(None)` when it gets through undecodable.
    fn admit_unless_skipped(
        &self,
        text: &str,
        describe: &RoleDescribe,
    ) -> Result<Option<ExtractRequest>, Refusal> {
        if self.defects.options_checked_before_type {
            if let Ok(request) = parse_request(text) {
                for (id, field) in &request.fields {
                    if !field.kind.is_known() {
                        if let Some(name) = field.options.keys().next() {
                            return Err(Refusal::invalid_params(
                                format!("fields.{id}.{name}"),
                                format!("{name} is not an option of any field type"),
                            ));
                        }
                    }
                }
            }
        }
        match admit(text, describe, |model| self.row(model), MAX_BODY_BYTES) {
            Ok(request) => Ok(Some(request)),
            Err(refusal) if self.defects.skips(&refusal) => Ok(parse_request(text).ok()),
            Err(mut refusal) => {
                if self.defects.non_extraction_kind_dropped {
                    refusal.detail.kind = None;
                }
                Err(refusal)
            }
        }
    }
}

#[derive(Clone)]
pub struct FakeRoute {
    module: Arc<FakeModule>,
}

#[async_trait]
impl ExtractorRoute for FakeRoute {
    async fn request(&self, method: &str, params: &str) -> Result<Reply, RouteFailure> {
        Ok(match method {
            ops::ROLE_DESCRIBE => Reply::Response(
                serde_json::to_string(&self.module.describe()).expect("describe encodes"),
            ),
            ops::EXTRACT_RUN => self.module.extract(params).await,
            other => refused(Refusal::invalid_params(
                "params",
                format!("unknown method {other}"),
            )),
        })
    }
}

pub struct FakeSubject {
    module: Arc<FakeModule>,
}

impl FakeSubject {
    pub fn new(defects: Defects) -> Self {
        Self {
            module: Arc::new(FakeModule {
                defects,
                stand_in: Arc::new(StandIn::default()),
                batches: Mutex::new(BTreeMap::new()),
            }),
        }
    }
}

#[async_trait]
impl ExtractorSubject for FakeSubject {
    type Route = FakeRoute;

    async fn route(&self) -> Result<FakeRoute, String> {
        Ok(FakeRoute {
            module: self.module.clone(),
        })
    }

    fn served_models(&self) -> Vec<ServedModel> {
        vec![ServedModel {
            model: SERVED.into(),
            provider_model: PROVIDER_MODEL.into(),
            priced: true,
            images: Some(ImageLimits::workers_ai(4)),
            pass_tokens: PASS_TOKENS,
            context_tokens: None,
            egress_host: EGRESS.into(),
        }]
    }

    fn non_extraction_model(&self) -> Option<String> {
        Some(CHAT.into())
    }

    fn kindless_model(&self) -> Option<String> {
        Some(UNTYPED.into())
    }

    async fn received(&self, state: &str) -> Vec<String> {
        self.module
            .stand_in
            .received
            .lock()
            .unwrap()
            .get(state)
            .cloned()
            .unwrap_or_default()
    }
}
