use super::*;
use crate::wire::{head_choices::HeadChoicesAnswer, spellings};
use crate::RunnerRouteFacts;

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn requires_wire_api_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe = answer(&route, ops::ROLE_DESCRIBE, &json!({}), name).await?;
        let mut catalog = fixture::catalog();
        catalog.models.truncate(1);
        let mut other = catalog.models[0].clone();
        other.model = "model-b".into();
        catalog.models.push(other);
        let keys: Vec<_> = catalog
            .models
            .iter()
            .map(|model| model.route_key())
            .collect();
        catalog.runners.truncate(1);
        catalog.runners[0].route_keys = keys.clone();
        self.subject.configure_catalog(&catalog).await?;
        let wire_api = spellings::WIRE_APIS[1];
        let auth_method = spellings::AUTH_METHODS[0];
        if !self
            .subject
            .configure_runner_route_facts(&[
                RunnerRouteFacts::new(&keys[0], wire_api, auth_method),
                RunnerRouteFacts::new(&keys[1], wire_api, auth_method),
            ])
            .await?
        {
            return Ok(Ending::NotApplicable(
                "configure_runner_route_facts is unsupported".into(),
            ));
        }
        for (key, endpoint) in [(&keys[0], Some("https://example.com/v1")), (&keys[1], None)] {
            if !self.subject.configure_route_wire_api(key, endpoint).await? {
                return Ok(Ending::NotApplicable(
                    "configure_route_wire_api is unsupported".into(),
                ));
            }
        }
        self.subject
            .configure_catalog_declarations(&[CatalogModule {
                principal: fixture::RUNNER_ONE.into(),
                nonce_verified: true,
                role_declarations: [(
                    crate::wire::ROLE_DECLARATIONS_KEY.into(),
                    json!({"runner": {
                        "pairs": [{"wire_api": wire_api, "auth_method": auth_method}],
                        "requires_wire_api": true
                    }}),
                )]
                .into(),
            }])
            .await?;

        let selected = select(&route, &base(), name).await?;
        if selected.route.route_key() != keys[0] || selected.runner.module != fixture::RUNNER_ONE {
            return Err(format!(
                "{name}: expected only served row {}, got {selected:?}",
                keys[0]
            ));
        }
        refused(
            &route,
            ops::ROUTE_SELECT,
            &with(base(), "pin", json!(keys[1])),
            RefusalCode::PinUnavailable,
            name,
        )
        .await?;
        if describe.serves(capabilities::HEAD_CHOICES) {
            let choices: HeadChoicesAnswer = answer(
                &route,
                ops::ROUTE_HEAD_CHOICES,
                &json!({"runner_module": fixture::RUNNER_ONE}),
                name,
            )
            .await?;
            if choices
                .models
                .iter()
                .any(|model| format!("{}/{}", model.provider, model.model) != keys[0])
            {
                return Err(format!(
                    "{name}: head choices offered a row without served wire.api: {choices:?}"
                ));
            }
        }

        if !self
            .subject
            .configure_route_wire_api(&keys[1], Some("https://example.com/v2"))
            .await?
        {
            return Ok(Ending::NotApplicable(
                "configure_route_wire_api is unsupported".into(),
            ));
        }
        let selected = select(&route, &with(base(), "pin", json!(keys[1])), name).await?;
        if selected.route.route_key() != keys[1] || selected.runner.module != fixture::RUNNER_ONE {
            return Err(format!(
                "{name}: newly served wire.api did not admit {}: {selected:?}",
                keys[1]
            ));
        }
        // Two endpoint-free rows distinguish a pair override from its runner's
        // default in both directions; neither row can hide behind a served API.
        if !self
            .subject
            .configure_runner_route_facts(&[
                RunnerRouteFacts::new(&keys[0], wire_api, auth_method),
                RunnerRouteFacts::new(&keys[1], spellings::WIRE_APIS[0], auth_method),
            ])
            .await?
        {
            return Ok(Ending::NotApplicable(
                "configure_runner_route_facts is unsupported".into(),
            ));
        }
        for key in &keys {
            if !self.subject.configure_route_wire_api(key, None).await? {
                return Ok(Ending::NotApplicable(
                    "configure_route_wire_api is unsupported".into(),
                ));
            }
        }
        for runner_default in [true, false] {
            self.subject
                .configure_catalog_declarations(&[CatalogModule {
                    principal: fixture::RUNNER_ONE.into(),
                    nonce_verified: true,
                    role_declarations: [(
                        crate::wire::ROLE_DECLARATIONS_KEY.into(),
                        json!({"runner": {
                            "pairs": [
                                {"wire_api": wire_api, "auth_method": auth_method,
                                 "requires_wire_api": !runner_default},
                                {"wire_api": spellings::WIRE_APIS[0], "auth_method": auth_method}
                            ],
                            "requires_wire_api": runner_default
                        }}),
                    )]
                    .into(),
                }])
                .await?;
            let (allowed, blocked) = if runner_default {
                (&keys[0], &keys[1])
            } else {
                (&keys[1], &keys[0])
            };
            for request in [base(), with(base(), "pin", json!(allowed))] {
                let selected = select(&route, &request, name).await?;
                if selected.route.route_key() != *allowed
                    || selected.runner.module != fixture::RUNNER_ONE
                {
                    return Err(format!(
                        "{name}: pair override expected {allowed}, got {selected:?}"
                    ));
                }
            }
            refused(
                &route,
                ops::ROUTE_SELECT,
                &with(base(), "pin", json!(blocked)),
                RefusalCode::PinUnavailable,
                name,
            )
            .await?;
            if describe.serves(capabilities::HEAD_CHOICES) {
                let choices: HeadChoicesAnswer = answer(
                    &route,
                    ops::ROUTE_HEAD_CHOICES,
                    &json!({"runner_module": fixture::RUNNER_ONE}),
                    name,
                )
                .await?;
                if choices
                    .models
                    .iter()
                    .any(|model| format!("{}/{}", model.provider, model.model) != *allowed)
                {
                    return Err(format!("{name}: pair override head choices expected only {allowed}, got {choices:?}"));
                }
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
    async fn requires_wire_api_served_rows_only_then_endpoint_appears() {
        for subject in [
            FakeSubject::new(Defects::default()),
            FakeSubject::without_extensions(Defects::default()),
        ] {
            assert_eq!(
                Suite::new(&subject).run("runner_requires_wire_api").await,
                Ok(Ending::Passed)
            );
        }
    }

    #[tokio::test]
    async fn requires_wire_api_faulty_routers_are_rejected_by_check_name() {
        let name = "runner_requires_wire_api";
        let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
        for fault in [
            name,
            "requires_wire_api_head_choices_ignored",
            "requires_wire_api_stale",
            "requires_wire_api_pair_false_ignored",
            "requires_wire_api_pair_true_ignored",
        ] {
            let subject = FakeSubject::new(Defects {
                extension: Some(fault),
                ..Defects::default()
            });
            let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
            assert_eq!(report.check, name);
            assert!(
                matches!(report.outcome, crate::CheckOutcome::Failed { .. }),
                "{fault}: {report:?}"
            );
        }
    }

    #[tokio::test]
    async fn requires_wire_api_missing_hooks_are_skipped_not_passed() {
        for (defects, hook) in [
            (
                Defects {
                    route_wire_api_unsupported: true,
                    ..Defects::default()
                },
                "configure_route_wire_api",
            ),
            (
                Defects {
                    runner_route_facts_unsupported: true,
                    ..Defects::default()
                },
                "configure_runner_route_facts",
            ),
        ] {
            let subject = FakeSubject::new(defects);
            assert!(
                matches!(Suite::new(&subject).run("runner_requires_wire_api").await,
                Ok(Ending::NotApplicable(reason)) if reason.contains(hook))
            );
        }
    }
}
