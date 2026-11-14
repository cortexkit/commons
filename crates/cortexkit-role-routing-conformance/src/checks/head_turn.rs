use super::*;

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn head_turn_check(&self, name: &str) -> Result<Ending, String> {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let describe: RoleDescribe =
            answer(&runner, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        if !describe.serves_account_head_turn() {
            return Ok(Ending::NotApplicable(
                "account_head_turn capability gate".into(),
            ));
        }
        let mut catalog = fixture::catalog();
        catalog.accounts.truncate(1);
        let account_id = catalog.accounts[0].account_id.clone();
        self.subject.configure_catalog(&catalog).await?;
        self.subject
            .configure_account_concurrency(&account_id, 1)
            .await?;

        let mut seed = Self::account_for("head-seed", "head-seed", "model-c");
        if name == "account_head_turn_runner" {
            seed["head_turn"] = json!(true);
        }
        let admitted: AccountForAnswer =
            answer(&runner, ops::ROUTE_ACCOUNT_FOR, &seed, "seed admission").await?;
        if admitted.account.account_id != account_id {
            return Err("seed admission did not use the sole eligible account".into());
        }
        Self::head_turn_full(&runner, "head-ordinary", None).await?;
        Self::head_turn_full(&runner, "head-false", Some(false)).await?;

        match name {
            "account_head_turn_runner" => {
                let mut head = Self::account_for("head-full", "head-full", "model-c");
                head["head_turn"] = json!(true);
                let admitted: AccountForAnswer = answer(
                    &runner,
                    ops::ROUTE_ACCOUNT_FOR,
                    &head,
                    "head turn on full account",
                )
                .await?;
                if admitted.account.account_id != account_id {
                    return Err("head turn changed the eligible account".into());
                }
                Self::head_turn_full(&runner, "head-still-full", None).await?;
            }
            "account_head_turn_non_runner" => {
                // Media providers may admit their own remote engines, but are not runners.
                self.subject.configure_catalog_declarations(&[CatalogModule {
                    principal: "head-media".into(),
                    nonce_verified: true,
                    role_declarations: [(
                        cortexkit_role_routing::select_engine::ROLE_DECLARATIONS_KEY.into(),
                        json!({"engines":[{"kind":"transcribe", "provider":fixture::SUBSCRIPTION_PROVIDER,
                            "model":"model-c", "local":false, "formats":["audio/wav"],
                            "latency_class":"interactive"}]}),
                    )].into(),
                }]).await?;
                let media = self.route_as("head-media").await?;
                Self::head_turn_full(&media, "media-ordinary", None).await?;
                Self::head_turn_full(&media, "media-head", Some(true)).await?;
            }
            _ => return Err(format!("unknown head-turn check {name}")),
        }
        Ok(Ending::Passed)
    }

    async fn head_turn_full<R: RoutingRoute>(route: &R, id: &str, head: Option<bool>) -> Check {
        let mut params = Self::account_for(id, id, "model-c");
        if let Some(head) = head {
            params["head_turn"] = json!(head);
        }
        let refusal = refused(
            route,
            ops::ROUTE_ACCOUNT_FOR,
            &params,
            RefusalCode::NoRoute,
            id,
        )
        .await?;
        if refusal.detail.get("reason") != Some(&json!("concurrency_full"))
            || refusal
                .detail
                .get("reset_at_ms")
                .and_then(Value::as_u64)
                .is_none_or(|at| at <= now_ms())
        {
            return Err(format!(
                "{id}: expected concurrency_full with a future reset_at_ms, got {refusal}"
            ));
        }
        Ok(())
    }
}
