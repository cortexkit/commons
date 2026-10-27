//! The checks, each run against the live module through its route.

use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use cortexkit_role_extractor::{
    describe::{check_describe, ModelEntry, RoleDescribe},
    errors::{self, ErrorClass, Refusal},
    field::{types, Field, FieldType},
    image::Image,
    ops,
    provider::{read_reply, ProviderRequest},
    reply::{ExtractReply, ItemResult},
    request::{estimated_reply_bytes, ExtractRequest, REPLY_FRAME_BYTES},
    spend::{cost_nanos, nanos_to_usd_text, usd_to_nanos, Rounding},
    vectors::PROVIDER_REPLIES_JSON,
    IndexMap, PROVIDES,
};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Map, Value};

use crate::{
    route::{ExtractorRoute, Reply},
    subject::{ExtractorSubject, Scripted, ServedModel},
};

/// How a check that did not fail ended.
pub enum Ending {
    Passed,
    /// The module does not do what the check asks about.
    NotApplicable(String),
}

type Check = Result<Ending, String>;

/// A change that turns a valid request into a bad one.
type Edit = fn(&mut Value);

/// A one-pixel PNG, as base64.
const PIXEL_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// The largest request the suite builds to exceed a limit.
const OVERSIZE_CAP: u64 = 64 * 1024 * 1024;

fn pixel_url() -> String {
    format!("data:image/png;base64,{PIXEL_BASE64}")
}

/// The bytes of the one-pixel PNG.
fn pixel_bytes() -> Vec<u8> {
    Image::DataUrl(pixel_url())
        .decode()
        .and_then(Result::ok)
        .map(|decoded| decoded.bytes)
        .expect("the one-pixel PNG decodes")
}

/// One of Clairic's captured replies from the role crate's
/// `provider-replies.json`: the status and the body the stand-in replays,
/// and, where the suite sends the same request again, its state and its
/// fields as written in the file.
#[derive(Deserialize)]
struct Probe {
    name: String,
    status: u16,
    #[serde(default)]
    sent: Option<Sent>,
    body: String,
}

#[derive(Deserialize)]
struct Sent {
    state: String,
    fields: Box<RawValue>,
}

#[derive(Deserialize)]
struct Probes {
    replies: Vec<Probe>,
}

/// The captured reply `name`, such as `02-span-offsets`.
fn probe(name: &str) -> Result<Probe, String> {
    let probes: Probes = serde_json::from_str(PROVIDER_REPLIES_JSON)
        .map_err(|e| format!("provider-replies.json does not decode: {e}"))?;
    probes
        .replies
        .into_iter()
        .find(|probe| probe.name == name)
        .ok_or_else(|| format!("provider-replies.json has no reply {name}"))
}

/// What the role crate reads `probe`'s reply as: the item a module answers
/// with.
fn expected_item(probe: &Probe) -> ItemResult {
    match read_reply(probe.status, &probe.body) {
        Ok(result) => result.into_item(0),
        Err(error) => ItemResult::failed(0, error),
    }
}

/// The suite's state for one run: the route, the decoded `role.describe`,
/// and the run's unique prefix for batch ids and item states.
pub struct Suite<'a, S: ExtractorSubject> {
    subject: &'a S,
    route: S::Route,
    describe: RoleDescribe,
    run: String,
    counter: AtomicUsize,
}

