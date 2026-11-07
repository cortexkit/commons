//! Media selection checks use explicit engine facts, never a provider's live catalog.

use super::*;
use crate::{wire::select_engine as engine, EngineFacts};

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../tests/fake/mod.rs"]
mod fake;

const MEDIA: &str = "example-media";
const OTHER_MEDIA: &str = "other-media";

fn entry(model: &str, local: bool) -> Value {
    json!({"kind":"transcribe", "provider":fixture::SUBSCRIPTION_PROVIDER,
        "model":model,"local":local,"formats":["audio/wav"],"latency_class":"interactive"})
}

fn module(principal: &str, engines: Vec<Value>) -> CatalogModule {
    CatalogModule {
        principal: principal.into(),
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

async fn selected<R: RoutingRoute>(
    route: &R,
    params: &Value,
) -> Result<engine::SelectEngineAnswer, String> {
    let value: engine::SelectEngineAnswer =
        answer(route, ops::ROUTE_SELECT_ENGINE, params, "engine selection").await?;
    let parsed = engine::parse_select_engine_request(params).map_err(|e| e.to_string())?;
    engine::check_answer(&parsed, &value).map_err(|e| e.to_string())?;
    Ok(value)
}

async fn expect_engine<R: RoutingRoute>(
    route: &R,
    params: &Value,
    model: &str,
    owner: Option<&str>,
    index: Option<usize>,
) -> Check {
    let value = selected(route, params).await?;
    if value.engine.model != model
        || value.engine.module.as_deref() != owner
        || value.engine.index != index
    {
        return Err(format!(
            "expected {owner:?}/{model} index {index:?}, got {:?}",
            value.engine
        ));
    }
    Ok(())
}

async fn no_engine<R: RoutingRoute>(route: &R, params: &Value, filter: &str) -> Check {
    let refusal = refused(
        route,
        ops::ROUTE_SELECT_ENGINE,
        params,
        RefusalCode::NoEngine,
        "empty engine set",
    )
    .await?;
    if refusal.detail.get("filter") != Some(&json!(filter))
        || refusal.detail.contains_key("reset_at_ms")
        || refusal.detail.contains_key("retry_after_ms")
    {
        return Err(format!(
            "no_engine must name {filter} without a time: {:?}",
            refusal.detail
        ));
    }
    Ok(())
}

async fn admitted<R: RoutingRoute>(route: &R, params: &Value) -> Result<AccountForAnswer, String> {
    let value: AccountForAnswer =
        answer(route, ops::ROUTE_ACCOUNT_FOR, params, "media admission").await?;
    if value.admission.state != "admitted"
        || params["admission_id"] != value.admission.id
        || params["provider"] != value.account.provider
    {
        return Err(format!(
            "media admission did not honour the request: {value:?}"
        ));
    }
    Ok(value)
}

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn select_engine_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        if !describe.serves(capabilities::SELECT_ENGINE) {
            return Ok(Ending::NotApplicable(
                "role.describe does not declare select_engine".into(),
            ));
        }
        if !describe.ops.iter().any(|op| op == ops::ROUTE_SELECT_ENGINE) {
            return Err("select_engine must declare route.select_engine".into());
        }
        let remote = entry("model-c", false);
        self.subject
            .configure_catalog_declarations(&[module(MEDIA, vec![remote.clone()])])
            .await?;
        match name {
            "select_engine_opt_in" => {
                check_describe(&describe).map_err(|e| e.to_string())?;
                selected(&route, &request()).await?;
            }
            "select_engine_slot_malformed"
            | "select_engine_slot_duplicate"
            | "select_engine_slot_empty_formats"
            | "select_engine_slot_unverified" => {
                let mut bad = remote.clone();
                match name {
                    "select_engine_slot_malformed" => bad["local"] = json!("false"),
                    "select_engine_slot_empty_formats" => bad["formats"] = json!([]),
                    _ => {}
                }
                let mut slot = module(MEDIA, vec![remote.clone(), bad]);
                if name == "select_engine_slot_unverified" {
                    slot.nonce_verified = false;
                    slot.role_declarations.insert(
                        engine::ROLE_DECLARATIONS_KEY.into(),
                        json!({"engines":[remote.clone()]}),
                    );
                }
                self.subject.configure_catalog_declarations(&[slot]).await?;
                no_engine(&route, &request(), "kind").await?;
                if name == "select_engine_slot_malformed" {
                    for (field, value) in [
                        ("unknown", json!(true)),
                        ("model", json!("x".repeat(257))),
                        ("bias_limit", json!({"unit":"phrase","max":0})),
                    ] {
                        let mut bad = entry("other-model", false);
                        bad[field] = value;
                        self.subject
                            .configure_catalog_declarations(&[module(
                                MEDIA,
                                vec![remote.clone(), bad],
                            )])
                            .await?;
                        no_engine(&route, &request(), "kind").await?;
                    }
                    let mut bad = entry("realtime-local", true);
                    bad["kind"] = json!("realtime");
                    self.subject
                        .configure_catalog_declarations(&[module(MEDIA, vec![remote.clone(), bad])])
                        .await?;
                    no_engine(&route, &request(), "kind").await?;
                    let mut slot = module(MEDIA, vec![remote]);
                    slot.role_declarations
                        .get_mut(engine::ROLE_DECLARATIONS_KEY)
                        .unwrap()["unknown"] = json!(true);
                    self.subject.configure_catalog_declarations(&[slot]).await?;
                    no_engine(&route, &request(), "kind").await?;
                }
            }
            "select_engine_request" => {
                let mut params = request();
                params["consumer"] = json!(fixture::UNCONFIGURED);
                refused(
                    &route,
                    ops::ROUTE_SELECT_ENGINE,
                    &params,
                    RefusalCode::UnknownConsumer,
                    "unknown consumer",
                )
                .await?;
                let foreign = self.route_as(fixture::OTHER_CONSUMER).await?;
                refused(
                    &foreign,
                    ops::ROUTE_SELECT_ENGINE,
                    &request(),
                    RefusalCode::ConsumerNotOwned,
                    "foreign consumer",
                )
                .await?;
                params = request();
                params["privacy"] = json!("anything");
                self.engine_invalid(&route, &params, RefusalCode::InvalidRequest, "privacy")
                    .await?;
                params = request();
                params["unknown"] = json!(true);
                self.engine_invalid(&route, &params, RefusalCode::InvalidRequest, "unknown")
                    .await?;
                params = request();
                params["consumer"] = json!(fixture::CODEMODE);
                self.subject
                    .configure_catalog_declarations(&[module(MEDIA, vec![entry("local", true)])])
                    .await?;
                expect_engine(&route, &params, "local", Some(MEDIA), None).await?;
                self.subject.configure_catalog_declarations(&[]).await?;
                params["caller_local"] = json!([entry("caller", true)]);
                expect_engine(&route, &params, "caller", None, Some(0)).await?;
            }
            "select_engine_requirements_valid" => {
                for (kind, requirements, field) in [
                    ("transcribe", json!({"unknown":true}), "unknown"),
                    ("realtime", json!({"format":"audio/wav"}), "format"),
                    (
                        "transcribe",
                        json!({"latency_class":"unknown"}),
                        "latency_class",
                    ),
                    ("transcribe", json!({"input_bytes":-1}), "input_bytes"),
                ] {
                    let mut params = request();
                    params["kind"] = json!(kind);
                    params["requirements"] = requirements;
                    self.engine_invalid(&route, &params, RefusalCode::InvalidRequirements, field)
                        .await?;
                }
            }
            "select_engine_caller_local" => {
                let mut params = request();
                params["caller_local"] = json!([remote]);
                self.engine_invalid(
                    &route,
                    &params,
                    RefusalCode::InvalidRequest,
                    "caller_local[0].local",
                )
                .await?;
            }
            "select_engine_filters" => {
                let mut params = request();
                params["kind"] = json!("image");
                params["requirements"] = json!({"language":"fr"});
                params["privacy"] = json!("local_only");
                no_engine(&route, &params, "kind").await?;
                params["kind"] = json!("transcribe");
                no_engine(&route, &params, "requirements").await?;
                params["requirements"] = json!({});
                self.subject
                    .cool_down(
                        &format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER),
                        now_ms() + 3_600_000,
                    )
                    .await?;
                no_engine(&route, &params, "privacy").await?;
                params["privacy"] = json!("remote_allowed");
                no_engine(&route, &params, "reachability").await?;
                self.subject.configure_catalog_declarations(&[]).await?;
                no_engine(&route, &request(), "kind").await?;
            }
            "select_engine_match_requirements" => self.engine_requirements(&route).await?,
            "select_engine_privacy" => {
                self.subject
                    .configure_catalog_declarations(&[module(
                        MEDIA,
                        vec![remote, entry("z-local", true)],
                    )])
                    .await?;
                let mut params = request();
                params["privacy"] = json!("local_only");
                expect_engine(&route, &params, "z-local", Some(MEDIA), None).await?;
                expect_engine(&route, &request(), "model-c", Some(MEDIA), None).await?;
                self.subject
                    .configure_catalog_declarations(&[module(MEDIA, vec![entry("local", true)])])
                    .await?;
                expect_engine(&route, &request(), "local", Some(MEDIA), None).await?;
                self.subject.configure_catalog_declarations(&[]).await?;
                params["caller_local"] = json!([entry("caller", true)]);
                expect_engine(&route, &params, "caller", None, Some(0)).await?;
            }
            "select_engine_reachability" => {
                self.subject
                    .cool_down(
                        &format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER),
                        now_ms() + 3_600_000,
                    )
                    .await?;
                no_engine(&route, &request(), "reachability").await?;
                self.subject
                    .configure_catalog_declarations(&[module(MEDIA, vec![entry("model-c", true)])])
                    .await?;
                expect_engine(&route, &request(), "model-c", Some(MEDIA), None).await?;
                self.subject.configure_catalog_declarations(&[]).await?;
                let mut params = request();
                params["caller_local"] = json!([entry("model-c", true)]);
                expect_engine(&route, &params, "model-c", None, Some(0)).await?;
            }
            "select_engine_answer" => {
                let before = now_ms();
                let value = selected(&route, &request()).await?;
                let after = now_ms();
                if value.expires_at_ms < before + engine::DECISION_TTL_MS
                    || value.expires_at_ms > after + engine::DECISION_TTL_MS
                {
                    return Err("engine expiry must be exactly ten minutes after selection".into());
                }
                if value.engine.local
                    || value.engine.caller_local
                    || value.engine.module.as_deref() != Some(MEDIA)
                    || value.engine.index.is_some()
                    || value.engine.provider != fixture::SUBSCRIPTION_PROVIDER
                    || value.engine.model != "model-c"
                {
                    return Err("wrong remote module engine member matrix".into());
                }
                self.subject
                    .configure_catalog_declarations(&[module(MEDIA, vec![entry("local", true)])])
                    .await?;
                let local = selected(&route, &request()).await?;
                if !local.engine.local || local.engine.caller_local {
                    return Err("wrong local module engine member matrix".into());
                }
                self.subject.configure_catalog_declarations(&[]).await?;
                let mut params = request();
                params["caller_local"] = json!([entry("caller", true)]);
                expect_engine(&route, &params, "caller", None, Some(0)).await?;
            }
            "select_engine_ranking" => self.engine_ranking(&route).await?,
            "select_engine_report" | "select_engine_explain" => {
                let value = selected(&route, &request()).await?;
                if name == "select_engine_report" {
                    let report = json!({"decision_id":value.decision_id,"outcome":"completed"});
                    let recorded: ReportAnswer =
                        answer(&route, ops::ROUTE_REPORT, &report, "engine report").await?;
                    if recorded.decision_id != value.decision_id
                        || recorded.outcome.as_str() != "completed"
                    {
                        return Err("engine report must join decision_id and outcome".into());
                    }
                    refused(
                        &route,
                        ops::ROUTE_REPORT,
                        &json!({"decision_id":value.decision_id}),
                        RefusalCode::InvalidRequest,
                        "report without outcome",
                    )
                    .await?;
                    let foreign = self.route_as(fixture::OTHER_CONSUMER).await?;
                    refused(
                        &foreign,
                        ops::ROUTE_REPORT,
                        &report,
                        RefusalCode::UnknownDecision,
                        "foreign engine report",
                    )
                    .await?;
                } else if describe.serves(capabilities::EXPLAIN) {
                    refused(
                        &route,
                        ops::ROUTE_EXPLAIN,
                        &json!({"decision_id":value.decision_id}),
                        RefusalCode::UnknownDecision,
                        "engine explain",
                    )
                    .await?;
                } else {
                    return Ok(Ending::NotApplicable(
                        "role.describe does not declare explain".into(),
                    ));
                }
            }
            _ => self.engine_admission(name, &route).await?,
        }
        Ok(Ending::Passed)
    }

    async fn engine_invalid(
        &self,
        route: &S::Route,
        params: &Value,
        code: RefusalCode,
        field: &str,
    ) -> Check {
        let refusal = refused(
            route,
            ops::ROUTE_SELECT_ENGINE,
            params,
            code,
            "invalid engine request",
        )
        .await?;
        if refusal.detail.get("field") != Some(&json!(field)) {
            return Err(format!("expected field {field}, got {:?}", refusal.detail));
        }
        Ok(())
    }

    async fn engine_requirements(&self, route: &S::Route) -> Check {
        let rows = [
            (
                "language",
                json!("EN-gb"),
                "languages",
                json!(["en"]),
                json!(["fr"]),
            ),
            (
                "voice",
                json!("alto"),
                "voices",
                json!(["alto"]),
                json!(["tenor"]),
            ),
            (
                "format",
                json!("AUDIO/L16;rate=16000;channels=1"),
                "formats",
                json!(["audio/L16;rate=16000"]),
                json!(["audio/L16;rate=8000"]),
            ),
            (
                "duration_ms",
                json!(100),
                "max_duration_ms",
                json!(100),
                json!(99),
            ),
            (
                "input_bytes",
                json!(100),
                "max_input_bytes",
                json!(100),
                json!(99),
            ),
            (
                "reference_formats",
                json!(["image/png", "image/jpeg"]),
                "reference_formats",
                json!(["image/png", "image/jpeg"]),
                json!(["image/png"]),
            ),
            (
                "bias",
                json!(true),
                "bias_limit",
                json!({"unit":"phrase","max":3}),
                Value::Null,
            ),
            (
                "size",
                json!("large"),
                "sizes",
                json!(["large"]),
                json!(["small"]),
            ),
            (
                "resolution",
                json!("1080p"),
                "resolutions",
                json!(["1080p"]),
                json!(["720p"]),
            ),
            (
                "quality",
                json!("high"),
                "qualities",
                json!(["high"]),
                json!(["low"]),
            ),
            (
                "latency_class",
                json!("interactive"),
                "latency_class",
                json!("realtime"),
                json!("batch"),
            ),
        ];
        for (requirement, wanted, fact, good, bad) in rows {
            let mut yes = entry("z-match", true);
            yes[fact] = good;
            let mut no = yes.clone();
            no["model"] = json!("a-mismatch");
            if bad.is_null() {
                no.as_object_mut().unwrap().remove(fact);
            } else {
                no[fact] = bad;
            }
            self.subject
                .configure_catalog_declarations(&[module(MEDIA, vec![no.clone(), yes])])
                .await?;
            let mut params = request();
            params["requirements"] = json!({requirement:wanted});
            expect_engine(route, &params, "z-match", Some(MEDIA), None)
                .await
                .map_err(|e| format!("{requirement}: {e}"))?;
            no.as_object_mut().unwrap().remove(fact);
            // Formats and latency are required engine facts; optional facts must
            // fail matching when omitted, not silently remove the requirement.
            if fact != "formats" && fact != "latency_class" {
                self.subject
                    .configure_catalog_declarations(&[module(MEDIA, vec![no])])
                    .await?;
                no_engine(route, &params, "requirements").await?;
            }
        }
        let mut params = request();
        params["requirements"] = json!({"bias":false});
        self.subject
            .configure_catalog_declarations(&[module(MEDIA, vec![entry("unbiased", true)])])
            .await?;
        expect_engine(route, &params, "unbiased", Some(MEDIA), None).await?;
        for (languages, wanted) in [(json!(["en-GB"]), "en"), (json!(["en"]), "english")] {
            let mut value = entry("language", true);
            value["languages"] = languages;
            self.subject
                .configure_catalog_declarations(&[module(MEDIA, vec![value])])
                .await?;
            params["requirements"] = json!({"language":wanted});
            no_engine(route, &params, "requirements").await?;
        }
        let mut unknown = entry("unknown-latency", true);
        unknown["latency_class"] = json!("future-class");
        self.subject
            .configure_catalog_declarations(&[module(MEDIA, vec![unknown])])
            .await?;
        params["requirements"] = json!({"latency_class":"batch"});
        no_engine(route, &params, "requirements").await?;
        params["requirements"] = json!({});
        expect_engine(route, &params, "unknown-latency", Some(MEDIA), None).await?;
        Ok(())
    }

    async fn engine_ranking(&self, route: &S::Route) -> Check {
        let mut params = request();
        params["caller_local"] = json!([entry("caller", true)]);
        self.subject
            .configure_catalog_declarations(&[
                module(
                    "z-media",
                    vec![entry("z-model", true), entry("a-model", true)],
                ),
                module(
                    "a-media",
                    vec![entry("z-model", true), entry("a-model", true)],
                ),
            ])
            .await?;
        let facts = |model: &str| {
            EngineFacts::new(
                engine::Kind::Transcribe,
                fixture::SUBSCRIPTION_PROVIDER,
                model,
            )
        };
        for (fixtures, model, owner, index) in [
            (
                vec![
                    facts("z-model").with_accuracy(90).with_cost(100),
                    facts("a-model").with_accuracy(80).with_cost(1),
                ],
                "z-model",
                Some("a-media"),
                None,
            ),
            (
                vec![
                    facts("z-model").with_accuracy(90).with_cost(1),
                    facts("a-model").with_accuracy(90).with_cost(100),
                ],
                "z-model",
                Some("a-media"),
                None,
            ),
            (
                vec![
                    facts("a-model").with_accuracy(90),
                    facts("caller").with_accuracy(90).with_cost(1),
                ],
                "caller",
                None,
                Some(0),
            ),
            (vec![], "caller", None, Some(0)),
        ] {
            self.subject.configure_engine_facts(&fixtures).await?;
            expect_engine(route, &params, model, owner, index).await?;
        }
        params.as_object_mut().unwrap().remove("caller_local");
        expect_engine(route, &params, "a-model", Some("a-media"), None).await?;
        let mut first_provider = entry("z-model", true);
        first_provider["provider"] = json!("a-provider");
        self.subject
            .configure_catalog_declarations(&[module(
                "a-media",
                vec![entry("a-model", true), first_provider],
            )])
            .await?;
        expect_engine(route, &params, "z-model", Some("a-media"), None).await?;
        self.subject.configure_catalog_declarations(&[]).await?;
        let mut second = entry("caller", true);
        second["kind"] = json!("image");
        params["caller_local"] = json!([second, entry("caller", true)]);
        expect_engine(route, &params, "caller", None, Some(1)).await?;
        Ok(())
    }

    async fn engine_admission(&self, name: &str, route: &S::Route) -> Check {
        self.subject
            .configure_catalog_declarations(&[
                module(MEDIA, vec![entry("model-c", false)]),
                module(OTHER_MEDIA, vec![entry("model-c", false)]),
            ])
            .await?;
        let media = self.route_as(MEDIA).await?;
        let mut params = json!({"admission_id":"engine-admission", "conversation_key":"engine-conversation", "provider":fixture::SUBSCRIPTION_PROVIDER, "model":"model-c", "hold_wait_max_s":0});
        if name == "select_engine_media_admission" {
            admitted(&media, &params).await?;
            params["admission_id"] = json!("outside-slot");
            params["model"] = json!("model-d");
            refused(
                &media,
                ops::ROUTE_ACCOUNT_FOR,
                &params,
                RefusalCode::AccountNotAuthorized,
                "out-of-slot model",
            )
            .await?;
            let decision = selected(route, &request()).await?;
            params["decision_id"] = json!(decision.decision_id);
            refused(
                &media,
                ops::ROUTE_ACCOUNT_FOR,
                &params,
                RefusalCode::AccountNotAuthorized,
                "slot authorization precedes decision binding",
            )
            .await?;
            params.as_object_mut().unwrap().remove("decision_id");
            let unverified = CatalogModule {
                nonce_verified: false,
                ..module(MEDIA, vec![entry("model-c", false)])
            };
            self.subject
                .configure_catalog_declarations(&[unverified])
                .await?;
            params["admission_id"] = json!("unverified");
            params["model"] = json!("model-c");
            refused(
                &media,
                ops::ROUTE_ACCOUNT_FOR,
                &params,
                RefusalCode::AccountNotAuthorized,
                "unattested module",
            )
            .await?;
            self.subject
                .configure_catalog_declarations(&[module(MEDIA, vec![entry("model-c", true)])])
                .await?;
            params["admission_id"] = json!("local-slot");
            refused(
                &media,
                ops::ROUTE_ACCOUNT_FOR,
                &params,
                RefusalCode::AccountNotAuthorized,
                "local pair is not a remote slot engine",
            )
            .await?;
            return Ok(());
        }
        let mut selection = request();
        if name == "select_engine_admission_caller_local" {
            self.subject.configure_catalog_declarations(&[]).await?;
            selection["caller_local"] = json!([entry("model-c", true)]);
        }
        let decision = selected(route, &selection).await?;
        params["decision_id"] = json!(decision.decision_id);
        if name == "select_engine_admission_caller_local" {
            self.subject
                .configure_catalog_declarations(&[module(MEDIA, vec![entry("model-c", false)])])
                .await?;
            refused(
                &media,
                ops::ROUTE_ACCOUNT_FOR,
                &params,
                RefusalCode::UnknownDecision,
                "caller-local decision",
            )
            .await?;
        } else if name == "select_engine_admission_foreign" {
            let other = self.route_as(OTHER_MEDIA).await?;
            refused(
                &other,
                ops::ROUTE_ACCOUNT_FOR,
                &params,
                RefusalCode::UnknownDecision,
                "other attested media module",
            )
            .await?;
        } else if name == "select_engine_admission_pair" {
            self.subject
                .configure_catalog_declarations(&[module(
                    MEDIA,
                    vec![entry("model-c", false), entry("model-d", false)],
                )])
                .await?;
            params["model"] = json!("model-d");
            refused(
                &media,
                ops::ROUTE_ACCOUNT_FOR,
                &params,
                RefusalCode::UnknownDecision,
                "different authorized pair",
            )
            .await?;
        } else {
            let first = admitted(&media, &params).await?;
            if name == "select_engine_admission_expiry" || name == "select_engine_admission_replay"
            {
                self.subject.advance_clock(engine::DECISION_TTL_MS).await?;
                if name == "select_engine_admission_expiry" {
                    params["admission_id"] = json!("expired-new-admission");
                    refused(
                        &media,
                        ops::ROUTE_ACCOUNT_FOR,
                        &params,
                        RefusalCode::UnknownDecision,
                        "expired decision",
                    )
                    .await?;
                } else {
                    let replay: AccountForAnswer = answer(
                        &media,
                        ops::ROUTE_ACCOUNT_FOR,
                        &params,
                        "post-expiry replay",
                    )
                    .await?;
                    if replay != first {
                        return Err("post-expiry replay changed the first admission answer".into());
                    }
                }
            } else if name != "select_engine_admission_before_expiry" {
                return Err(format!("unknown engine check {name}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fake::{Defects, FakeSubject};
    use std::collections::BTreeSet;

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Rule {
        id: String,
        cases: Vec<String>,
        bug_shaped: bool,
    }

    fn rules() -> Vec<Rule> {
        serde_json::from_str(include_str!("../../rule-map.json")).unwrap()
    }

    #[test]
    fn select_engine_rule_map_covers_pinned_ids() {
        let contract = include_str!("../../../cortexkit-role-routing/CONTRACT.md");
        let ids: BTreeSet<_> = contract
            .lines()
            .filter_map(|line| {
                line.strip_prefix("- [pinned] `select_engine.")
                    .and_then(|tail| tail.split_once("`:"))
            })
            .map(|(id, _)| format!("select_engine.{id}"))
            .collect();
        let mapped = rules();
        let map_ids: BTreeSet<_> = mapped.iter().map(|rule| rule.id.clone()).collect();
        assert_eq!(ids, map_ids);
        assert_eq!(map_ids.len(), mapped.len(), "duplicate mapped rule");
        for addition in [
            "select_engine.media_admission",
            "select_engine.no_engine_code",
        ] {
            assert!(ids.contains(addition));
        }
        let section = contract
            .split("## 13. `route.select_engine`")
            .nth(1)
            .unwrap()
            .split("\n## ")
            .next()
            .unwrap();
        for line in section
            .lines()
            .filter(|line| line.starts_with("- [pinned]"))
        {
            assert!(
                line.starts_with("- [pinned] `select_engine.") && line.contains("`:"),
                "pinned rule without id: {line}"
            );
        }
        // Every current rule is kit-checked, so the contract must not declare
        // an exclusion that the map silently fails to preserve.
        assert!(!contract.contains("not_kit_checked"));
        let cases: BTreeSet<_> = mapped
            .iter()
            .flat_map(|rule| rule.cases.iter().map(String::as_str))
            .collect();
        let registered: BTreeSet<_> = crate::CHECKS
            .iter()
            .filter(|spec| spec.name.starts_with("select_engine_"))
            .map(|spec| spec.name)
            .collect();
        assert_eq!(cases, registered);
        for rule in mapped {
            assert!(!rule.cases.is_empty(), "{} has no checked cases", rule.id);
        }
    }

    #[tokio::test]
    async fn select_engine_mapped_cases_pass_and_reject_faults_by_name() {
        let rules = rules();
        let cases: BTreeSet<_> = rules
            .iter()
            .flat_map(|rule| rule.cases.iter().map(String::as_str))
            .collect();
        for name in cases {
            let good = FakeSubject::new(Defects::default());
            assert_eq!(
                Suite::new(&good).run(name).await,
                Ok(Ending::Passed),
                "conforming fake: {name}"
            );
            // Each faulty fake changes the behavior named by the case. Report
            // construction adds that case name to the provider's failure reason.
            let name: &'static str = crate::CHECKS
                .iter()
                .find(|spec| spec.name == name)
                .unwrap()
                .name;
            let bad = FakeSubject::new(Defects {
                engine_fault: name,
                ..Defects::default()
            });
            let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
            let report = crate::runner::run_check(&Suite::new(&bad), spec).await;
            assert!(
                matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                "fault was not rejected by {name}: {report:?}"
            );
            assert_eq!(report.check, name);
        }
        // The map currently marks every rule as general coverage. A bug-shaped
        // entry needs a dedicated isolated faulty implementation.
        assert!(rules.iter().all(|rule| !rule.bug_shaped));
    }

    #[tokio::test]
    async fn select_engine_each_requirement_fault_is_rejected() {
        for member in [
            "language",
            "voice",
            "format",
            "duration_ms",
            "input_bytes",
            "reference_formats",
            "bias",
            "size",
            "resolution",
            "quality",
            "latency_class",
        ] {
            let fault = match member {
                "language" => "requirement:language",
                "voice" => "requirement:voice",
                "format" => "requirement:format",
                "duration_ms" => "requirement:duration_ms",
                "input_bytes" => "requirement:input_bytes",
                "reference_formats" => "requirement:reference_formats",
                "bias" => "requirement:bias",
                "size" => "requirement:size",
                "resolution" => "requirement:resolution",
                "quality" => "requirement:quality",
                _ => "requirement:latency_class",
            };
            let bad = FakeSubject::new(Defects {
                engine_fault: fault,
                ..Defects::default()
            });
            let result = Suite::new(&bad)
                .run("select_engine_match_requirements")
                .await;
            assert!(
                result.as_ref().is_err_and(|reason| reason.contains(member)),
                "ignored {member} not rejected by name: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn select_engine_without_group_is_not_applicable() {
        let subject = FakeSubject::without_extensions(Defects::default());
        for spec in crate::CHECKS
            .iter()
            .filter(|spec| spec.name.starts_with("select_engine_"))
        {
            assert!(
                matches!(
                    Suite::new(&subject).run(spec.name).await,
                    Ok(Ending::NotApplicable(_))
                ),
                "{}",
                spec.name
            );
        }
    }

    struct DefaultHooks(FakeSubject);
    #[async_trait::async_trait]
    impl RoutingSubject for DefaultHooks {
        type Route = fake::FakeRoute;
        async fn reset(&self) -> Result<(), String> {
            self.0.reset().await
        }
        async fn configure_consumers(&self, values: &[ConsumerConfig]) -> Result<(), String> {
            self.0.configure_consumers(values).await
        }
        async fn configure_catalog(&self, catalog: &crate::Catalog) -> Result<(), String> {
            self.0.configure_catalog(catalog).await
        }
        async fn configure_catalog_declarations(
            &self,
            modules: &[CatalogModule],
        ) -> Result<(), String> {
            self.0.configure_catalog_declarations(modules).await
        }
        async fn pin_for_project(
            &self,
            project: &str,
            consumer: &str,
            pin: &str,
        ) -> Result<(), String> {
            self.0.pin_for_project(project, consumer, pin).await
        }
        async fn cool_down(&self, route: &str, until: u64) -> Result<(), String> {
            self.0.cool_down(route, until).await
        }
        async fn route(&self, stamp: &RouteStamp) -> Result<Self::Route, String> {
            self.0.route(stamp).await
        }
    }

    #[tokio::test]
    async fn select_engine_missing_hooks_fail_by_name() {
        let subject = DefaultHooks(FakeSubject::new(Defects::default()));
        for (case, hook) in [
            ("select_engine_ranking", "configure_engine_facts"),
            ("select_engine_admission_expiry", "advance_clock"),
        ] {
            let result = Suite::new(&subject).run(case).await;
            assert!(
                result.as_ref().is_err_and(|reason| reason.contains(hook)),
                "{case}: {result:?}"
            );
        }
    }
}
