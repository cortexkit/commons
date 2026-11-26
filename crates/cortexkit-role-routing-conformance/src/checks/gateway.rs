use super::*;

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn gateway_check(&self, name: &str) -> Result<Ending, String> {
        let mut catalog = fixture::catalog();
        catalog.models.retain(|model| {
            model.provider == fixture::SUBSCRIPTION_PROVIDER && model.model == "model-c"
        });
        let model = &catalog.models[0];
        let key = format!("anthropic-gw-example/{}", model.model);
        catalog.runners.truncate(1);
        catalog.runners[0].route_keys.clear();
        catalog.runners[0].harnesses = Some(vec![fixture::HARNESS.into()]);
        catalog.accounts.truncate(1);
        catalog.accounts[0].provider = "anthropic-gw-example".into();
        catalog.accounts[0].serves = vec![model.provider.clone()];
        catalog.accounts[0].method = "gateway".into();
        self.subject.configure_catalog(&catalog).await?;
        self.subject
            .configure_catalog_declarations(&[CatalogModule {
                principal: fixture::RUNNER_ONE.into(),
                nonce_verified: true,
                role_declarations: [(
                    crate::wire::ROLE_DECLARATIONS_KEY.into(),
                    json!({"runner": {
                        "harnesses": [fixture::HARNESS],
                        "follows_account_choice": true
                    }}),
                )]
                .into(),
            }])
            .await?;
        let snapshot = json!({"generation": "one", "harnesses": {(fixture::HARNESS): [
            {"provider": catalog.accounts[0].provider, "models": "all"}
        ]}});
        if !self
            .subject
            .configure_runner_serves(fixture::RUNNER_ONE, &snapshot)
            .await?
        {
            return Ok(Ending::NotApplicable(
                "configure_runner_serves is unsupported".into(),
            ));
        }
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
                "provider": catalog.accounts[0].provider,
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
        // The same gateway id no longer has catalog rows if its credential
        // names no provider or names more than one.
        let request = with(base(), "pin", json!(key));
        for serves in [
            vec![],
            vec![model.provider.clone(), "unrelated-provider".into()],
        ] {
            let mut unresolved = catalog.clone();
            unresolved.accounts[0].serves = serves;
            self.subject.configure_catalog(&unresolved).await?;
            refused(
                &consumer,
                ops::ROUTE_SELECT,
                &request,
                RefusalCode::PinUnavailable,
                "a gateway with no unique catalog provider gets no route",
            )
            .await?;
        }
        Ok(Ending::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::select_engine::fake::{Defects, FakeSubject};

    #[tokio::test]
    async fn gateway_serves_selects_and_admits_its_credential() {
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
        for fault in [
            name,
            "gateway_wrong_account_method",
            "gateway_serves_ambiguous",
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
    async fn gateway_missing_serves_hook_is_skipped_not_passed() {
        let subject = FakeSubject::new(Defects {
            runner_serves_unsupported: true,
            ..Defects::default()
        });
        assert!(
            matches!(Suite::new(&subject).run("runner_gateway_credential").await,
            Ok(Ending::NotApplicable(reason)) if reason.contains("configure_runner_serves"))
        );
    }
}
