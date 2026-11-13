//! Optional consumer, harness, eligibility, price and attempt cases.
use super::*;
use crate::fixture::{
    OTHER_CONSUMER, PRICED_PROVIDER, RUNNER_ONE, RUNNER_TWO, SUBSCRIPTION_PROVIDER,
};
use crate::wire::{price::Price, select::check_answer_serving_with_harnesses, spellings};
use crate::RunnerRouteFacts;

pub(super) const CASES: &[(&str, &str)] = &[
    (
        "placement_consumer_declarations",
        capabilities::CONSUMER_DECLARATIONS,
    ),
    (
        "placement_harness_facts",
        capabilities::RUNNER_HARNESS_FACTS,
    ),
    ("placement_decision_harness", capabilities::DECISION_HARNESS),
    ("placement_select_kind", capabilities::SELECT_KIND),
    (
        "placement_eligible_routes",
        capabilities::SELECT_ELIGIBLE_ROUTES,
    ),
    ("placement_select_price", capabilities::SELECT_PRICE),
    (
        "placement_attempt_outcomes",
        capabilities::REPORT_ATTEMPT_OUTCOMES,
    ),
];

fn slot(principal: &str, body: Value) -> crate::CatalogModule {
    crate::CatalogModule {
        principal: principal.into(),
        nonce_verified: true,
        role_declarations: [(crate::wire::ROLE_DECLARATIONS_KEY.into(), body)].into(),
    }
}

