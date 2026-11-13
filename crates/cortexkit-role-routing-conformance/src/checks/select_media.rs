//! Non-llm selection uses the same operation, but a different answer contract.

use super::*;
use crate::wire::select_engine as engine;

const MEDIA: &str = "example-media";

fn entry(model: &str, local: bool) -> Value {
    json!({"kind":"transcribe","provider":fixture::SUBSCRIPTION_PROVIDER,
        "model":model,"local":local,"formats":["audio/wav"],"latency_class":"interactive"})
}

fn module(engines: Vec<Value>) -> CatalogModule {
    CatalogModule {
        principal: MEDIA.into(),
        nonce_verified: true,
        role_declarations: [(
            engine::ROLE_DECLARATIONS_KEY.into(),
            json!({"engines":engines}),
        )]
        .into(),
    }
}

fn request() -> Value {
    json!({"consumer":SUMMARY,"kind":"transcribe","requirements":{},"privacy":"remote_allowed"})
}

async fn selected<R: RoutingRoute>(route: &R, params: &Value) -> Result<Value, String> {
    let value: Value = answer(route, ops::ROUTE_SELECT, params, "media select").await?;
    let decoded: crate::wire::select_media::SelectMediaAnswer =
        serde_json::from_value(value.clone()).map_err(|e| format!("media answer: {e}"))?;
    let request = crate::wire::select_media::parse_select_media_request(
        params,
        capabilities::KNOWN,
        &["transcribe", "video"],
    )
    .map_err(|e| e.to_string())?;
    crate::wire::select_media::check_answer(&request, &decoded).map_err(|e| e.to_string())?;
    Ok(value)
}

