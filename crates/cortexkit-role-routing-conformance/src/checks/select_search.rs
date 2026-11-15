//! Search selection shares media answers, but never media requirements or accounts.

use super::*;
use crate::wire::{select_media::SelectMediaAnswer, select_search as search};

const SEARCH: &str = "example-search";

fn entry(model: &str) -> Value {
    json!({"provider":fixture::SUBSCRIPTION_PROVIDER,"model":model})
}

fn module(slot: Value) -> CatalogModule {
    CatalogModule {
        principal: SEARCH.into(),
        nonce_verified: true,
        role_declarations: [(search::ROLE_DECLARATIONS_KEY.into(), slot)].into(),
    }
}

fn request() -> Value {
    json!({"consumer":SUMMARY,"kind":"search","requirements":{},"privacy":"remote_allowed"})
}

async fn selected<R: RoutingRoute>(route: &R, params: &Value) -> Result<Value, String> {
    let value: Value = answer(route, ops::ROUTE_SELECT, params, "search select").await?;
    let decoded: SelectMediaAnswer =
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    let parsed = search::parse_select_search_request(params, capabilities::KNOWN, &["search"])
        .map_err(|e| e.to_string())?;
    search::check_answer(&parsed, &decoded).map_err(|e| e.to_string())?;
    if decoded.engine.module.as_deref() != Some(SEARCH) {
        return Err("search answer did not name the declaring module".into());
    }
    let now = now_ms();
    if decoded.expires_at_ms < now + 595_000 || decoded.expires_at_ms > now + 605_000 {
        return Err("search decision does not expire ten minutes after selection".into());
    }
    Ok(value)
}

