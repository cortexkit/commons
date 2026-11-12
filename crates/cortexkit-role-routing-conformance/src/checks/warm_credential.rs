use super::*;

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn warm_credential_check(&self, name: &str) -> Result<Ending, String> {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let describe: RoleDescribe =
            answer(&runner, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let undeclared = name == "warm_credential_undeclared_refused";
        if describe.serves(capabilities::ACCOUNT_WARM_CREDENTIAL) == undeclared {
            return Ok(Ending::NotApplicable(
                "account_warm_credential declaration does not match this case".into(),
            ));
        }
        if undeclared {
            let mut params = Self::account_for("warm-undeclared", "warm-undeclared", "model-c");
            params["warm_credential"] = json!("credential-2");
            warm_refused(&runner, &params).await?;
            return Ok(Ending::Passed);
        }

        let cold_params = Self::account_for("warm-cold", "warm-existing", "model-c");
        let cold: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &cold_params,
            "cold admission",
        )
        .await?;
        let catalog = fixture::catalog();
        let original = catalog
            .accounts
            .iter()
            .find(|a| {
                a.account_id == cold.account.account_id
                    && a.credential_id == cold.account.credential_id
            })
            .ok_or("cold admission is not a seeded credential/account pair")?;
        let other = catalog
            .accounts
            .iter()
            .find(|a| a.account_id != original.account_id)
            .ok_or("warm credential fixture needs two accounts")?;
        let mut params = Self::account_for("warm-admission", "warm-fresh", "model-c");
        params["warm_credential"] = json!(other.credential_id);
        match name {
            "warm_credential_hard_filter_ignored" => {
                params["candidates"] = json!({"view_digest":"warm-view", "accounts":[{
                    "account_id":original.account_id, "credential_id":original.credential_id
                }]});
            }
            "warm_credential_unknown_ignored" => {
                params["warm_credential"] = json!("credential-never-seeded");
            }
            "warm_credential_existing_affinity" => {
                params["conversation_key"] = cold_params["conversation_key"].clone();
            }
            "warm_credential_non_admit_refused" => {
                for mode in ["renew", "reassign"] {
                    params["mode"] = json!(mode);
                    params["conversation_key"] = cold_params["conversation_key"].clone();
                    params["expected_pin"] =
                        json!({"id":cold.pin.id, "generation":cold.pin.generation});
                    warm_refused(&runner, &params).await?;
                }
                if describe.serves(capabilities::ACCOUNT_CONTINUE) {
                    let continued = json!({
                        "mode":"continue", "run_id":"warm-run", "admission_id":cold.admission.id,
                        "conversation_key":cold_params["conversation_key"],
                        "previous_decision_id":cold.reason.decision_id,
                        "exhausted": {"credential_id":original.credential_id,
                            "account":original.account_id, "limit_window":"five_hour", "observed_at_ms":now_ms()},
                        "selection_id":"warm-selection", "warm_credential":other.credential_id
                    });
                    warm_refused(&runner, &continued).await?;
                }
                return Ok(Ending::Passed);
            }
            "warm_credential_fresh_honoured" | "warm_credential_admission_conflict" => {}
            _ => return Err(format!("unknown warm credential check {name}")),
        }
        let first: Value =
            answer(&runner, ops::ROUTE_ACCOUNT_FOR, &params, "hinted admission").await?;
        let got: AccountForAnswer = serde_json::from_value(first.clone())
            .map_err(|e| format!("hinted admission does not decode: {e}"))?;
        let expected = match name {
            "warm_credential_fresh_honoured" => Some(other),
            "warm_credential_hard_filter_ignored" | "warm_credential_existing_affinity" => {
                Some(original)
            }
            _ => None,
        };
        if let Some(expected) = expected {
            if got.account.account_id != expected.account_id
                || got.account.credential_id != expected.credential_id
            {
                return Err(format!(
                    "expected credential {}, got {} on {}",
                    expected.credential_id, got.account.credential_id, got.account.account_id
                ));
            }
        } else if !catalog.accounts.iter().any(|a| {
            a.account_id == got.account.account_id && a.credential_id == got.account.credential_id
        }) {
            return Err("ignored hint must fall back to a seeded credential/account pair".into());
        }
        if name == "warm_credential_admission_conflict" {
            let mut changed = params.clone();
            changed["warm_credential"] = json!(original.credential_id);
            refused(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &changed,
                RefusalCode::AdmissionConflict,
                "same admission id with changed hint",
            )
            .await?;
            let replay: Value =
                answer(&runner, ops::ROUTE_ACCOUNT_FOR, &params, "hint replay").await?;
            if replay != first {
                return Err("hint conflict replaced the original admission answer".into());
            }
        }
        if name == "warm_credential_fresh_honoured" {
            params.as_object_mut().unwrap().remove("warm_credential");
            params["admission_id"] = json!("warm-follow-up");
            let next: AccountForAnswer =
                answer(&runner, ops::ROUTE_ACCOUNT_FOR, &params, "warm affinity").await?;
            if next.account != got.account {
                return Err("honoured hint did not become conversation affinity".into());
            }
        }
        Ok(Ending::Passed)
    }
}

async fn warm_refused<R: RoutingRoute>(runner: &R, params: &Value) -> Check {
    let refusal = refused(
        runner,
        ops::ROUTE_ACCOUNT_FOR,
        params,
        RefusalCode::InvalidRequest,
        "unsupported warm credential hint",
    )
    .await?;
    if refusal.detail.get("field") != Some(&json!("warm_credential")) {
        return Err(format!(
            "warm credential refusal names the wrong field: {:?}",
            refusal.detail
        ));
    }
    Ok(())
}