async fn invalid<R: RoutingRoute>(
    route: &R,
    params: &Value,
    code: RefusalCode,
    field: &str,
) -> Check {
    let refusal = refused(route, ops::ROUTE_SELECT, params, code, "media validation").await?;
    if refusal.detail.get("field") != Some(&json!(field)) {
        return Err(format!("expected field {field}, got {:?}", refusal.detail));
    }
    Ok(())
}

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn select_media_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        if !describe.serves("select_media") {
            return Ok(Ending::NotApplicable(
                "role.describe does not declare select_media".into(),
            ));
        }
        if !describe.serves("select_kind") {
            return Err("select_media requires select_kind".into());
        }
        if describe.kinds.iter().any(|kind| kind.as_str() == "llm") {
            return Err("select_media kinds must list only non-llm kinds".into());
        }
        let kind = if name == "select_media_video" {
            "video"
        } else {
            "transcribe"
        };
        if name != "select_media_llm_refuses_media"
            && !describe.kinds.iter().any(|listed| listed.as_str() == kind)
        {
            return Ok(Ending::NotApplicable(format!(
                "select_media does not serve {kind}"
            )));
        }
        let required_fields: &[&str] = match name {
            "select_media_video" => &["output_duration_ms", "aspect_ratio", "audio"],
            "select_media_metric_limit" | "select_media_ranking" => &["metric_limit"],
            _ => &[],
        };
        if let Some(field) = required_fields.iter().find(|field| {
            !describe
                .requirement_fields
                .iter()
                .any(|listed| listed == **field)
        }) {
            return Ok(Ending::NotApplicable(format!(
                "router does not read {field}"
            )));
        }
        self.subject
            .configure_catalog_declarations(&[module(vec![entry("model-c", false)])])
            .await?;
        let mut params = request();
        match name {
            "select_media_ungraded_level" => self.media_ungraded_level(&route, &describe).await?,
            "select_media_privacy" => {
                params.as_object_mut().unwrap().remove("privacy");
                invalid(&route, &params, RefusalCode::InvalidRequest, "privacy").await?;
                params["privacy"] = json!("local_only");
                refused(
                    &route,
                    ops::ROUTE_SELECT,
                    &params,
                    RefusalCode::NoRoute,
                    "remote engine violates local_only",
                )
                .await?;
                params["caller_local"] = json!([entry("caller", true)]);
                if selected(&route, &params).await?["engine"]["caller_local"] != true {
                    return Err("local-only request did not select offered local engine".into());
                }
                params["caller_local"][0]["local"] = json!(false);
                invalid(
                    &route,
                    &params,
                    RefusalCode::InvalidRequest,
                    "caller_local[0].local",
                )
                .await?;
            }
            "select_media_video" => {
                let mut video = entry("model-c", false);
                video["kind"] = json!("video");
                self.subject
                    .configure_catalog_declarations(&[module(vec![video])])
                    .await?;
                let mut facts = crate::SelectionFacts::new();
                let mut media = crate::MediaRouteFacts::new();
                media.max_output_duration_ms = Some(1000);
                media.aspect_ratios = vec!["16:9".into()];
                media.audio = Some(true);
                facts
                    .media
                    .insert(format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER), media);
                self.subject.configure_selection_facts(&facts).await?;
                params["kind"] = json!("video");
                params["requirements"] =
                    json!({"output_duration_ms":1000,"aspect_ratio":"16:9","audio":true});
                selected(&route, &params).await?;
                for (field, value) in [
                    ("output_duration_ms", json!(1001)),
                    ("aspect_ratio", json!("4:3")),
                    ("audio", json!(false)),
                ] {
                    let mut bad = params.clone();
                    bad["requirements"][field] = value;
                    refused(
                        &route,
                        ops::ROUTE_SELECT,
                        &bad,
                        RefusalCode::NoRoute,
                        "unmet video requirement",
                    )
                    .await?;
                }
                self.subject
                    .configure_selection_facts(&crate::SelectionFacts::new())
                    .await?;
                refused(
                    &route,
                    ops::ROUTE_SELECT,
                    &params,
                    RefusalCode::NoRoute,
                    "missing video facts",
                )
                .await?;
            }
            "select_media_ranking" => {
                if !describe.serves(capabilities::ENGINE_EVALS) {
                    return Ok(Ending::NotApplicable("engine_evals not declared".into()));
                }
                self.subject
                    .configure_catalog_declarations(&[module(vec![
                        entry("model-c", false),
                        entry("model-d", false),
                    ])])
                    .await?;
                let mut facts = crate::SelectionFacts::new();
                for model in ["model-c", "model-d"] {
                    let mut media = crate::MediaRouteFacts::new();
                    media.grade = Some(400);
                    facts
                        .media
                        .insert(format!("{}/{model}", fixture::SUBSCRIPTION_PROVIDER), media);
                }
                self.subject.configure_selection_facts(&facts).await?;
                self.subject
                    .configure_engine_facts(&[
                        crate::EngineFacts::new(
                            engine::Kind::Transcribe,
                            fixture::SUBSCRIPTION_PROVIDER,
                            "model-c",
                        )
                        .with_cost(100),
                        crate::EngineFacts::new(
                            engine::Kind::Transcribe,
                            fixture::SUBSCRIPTION_PROVIDER,
                            "model-d",
                        )
                        .with_cost(1),
                    ])
                    .await?;
                let media = self.route_as(MEDIA).await?;
                for (model, score) in [("model-c", 0.9), ("model-d", 0.5)] {
                    let eval = json!({"kind":"transcribe","engine":{"provider":fixture::SUBSCRIPTION_PROVIDER,"model":model},"metric":"wer","score":score,"set":{"id":"canonical","items":10},"measured_at_ms":now_ms()});
                    let _: Value = answer(
                        &media,
                        ops::ROUTE_ENGINE_EVAL,
                        &eval,
                        "media ranking evaluation",
                    )
                    .await?;
                }
                if selected(&route, &params).await?["engine"]["model"] != "model-c" {
                    return Err("equal grades must prefer the better measured result".into());
                }
                facts
                    .media
                    .get_mut(&format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER))
                    .unwrap()
                    .grade = None;
                self.subject.configure_selection_facts(&facts).await?;
                if selected(&route, &params).await?["engine"]["model"] != "model-d" {
                    return Err("ungraded route must rank after graded routes".into());
                }
                params["requirements"] = json!({"metric_limit":{"metric":"wer","at_least":0.8}});
                if selected(&route, &params).await?["engine"]["model"] != "model-c" {
                    return Err("hard metric limit did not filter measured engines".into());
                }
                params["requirements"] = json!({"metric_limit":{"metric":"wer","at_most":0.6}});
                if selected(&route, &params).await?["engine"]["model"] != "model-d" {
                    return Err("at_most metric limit did not filter measured engines".into());
                }
            }
            "select_media_ladder" => {
                let caller = format!("{}/caller", fixture::SUBSCRIPTION_PROVIDER);
                let module = format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER);
                self.subject
                    .configure_consumer_mode(
                        SUMMARY,
                        &ConsumerMode::Ladder {
                            entries: vec![module.clone(), caller.clone()],
                        },
                    )
                    .await?;
                params["caller_local"] = json!([entry("caller", true)]);
                if selected(&route, &params).await?["engine"]["caller_local"] != false {
                    return Err("module ladder entry lost to caller-local demand ranking".into());
                }
                self.subject
                    .configure_consumer_mode(
                        SUMMARY,
                        &ConsumerMode::Ladder {
                            entries: vec![caller, module],
                        },
                    )
                    .await?;
                if selected(&route, &params).await?["engine"]["caller_local"] != true {
                    return Err("caller-local ladder entry was skipped".into());
                }
                params.as_object_mut().unwrap().remove("caller_local");
                if selected(&route, &params).await?["engine"]["model"] != "model-c" {
                    return Err("unoffered caller-local ladder entry was not skipped".into());
                }
            }
            "select_media_requirements" => {
                for (field, wanted, fact, value) in [
                    ("language", json!("en-GB"), "languages", json!(["en"])),
                    ("voice", json!("alto"), "voices", json!(["alto"])),
                    (
                        "format",
                        json!("audio/wav"),
                        "formats",
                        json!(["audio/wav"]),
                    ),
                    ("duration_ms", json!(100), "max_duration_ms", json!(100)),
                    ("input_bytes", json!(100), "max_input_bytes", json!(100)),
                    (
                        "reference_formats",
                        json!(["image/png"]),
                        "reference_formats",
                        json!(["image/png"]),
                    ),
                    (
                        "bias",
                        json!(true),
                        "bias_limit",
                        json!({"unit":"phrase","max":3}),
                    ),
                    ("size", json!("large"), "sizes", json!(["large"])),
                    (
                        "resolution",
                        json!("1080p"),
                        "resolutions",
                        json!(["1080p"]),
                    ),
                    ("quality", json!("high"), "qualities", json!(["high"])),
                    (
                        "latency_class",
                        json!("interactive"),
                        "latency_class",
                        json!("realtime"),
                    ),
                    ("metric", json!("wer"), "formats", json!(["audio/wav"])),
                ] {
                    let mut engine = entry("model-c", false);
                    engine[fact] = value;
                    self.subject
                        .configure_catalog_declarations(&[module(vec![engine])])
                        .await?;
                    params["requirements"] = json!({field:wanted});
                    if !describe
                        .requirement_fields
                        .iter()
                        .any(|listed| listed == field)
                    {
                        invalid(&route, &params, RefusalCode::InvalidRequirements, field).await?;
                        continue;
                    }
                    selected(&route, &params)
                        .await
                        .map_err(|e| format!("{field}: {e}"))?;
                    if field != "metric" {
                        let mut mismatch = entry("model-c", false);
                        if field == "format" {
                            mismatch["formats"] = json!(["audio/flac"]);
                        }
                        if field == "latency_class" {
                            mismatch["latency_class"] = json!("batch");
                        }
                        self.subject
                            .configure_catalog_declarations(&[module(vec![mismatch])])
                            .await?;
                        refused(
                            &route,
                            ops::ROUTE_SELECT,
                            &params,
                            RefusalCode::NoRoute,
                            "missing or mismatched engine fact",
                        )
                        .await?;
                    }
                }
            }
            "select_media_llm_refuses_media" => {
                for (field, value) in [
                    ("language", json!("en")),
                    ("voice", json!("alto")),
                    ("format", json!("audio/wav")),
                    ("duration_ms", json!(1)),
                    ("input_bytes", json!(1)),
                    ("reference_formats", json!([])),
                    ("bias", json!(true)),
                    ("size", json!("large")),
                    ("resolution", json!("1080p")),
                    ("quality", json!("high")),
                    ("latency_class", json!("batch")),
                    ("metric", json!("wer")),
                    ("output_duration_ms", json!(1)),
                    ("aspect_ratio", json!("16:9")),
                    ("audio", json!(true)),
                    ("metric_limit", json!({"metric":"wer","at_most":0.1})),
                ] {
                    let mut llm = base();
                    llm["requirements"][field] = value;
                    invalid(&route, &llm, RefusalCode::InvalidRequirements, field).await?;
                }
                for (field, value) in [
                    ("privacy", json!("local_only")),
                    ("caller_local", json!([])),
                ] {
                    let mut llm = base();
                    llm[field] = value;
                    invalid(&route, &llm, RefusalCode::InvalidRequest, field).await?;
                }
            }
            "select_media_refuses_llm" => {
                for (field, value) in [
                    ("eq", json!(0)),
                    ("context_window", json!(1)),
                    ("max_output_tokens", json!(1)),
                    ("tools", json!("none")),
                    ("tool_capabilities", json!([])),
                    ("modalities", json!([])),
                    ("structured_output", json!(true)),
                    ("questions", json!(1)),
                    ("input_tokens", json!(1)),
                    ("max_cost_usd", json!(1)),
                ] {
                    params["requirements"] = json!({"iq":0, field:value});
                    invalid(&route, &params, RefusalCode::InvalidRequirements, field).await?;
                }
            }
            "select_media_metric_limit" => {
                for limit in [
                    json!({"metric":"wer"}),
                    json!({"metric":"wer","at_most":0.2,"at_least":0.1}),
                    json!({"metric":"wer","at_most":"NaN"}),
                ] {
                    params["requirements"] = json!({"metric_limit":limit});
                    invalid(
                        &route,
                        &params,
                        RefusalCode::InvalidRequirements,
                        "metric_limit",
                    )
                    .await?;
                }
                for bound in ["at_most", "at_least"] {
                    params["requirements"] = json!({"metric_limit":{"metric":"wer",bound:0.5}});
                    refused(
                        &route,
                        ops::ROUTE_SELECT,
                        &params,
                        RefusalCode::NoRoute,
                        "missing evaluation fails hard limit",
                    )
                    .await?;
                }
            }
            "select_media_unlisted_kind" => {
                selected(&route, &params).await?;
                params["kind"] = json!("future_kind");
                invalid(&route, &params, RefusalCode::InvalidRequest, "kind").await?;
            }
            "select_media_answer" => {
                let before = now_ms();
                let value = selected(&route, &params).await?;
                let after = now_ms();
                let expiry = value["expires_at_ms"].as_u64().ok_or("missing expiry")?;
                if expiry < before + engine::DECISION_TTL_MS
                    || expiry > after + engine::DECISION_TTL_MS
                    || value["engine"]["module"] != MEDIA
                    || value["engine"]["caller_local"] != false
                {
                    return Err("wrong media engine identity or expiry".into());
                }
                if serde_json::from_value::<SelectAnswer>(value).is_ok() {
                    return Err(
                        "llm answer accepted a media answer without its required members".into(),
                    );
                }
                let llm: Value = answer(&route, ops::ROUTE_SELECT, &base(), "llm answer").await?;
                for field in [
                    "route",
                    "runner",
                    "context_window",
                    "max_output_tokens",
                    "explanation",
                ] {
                    let mut bad = llm.clone();
                    bad.as_object_mut().unwrap().remove(field);
                    if serde_json::from_value::<SelectAnswer>(bad).is_ok() {
                        return Err(format!("llm answer no longer requires {field}"));
                    }
                }
            }
            "select_media_pin" => {
                params["pin"] = json!(format!("{}/caller", fixture::SUBSCRIPTION_PROVIDER));
                params["caller_local"] = json!([entry("caller", true)]);
                let value = selected(&route, &params).await?;
                if value["engine"]["caller_local"] != true || value["engine"]["index"] != 0 {
                    return Err("pin did not select caller-local engine".into());
                }
                params.as_object_mut().unwrap().remove("caller_local");
                refused(
                    &route,
                    ops::ROUTE_SELECT,
                    &params,
                    RefusalCode::PinUnavailable,
                    "missing caller-local pin",
                )
                .await?;
                params["pin"] = json!(format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER));
                params["caller_local"] = json!([entry("model-c", true)]);
                if selected(&route, &params).await?["engine"]["caller_local"] != false {
                    return Err("module pin selected caller-local engine".into());
                }
            }
            "select_media_price" => {
                let mut facts = crate::SelectionFacts::new();
                let mut media = crate::MediaRouteFacts::new();
                media.price = Some(serde_json::from_value(json!({"basis":"metered","rates":{"input.per_audio_second":{"rate":2,"step":1,"minimum":0},"input.per_character":{"rate":0.01},"output.per_video_second":{"rate":4}}})).unwrap());
                facts
                    .media
                    .insert(format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER), media);
                self.subject.configure_selection_facts(&facts).await?;
                let value = selected(&route, &params).await?;
                if value["price"]["rates"]["input.per_audio_second"]["rate"] != 2
                    || value["price"]["rates"].get("output.per_image").is_some()
                {
                    return Err("unit price keys were renamed or invented".into());
                }
                let price: crate::wire::select_media::UnitPrice =
                    serde_json::from_value(value["price"].clone()).map_err(|e| e.to_string())?;
                if price.rate("input.per_second").is_some()
                    || price.rate("output.per_image").is_some()
                {
                    return Err("unmatched unit borrowed another rate".into());
                }
                self.subject
                    .configure_selection_facts(&crate::SelectionFacts::new())
                    .await?;
                if selected(&route, &params).await?.get("price").is_some() {
                    return Err("unpriced job must omit price".into());
                }
            }
            "select_media_demand" | "select_media_policy" => {
                self.subject
                    .configure_catalog_declarations(&[module(vec![
                        entry("model-c", false),
                        entry("model-d", false),
                    ])])
                    .await?;
                let mut facts = crate::SelectionFacts::new();
                for (model, grade) in [("model-c", 400), ("model-d", 200)] {
                    let mut media = crate::MediaRouteFacts::new();
                    media.grade = Some(grade);
                    facts
                        .media
                        .insert(format!("{}/{model}", fixture::SUBSCRIPTION_PROVIDER), media);
                }
                self.subject.configure_selection_facts(&facts).await?;
                self.subject
                    .configure_engine_facts(&[
                        crate::EngineFacts::new(
                            engine::Kind::Transcribe,
                            fixture::SUBSCRIPTION_PROVIDER,
                            "model-c",
                        )
                        .with_cost(100),
                        crate::EngineFacts::new(
                            engine::Kind::Transcribe,
                            fixture::SUBSCRIPTION_PROVIDER,
                            "model-d",
                        )
                        .with_cost(1),
                    ])
                    .await?;
                if name == "select_media_policy" {
                    params["exclude"] =
                        json!([format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER)]);
                } else {
                    params["requirements"]["iq"] = json!(50);
                }
                if selected(&route, &params).await?["engine"]["model"] != "model-d" {
                    return Err("media demand or policy ignored".into());
                }
                if name == "select_media_demand" {
                    params["requirements"] = json!({});
                    if selected(&route, &params).await?["engine"]["model"] != "model-c" {
                        return Err("no level must select highest grade before cost".into());
                    }
                    params["requirements"]["iq"] = json!(51);
                    if selected(&route, &params).await?["engine"]["model"] != "model-c" {
                        return Err("relative grade floor was not applied".into());
                    }
                    if describe.serves(capabilities::CONSUMER_DECLARATIONS) {
                        let declared = format!("{CONSUMER}.media");
                        let declaration = CatalogModule {
                            principal: CONSUMER.into(),
                            nonce_verified: true,
                            role_declarations: [(
                                crate::wire::ROLE_DECLARATIONS_KEY.into(),
                                json!({"consumers":[{"name":declared,"iq":51}]}),
                            )]
                            .into(),
                        };
                        self.subject
                            .configure_catalog_declarations(&[
                                module(vec![entry("model-c", false), entry("model-d", false)]),
                                declaration,
                            ])
                            .await?;
                        params["consumer"] = json!(declared);
                        params["requirements"] = json!({});
                        if selected(&route, &params).await?["engine"]["model"] != "model-c" {
                            return Err("declared default iq was ignored".into());
                        }
                    }
                    params["requirements"]["iq"] = json!(0);
                    if selected(&route, &params).await?["engine"]["model"] != "model-d" {
                        return Err("explicit zero did not admit the cheaper lower grade".into());
                    }
                    let mut caller = crate::MediaRouteFacts::new();
                    caller.grade = Some(200);
                    facts
                        .media
                        .insert(format!("{}/caller", fixture::SUBSCRIPTION_PROVIDER), caller);
                    self.subject.configure_selection_facts(&facts).await?;
                    self.subject
                        .configure_engine_facts(&[
                            crate::EngineFacts::new(
                                engine::Kind::Transcribe,
                                fixture::SUBSCRIPTION_PROVIDER,
                                "model-c",
                            )
                            .with_cost(100),
                            crate::EngineFacts::new(
                                engine::Kind::Transcribe,
                                fixture::SUBSCRIPTION_PROVIDER,
                                "model-d",
                            )
                            .with_cost(1),
                            crate::EngineFacts::new(
                                engine::Kind::Transcribe,
                                fixture::SUBSCRIPTION_PROVIDER,
                                "caller",
                            )
                            .with_cost(0),
                        ])
                        .await?;
                    params["caller_local"] = json!([entry("caller", true)]);
                    if selected(&route, &params).await?["engine"]["caller_local"] != true {
                        return Err(
                            "caller-local engine did not participate in demand ranking".into()
                        );
                    }
                } else {
                    if describe.serves(capabilities::SELECT_ELIGIBLE_ROUTES) {
                        params["eligible_routes"] = json!([]);
                        refused(
                            &route,
                            ops::ROUTE_SELECT,
                            &params,
                            RefusalCode::NoRoute,
                            "empty eligible subset",
                        )
                        .await?;
                        params.as_object_mut().unwrap().remove("eligible_routes");
                    }
                    if describe.serves(capabilities::SELECT_EXCLUDE_PROVIDERS) {
                        params["exclude_providers"] = json!([fixture::SUBSCRIPTION_PROVIDER]);
                        refused(
                            &route,
                            ops::ROUTE_SELECT,
                            &params,
                            RefusalCode::NoRoute,
                            "provider exclusion",
                        )
                        .await?;
                        params.as_object_mut().unwrap().remove("exclude_providers");
                    }
                    facts
                        .media
                        .get_mut(&format!("{}/model-d", fixture::SUBSCRIPTION_PROVIDER))
                        .unwrap()
                        .not_for = vec![SUMMARY.into()];
                    self.subject.configure_selection_facts(&facts).await?;
                    refused(
                        &route,
                        ops::ROUTE_SELECT,
                        &params,
                        RefusalCode::NoRoute,
                        "not_for excludes remaining module engine",
                    )
                    .await?;
                }
            }
            "select_media_report_admission" => {
                let mut other = module(vec![entry("model-c", false)]);
                other.principal = "other-media".into();
                self.subject
                    .configure_catalog_declarations(&[module(vec![entry("model-c", false)]), other])
                    .await?;
                let selected = selected(&route, &params).await?;
                let report = json!({"decision_id":selected["decision_id"],"outcome":"completed","attempt_outcome":{"status":"served"}});
                let _: Value = answer(&route, ops::ROUTE_REPORT, &report, "media report").await?;
                let foreign_consumer = self.route_as(fixture::OTHER_CONSUMER).await?;
                refused(
                    &foreign_consumer,
                    ops::ROUTE_REPORT,
                    &report,
                    RefusalCode::UnknownDecision,
                    "foreign media report",
                )
                .await?;
                let media = self.route_as(MEDIA).await?;
                let admission = json!({"decision_id":selected["decision_id"],"provider":fixture::SUBSCRIPTION_PROVIDER,"model":"model-c","admission_id":"media-admission","mode":"new"});
                let _: Value = answer(
                    &media,
                    ops::ROUTE_ACCOUNT_FOR,
                    &admission,
                    "media admission",
                )
                .await?;
                let other = self.route_as("other-media").await?;
                refused(
                    &other,
                    ops::ROUTE_ACCOUNT_FOR,
                    &admission,
                    RefusalCode::UnknownDecision,
                    "media decision module binding",
                )
                .await?;
                self.subject.advance_clock(engine::DECISION_TTL_MS).await?;
                let mut expired = admission.clone();
                expired["admission_id"] = json!("expired-media-admission");
                refused(
                    &media,
                    ops::ROUTE_ACCOUNT_FOR,
                    &expired,
                    RefusalCode::UnknownDecision,
                    "expired media decision",
                )
                .await?;
                let _: Value = answer(
                    &media,
                    ops::ROUTE_ACCOUNT_FOR,
                    &admission,
                    "media admission replay after expiry",
                )
                .await?;
            }
            _ => unreachable!(),
        }
        Ok(Ending::Passed)
    }

    async fn media_ungraded_level(&self, route: &S::Route, describe: &RoleDescribe) -> Check {
        let provider = fixture::SUBSCRIPTION_PROVIDER;
        let costs = |c, d| {
            [
                crate::EngineFacts::new(engine::Kind::Transcribe, provider, "model-c").with_cost(c),
                crate::EngineFacts::new(engine::Kind::Transcribe, provider, "model-d").with_cost(d),
                crate::EngineFacts::new(engine::Kind::Transcribe, provider, "caller").with_cost(0),
            ]
        };
        let expect = |value: Value, model: &str, reason: &str| -> Check {
            if value["engine"]["model"] != model {
                return Err(reason.into());
            }
            Ok(())
        };
        self.subject
            .configure_catalog_declarations(&[module(vec![
                entry("model-c", false),
                entry("model-d", false),
            ])])
            .await?;
        self.subject
            .configure_selection_facts(&crate::SelectionFacts::new())
            .await?;
        self.subject.configure_engine_facts(&costs(10, 1)).await?;
        let mut params = request();
        params["requirements"]["iq"] = json!(75);
        expect(
            selected(route, &params).await?,
            "model-d",
            "an all-ungraded catalog must use cost order under a stated level",
        )?;

        if describe.serves(capabilities::CONSUMER_DECLARATIONS) {
            let declared = format!("{CONSUMER}.media");
            let declaration = CatalogModule {
                principal: CONSUMER.into(),
                nonce_verified: true,
                role_declarations: [(
                    crate::wire::ROLE_DECLARATIONS_KEY.into(),
                    json!({"consumers":[{"name":declared,"iq":25}]}),
                )]
                .into(),
            };
            self.subject
                .configure_catalog_declarations(&[
                    module(vec![entry("model-c", false), entry("model-d", false)]),
                    declaration,
                ])
                .await?;
            let mut defaults_probe = crate::SelectionFacts::new();
            for (model, grade) in [("model-c", 400), ("model-d", 200)] {
                let mut media = crate::MediaRouteFacts::new();
                media.grade = Some(grade);
                defaults_probe
                    .media
                    .insert(format!("{provider}/{model}"), media);
            }
            self.subject
                .configure_selection_facts(&defaults_probe)
                .await?;
            params["consumer"] = json!(declared);
            params["requirements"] = json!({});
            expect(selected(route, &params).await?, "model-d", "the declared level must actually change grade-first selection into cheapest adequate selection")?;
            self.subject
                .configure_selection_facts(&crate::SelectionFacts::new())
                .await?;
            expect(
                selected(route, &params).await?,
                "model-d",
                "a declared level must not exclude an all-ungraded catalog",
            )?;
            params["caller_local"] = json!([entry("caller", true)]);
            expect(
                selected(route, &params).await?,
                "caller",
                "an ungraded caller-local engine must stay eligible under a declared level",
            )?;
        }
        self.subject
            .configure_catalog_declarations(&[module(vec![
                entry("model-c", false),
                entry("model-d", false),
            ])])
            .await?;
        params["consumer"] = json!(SUMMARY);
        params["caller_local"] = json!([entry("caller", true)]);

        let mut facts = crate::SelectionFacts::new();
        params["requirements"]["iq"] = json!(75);
        let mut graded = crate::MediaRouteFacts::new();
        graded.grade = Some(400);
        facts.media.insert(format!("{provider}/model-c"), graded);
        self.subject.configure_selection_facts(&facts).await?;
        expect(
            selected(route, &params).await?,
            "model-c",
            "adequate graded routes must precede cheaper ungraded routes",
        )?;
        let mut below = crate::MediaRouteFacts::new();
        below.grade = Some(200);
        facts.media.insert(format!("{provider}/model-d"), below);
        self.subject.configure_selection_facts(&facts).await?;
        expect(
            selected(route, &params).await?,
            "model-c",
            "a graded route below the level must remain ineligible",
        )?;

        self.subject.configure_catalog_declarations(&[]).await?;
        params["requirements"]["iq"] = json!(75);
        expect(
            selected(route, &params).await?,
            "caller",
            "with no routable graded route, an ungraded caller-local engine must remain selectable",
        )?;

        params.as_object_mut().unwrap().remove("caller_local");
        self.subject
            .configure_catalog_declarations(&[module(vec![
                entry("model-c", false),
                entry("model-d", false),
            ])])
            .await?;
        self.subject
            .configure_selection_facts(&crate::SelectionFacts::new())
            .await?;
        self.subject.configure_engine_facts(&costs(1, 1)).await?;
        expect(
            selected(route, &params).await?,
            "model-c",
            "equal-cost ungraded routes must use deterministic identity order",
        )?;
        if describe.serves(capabilities::ENGINE_EVALS) {
            let media = self.route_as(MEDIA).await?;
            for (model, score) in [("model-c", 0.5), ("model-d", 0.9)] {
                let eval = json!({"kind":"transcribe","engine":{"provider":provider,"model":model},"metric":"wer","score":score,"set":{"id":"ungraded","items":10},"measured_at_ms":now_ms()});
                let _: Value = answer(
                    &media,
                    ops::ROUTE_ENGINE_EVAL,
                    &eval,
                    "ungraded evaluation tie break",
                )
                .await?;
            }
            expect(
                selected(route, &params).await?,
                "model-d",
                "equal-cost ungraded routes must use measured result before identity",
            )?;
            self.subject.configure_engine_facts(&costs(1, 10)).await?;
            expect(
                selected(route, &params).await?,
                "model-c",
                "ungraded expected job cost must precede measured result",
            )?;
        }
        self.subject
            .configure_engine_facts(&[
                crate::EngineFacts::new(engine::Kind::Transcribe, provider, "model-c"),
                crate::EngineFacts::new(engine::Kind::Transcribe, provider, "model-d").with_cost(1),
            ])
            .await?;
        expect(
            selected(route, &params).await?,
            "model-d",
            "known expected cost must precede unknown cost among ungraded routes",
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::select_engine::fake::{Defects, FakeSubject};
    use super::*;

    macro_rules! cases {
        ($($name:ident),* $(,)?) => { $(
            #[tokio::test]
            async fn $name() {
                let subject = if stringify!($name) == "select_media_ungraded_level" {
                    FakeSubject::with_media_declarations(Defects::default())
                } else {
                    FakeSubject::with_media(Defects::default())
                };
                assert_eq!(Suite::new(&subject).run(stringify!($name)).await, Ok(Ending::Passed));
            }
        )* };
    }
    cases!(
        select_media_privacy,
        select_media_requirements,
        select_media_llm_refuses_media,
        select_media_refuses_llm,
        select_media_video,
        select_media_ranking,
        select_media_ladder,
        select_media_metric_limit,
        select_media_unlisted_kind,
        select_media_answer,
        select_media_pin,
        select_media_price,
        select_media_demand,
        select_media_ungraded_level,
        select_media_policy,
        select_media_report_admission
    );

    #[tokio::test]
    async fn select_media_faulty_routers_fail_by_check_name() {
        for name in crate::CHECKS
            .iter()
            .map(|case| case.name)
            .filter(|name| name.starts_with("select_media_"))
        {
            let subject = FakeSubject::with_media(Defects {
                extension: Some(name),
                ..Defects::default()
            });
            let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
            let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
            assert!(
                matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                "{name} accepted an incorrect router: {report:?}"
            );
            assert_eq!(report.check, name);
        }
    }

    #[test]
    fn select_media_rule_map_covers_pinned_rules_and_registered_cases() {
        use std::collections::BTreeSet;
        let contract = include_str!("../../../cortexkit-role-routing/CONTRACT.md");
        let ids: BTreeSet<_> = contract
            .lines()
            .filter_map(|line| {
                line.strip_prefix("- [pinned] `select_media.")
                    .and_then(|tail| tail.split_once("`:"))
            })
            .map(|(id, _)| format!("select_media.{id}"))
            .collect();
        let map: Vec<Value> = serde_json::from_str(include_str!("../../rule-map.json")).unwrap();
        let mapped: Vec<_> = map
            .iter()
            .filter(|rule| rule["id"].as_str().unwrap().starts_with("select_media."))
            .collect();
        let map_ids: BTreeSet<_> = mapped
            .iter()
            .map(|rule| rule["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, map_ids);
        assert_eq!(mapped.len(), map_ids.len());
        let cases: BTreeSet<_> = mapped
            .iter()
            .flat_map(|rule| {
                rule["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|case| case.as_str().unwrap())
            })
            .collect();
        let registered: BTreeSet<_> = crate::CHECKS
            .iter()
            .filter(|spec| spec.name.starts_with("select_media_"))
            .map(|spec| spec.name)
            .collect();
        assert_eq!(cases, registered);
    }

    #[tokio::test]
    async fn select_media_ungraded_order_and_floor_faults_fail_by_check_name() {
        let spec = crate::CHECKS
            .iter()
            .find(|spec| spec.name == "select_media_ungraded_level")
            .unwrap();
        for fault in [
            "select_media_ungraded_cost_first",
            "select_media_ungraded_below_level",
        ] {
            let subject = FakeSubject::with_media(Defects {
                extension: Some(fault),
                ..Defects::default()
            });
            let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
            assert!(
                matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                "{fault}: {report:?}"
            );
            assert_eq!(report.check, "select_media_ungraded_level");
        }
    }
}