async fn no_route<R: RoutingRoute>(route: &R, params: &Value) -> Check {
    refused(
        route,
        ops::ROUTE_SELECT,
        params,
        RefusalCode::NoRoute,
        "empty candidate set",
    )
    .await
    .map(|_| ())
}

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn placement_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let group = CASES.iter().find(|(case, _)| *case == name).unwrap().1;
        if !describe.serves(group) {
            return Ok(Ending::NotApplicable(format!(
                "role.describe does not declare {group}"
            )));
        }
        let key_a = format!("{PRICED_PROVIDER}/model-a");
        let key_b = format!("{PRICED_PROVIDER}/model-b");
        let wire_api = spellings::WIRE_APIS[0];
        let auth_method = spellings::AUTH_METHODS[1];
        if matches!(
            name,
            "placement_consumer_declarations" | "placement_harness_facts"
        ) {
            // Both placement checks declare a runner pair. Give every fixture
            // route the same explicit wire format and its accounts' OAuth method
            // so pair matching cannot depend on the subject's catalog defaults.
            let facts: Vec<_> = fixture::catalog()
                .models
                .iter()
                .map(|model| RunnerRouteFacts::new(model.route_key(), wire_api, auth_method))
                .collect();
            if !self.subject.configure_runner_route_facts(&facts).await? {
                return Ok(Ending::NotApplicable(
                    "configure_runner_route_facts is unsupported".into(),
                ));
            }
        }
        match name {
            "placement_consumer_declarations" => {
                let declared = "example-consumer.task";
                let mut facts = crate::SelectionFacts::new();
                let search_route = format!("{SUBSCRIPTION_PROVIDER}/model-d");
                facts.kinds.insert(search_route.clone(), "search".into());
                self.subject.configure_selection_facts(&facts).await?;
                let body = json!({"consumers":[{"name":declared,"iq":40,"eq":10,"ladder":[key_b,key_a],"placement":"caller_runner","selectors":[OTHER_CONSUMER],"kind":"llm"},{"name":"example-consumer.search","iq":40,"kind":"search"},{"name":"foreign.task","iq":40}]});
                self.subject
                    .configure_catalog_declarations(&[slot(CONSUMER, body)])
                    .await?;
                let params = json!({"consumer":declared,"requirements":{},"runners":[RUNNER_TWO]});
                let selected: SelectAnswer =
                    answer(&route, ops::ROUTE_SELECT, &params, "declared iq and ladder").await?;
                if selected.route.route_key() != key_b || selected.runner.module != RUNNER_TWO {
                    return Err(
                        "declared ladder or caller_runner placement was not honoured".into(),
                    );
                }
                for runners in [
                    None,
                    Some(json!([])),
                    Some(json!([RUNNER_ONE, RUNNER_TWO])),
                    Some(json!([CONSUMER])),
                ] {
                    let mut unresolved = params.clone();
                    match runners {
                        Some(runners) => unresolved["runners"] = runners,
                        None => {
                            unresolved.as_object_mut().unwrap().remove("runners");
                        }
                    }
                    let refusal = refused(
                        &route,
                        ops::ROUTE_SELECT,
                        &unresolved,
                        RefusalCode::NoRoute,
                        "unresolved caller_runner placement",
                    )
                    .await?;
                    if refusal.detail.get("reason") != Some(&json!("caller_runner_unresolved"))
                        || refusal.detail.contains_key("reset_at_ms")
                    {
                        return Err(
                            "unresolved caller_runner placement must name its non-timed cause"
                                .into(),
                        );
                    }
                }
                let search_params = json!({"consumer":"example-consumer.search","requirements":{}});
                if describe.serves(capabilities::SELECT_KIND) {
                    let search: SelectAnswer = answer(
                        &route,
                        ops::ROUTE_SELECT,
                        &search_params,
                        "declared search kind",
                    )
                    .await?;
                    if search.route.route_key() != search_route {
                        return Err("declared kind escaped its route kind".into());
                    }
                } else {
                    no_route(&route, &search_params).await?;
                }
                let mut child = params.clone();
                child["consumer"] = json!(format!("{declared}.child"));
                let _: SelectAnswer =
                    answer(&route, ops::ROUTE_SELECT, &child, "dotted inheritance").await?;
                let selector = self.route_as(OTHER_CONSUMER).await?;
                let _: SelectAnswer =
                    answer(&selector, ops::ROUTE_SELECT, &params, "listed selector").await?;
                let outsider = self.route_as("unlisted-selector").await?;
                refused(
                    &outsider,
                    ops::ROUTE_SELECT,
                    &params,
                    RefusalCode::ConsumerNotOwned,
                    "unlisted selector",
                )
                .await?;
                let owned = self
                    .subject
                    .route(&crate::RouteStamp::scoped(
                        "unlisted-selector",
                        crate::ScopeStamp {
                            owner: CONSUMER.into(),
                            scope_ref: "scope".into(),
                            scope_epoch: 1,
                        },
                    ))
                    .await?;
                let _: SelectAnswer =
                    answer(&owned, ops::ROUTE_SELECT, &params, "owner scope").await?;
                let mut explicit = params.clone();
                explicit["requirements"]["iq"] = json!(0);
                let _: SelectAnswer =
                    answer(&route, ops::ROUTE_SELECT, &explicit, "explicit zero wins").await?;
                explicit["exclude"] = json!([
                    key_a,
                    key_b,
                    format!("{SUBSCRIPTION_PROVIDER}/model-c"),
                    format!("{SUBSCRIPTION_PROVIDER}/model-d")
                ]);
                no_route(&route, &explicit).await?;
                let invalid_name = json!({"consumer":"foreign.task","requirements":{"iq":40}});
                refused(
                    &route,
                    ops::ROUTE_SELECT,
                    &invalid_name,
                    RefusalCode::UnknownConsumer,
                    "foreign declaration ignored",
                )
                .await?;
                self.subject.configure_catalog_declarations(&[slot(CONSUMER, json!({"consumers":[{"name":declared,"iq":40,"placement":{"module":RUNNER_ONE}}]}))]).await?;
                no_route(&route, &params).await?;
                let mut params = params;
                params.as_object_mut().unwrap().remove("runners");
                let selected: SelectAnswer =
                    answer(&route, ops::ROUTE_SELECT, &params, "module placement").await?;
                if selected.runner.module != RUNNER_ONE {
                    return Err("module placement was not honoured".into());
                }
                let runner_consumer = format!("{RUNNER_ONE}.task");
                self.subject.configure_catalog_declarations(&[slot(RUNNER_ONE, json!({"runner":{"pairs":[{"wire_api":wire_api,"auth_method":auth_method}]},"consumers":[{"name":runner_consumer,"iq":40,"placement":"caller_runner"}]}))]).await?;
                let runner_route = self.route_as(RUNNER_ONE).await?;
                let mut runner_params = json!({"consumer":runner_consumer,"requirements":{}});
                for runners in [
                    None,
                    Some(json!([RUNNER_ONE])),
                    Some(json!([RUNNER_TWO, RUNNER_ONE])),
                ] {
                    if let Some(runners) = runners {
                        runner_params["runners"] = runners;
                    }
                    let selected: SelectAnswer = answer(
                        &runner_route,
                        ops::ROUTE_SELECT,
                        &runner_params,
                        "attested caller_runner placement",
                    )
                    .await?;
                    if selected.runner.module != RUNNER_ONE {
                        return Err("caller_runner did not use the attested catalog runner".into());
                    }
                }
                for runners in [json!([]), json!([RUNNER_TWO]), json!([CONSUMER])] {
                    runner_params["runners"] = runners;
                    no_route(&runner_route, &runner_params).await?;
                }
            }
            "placement_harness_facts" => {
                self.subject.configure_catalog_declarations(&[slot(RUNNER_ONE, json!({"runner":{"pairs":[{"wire_api":wire_api,"auth_method":auth_method}],"harnesses":["alpha","beta"],"default_harness":"alpha","harness_limits":{"alpha":{"starts_sessions":false,"capabilities":["future_task"]},"beta":{"starts_sessions":true,"capabilities":["one_off","fork_answer","output_cap"],"exclude":[key_a]}}}}))]).await?;
                let mut params = base();
                params["runners"] = json!([RUNNER_ONE]);
                if describe.serves(capabilities::SELECT_HARNESS) {
                    params["harness"] = json!("alpha");
                    let refusal = refused(
                        &route,
                        ops::ROUTE_SELECT,
                        &params,
                        RefusalCode::NoRoute,
                        "non-starting harness",
                    )
                    .await?;
                    if refusal.detail.get("reason") != Some(&json!("harness_unavailable")) {
                        return Err("non-starting harness must name harness_unavailable".into());
                    }
                    params["harness"] = json!("beta");
                }
                let selected = select(&route, &params, "harness exclusion").await?;
                if selected.route.route_key() == key_a
                    || ((describe.serves(capabilities::SELECT_HARNESS)
                        || describe.serves(capabilities::DECISION_HARNESS))
                        && selected.runner.harness.as_deref() != Some("beta"))
                {
                    return Err("harness facts did not narrow selection".into());
                }
            }
            "placement_decision_harness" => {
                let catalog = fixture::catalog();
                self.subject.configure_catalog(&catalog).await?;
                for runner in &catalog.runners {
                    let mut params = base();
                    params["runners"] = json!([runner.module]);
                    let selected = select(&route, &params, "runner and harness decision").await?;
                    let request =
                        parse_select_request_serving(&params, capabilities::KNOWN).unwrap();
                    check_answer_serving_with_harnesses(
                        &request,
                        &selected,
                        &describe.capabilities,
                        runner.harnesses.as_deref(),
                    )?;
                    if selected.runner.module != runner.module
                        || (runner.default_harness.is_some()
                            && selected.runner.harness != runner.default_harness)
                    {
                        return Err("decision must bind the chosen module and harness".into());
                    }
                }
            }
            "placement_select_kind" => {
                self.subject
                    .configure_selection_facts(&crate::placement_facts::facts())
                    .await?;
                let mut params = base();
                params["kind"] = json!("search");
                let selected = select(&route, &params, "search kind").await?;
                if selected.route.route_key() != key_b {
                    return Err("select kind escaped search routes".into());
                }
                params.as_object_mut().unwrap().remove("kind");
                let selected = select(&route, &params, "default llm kind").await?;
                if selected.route.route_key() == key_b {
                    return Err("omitted kind did not default to llm".into());
                }
                params["kind"] = json!("future_kind");
                no_route(&route, &params).await?;
            }
            "placement_eligible_routes" => {
                let mut params = base();
                params["eligible_routes"] = json!([{"provider":PRICED_PROVIDER,"model":"model-b"}]);
                let selected = select(&route, &params, "eligible subset").await?;
                if selected.route.route_key() != key_b {
                    return Err("router expanded eligible_routes".into());
                }
                params["eligible_routes"] = json!([]);
                no_route(&route, &params).await?;
            }
            "placement_select_price" => {
                self.subject
                    .configure_selection_facts(&crate::placement_facts::facts())
                    .await?;
                for (pin, expected) in [
                    (
                        key_a,
                        json!({"basis":"metered","rates_per_mtok":{"input":4,"output":20,"cache_read":1,"cache_write":5,"cache_write_ttl":{"5m":5,"1h":8}}}),
                    ),
                    (
                        format!("{SUBSCRIPTION_PROVIDER}/model-c"),
                        json!({"basis":"subscription_equivalent","multiplier":20,"rates_per_mtok":{"input":1.0,"output":5.0,"cache_write_ttl":{"30m":1.5,"12h":2.0,"24h":2.5}},"estimated_task_usd":1}),
                    ),
                ] {
                    let mut params = base();
                    params["pin"] = json!(pin);
                    let selected = select(&route, &params, "price fixture").await?;
                    let expected: Price = serde_json::from_value(expected).unwrap();
                    if selected.price != Some(expected) {
                        return Err(format!(
                            "wrong select price for {pin}: {:?}",
                            selected.price
                        ));
                    }
                    selected.price.unwrap().check().map_err(str::to_owned)?;
                }
                let mut params = base();
                params["pin"] = json!(format!("{SUBSCRIPTION_PROVIDER}/model-d"));
                if select(&route, &params, "unpriced route")
                    .await?
                    .price
                    .is_some()
                {
                    return Err("unpriced route must omit price".into());
                }
            }
            "placement_attempt_outcomes" => {
                for status in [
                    "served",
                    "served_empty",
                    "quota",
                    "throttled",
                    "credential",
                    "unavailable",
                    "rejected",
                    "future",
                ] {
                    let selected = select(&route, &base(), "attempt decision").await?;
                    let legacy = match status {
                        "served" | "served_empty" => "completed",
                        "rejected" | "future" => "cancelled",
                        _ => "failed",
                    };
                    let mut params = json!({"decision_id":selected.decision_id,"outcome":legacy,"attempt_outcome":{"status":status}});
                    let _: Value = answer(
                        &route,
                        ops::ROUTE_REPORT,
                        &params,
                        "attempt status without a provider wait",
                    )
                    .await?;
                    if status == "throttled" {
                        let selected =
                            select(&route, &base(), "throttled attempt with provider wait").await?;
                        params["decision_id"] = json!(selected.decision_id);
                        params["attempt_outcome"]["retry_after_ms"] = json!(10);
                        let _: Value = answer(
                            &route,
                            ops::ROUTE_REPORT,
                            &params,
                            "provider-named throttle wait",
                        )
                        .await?;
                    }
                }
                let natural = select(&route, &base(), "natural route").await?;
                let kept = another_route_key(&natural.route.route_key())?;
                for (status, legacy) in [("future", "failed"), ("rejected", "cancelled")] {
                    let keyed = with(
                        base(),
                        "affinity",
                        json!(format!("attempt-safety-{status}")),
                    );
                    let selected = select(
                        &route,
                        &with(keyed.clone(), "pin", json!(kept)),
                        "nondefault affinity route",
                    )
                    .await?;
                    let held = select(&route, &keyed, "held affinity route").await?;
                    if held.route != selected.route {
                        return Err("attempt fixture did not establish nondefault affinity".into());
                    }
                    let params = json!({"decision_id":held.decision_id,"outcome":legacy,"attempt_outcome":{"status":status},"reset_at_ms":now_ms()+60_000});
                    let _: Value =
                        answer(&route, ops::ROUTE_REPORT, &params, "attempt counts nothing")
                            .await?;
                    let again = select(&route, &keyed, "unchanged affinity route").await?;
                    if again.route != selected.route {
                        return Err(format!("{status} attempt penalized a route"));
                    }
                }
                let again = select(&route, &base(), "retry validation decision").await?;
                let params = json!({"decision_id":again.decision_id,"outcome":"cancelled","attempt_outcome":{"status":"rejected","retry_after_ms":10}});
                let refusal = refused(
                    &route,
                    ops::ROUTE_REPORT,
                    &params,
                    RefusalCode::InvalidRequest,
                    "retry only on throttled",
                )
                .await?;
                if refusal.detail.get("field") != Some(&json!("attempt_outcome.retry_after_ms")) {
                    return Err("retry refusal must name attempt_outcome.retry_after_ms".into());
                }
            }
            _ => unreachable!(),
        }
        Ok(Ending::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::select_engine::fake::{Defects, FakeSubject};

    #[tokio::test]
    async fn placement_harness_facts_uses_matching_runner_pairs() {
        let subject = FakeSubject::with_placement(Defects::default());
        assert_eq!(
            Suite::new(&subject).run("placement_harness_facts").await,
            Ok(Ending::Passed)
        );
    }

    #[tokio::test]
    async fn placement_harness_facts_rejects_ignored_exclusion() {
        let subject = FakeSubject::with_placement(Defects {
            extension: Some("placement_harness_exclusion"),
            ..Defects::default()
        });
        let spec = crate::CHECKS
            .iter()
            .find(|spec| spec.name == "placement_harness_facts")
            .unwrap();
        let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
        assert_eq!(report.check, "placement_harness_facts");
        assert!(
            matches!(&report.outcome, crate::CheckOutcome::Failed { reason }
                if reason == "harness facts did not narrow selection"),
            "{report:?}"
        );
    }

    #[tokio::test]
    async fn placement_pair_facts_unsupported_are_skipped_not_passed() {
        let subject = FakeSubject::with_placement(Defects {
            runner_route_facts_unsupported: true,
            ..Defects::default()
        });
        for name in ["placement_consumer_declarations", "placement_harness_facts"] {
            assert!(
                matches!(Suite::new(&subject).run(name).await,
                    Ok(Ending::NotApplicable(reason))
                    if reason.contains("configure_runner_route_facts")),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn fake_runner_pair_tokens_must_be_known() {
        let wire_api = spellings::WIRE_APIS[0];
        let auth_method = spellings::AUTH_METHODS[1];
        for explicit_facts in [false, true] {
            for (wire, auth, matches) in [
                (wire_api, auth_method, true),
                ("messages", auth_method, false),
                (wire_api, "api_key", false),
                ("messages", "api_key", false),
            ] {
                let subject = FakeSubject::with_placement(Defects::default());
                let suite = Suite::new(&subject);
                suite.fresh().await.unwrap();
                let mut catalog = fixture::catalog();
                catalog.models.truncate(1);
                catalog.runners.truncate(1);
                let key = catalog.models[0].route_key();
                catalog.runners[0].route_keys = vec![key.clone()];
                subject.configure_catalog(&catalog).await.unwrap();
                if explicit_facts {
                    assert!(subject
                        .configure_runner_route_facts(&[RunnerRouteFacts::new(
                            &key,
                            wire_api,
                            auth_method,
                        )])
                        .await
                        .unwrap());
                }
                subject
                    .configure_catalog_declarations(&[slot(
                        RUNNER_ONE,
                        json!({"runner":{"pairs":[{"wire_api":wire,"auth_method":auth}]}}),
                    )])
                    .await
                    .unwrap();
                let route = suite.route_as(CONSUMER).await.unwrap();
                let mut params = base();
                params["runners"] = json!([RUNNER_ONE]);
                if matches {
                    select(&route, &params, "known runner pair").await.unwrap();
                } else {
                    no_route(&route, &params).await.unwrap();
                }
            }
        }
    }
}
