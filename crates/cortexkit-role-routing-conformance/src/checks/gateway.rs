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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::select_engine::fake::{Defects, FakeSubject};

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
