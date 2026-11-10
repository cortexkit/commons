use super::*;
use crate::{subject::RunnerRouteFacts, wire::head_choices::HeadChoicesAnswer};

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn harness_limits_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let pairs = name == "harness_limits_pairs_narrow";
        let invalid = name == "harness_limits_invalid_falls_back";
        let mut catalog = fixture::catalog();
        catalog.models.truncate(2);
        if !pairs {
            catalog.models[1].provider = "other-provider".into();
        }
        let keys: Vec<_> = catalog
            .models
            .iter()
            .map(|model| model.route_key())
            .collect();
        catalog.runners.truncate(1);
        catalog.runners[0].route_keys = keys.clone();
        self.subject.configure_catalog(&catalog).await?;
        if pairs
            && !self
                .subject
                .configure_runner_route_facts(&[
                    RunnerRouteFacts::new(&keys[0], "example.responses", "apikey"),
                    RunnerRouteFacts::new(&keys[1], "example.messages", "oauth"),
                ])
                .await?
        {
            return Ok(Ending::NotApplicable(
                "configure_runner_route_facts is unsupported".into(),
            ));
        }
        let mut limits = if pairs {
            json!({"alpha": {"pairs": [{"wire_api": "example.messages", "auth_method": "oauth"}]}})
        } else {
            json!({"alpha": {"explicit_providers": ["other-provider"]}})
        };
        if invalid {
            limits["typo"] = json!({});
        }
        self.subject.configure_catalog_declarations(&[CatalogModule {
            principal: fixture::RUNNER_ONE.into(),
            nonce_verified: true,
            role_declarations: [(crate::wire::ROLE_DECLARATIONS_KEY.into(), json!({"runner": {
                "pairs": [{"wire_api": "example.responses", "auth_method": "apikey"},
                          {"wire_api": "example.messages", "auth_method": "oauth"}],
                "standing_limits": {"explicit_providers": [fixture::PRICED_PROVIDER, "other-provider"]},
                "harnesses": ["alpha", "beta"], "default_harness": "alpha",
                "harness_limits": limits
            }}))].into(),
        }]).await?;

        // An omitted select harness resolves the declared default before matching.
        let selected = select(&route, &base(), name).await?;
        if !invalid && selected.route.route_key() != keys[1] {
            return Err(format!(
                "{name}: default alpha must select {}, got {}",
                keys[1],
                selected.route.route_key()
            ));
        }
        let allowed = select(&route, &with(base(), "pin", json!(keys[1])), name).await?;
        if allowed.runner.module != fixture::RUNNER_ONE {
            return Err(format!(
                "{name}: runner was lost after reading harness_limits"
            ));
        }
        if invalid {
            select(&route, &with(base(), "pin", json!(keys[0])), name).await?;
        } else {
            refused(
                &route,
                ops::ROUTE_SELECT,
                &with(base(), "pin", json!(keys[0])),
                RefusalCode::PinUnavailable,
                name,
            )
            .await?;
        }
        if describe.serves(SELECT_HARNESS) {
            for key in &keys {
                let request = with(with(base(), "harness", json!("beta")), "pin", json!(key));
                select(&route, &request, "beta inherits runner values").await?;
            }
            if invalid {
                let request = with(
                    with(base(), "harness", json!("alpha")),
                    "pin",
                    json!(keys[0]),
                );
                select(
                    &route,
                    &request,
                    "invalid map discards even valid alpha restrictions",
                )
                .await?;
            }
        }
        if describe.serves(crate::wire::capabilities::HEAD_CHOICES) {
            let baseline: HeadChoicesAnswer = answer(
                &route,
                ops::ROUTE_HEAD_CHOICES,
                &json!({"runner_module": fixture::RUNNER_ONE}),
                "runner-level head choices",
            )
            .await?;
            if baseline
                .models
                .iter()
                .any(|model| !keys.contains(&format!("{}/{}", model.provider, model.model)))
            {
                return Err(format!(
                    "{name}: runner-level choices offered a route the runner cannot run"
                ));
            }
            if !invalid
                && !baseline
                    .models
                    .iter()
                    .any(|model| format!("{}/{}", model.provider, model.model) == keys[0])
            {
                return Ok(Ending::NotApplicable(format!(
                    "runner-level head choices do not offer {}; harness narrowing is inconclusive",
                    keys[0]
                )));
            }
            for harness in [Some("alpha"), Some("beta")] {
                let mut request = json!({"runner_module": fixture::RUNNER_ONE});
                if let Some(harness) = harness {
                    request["harness"] = json!(harness);
                }
                let choices: HeadChoicesAnswer =
                    answer(&route, ops::ROUTE_HEAD_CHOICES, &request, name).await?;
                let mut got: Vec<_> = choices
                    .models
                    .iter()
                    .map(|model| format!("{}/{}", model.provider, model.model))
                    .collect();
                got.sort();
                let mut expected = if !invalid && harness == Some("alpha") {
                    vec![keys[1].clone()]
                } else {
                    keys.clone()
                };
                expected.sort();
                if got.iter().any(|key| !expected.contains(key)) {
                    return Err(format!(
                        "{name}: harness {harness:?} expected a subset of {expected:?}, got {got:?}"
                    ));
                }
            }
            let refusal = refused(
                &route,
                ops::ROUTE_HEAD_CHOICES,
                &json!({"runner_module": fixture::RUNNER_ONE, "harness": "unhosted"}),
                RefusalCode::NoRoute,
                "unknown head-choice harness",
            )
            .await?;
            let detail: NoRouteDetail = refusal
                .detail_as()
                .ok_or("missing harness refusal detail")?;
            if detail.reason != NoRouteReason::HarnessUnavailable
                || !refusal.message.contains("unhosted")
            {
                return Err(format!(
                    "unknown harness must name harness_unavailable: {refusal:?}"
                ));
            }
        }
        Ok(Ending::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::select_engine::fake::{Defects, FakeSubject};

    #[tokio::test]
    async fn harness_limits_pair_narrowing() {
        let subject = FakeSubject::new(Defects::default());
        assert_eq!(
            Suite::new(&subject)
                .run("harness_limits_pairs_narrow")
                .await,
            Ok(Ending::Passed)
        );
    }

    #[tokio::test]
    async fn harness_limits_provider_narrowing_and_invalid_fallback() {
        let subject = FakeSubject::new(Defects::default());
        for name in [
            "harness_limits_providers_narrow",
            "harness_limits_invalid_falls_back",
        ] {
            assert_eq!(
                Suite::new(&subject).run(name).await,
                Ok(Ending::Passed),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn harness_limits_faulty_routers_are_rejected_by_check_name() {
        for name in [
            "harness_limits_pairs_narrow",
            "harness_limits_providers_narrow",
            "harness_limits_invalid_falls_back",
        ] {
            let subject = FakeSubject::new(Defects {
                extension: Some(name),
                ..Defects::default()
            });
            let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
            let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
            assert_eq!(report.check, name);
            assert!(
                matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                "{report:?}"
            );
        }
    }

    #[tokio::test]
    async fn harness_limits_unsupported_pair_facts_are_skipped_not_passed() {
        let subject = FakeSubject::new(Defects {
            runner_route_facts_unsupported: true,
            ..Defects::default()
        });
        assert!(
            matches!(Suite::new(&subject).run("harness_limits_pairs_narrow").await,
            Ok(Ending::NotApplicable(reason)) if reason.contains("configure_runner_route_facts"))
        );
        for name in [
            "harness_limits_providers_narrow",
            "harness_limits_invalid_falls_back",
        ] {
            assert_eq!(
                Suite::new(&subject).run(name).await,
                Ok(Ending::Passed),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn harness_limits_picker_overoffers_are_rejected_even_when_selection_narrows() {
        let subject = FakeSubject::new(Defects {
            extension: Some("harness_limits_head_choices_ignored"),
            ..Defects::default()
        });
        for name in [
            "harness_limits_pairs_narrow",
            "harness_limits_providers_narrow",
        ] {
            let error = Suite::new(&subject).run(name).await.unwrap_err();
            assert!(
                error.contains("harness Some(\"alpha\") expected"),
                "{name}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn harness_limits_picker_narrowing_without_baseline_offer_is_skipped() {
        let subject = FakeSubject::new(Defects {
            extension: Some("harness_limits_baseline_omits_excluded"),
            ..Defects::default()
        });
        for name in [
            "harness_limits_pairs_narrow",
            "harness_limits_providers_narrow",
        ] {
            assert!(
                matches!(Suite::new(&subject).run(name).await,
                Ok(Ending::NotApplicable(reason)) if reason.contains("narrowing is inconclusive")),
                "{name}"
            );
        }
    }
}
