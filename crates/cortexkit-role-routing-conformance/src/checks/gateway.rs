use super::*;
use crate::RunnerRouteFacts;

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn gateway_check(&self, name: &str) -> Result<Ending, String> {
        let mut catalog = fixture::catalog();
        catalog.models.retain(|model| {
            model.provider == fixture::SUBSCRIPTION_PROVIDER && model.model == "model-c"
        });
        catalog.models[0].provider = "anthropic-gw-example".into();
        let model = &catalog.models[0];
        let key = model.route_key();
        catalog.runners.truncate(1);
        catalog.runners[0].route_keys = vec![key.clone()];
        catalog.accounts.truncate(1);
        catalog.accounts[0].provider.clone_from(&model.provider);
        catalog.accounts[0].method = "gateway".into();
        self.subject.configure_catalog(&catalog).await?;
        if !self
            .subject
            .configure_runner_route_facts(&[RunnerRouteFacts::new(
                &key,
                "anthropic.messages",
                "gateway",
            )])
            .await?
        {
            return Ok(Ending::NotApplicable(
                "configure_runner_route_facts is unsupported".into(),
            ));
        }
        // The gateway credential owns its endpoint, so a catalog endpoint is
        // unnecessary for this pair's default requires_wire_api=false.
        if !self.subject.configure_route_wire_api(&key, None).await? {
            return Ok(Ending::NotApplicable(
                "configure_route_wire_api is unsupported".into(),
            ));
        }
        self.subject
            .configure_catalog_declarations(&[CatalogModule {
                principal: fixture::RUNNER_ONE.into(),
                nonce_verified: true,
                role_declarations: [(
                    crate::wire::ROLE_DECLARATIONS_KEY.into(),
                    json!({"runner": {
                        "pairs": [{"wire_api": "anthropic.messages", "auth_method": "gateway"}],
                        "follows_account_choice": true
                    }}),
                )]
                .into(),
            }])
            .await?;

        let consumer = self.route_as(CONSUMER).await?;
        let selected = select(&consumer, &base(), name).await?;
        if selected.route.route_key() != key || selected.runner.module != fixture::RUNNER_ONE {
            return Err(format!(
                "{name}: expected gateway route {key} on {}, got {selected:?}",
                fixture::RUNNER_ONE
            ));
        }
        let runner = self.route_as(&selected.runner.module).await?;
        let admitted: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &json!({
                "provider": model.provider,
                "model": model.model,
                "conversation_key": "gateway-conversation",
                "admission_id": "gateway-admission",
                "decision_id": selected.decision_id
            }),
            name,
        )
        .await?;
        let expected = &catalog.accounts[0];
        if admitted.account.provider != expected.provider
            || admitted.account.account_id != expected.account_id
            || admitted.account.credential_id != expected.credential_id
            || admitted.account.method != "gateway"
        {
            return Err(format!(
                "{name}: expected the seeded gateway credential/account {expected:?}, got {:?}",
                admitted.account
            ));
        }
        Ok(Ending::Passed)
    }

    pub(super) async fn gateway_explicit_providers_check(
        &self,
        name: &str,
    ) -> Result<Ending, String> {
        let mut catalog = fixture::catalog();
        catalog.models.truncate(2);
        catalog.models[0].provider = "anthropic".into();
        catalog.models[0].model = "claude-opus-5-5".into();
        catalog.models[1].provider = "openai".into();
        catalog.models[1].model = "model-example".into();
        let keys: Vec<_> = catalog
            .models
            .iter()
            .map(|model| model.route_key())
            .collect();
        catalog.runners.truncate(1);
        catalog.runners[0].route_keys = keys.clone();
        catalog.accounts.truncate(1);
        catalog.accounts[0].provider = "anthropic-gw-x".into();
        catalog.accounts[0].method = "gateway".into();

        for per_harness in [false, true] {
            for (providers, admitted_routes) in [
                (json!(["anthropic"]), [true, false]),
                (json!(["anthropic", "anthropic-gw-x"]), [true, false]),
                (json!(["anthropic-gw-x"]), [false, false]),
                // Both rows must be runnable when their served providers are
                // allowed, so an unavailable OpenAI route cannot pass a refusal.
                (json!(["anthropic", "openai"]), [true, true]),
            ] {
                // Rebuild the same two routes so decisions cannot preserve an
                // admission from a previous allow-list or harness variant.
                self.fresh().await?;
                self.subject.configure_catalog(&catalog).await?;
                if !self
                    .subject
                    .configure_runner_route_facts(&[
                        RunnerRouteFacts::new(&keys[0], "anthropic.messages", "gateway"),
                        RunnerRouteFacts::new(&keys[1], "openai.responses", "gateway"),
                    ])
                    .await?
                {
                    return Ok(Ending::NotApplicable(
                        "configure_runner_route_facts is unsupported".into(),
                    ));
                }
                let mut declaration = json!({
                    "pairs": [
                        {"wire_api": "anthropic.messages", "auth_method": "gateway"},
                        {"wire_api": "openai.responses", "auth_method": "gateway"}
                    ],
                    "follows_account_choice": true,
                    "harnesses": ["alpha", "beta"],
                    "default_harness": "alpha"
                });
                if per_harness {
                    declaration["harness_limits"] = json!({"alpha": {
                        "explicit_providers": providers
                    }});
                } else {
                    declaration["standing_limits"] = json!({"explicit_providers": providers});
                }
                self.subject
                    .configure_catalog_declarations(&[CatalogModule {
                        principal: fixture::RUNNER_ONE.into(),
                        nonce_verified: true,
                        role_declarations: [(
                            crate::wire::ROLE_DECLARATIONS_KEY.into(),
                            json!({"runner": declaration}),
                        )]
                        .into(),
                    }])
                    .await?;

                let consumer = self.route_as(CONSUMER).await?;
                let context = format!("{name}: per_harness={per_harness}, providers={providers}");
                for (index, admitted_route) in admitted_routes.into_iter().enumerate() {
                    let request = with(base(), "pin", json!(keys[index]));
                    if !admitted_route {
                        refused(
                            &consumer,
                            ops::ROUTE_SELECT,
                            &request,
                            RefusalCode::PinUnavailable,
                            &context,
                        )
                        .await?;
                        continue;
                    }
                    let selected = select(&consumer, &request, &context).await?;
                    if selected.route.route_key() != keys[index]
                        || selected.runner.module != fixture::RUNNER_ONE
                    {
                        return Err(format!(
                            "{context}: expected served route {}, got {selected:?}",
                            keys[index]
                        ));
                    }
                    let runner = self.route_as(&selected.runner.module).await?;
                    let admitted: AccountForAnswer = answer(
                        &runner,
                        ops::ROUTE_ACCOUNT_FOR,
                        &json!({
                            "provider": catalog.models[index].provider,
                            "model": catalog.models[index].model,
                            "conversation_key": format!("gateway-limit-conversation-{index}"),
                            "admission_id": format!("gateway-limit-admission-{index}"),
                            "decision_id": selected.decision_id
                        }),
                        &context,
                    )
                    .await?;
                    let expected = &catalog.accounts[0];
                    if admitted.account.provider != expected.provider
                        || admitted.account.account_id != expected.account_id
                        || admitted.account.credential_id != expected.credential_id
                        || admitted.account.method != "gateway"
                    {
                        return Err(format!(
                            "{context}: expected gateway credential {expected:?}, got {:?}",
                            admitted.account
                        ));
                    }
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
    async fn gateway_explicit_providers_compare_served_provider() {
        for subject in [
            FakeSubject::new(Defects::default()),
            FakeSubject::without_extensions(Defects::default()),
            FakeSubject::with_placement(Defects::default()),
        ] {
            assert_eq!(
                Suite::new(&subject)
                    .run("runner_gateway_explicit_providers")
                    .await,
                Ok(Ending::Passed)
            );
        }
    }

    #[tokio::test]
    async fn gateway_explicit_providers_faulty_routers_are_rejected_by_check_name() {
        let name = "runner_gateway_explicit_providers";
        let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
        for fault in [
            name,
            "gateway_explicit_providers_harness",
            "gateway_explicit_providers_alias",
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
            if fault == "gateway_explicit_providers_harness" {
                assert!(
                    matches!(&report.outcome, crate::CheckOutcome::Failed { reason }
                    if reason.contains("per_harness=true")),
                    "{report:?}"
                );
            }
            println!("{fault}: {report:?}");
        }
    }

    #[tokio::test]
    async fn gateway_explicit_providers_missing_facts_are_skipped_not_passed() {
        let subject = FakeSubject::new(Defects {
            runner_route_facts_unsupported: true,
            ..Defects::default()
        });
        assert!(
            matches!(Suite::new(&subject).run("runner_gateway_explicit_providers").await,
            Ok(Ending::NotApplicable(reason)) if reason.contains("configure_runner_route_facts"))
        );
    }

    #[tokio::test]
    async fn gateway_pair_selects_and_admits_its_credential() {
        for subject in [
            FakeSubject::new(Defects::default()),
            FakeSubject::without_extensions(Defects::default()),
        ] {
            assert_eq!(
                Suite::new(&subject).run("runner_gateway_credential").await,
                Ok(Ending::Passed)
            );
        }
    }

    #[tokio::test]
    async fn gateway_faulty_routers_are_rejected_by_check_name() {
        let name = "runner_gateway_credential";
        let spec = crate::CHECKS.iter().find(|spec| spec.name == name).unwrap();
        for fault in [name, "gateway_wrong_account_method"] {
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
    async fn gateway_missing_hooks_are_skipped_not_passed() {
        for (defects, hook) in [
            (
                Defects {
                    runner_route_facts_unsupported: true,
                    ..Defects::default()
                },
                "configure_runner_route_facts",
            ),
            (
                Defects {
                    route_wire_api_unsupported: true,
                    ..Defects::default()
                },
                "configure_route_wire_api",
            ),
        ] {
            let subject = FakeSubject::new(defects);
            assert!(
                matches!(Suite::new(&subject).run("runner_gateway_credential").await,
                Ok(Ending::NotApplicable(reason)) if reason.contains(hook))
            );
        }
    }
}