async fn invalid<R: RoutingRoute>(
    route: &R,
    params: &Value,
    code: RefusalCode,
    field: &str,
) -> Check {
    let refusal = refused(route, ops::ROUTE_SELECT, params, code, "search validation").await?;
    if refusal.detail.get("field") != Some(&json!(field)) {
        return Err(format!("expected field {field}, got {:?}", refusal.detail));
    }
    Ok(())
}

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn select_search_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let listed = describe.kinds.iter().any(|kind| kind.as_str() == "search");
        if !describe.serves(capabilities::SELECT_SEARCH) {
            if listed {
                return Err("search is listed without select_search".into());
            }
            if name == "select_search_advertisement" && describe.serves(capabilities::SELECT_MEDIA)
            {
                invalid(&route, &request(), RefusalCode::InvalidRequest, "kind").await?;
                return Ok(Ending::Passed);
            }
            return Ok(Ending::NotApplicable(
                "role.describe does not declare select_search".into(),
            ));
        }
        if !listed
            || !describe.serves(capabilities::SELECT_KIND)
            || !describe.serves(capabilities::SELECT_MEDIA)
        {
            return Err(
                "select_search requires select_kind, select_media and listed search kind".into(),
            );
        }
        self.subject
            .configure_catalog_declarations(&[module(json!({"routes":[entry("model-c")]}))])
            .await?;
        let mut params = request();
        match name {
            "select_search_advertisement" => {
                selected(&route, &params).await?;
                params["kind"] = json!("unlisted-search-kind");
                invalid(&route, &params, RefusalCode::InvalidRequest, "kind").await?;
            }
            "select_search_declarations" => {
                selected(&route, &params).await?;
                let good = entry("model-c");
                for bad in [
                    json!({}),
                    json!({"routes":null}),
                    json!({"routes":[good.clone()],"unknown":true}),
                    json!({"routes":[good.clone(),good.clone()]}),
                    json!({"routes":[good.clone(),{"provider":"p","model":""}]}),
                    json!({"routes":[good.clone(),{"provider":"","model":"m"}]}),
                    json!({"routes":[good.clone(),{"provider":"p","model":"x".repeat(257)}]}),
                    json!({"routes":[good.clone(),{"provider":"x".repeat(257),"model":"m"}]}),
                    json!({"routes":[good.clone(),{"provider":"p","model":"m","account":"hidden"}]}),
                ] {
                    self.subject
                        .configure_catalog_declarations(&[module(bad)])
                        .await?;
                    refused(
                        &route,
                        ops::ROUTE_SELECT,
                        &params,
                        RefusalCode::NoRoute,
                        "atomic search slot",
                    )
                    .await?;
                }
                let wrong_slot = CatalogModule {
                    principal: SEARCH.into(),
                    nonce_verified: true,
                    role_declarations: [(
                        crate::wire::ROLE_DECLARATIONS_KEY.into(),
                        json!({"routes":[good]}),
                    )]
                    .into(),
                };
                self.subject
                    .configure_catalog_declarations(&[wrong_slot])
                    .await?;
                refused(
                    &route,
                    ops::ROUTE_SELECT,
                    &params,
                    RefusalCode::NoRoute,
                    "search routes outside search-provider slot",
                )
                .await?;
            }
            "select_search_nonce" => {
                selected(&route, &params).await?;
                let mut unverified = module(json!({"routes":[entry("model-c")]}));
                unverified.nonce_verified = false;
                self.subject
                    .configure_catalog_declarations(&[unverified])
                    .await?;
                refused(
                    &route,
                    ops::ROUTE_SELECT,
                    &params,
                    RefusalCode::NoRoute,
                    "unverified search slot",
                )
                .await?;
            }
            "select_search_requirements" => {
                selected(&route, &params).await?;
                for iq in [0, 100] {
                    params["requirements"] = json!({"iq":iq});
                    selected(&route, &params).await?;
                }
                for iq in [json!(101), json!(-1), json!(1.5), json!("50")] {
                    params["requirements"] = json!({"iq":iq});
                    invalid(&route, &params, RefusalCode::InvalidRequirements, "iq").await?;
                }
                for field in crate::wire::select::REQUIREMENT_FIELDS
                    .iter()
                    .chain(crate::wire::select_media::REQUIREMENT_FIELDS)
                    .chain(["future_requirement"].iter())
                {
                    if *field == "iq" {
                        continue;
                    }
                    params["requirements"] = json!({(*field): null});
                    invalid(&route, &params, RefusalCode::InvalidRequirements, field).await?;
                }
            }
            "select_search_privacy" => {
                params.as_object_mut().unwrap().remove("privacy");
                invalid(&route, &params, RefusalCode::InvalidRequest, "privacy").await?;
                params["privacy"] = json!("future");
                invalid(&route, &params, RefusalCode::InvalidRequest, "privacy").await?;
                params["privacy"] = json!("local_only");
                refused(
                    &route,
                    ops::ROUTE_SELECT,
                    &params,
                    RefusalCode::NoRoute,
                    "search is remote",
                )
                .await?;
                params["privacy"] = json!("remote_allowed");
                for value in [json!([]), json!(null)] {
                    params["caller_local"] = value;
                    invalid(&route, &params, RefusalCode::InvalidRequest, "caller_local").await?;
                }
            }
            "select_search_allowance" => {
                params["allowance_left"] = json!([entry("model-c")]);
                invalid(
                    &route,
                    &params,
                    RefusalCode::InvalidRequest,
                    "allowance_left[0]",
                )
                .await?;
                params["eligible_routes"] = json!([]);
                invalid(
                    &route,
                    &params,
                    RefusalCode::InvalidRequest,
                    "allowance_left[0]",
                )
                .await?;
                params["eligible_routes"] = json!([entry("model-c")]);
                params["allowance_left"] = json!([entry("model-c"), entry("model-c")]);
                selected(&route, &params).await?;
                params["allowance_left"][1] = entry("model-d");
                invalid(
                    &route,
                    &params,
                    RefusalCode::InvalidRequest,
                    "allowance_left[1]",
                )
                .await?;
                for kind in describe
                    .kinds
                    .iter()
                    .map(|kind| kind.as_str())
                    .chain(std::iter::once("llm"))
                {
                    if kind == "search" {
                        continue;
                    }
                    params["kind"] = json!(kind);
                    params["allowance_left"] = json!([]);
                    params["requirements"] = json!({"iq":0});
                    if kind == "llm" {
                        params.as_object_mut().unwrap().remove("privacy");
                    }
                    invalid(
                        &route,
                        &params,
                        RefusalCode::InvalidRequest,
                        "allowance_left",
                    )
                    .await?;
                }
                params = request();
                params["eligible_routes"] = json!([]);
                refused(
                    &route,
                    ops::ROUTE_SELECT,
                    &params,
                    RefusalCode::NoRoute,
                    "empty search subset",
                )
                .await?;
            }
            "select_search_ranking" => self.search_ranking(&route, &mut params).await?,
            "select_search_reports" => {
                if !describe.serves(capabilities::REPORT_ATTEMPT_OUTCOMES) {
                    return Err("search decisions require report_attempt_outcomes".into());
                }
                let statuses = [
                    ("served", "completed"),
                    ("served_empty", "completed"),
                    ("quota", "failed"),
                    ("throttled", "failed"),
                    ("credential", "failed"),
                    ("unavailable", "failed"),
                    ("rejected", "cancelled"),
                ];
                // A failure report can make the route unavailable to later selections.
                // Select every decision first so status validation is independent of that effect.
                let mut decisions = Vec::new();
                for _ in statuses {
                    decisions.push((
                        selected(&route, &params).await?,
                        selected(&route, &params).await?,
                    ));
                }
                for ((status, legacy), (decision, invalid_decision)) in
                    statuses.into_iter().zip(decisions)
                {
                    let mut report = json!({"decision_id":decision["decision_id"],"outcome":legacy,"attempt_outcome":{"status":status}});
                    if status == "throttled" {
                        report["attempt_outcome"]["retry_after_ms"] = json!(0);
                    }
                    let _: Value =
                        answer(&route, ops::ROUTE_REPORT, &report, "search attempt status").await?;
                    if status != "throttled" {
                        report["decision_id"] = invalid_decision["decision_id"].clone();
                        report["attempt_outcome"]["retry_after_ms"] = json!(1);
                        let refusal = refused(
                            &route,
                            ops::ROUTE_REPORT,
                            &report,
                            RefusalCode::InvalidRequest,
                            "search non-throttled wait",
                        )
                        .await?;
                        if refusal.detail.get("field")
                            != Some(&json!("attempt_outcome.retry_after_ms"))
                        {
                            return Err("search wait refusal did not name retry_after_ms".into());
                        }
                    }
                }
            }
            _ => unreachable!("registered search check"),
        }
        Ok(Ending::Passed)
    }

    async fn search_ranking<R: RoutingRoute>(&self, route: &R, params: &mut Value) -> Check {
        self.subject
            .configure_catalog_declarations(&[module(
                json!({"routes":[entry("model-c"),entry("model-d")]}),
            )])
            .await?;
        let mut facts = crate::SelectionFacts::new();
        for (model, rate) in [("model-c", 10), ("model-d", 1)] {
            let mut search = crate::SearchRouteFacts::new();
            search.grade = Some(400);
            search.price = Some(crate::wire::select_media::UnitPrice::metered(
                [(
                    fixture::SEARCH_PRICE_KEY.into(),
                    crate::wire::select_media::UnitRate::new(rate.into()),
                )]
                .into(),
            ));
            facts.search.insert(
                format!("{}/{model}", fixture::SUBSCRIPTION_PROVIDER),
                search,
            );
        }
        self.subject.configure_selection_facts(&facts).await?;
        params["eligible_routes"] = json!([entry("model-c"), entry("model-d")]);
        params["allowance_left"] = json!([entry("model-c")]);
        params["requirements"] = json!({"iq":75});
        let selected_free = selected(route, params).await?;
        if selected_free["engine"]["model"] != "model-c" {
            return Err("adequate allowance route lost to cheaper paid route".into());
        }
        if selected_free["price"]["rates"][fixture::SEARCH_PRICE_KEY]["rate"].as_f64() != Some(10.0)
            || selected_free["price"].get("estimated_task_usd").is_some()
        {
            return Err(
                "allowance answer lost its catalog rate or invented a quantity estimate".into(),
            );
        }
        let free_key = format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER);
        facts.search.get_mut(&free_key).unwrap().grade = Some(100);
        self.subject.configure_selection_facts(&facts).await?;
        if selected(route, params).await?["engine"]["model"] != "model-d" {
            return Err("below-level allowance route displaced adequate paid route".into());
        }
        params["requirements"] = json!({});
        if selected(route, params).await?["engine"]["model"] != "model-d" {
            return Err("allowance zero outranked grade without a level".into());
        }
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
                let subject = FakeSubject::with_search(Defects::default());
                assert_eq!(Suite::new(&subject).run(stringify!($name)).await, Ok(Ending::Passed));
            }
        )* };
    }

    cases!(
        select_search_advertisement,
        select_search_declarations,
        select_search_nonce,
        select_search_requirements,
        select_search_privacy,
        select_search_allowance,
        select_search_ranking,
        select_search_reports
    );

    #[tokio::test]
    async fn select_search_absent_group_refuses_search() {
        let subject = FakeSubject::with_media(Defects::default());
        assert_eq!(
            Suite::new(&subject)
                .run("select_search_advertisement")
                .await,
            Ok(Ending::Passed)
        );
    }

    #[tokio::test]
    async fn select_search_faulty_routers_fail_by_check_name() {
        for spec in crate::CHECKS
            .iter()
            .filter(|spec| spec.name.starts_with("select_search_"))
        {
            let subject = FakeSubject::with_search(Defects {
                extension: Some(spec.name),
                ..Defects::default()
            });
            let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
            assert_eq!(report.check, spec.name);
            assert!(
                matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                "{} accepted an incorrect router: {report:?}",
                spec.name
            );
            println!("intentional incorrect router rejected: {report:?}");
        }
    }

    #[test]
    fn select_search_rule_map_covers_pinned_rules_and_registered_cases() {
        use std::collections::BTreeSet;
        let contract = include_str!("../../../cortexkit-role-routing/CONTRACT.md");
        let ids: BTreeSet<_> = contract
            .lines()
            .filter_map(|line| {
                line.strip_prefix("- [pinned] `select_search.")
                    .and_then(|tail| tail.split_once("`:"))
            })
            .map(|(id, _)| format!("select_search.{id}"))
            .collect();
        let map: Vec<Value> = serde_json::from_str(include_str!("../../rule-map.json")).unwrap();
        let mapped: Vec<_> = map
            .iter()
            .filter(|rule| rule["id"].as_str().unwrap().starts_with("select_search."))
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
            .filter(|spec| spec.name.starts_with("select_search_"))
            .map(|spec| spec.name)
            .collect();
        assert_eq!(cases, registered);
    }
}