impl<'a, S> Suite<'a, S>
where
    S: ExtractorSubject,
    S::Route: ExtractorRoute,
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
            "describe_complete" => self.describe_complete(),
            "bad_batch_id_refused" => self.bad_batch_id().await,
            "unknown_model_refused" => self.unknown_model().await,
            "non_extraction_model_refused" => self.non_extraction_model().await,
            "kindless_model_refused" => self.kindless_model().await,
            "field_id_refused_naming_field" => self.field_ids().await,
            "unknown_field_type_refused_before_options" => self.unknown_type().await,
            "misspelt_or_misplaced_option_refused" => self.unknown_options().await,
            "missing_required_option_refused" => self.required_options().await,
            "option_value_out_of_range_refused" => self.option_values().await,
            "instructions_refused_empty_or_over_bound" => self.instructions().await,
            "item_state_and_reference_date_refused" => self.items().await,
            "unknown_member_refused" => self.unknown_members().await,
            "counts_refused_with_limit" => self.counts().await,
            "images_refused_before_any_call" => self.images().await,
            "oversize_request_refused" => self.oversize().await,
            "reply_over_frame_refused" => self.reply_over_frame().await,
            "clairic_request_mapped" => self.mapping().await,
            "reuse_refused_naming_part" => self.reuse().await,
            "image_spellings_are_one_batch" => self.image_spellings().await,
            "zero_ceiling_sends_nothing" => self.zero_ceiling().await,
            "provider_answers_forwarded" => self.answers_forwarded().await,
            "spans_forwarded_unconverted" => self.spans_unconverted().await,
            "provider_errors_mapped" => self.errors_mapped().await,
            "cost_usd_from_reported_tokens" => self.cost_from_usage().await,
            "image_reference_follows_declaration" => self.reference_follows_declaration().await,
            "image_reference_resolved_and_answered" => self.reference_resolved().await,
            "image_reference_oversized_refused_like_inline" => self.reference_oversized().await,
            "image_source_down_refused_transient" => self.reference_source_down().await,
            "image_reference_identity_is_its_handle" => self.reference_identity().await,
            other => Err(format!("the suite has no check named {other}")),
        }
    }

    // ---- helpers --------------------------------------------------------

    /// A name unique to this run.
    fn mint(&self, label: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{}-{label}-{n:06}", self.run)
    }

    fn served(&self) -> Vec<ServedModel> {
        self.subject.served_models()
    }

    /// The first served model `role.describe` lists.
    fn model(&self) -> Result<&ModelEntry, String> {
        let served: BTreeSet<String> = self.served().into_iter().map(|s| s.model).collect();
        self.describe
            .models
            .iter()
            .find(|entry| served.contains(&entry.model))
            .ok_or_else(|| "role.describe lists none of the models the catalog serves".to_owned())
    }

    /// The first type the model serves among `wanted`, in `wanted`'s order.
    fn served_type(model: &ModelEntry, wanted: &[&str]) -> Option<&'static str> {
        types::ALL
            .iter()
            .copied()
            .filter(|name| wanted.contains(name))
            .find(|name| model.field_types.contains(&FieldType::parse(name)))
    }

    fn serves(model: &ModelEntry, kind: &str) -> bool {
        model.field_types.contains(&FieldType::parse(kind))
    }

    /// A field every model can take: its first served type, with no
    /// options.
    fn plain_field(model: &ModelEntry) -> Value {
        json!({"type": model.field_types[0].as_str()})
    }

    /// A valid request on `model` with one plain field and `states`.
    fn base(model: &ModelEntry, batch_id: &str, states: &[&str]) -> Value {
        let items: Vec<Value> = states.iter().map(|state| json!({"state": state})).collect();
        json!({
            "batch_id": batch_id,
            "model": model.model,
            "fields": {"total": Self::plain_field(model)},
            "items": items,
        })
    }

    async fn send(&self, params: &Value) -> Result<Reply, String> {
        self.send_text(&params.to_string()).await
    }

    /// Send params as exact text, for a request whose member order matters.
    async fn send_text(&self, params: &str) -> Result<Reply, String> {
        self.route
            .request(ops::EXTRACT_RUN, params)
            .await
            .map_err(|failure| format!("the route failed: {}", failure.message))
    }

    async fn calls(&self, state: &str) -> usize {
        self.subject.received(state).await.len()
    }

    fn reply_of(what: &str, reply: Reply) -> Result<ExtractReply, String> {
        match reply {
            Reply::Response(text) => serde_json::from_str(&text)
                .map_err(|e| format!("{what}: the reply does not decode: {e}: {text}")),
            Reply::Error(body) => Err(format!("{what}: refused {}: {}", body.code, body.message)),
        }
    }

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

    /// `refusal` has `code`, the class the contract gives it, `field` and
    /// `limit` when one is expected.
    fn expect_refusal(
        what: &str,
        refusal: &Refusal,
        code: &str,
        field: Option<&str>,
        limit: Option<u64>,
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
        if limit.is_some() && refusal.detail.limit != limit {
            return Err(format!(
                "{what}: {code} states limit {:?}, expected {limit:?}",
                refusal.detail.limit
            ));
        }
        Ok(())
    }

    /// Send `bad`, expect `code` naming `field` (and `limit`), check that
    /// none of `states` reached the stand-in, then show the refusal wrote
    /// nothing: a valid body under the same `batch_id` is admitted.
    async fn refused(
        &self,
        what: &str,
        bad: &Value,
        states: &[&str],
        code: &str,
        field: Option<&str>,
        limit: Option<u64>,
    ) -> Result<Refusal, String> {
        let refusal = Self::refusal_of(what, self.send(bad).await?)?;
        Self::expect_refusal(what, &refusal, code, field, limit)?;
        for state in states {
            let calls = self.calls(state).await;
            if calls != 0 {
                return Err(format!(
                    "{what}: the stand-in was called {calls} times for a refused request"
                ));
            }
        }
        if let Some(batch_id) = bad.get("batch_id").and_then(Value::as_str) {
            if cortexkit_role_extractor::request::is_batch_id(batch_id) {
                let model = self.model()?;
                let state = self.mint("after");
                let good = Self::base(model, batch_id, &[&state]);
                Self::reply_of(
                    &format!("{what}: a valid body under the same batch_id after the refusal"),
                    self.send(&good).await?,
                )?;
            }
        }
        Ok(refusal)
    }

    /// A bad request built from a valid one on the first served model:
    /// `edit` changes it. Returns the request and its one item's state.
    fn edited(
        &self,
        label: &str,
        edit: impl FnOnce(&mut Value),
    ) -> Result<(Value, String), String> {
        let model = self.model()?;
        let state = self.mint(label);
        let mut request = Self::base(model, &self.mint(label), &[&state]);
        edit(&mut request);
        Ok((request, state))
    }

    /// One field `id` with `field`, refused naming `fields.<id>.<member>`.
    async fn field_refused(
        &self,
        what: &str,
        id: &str,
        field: Value,
        member: &str,
    ) -> Result<(), String> {
        let (request, state) = self.edited("field", |request| {
            request["fields"] = json!({ id: field });
        })?;
        self.refused(
            what,
            &request,
            &[&state],
            errors::INVALID_PARAMS,
            Some(&format!("fields.{id}.{member}")),
            None,
        )
        .await
        .map(|_| ())
    }

    // ---- checks ---------------------------------------------------------

    /// The first served model, when it takes images, on a module that
    /// declares `image_reference`, and a handle the subject's hook issued
    /// for `bytes` of `content_type`; or why the check is not applicable.
    async fn issue(
        &self,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<Result<(&ModelEntry, String), String>, String> {
        let model = self.model()?;
        if model.images.is_none() {
            return Ok(Err("the model takes no images".into()));
        }
        if !self.describe.declares_image_reference() {
            return Ok(Err("role.describe does not declare image_reference".into()));
        }
        Ok(self
            .subject
            .issue_image_reference(content_type, bytes)
            .await
            .map(|handle| (model, handle))
            .ok_or_else(|| "the subject has no hook to issue image references".into()))
    }

    /// A valid one-item request on `model` whose item carries `image`.
    fn with_image(model: &ModelEntry, batch_id: &str, state: &str, image: Value) -> Value {
        let mut request = Self::base(model, batch_id, &[state]);
        request["items"][0]["images"] = json!([image]);
        request
    }

    async fn reference_follows_declaration(&self) -> Check {
        let model = self.model()?;
        if model.images.is_none() {
            return Ok(Ending::NotApplicable("the model takes no images".into()));
        }
        let state = self.mint("unissued");
        let handle = self.mint("never-issued");
        let request = Self::with_image(
            model,
            &self.mint("unissued"),
            &state,
            json!({"handle": handle}),
        );
        let (what, code) = if self.describe.declares_image_reference() {
            (
                "a reference the module never issued, on a module that declares image_reference",
                errors::IMAGE_UNAVAILABLE,
            )
        } else {
            (
                "a reference, on a module that does not declare image_reference",
                errors::INVALID_PARAMS,
            )
        };
        self.refused(
            what,
            &request,
            &[&state],
            code,
            Some("items[0].images[0]"),
            None,
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn reference_resolved(&self) -> Check {
        let pixel = pixel_bytes();
        let (model, handle) = match self.issue("image/png", pixel.clone()).await? {
            Ok(issued) => issued,
            Err(reason) => return Ok(Ending::NotApplicable(reason)),
        };
        if !model
            .images
            .as_ref()
            .is_some_and(|limits| limits.formats.iter().any(|f| f == "image/png"))
        {
            return Ok(Ending::NotApplicable(
                "the model takes no PNG images".into(),
            ));
        }
        let state = self.mint("resolved");
        let request = Self::with_image(
            model,
            &self.mint("resolved"),
            &state,
            json!({"handle": handle}),
        );
        let what = "a reference the hook issued";
        Self::reply_of(what, self.send(&request).await?)?;
        let bodies = self.subject.received(&state).await;
        let [body] = bodies.as_slice() else {
            return Err(format!(
                "{what}: the stand-in received {} calls for the item, expected 1",
                bodies.len()
            ));
        };
        let got: ProviderRequest = serde_json::from_str(body)
            .map_err(|e| format!("{what}: the provider request does not decode: {e}"))?;
        let sent = got.images.as_deref().unwrap_or_default();
        let decoded = match sent {
            [image] => image.decode(),
            _ => None,
        };
        match decoded {
            Some(Ok(image)) if image.content_type == "image/png" && image.bytes == pixel => {
                Ok(Ending::Passed)
            }
            _ => Err(format!(
                "{what}: the stand-in received {body}, expected the image's bytes as its one image"
            )),
        }
    }

    async fn reference_oversized(&self) -> Check {
        let model = self.model()?;
        let Some(limits) = model.images.clone() else {
            return Ok(Ending::NotApplicable("the model takes no images".into()));
        };
        if limits.max_bytes_each >= OVERSIZE_CAP {
            return Ok(Ending::NotApplicable(format!(
                "images.max_bytes_each {} is over the suite's cap of {OVERSIZE_CAP}",
                limits.max_bytes_each
            )));
        }
        let format = limits.formats.first().cloned().unwrap_or_default();
        let mut bytes = pixel_bytes();
        bytes.resize(limits.max_bytes_each as usize + 1, 0);
        let (model, handle) = match self.issue(&format, bytes).await? {
            Ok(issued) => issued,
            Err(reason) => return Ok(Ending::NotApplicable(reason)),
        };
        let state = self.mint("oversized");
        let request = Self::with_image(
            model,
            &self.mint("oversized"),
            &state,
            json!({"handle": handle}),
        );
        self.refused(
            "a reference resolving to one byte over images.max_bytes_each",
            &request,
            &[&state],
            errors::INVALID_PARAMS,
            Some("items[0].images[0]"),
            None,
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn reference_source_down(&self) -> Check {
        let (model, handle) = match self.issue("image/png", pixel_bytes()).await? {
            Ok(issued) => issued,
            Err(reason) => return Ok(Ending::NotApplicable(reason)),
        };
        let state = self.mint("source-down");
        let request = Self::with_image(
            model,
            &self.mint("source-down"),
            &state,
            json!({"handle": handle}),
        );
        let what = "a reference while the image source is down";
        self.subject.set_image_source_down(true).await;
        let sent = self.send(&request).await;
        // The source comes back before any early return, so a failure here
        // cannot leave it down for the checks after this one.
        self.subject.set_image_source_down(false).await;
        let refusal = Self::refusal_of(what, sent?)?;
        Self::expect_refusal(
            what,
            &refusal,
            errors::IMAGE_SOURCE_UNAVAILABLE,
            Some("items[0].images[0]"),
            None,
        )?;
        if refusal.detail.retry_after_ms.is_none() {
            return Err(format!(
                "{what}: image_source_unavailable carries no detail.retry_after_ms"
            ));
        }
        if self.calls(&state).await != 0 {
            return Err(format!("{what}: the refused request reached the stand-in"));
        }
        let again = "the same body once the image source is back";
        Self::reply_of(again, self.send(&request).await?)
            .map_err(|e| format!("{e}: the refused request was recorded"))?;
        if self.calls(&state).await != 1 {
            return Err(format!("{again}: the item did not reach the stand-in once"));
        }
        Ok(Ending::Passed)
    }

    async fn reference_identity(&self) -> Check {
        let (model, handle) = match self.issue("image/png", pixel_bytes()).await? {
            Ok(issued) => issued,
            Err(reason) => return Ok(Ending::NotApplicable(reason)),
        };
        let state = self.mint("identity");
        let batch_id = self.mint("identity");
        let request = Self::with_image(model, &batch_id, &state, json!({"handle": handle}));
        Self::reply_of("the first call", self.send(&request).await?)?;
        Self::reply_of("a re-send with the same handle", self.send(&request).await?)?;
        let inline = Self::with_image(
            model,
            &batch_id,
            &state,
            json!({"content_type": "image/png", "base64": PIXEL_BASE64}),
        );
        let what = "a re-send carrying the same picture inline instead of the handle";
        let refusal = Self::refusal_of(what, self.send(&inline).await?)?;
        Self::expect_refusal(
            what,
            &refusal,
            errors::BATCH_ID_REUSE,
            Some("items[0]"),
            None,
        )?;
        Ok(Ending::Passed)
    }

    fn describe_complete(&self) -> Check {
        check_describe(&self.describe).map_err(|e| format!("check_describe: {e}"))?;
        if self.describe.role != PROVIDES {
            return Err(format!("role is {}", self.describe.role));
        }
        let served = self.served();
        let listed: BTreeSet<&str> = self
            .describe
            .models
            .iter()
            .map(|m| m.model.as_str())
            .collect();
        let catalog: BTreeSet<&str> = served.iter().map(|m| m.model.as_str()).collect();
        if listed != catalog {
            return Err(format!(
                "role.describe lists {listed:?}; the catalog serves {catalog:?}"
            ));
        }
        for served in &served {
            let entry = self.describe.model(&served.model).expect("listed");
            let name = &served.model;
            if entry.pass_tokens != served.pass_tokens {
                return Err(format!(
                    "{name}: pass_tokens {}, the provider's is {}",
                    entry.pass_tokens, served.pass_tokens
                ));
            }
            if entry.context_tokens != served.context_tokens {
                return Err(format!(
                    "{name}: context_tokens {:?}, the catalog says {:?}",
                    entry.context_tokens, served.context_tokens
                ));
            }
            if entry.images != served.images {
                return Err(format!(
                    "{name}: images {:?}, the catalog says {:?}",
                    entry.images, served.images
                ));
            }
            if entry.is_priced() != served.priced {
                return Err(format!(
                    "{name}: priced {}, the catalog says {}",
                    entry.is_priced(),
                    served.priced
                ));
            }
            if entry.egress_host != served.egress_host {
                return Err(format!(
                    "{name}: egress_host {}, expected {}",
                    entry.egress_host, served.egress_host
                ));
            }
        }
        Ok(Ending::Passed)
    }

    async fn bad_batch_id(&self) -> Check {
        for (label, batch_id) in [("tab", "tab\there".to_owned()), ("long", "x".repeat(129))] {
            let (request, state) = self.edited("batch", |request| {
                request["batch_id"] = json!(batch_id);
            })?;
            self.refused(
                &format!("a {label} batch_id"),
                &request,
                &[&state],
                errors::INVALID_PARAMS,
                Some("batch_id"),
                None,
            )
            .await?;
        }
        Ok(Ending::Passed)
    }

    async fn unknown_model(&self) -> Check {
        let model = format!("{}/not-in-the-catalog", self.mint("acme"));
        let (request, state) = self.edited("unknown", |request| {
            request["model"] = json!(model);
        })?;
        let refusal = self
            .refused(
                "an unknown model",
                &request,
                &[&state],
                errors::MODEL_UNKNOWN,
                None,
                None,
            )
            .await?;
        if refusal.detail.model.as_deref() != Some(model.as_str()) {
            return Err(format!(
                "model_unknown names model {:?}, expected {model}",
                refusal.detail.model
            ));
        }
        Ok(Ending::Passed)
    }

    async fn non_extraction_model(&self) -> Check {
        let Some(model) = self.subject.non_extraction_model() else {
            return Ok(Ending::NotApplicable(
                "the catalog has no row of another kind".into(),
            ));
        };
        let (request, state) = self.edited("chat", |request| {
            request["model"] = json!(model);
        })?;
        let refusal = self
            .refused(
                "a non-extraction model",
                &request,
                &[&state],
                errors::MODEL_NOT_EXTRACTOR,
                None,
                None,
            )
            .await?;
        if refusal.detail.model.as_deref() != Some(model.as_str()) {
            return Err(format!(
                "model_not_extractor names model {:?}",
                refusal.detail.model
            ));
        }
        if refusal.detail.kind.is_none() {
            return Err("model_not_extractor for a row with a kind leaves detail.kind out".into());
        }
        Ok(Ending::Passed)
    }

    async fn kindless_model(&self) -> Check {
        let Some(model) = self.subject.kindless_model() else {
            return Ok(Ending::NotApplicable(
                "the catalog has no row without a kind".into(),
            ));
        };
        let (request, state) = self.edited("kindless", |request| {
            request["model"] = json!(model);
        })?;
        let refusal = self
            .refused(
                "a row with no kind",
                &request,
                &[&state],
                errors::MODEL_NOT_EXTRACTOR,
                None,
                None,
            )
            .await?;
        if refusal.detail.model.as_deref() != Some(model.as_str()) {
            return Err(format!(
                "model_not_extractor names model {:?}",
                refusal.detail.model
            ));
        }
        if let Some(kind) = &refusal.detail.kind {
            return Err(format!(
                "model_not_extractor for a row with no kind states kind {kind}"
            ));
        }
        Ok(Ending::Passed)
    }

    async fn field_ids(&self) -> Check {
        let model = self.model()?;
        for id in ["due.date", "9lives"] {
            let (request, state) = self.edited("id", |request| {
                request["fields"] = json!({ id: Self::plain_field(model) });
            })?;
            self.refused(
                &format!("field id {id}"),
                &request,
                &[&state],
                errors::INVALID_PARAMS,
                Some(&format!("fields.{id}")),
                None,
            )
            .await?;
        }
        Ok(Ending::Passed)
    }

    async fn unknown_type(&self) -> Check {
        self.field_refused(
            "a field of unknown type with a misspelt option",
            "total",
            json!({"type": "rank", "decimal": 2}),
            "type",
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn unknown_options(&self) -> Check {
        let model = self.model()?;
        let mut misspelt = Self::plain_field(model);
        misspelt["decimal"] = json!(2);
        self.field_refused("a misspelt option", "total", misspelt, "decimal")
            .await?;
        let others: Vec<&str> = types::ALL
            .iter()
            .copied()
            .filter(|t| *t != types::NUMBER && *t != types::DURATION)
            .collect();
        let Some(kind) = Self::served_type(model, &others) else {
            return Ok(Ending::NotApplicable(
                "the model serves no type that refuses integer".into(),
            ));
        };
        self.field_refused(
            &format!("integer on a {kind} field"),
            "vendor",
            json!({"type": kind, "integer": true}),
            "integer",
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn required_options(&self) -> Check {
        let model = self.model()?;
        let cases = [
            (types::LIST, json!({"type": "list"}), "items"),
            (
                types::ESTIMATE,
                json!({"type": "estimate", "min": 1}),
                "max",
            ),
            (
                types::FORECAST,
                json!({"type": "forecast", "min": 0, "max": 100, "interval": "month"}),
                "horizon",
            ),
        ];
        let mut ran = 0;
        for (kind, field, member) in cases {
            if !Self::serves(model, kind) {
                continue;
            }
            self.field_refused(&format!("a {kind} without {member}"), "x", field, member)
                .await?;
            ran += 1;
        }
        if ran == 0 {
            return Ok(Ending::NotApplicable(
                "the model serves no type with a required option".into(),
            ));
        }
        Ok(Ending::Passed)
    }

    async fn option_values(&self) -> Check {
        let model = self.model()?;
        let mut zero = Self::plain_field(model);
        zero["null_threshold"] = json!(0);
        self.field_refused("null_threshold 0", "x", zero, "null_threshold")
            .await?;
        let cases = [
            (
                types::NUMBER,
                json!({"type": "number", "decimals": 7}),
                "decimals",
            ),
            (
                types::FORECAST,
                json!({"type": "forecast", "min": 0, "max": 100, "horizon": 65, "interval": "month"}),
                "horizon",
            ),
            (
                types::ESTIMATE,
                json!({"type": "estimate", "min": 5, "max": 5}),
                "max",
            ),
        ];
        for (kind, field, member) in cases {
            if Self::serves(model, kind) {
                self.field_refused(&format!("a {kind} with a bad {member}"), "x", field, member)
                    .await?;
            }
        }
        Ok(Ending::Passed)
    }

    async fn instructions(&self) -> Check {
        let model = self.model()?;
        let mut empty = Self::plain_field(model);
        empty["instructions"] = json!("");
        self.field_refused("empty instructions", "x", empty, "instructions")
            .await?;
        let bound = self.describe.limits.max_instruction_bytes;
        if bound.saturating_add(4096) > self.describe.limits.max_request_bytes
            || bound > OVERSIZE_CAP
        {
            return Ok(Ending::NotApplicable(
                "instructions over max_instruction_bytes would not fit max_request_bytes".into(),
            ));
        }
        let mut long = Self::plain_field(model);
        long["instructions"] = json!("x".repeat(bound as usize + 1));
        let (request, state) = self.edited("instr", |request| {
            request["fields"] = json!({ "x": long });
        })?;
        self.refused(
            "instructions over the bound",
            &request,
            &[&state],
            errors::INVALID_PARAMS,
            Some("fields.x.instructions"),
            Some(bound),
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn items(&self) -> Check {
        let ok = self.mint("ok");
        let (request, _) = self.edited("num", |request| {
            request["items"] = json!([{"state": ok}, {"state": 12}]);
        })?;
        self.refused(
            "a number state",
            &request,
            &[&ok],
            errors::INVALID_PARAMS,
            Some("items[1].state"),
            None,
        )
        .await?;
        let (request, _) = self.edited("empty", |request| {
            request["items"] = json!([{"state": ""}]);
        })?;
        self.refused(
            "an empty state without images",
            &request,
            &[],
            errors::INVALID_PARAMS,
            Some("items[0].state"),
            None,
        )
        .await?;
        let (request, state) = self.edited("date", |request| {
            request["items"][0]["reference_date"] = json!("2026-02-30");
        })?;
        self.refused(
            "reference_date 2026-02-30",
            &request,
            &[&state],
            errors::INVALID_PARAMS,
            Some("items[0].reference_date"),
            None,
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn unknown_members(&self) -> Check {
        let cases: [(&str, Edit, &str); 3] = [
            (
                "a request member max_cost",
                |r| r["max_cost"] = json!(1),
                "max_cost",
            ),
            (
                "an item member reference",
                |r| r["items"][0]["reference"] = json!("2026-01-01"),
                "items[0].reference",
            ),
            (
                "an options member quantile",
                |r| r["options"] = json!({"quantile": true}),
                "options.quantile",
            ),
        ];
        for (what, edit, field) in cases {
            let (request, state) = self.edited("member", edit)?;
            self.refused(
                what,
                &request,
                &[&state],
                errors::INVALID_PARAMS,
                Some(field),
                None,
            )
            .await?;
        }
        Ok(Ending::Passed)
    }

    async fn counts(&self) -> Check {
        let model = self.model()?;
        let (request, state) = self.edited("nofields", |request| {
            request["fields"] = json!({});
        })?;
        self.refused(
            "no fields",
            &request,
            &[&state],
            errors::INVALID_PARAMS,
            Some("fields"),
            Some(1),
        )
        .await?;
        let max_fields = model.max_fields;
        let fields: Map<String, Value> = (0..=max_fields)
            .map(|i| (format!("f{i}"), Self::plain_field(model)))
            .collect();
        let (request, state) = self.edited("fields", |request| {
            request["fields"] = Value::Object(fields);
        })?;
        self.refused(
            "one field over max_fields",
            &request,
            &[&state],
            errors::INVALID_PARAMS,
            Some("fields"),
            Some(max_fields),
        )
        .await?;
        let states: Vec<String> = (0..=model.max_items).map(|_| self.mint("many")).collect();
        let items: Vec<Value> = states.iter().map(|state| json!({"state": state})).collect();
        let (request, _) = self.edited("items", |request| {
            request["items"] = Value::Array(items);
        })?;
        let refs: Vec<&str> = states.iter().map(String::as_str).collect();
        self.refused(
            "one item over max_items",
            &request,
            &refs,
            errors::INVALID_PARAMS,
            Some("items"),
            Some(model.max_items),
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn images(&self) -> Check {
        let model = self.model()?;
        let Some(limits) = model.images.clone() else {
            let (request, state) = self.edited("noimg", |request| {
                request["items"][0]["images"] = json!([pixel_url()]);
            })?;
            self.refused(
                "an image on a model that takes none",
                &request,
                &[&state],
                errors::INVALID_PARAMS,
                Some("items[0].images"),
                Some(0),
            )
            .await?;
            return Ok(Ending::Passed);
        };
        let cases: [(&str, Value, &str); 2] = [
            (
                "a remote URL",
                json!(["https://example.com/screenshot.png"]),
                "items[0].images[0]",
            ),
            (
                "malformed base64",
                json!([{"content_type": "image/png", "base64": "not base64!"}]),
                "items[0].images[0]",
            ),
        ];
        for (what, images, field) in cases {
            let (request, state) = self.edited("img", |request| {
                request["items"][0]["images"] = images;
            })?;
            self.refused(
                what,
                &request,
                &[&state],
                errors::INVALID_PARAMS,
                Some(field),
                None,
            )
            .await?;
        }
        let too_many: Vec<String> = (0..=limits.max).map(|_| pixel_url()).collect();
        let (request, state) = self.edited("imgs", |request| {
            request["items"][0]["images"] = json!(too_many);
        })?;
        self.refused(
            "one image over images.max",
            &request,
            &[&state],
            errors::INVALID_PARAMS,
            Some("items[0].images"),
            Some(limits.max),
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn oversize(&self) -> Check {
        let max = self.describe.limits.max_request_bytes;
        if max > OVERSIZE_CAP {
            return Ok(Ending::NotApplicable(format!(
                "max_request_bytes {max} is over the suite's cap of {OVERSIZE_CAP}"
            )));
        }
        let state = format!("{}{}", self.mint("big"), "x".repeat(max as usize));
        let (request, _) = self.edited("big", |request| {
            request["items"][0]["state"] = json!(state);
        })?;
        self.refused(
            "an oversize request",
            &request,
            &[&state],
            errors::INVALID_PARAMS,
            Some("params"),
            Some(max),
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn reply_over_frame(&self) -> Check {
        let model = self.model()?;
        let fields: Map<String, Value> = (0..model.max_fields)
            .map(|i| (format!("f{i}"), Self::plain_field(model)))
            .collect();
        let batch_id = self.mint("frame");
        let mut states: Vec<String> = Vec::new();
        let mut request = json!({
            "batch_id": batch_id,
            "model": model.model,
            "fields": Value::Object(fields),
            "items": [],
            "options": {"state_text": true},
        });
        let limit = self.describe.limits.max_request_bytes;
        // Add items until the estimated reply crosses the frame, staying
        // inside the item and request limits.
        loop {
            let decoded: ExtractRequest = serde_json::from_value(request.clone())
                .map_err(|e| format!("building the batch: {e}"))?;
            if !decoded.items.is_empty()
                && estimated_reply_bytes(&decoded, model) > REPLY_FRAME_BYTES
            {
                break;
            }
            if states.len() as u64 >= model.max_items
                || request.to_string().len() as u64 + 1024 > limit
            {
                return Ok(Ending::NotApplicable(
                    "no batch within max_items and max_request_bytes reaches the reply frame"
                        .into(),
                ));
            }
            let state = self.mint("frame");
            request["items"]
                .as_array_mut()
                .expect("items is an array")
                .push(json!({"state": state}));
            states.push(state);
        }
        let refs: Vec<&str> = states.iter().map(String::as_str).collect();
        self.refused(
            "a batch whose reply crosses the frame",
            &request,
            &refs,
            errors::INVALID_PARAMS,
            Some("items"),
            Some(REPLY_FRAME_BYTES),
        )
        .await?;
        Ok(Ending::Passed)
    }

    async fn mapping(&self) -> Check {
        let model = self.model()?;
        let served = self
            .served()
            .into_iter()
            .find(|served| served.model == model.model)
            .ok_or("the first model is not served")?;
        let first =
            json!({"type": model.field_types[0].as_str(), "instructions": "The first value."});
        let second = if Self::serves(model, types::NUMBER) {
            json!({"type": "number", "decimals": 2})
        } else {
            Self::plain_field(model)
        };
        let (a, b) = (self.mint("map"), self.mint("map"));
        // Built as text, so the fields reach the module in this order:
        // `zeta` before `alpha`, against their sorted order.
        let text = format!(
            r#"{{"batch_id":{},"model":{},"fields":{{"zeta":{first},"alpha":{second}}},"items":[{{"state":{},"reference_date":"2026-03-10"}},{{"state":{}}}],"options":{{"quantiles":true}}}}"#,
            json!(self.mint("map")),
            json!(model.model),
            json!(a),
            json!(b),
        );
        Self::reply_of("an admitted batch", self.send_text(&text).await?)?;
        let decoded: ExtractRequest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for (index, state) in [&a, &b].into_iter().enumerate() {
            let bodies = self.subject.received(state).await;
            let body = bodies
                .first()
                .ok_or_else(|| format!("item {index} never reached the stand-in"))?;
            let got: ProviderRequest = serde_json::from_str(body)
                .map_err(|e| format!("item {index}: the provider request does not decode: {e}"))?;
            let expected =
                ProviderRequest::for_item(&decoded, index, served.provider_model.clone())
                    .expect("the item exists");
            if got != expected {
                return Err(format!(
                    "item {index}: the stand-in received {body}, expected {}",
                    serde_json::to_string(&expected).expect("encodes")
                ));
            }
            let order: Vec<&String> = got.fields.keys().collect();
            if order != ["zeta", "alpha"] {
                return Err(format!(
                    "item {index}: fields reached the provider in the order {order:?}"
                ));
            }
        }
        Ok(Ending::Passed)
    }

    async fn reuse(&self) -> Check {
        let model = self.model()?;
        let batch_id = self.mint("reuse");
        let (a, b) = (self.mint("reuse"), self.mint("reuse"));
        let original = Self::base(model, &batch_id, &[&a, &b]);
        Self::reply_of("the first call", self.send(&original).await?)?;
        let fresh = self.mint("reuse");
        let changed = self.mint("reuse");
        let edits: [(&str, Value); 4] = [
            ("fields", {
                let mut request = original.clone();
                request["fields"]["extra"] = Self::plain_field(model);
                request
            }),
            ("options", {
                let mut request = original.clone();
                request["options"] = json!({"quantiles": true});
                request
            }),
            ("items", {
                let mut request = original.clone();
                request["items"]
                    .as_array_mut()
                    .expect("items")
                    .push(json!({"state": fresh}));
                request
            }),
            ("items[1]", {
                let mut request = original.clone();
                request["items"][1]["state"] = json!(changed);
                request
            }),
        ];
        for (field, request) in edits {
            let what = format!("a re-send with a different {field}");
            let refusal = Self::refusal_of(&what, self.send(&request).await?)?;
            Self::expect_refusal(&what, &refusal, errors::BATCH_ID_REUSE, Some(field), None)?;
        }
        for state in [&fresh, &changed] {
            if self.calls(state).await != 0 {
                return Err("a refused re-send reached the stand-in".into());
            }
        }
        Self::reply_of("the original body again", self.send(&original).await?)?;
        Ok(Ending::Passed)
    }

    async fn image_spellings(&self) -> Check {
        let model = self.model()?;
        if model.images.is_none() {
            return Ok(Ending::NotApplicable("the model takes no images".into()));
        }
        let batch_id = self.mint("spell");
        let mut request = Self::base(model, &batch_id, &[""]);
        request["items"][0]["images"] = json!([pixel_url()]);
        Self::reply_of("a data: URL image", self.send(&request).await?)?;
        request["items"][0]["images"] = json!([{
            "content_type": "image/png",
            "base64": PIXEL_BASE64.trim_end_matches('='),
        }]);
        Self::reply_of(
            "the same image as {content_type, base64} under the same batch_id",
            self.send(&request).await?,
        )?;
        Ok(Ending::Passed)
    }

    async fn zero_ceiling(&self) -> Check {
        let Some(model) = self.describe.models.iter().find(|entry| {
            entry.is_priced()
                && self
                    .served()
                    .iter()
                    .any(|served| served.model == entry.model)
        }) else {
            return Ok(Ending::NotApplicable(
                "the module serves no priced model".into(),
            ));
        };
        let (a, b) = (self.mint("zero"), self.mint("zero"));
        let mut request = Self::base(model, &self.mint("zero"), &[&a, &b]);
        request["max_cost_usd"] = json!(0);
        let reply = Self::reply_of("a batch with max_cost_usd 0", self.send(&request).await?)?;
        for (index, item) in reply.items.iter().enumerate() {
            let error = item
                .error
                .as_ref()
                .ok_or_else(|| format!("item {index} has no error under a zero ceiling"))?;
            if error.code != errors::COST_EXCEEDED || error.class != ErrorClass::Permanent {
                return Err(format!(
                    "item {index} failed {} ({}), expected cost_exceeded, permanent",
                    error.code, error.class
                ));
            }
        }
        if reply.items.len() != 2 {
            return Err(format!("{} items answer 2", reply.items.len()));
        }
        for state in [&a, &b] {
            let calls = self.calls(state).await;
            if calls != 0 {
                return Err(format!(
                    "the stand-in was called {calls} times under a zero ceiling"
                ));
            }
        }
        Ok(Ending::Passed)
    }

    // ---- replies ----------------------------------------------------------

    /// Send the request probe `name` captured, on `model`, with `options`
    /// (JSON text), after scripting the stand-in to answer its state with
    /// the captured reply. Returns the probe and the module's reply, which
    /// has exactly one item, or why the check does not apply to this
    /// model.
    async fn replay(
        &self,
        name: &str,
        model: &ModelEntry,
        options: &str,
    ) -> Result<Result<(Probe, ExtractReply), String>, String> {
        let probe = probe(name)?;
        let sent = probe
            .sent
            .as_ref()
            .ok_or_else(|| format!("{name} carries no request to send"))?;
        let fields: IndexMap<String, Field> = serde_json::from_str(sent.fields.get())
            .map_err(|e| format!("{name}: its fields do not decode: {e}"))?;
        if let Some(kind) = fields
            .values()
            .map(|field| &field.kind)
            .find(|kind| !model.field_types.contains(kind))
        {
            return Ok(Err(format!("the model serves no {kind} fields")));
        }
        if fields.len() as u64 > model.max_fields {
            return Ok(Err(format!(
                "the model takes {} fields, {name} sends {}",
                model.max_fields,
                fields.len()
            )));
        }
        let expected = expected_item(&probe);
        if let Some((id, size)) = expected
            .answers
            .iter()
            .flatten()
            .map(|(id, answer)| (id, serde_json::to_string(answer).map_or(0, |a| a.len())))
            .find(|(_, size)| *size as u64 > model.max_answer_bytes)
        {
            return Ok(Err(format!(
                "{name}'s answer {id} is {size} bytes, over max_answer_bytes {}",
                model.max_answer_bytes
            )));
        }
        let text = format!(
            r#"{{"batch_id":{},"model":{},"fields":{},"items":[{{"state":{}}}],"options":{options}}}"#,
            json!(self.mint(name)),
            json!(model.model),
            sent.fields.get(),
            json!(sent.state),
        );
        if text.len() as u64 > self.describe.limits.max_request_bytes {
            return Ok(Err(format!("{name}'s request is over max_request_bytes")));
        }
        self.subject
            .script(
                &sent.state,
                Scripted {
                    status: probe.status,
                    body: probe.body.clone(),
                },
            )
            .await;
        let reply = Self::reply_of(name, self.send_text(&text).await?)?;
        if reply.items.len() != 1 {
            return Err(format!("{name}: {} items answer 1", reply.items.len()));
        }
        if self.calls(&sent.state).await == 0 {
            return Err(format!("{name}: the item never reached the stand-in"));
        }
        Ok(Ok((probe, reply)))
    }

    async fn answers_forwarded(&self) -> Check {
        let model = self.model()?;
        let name = "01-all-types";
        let (probe, reply) = match self
            .replay(name, model, r#"{"quantiles":true,"state_text":true}"#)
            .await?
        {
            Ok(replayed) => replayed,
            Err(why) => return Ok(Ending::NotApplicable(why)),
        };
        let item = &reply.items[0];
        let expected = expected_item(&probe);
        let want = expected.answers.as_ref().expect("probe 01 is answered");
        let got = item
            .answers
            .as_ref()
            .ok_or_else(|| format!("{name}: the item was not answered: {:?}", item.error))?;
        for (id, answer) in want {
            match got.get(id) {
                None => {
                    return Err(format!(
                        "{name}: the {} answer {id} is missing",
                        answer.kind
                    ))
                }
                Some(forwarded) if forwarded != answer => {
                    return Err(format!(
                        "{name}: the {} answer {id} is {}, the provider answered {}",
                        answer.kind,
                        serde_json::to_string(forwarded).unwrap_or_default(),
                        serde_json::to_string(answer).unwrap_or_default(),
                    ))
                }
                Some(_) => {}
            }
        }
        if got.len() != want.len() {
            return Err(format!(
                "{name}: {} answers, the provider gave {}",
                got.len(),
                want.len()
            ));
        }
        if item.truncated != expected.truncated {
            return Err(format!(
                "{name}: truncated is {:?}, the provider said {:?}",
                item.truncated, expected.truncated
            ));
        }
        if item.usage != expected.usage {
            return Err(format!(
                "{name}: usage is {:?}, the provider reported {:?}",
                item.usage, expected.usage
            ));
        }
        Ok(Ending::Passed)
    }

    async fn spans_unconverted(&self) -> Check {
        let model = self.model()?;
        let name = "02-span-offsets";
        let (probe, reply) = match self.replay(name, model, r#"{"state_text":true}"#).await? {
            Ok(replayed) => replayed,
            Err(why) => return Ok(Ending::NotApplicable(why)),
        };
        let item = &reply.items[0];
        let state = &probe.sent.as_ref().expect("replayed with its state").state;
        let answer = item
            .answers
            .as_ref()
            .and_then(|answers| answers.get("tracking_code"))
            .ok_or_else(|| format!("{name}: no tracking_code answer: {:?}", item.error))?;
        let span = answer
            .span
            .as_deref()
            .ok_or_else(|| format!("{name}: the tracking_code answer has no span"))?;
        let offsets: Option<[u64; 2]> = serde_json::from_str(span.get()).ok();
        if offsets != Some([53, 61]) {
            return Err(format!(
                "{name}: the span is {}, expected [53,61] as the provider wrote it, in Unicode scalar values",
                span.get()
            ));
        }
        if item.state_text.as_deref() != Some(state.as_str()) {
            return Err(format!(
                "{name}: state_text is {:?}, expected the state byte for byte",
                item.state_text
            ));
        }
        Ok(Ending::Passed)
    }

    async fn errors_mapped(&self) -> Check {
        let model = self.model()?;
        let cases = [
            ("08-unknown-type", 5006),
            ("09-unknown-option", 3030),
            ("11-empty-state", 3030),
        ];
        let states: Vec<String> = cases.iter().map(|_| self.mint("error")).collect();
        for ((name, _), state) in cases.iter().zip(&states) {
            let probe = probe(name)?;
            self.subject
                .script(
                    state,
                    Scripted {
                        status: probe.status,
                        body: probe.body,
                    },
                )
                .await;
        }
        let states_ref: Vec<&str> = states.iter().map(String::as_str).collect();
        let request = Self::base(model, &self.mint("error"), &states_ref);
        let reply = Self::reply_of(
            "a batch whose items the provider refuses",
            self.send(&request).await?,
        )?;
        if reply.items.len() != cases.len() {
            return Err(format!(
                "{} items answer {}",
                reply.items.len(),
                cases.len()
            ));
        }
        for ((index, item), (name, code)) in reply.items.iter().enumerate().zip(cases) {
            let error = item
                .error
                .as_ref()
                .ok_or_else(|| format!("item {index} ({name}) has no error"))?;
            if error.code != errors::INVALID_ITEM || error.class != ErrorClass::Permanent {
                return Err(format!(
                    "item {index} ({name}) failed {} ({}), expected invalid_item, permanent",
                    error.code, error.class
                ));
            }
            if !error.message.contains(&code.to_string()) {
                return Err(format!(
                    "item {index} ({name}): the message does not carry Workers AI code {code}: {}",
                    error.message
                ));
            }
            if item.answers.is_some() {
                return Err(format!("item {index} ({name}) has answers and an error"));
            }
            if item
                .usage
                .as_ref()
                .is_some_and(|usage| usage.input_tokens.is_some())
            {
                return Err(format!(
                    "item {index} ({name}) reports input_tokens the provider never sent"
                ));
            }
        }
        Ok(Ending::Passed)
    }

    async fn cost_from_usage(&self) -> Check {
        let served = self.served();
        let Some(model) = self.describe.models.iter().find(|entry| {
            entry.is_priced()
                && Self::serves(entry, types::LIST)
                && served.iter().any(|served| served.model == entry.model)
        }) else {
            return Ok(Ending::NotApplicable(
                "the module serves no priced model that takes list fields".into(),
            ));
        };
        let rate = model
            .price
            .as_ref()
            .and_then(|price| price.input_per_mtok_usd.as_ref())
            .and_then(|usd| usd_to_nanos(usd, Rounding::Up))
            .and_then(|nanos| i64::try_from(nanos).ok())
            .ok_or("the model's input price is not a number of USD")?;
        let name = "03-list-decimals";
        let (probe, reply) = match self.replay(name, model, "{}").await? {
            Ok(replayed) => replayed,
            Err(why) => return Ok(Ending::NotApplicable(why)),
        };
        let item = &reply.items[0];
        let reported = expected_item(&probe)
            .usage
            .and_then(|usage| usage.input_tokens)
            .expect("probe 03 reports input_tokens");
        if item.usage.as_ref().and_then(|usage| usage.input_tokens) != Some(reported) {
            return Err(format!(
                "{name}: usage is {:?}, the provider reported input_tokens {reported}",
                item.usage
            ));
        }
        let expected = cost_nanos(reported, rate).map_err(|e| e.to_string())?;
        // The batch has one item, answered, so its cost is that item's.
        let cost = reply
            .cost_usd
            .as_ref()
            .ok_or_else(|| format!("{name}: the reply has no cost_usd"))?;
        let nanos = usd_to_nanos(cost, Rounding::Up)
            .ok_or_else(|| format!("{name}: cost_usd {cost} is not a number of USD"))?;
        if nanos != expected {
            return Err(format!(
                "{name}: cost_usd is {cost}, expected {} USD: the reported input_tokens {reported} at the input price",
                nanos_to_usd_text(expected)
            ));
        }
        Ok(Ending::Passed)
    }
}
