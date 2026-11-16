use super::*;

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn fork_of_check(&self, name: &str) -> Result<Ending, String> {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let describe: RoleDescribe =
            answer(&runner, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let undeclared = name == "fork_of_undeclared_refused";
        if describe.serves(capabilities::ACCOUNT_FORK_OF) == undeclared {
            return Ok(Ending::NotApplicable(
                "account_fork_of declaration does not match this case".into(),
            ));
        }
        let mut params = Self::account_for("fork-admission", "fork-conversation", "model-c");
        params["fork_of"] = json!({"admission_id":"fork-parent"});
        let catalog = fixture::catalog();
        let parent_account = &catalog.accounts[0];
        let sibling = &catalog.accounts[1];
        let mut parent_params = Self::account_for("fork-parent", "parent-conversation", "model-c");
        parent_params["candidates"] = json!({"view_digest":"fork-parent-view", "accounts":[{
            "account_id":parent_account.account_id, "credential_id":parent_account.credential_id
        }]});
        let parent: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &parent_params,
            "parent admission",
        )
        .await?;
        if parent.account.account_id != parent_account.account_id
            || parent.account.credential_id != parent_account.credential_id
        {
            return Err("parent admission escaped its exact candidate".into());
        }
        if undeclared {
            fork_invalid(&runner, &params).await?;
            return Ok(Ending::Passed);
        }
        match name {
            "fork_of_parent_account" => {
                let _: AccountObserveAnswer = answer(
                    &runner,
                    ops::ROUTE_ACCOUNT_OBSERVE,
                    &json!({"event_id":"fork-parent-started", "kind":"admission_started",
                        "admission_id":parent.admission.id, "run_id":"fork-parent-run",
                        "served_account_id":parent.account.account_id,
                        "credential_id":parent.account.credential_id, "observed_at_ms":now_ms()}),
                    "start parent",
                )
                .await?;
                let control: AccountForAnswer = answer(
                    &runner,
                    ops::ROUTE_ACCOUNT_FOR,
                    &Self::account_for("fork-cold-control", "fork-cold-control", "model-c"),
                    "ordinary fresh admission with busy parent",
                )
                .await?;
                if control.account.account_id == parent.account.account_id {
                    return Ok(Ending::NotApplicable(
                        "ordinary fresh admission also chose the parent's account; the router's scoring does not establish a better sibling".into(),
                    ));
                }
                if control.account.account_id != sibling.account_id
                    || control.account.credential_id != sibling.credential_id
                {
                    return Err("cold control escaped the seeded sibling account".into());
                }
                // Release the control's load so the parent remains the busy
                // account when the fork is admitted.
                for kind in ["admission_started", "run_finished"] {
                    let _: AccountObserveAnswer = answer(
                        &runner, ops::ROUTE_ACCOUNT_OBSERVE,
                        &json!({"event_id":format!("fork-control-{kind}"), "kind":kind,
                            "admission_id":control.admission.id, "run_id":"fork-control-run",
                            "served_account_id":control.account.account_id,
                            "credential_id":control.account.credential_id, "observed_at_ms":now_ms()}),
                        "finish scoring control",
                    ).await?;
                }
                let fork: AccountForAnswer =
                    answer(&runner, ops::ROUTE_ACCOUNT_FOR, &params, "fork admission").await?;
                if fork.account != parent.account {
                    return Err(
                        "fork did not retain the parent's exact account and credential".into(),
                    );
                }
                params.as_object_mut().unwrap().remove("fork_of");
                params["admission_id"] = json!("fork-affinity-follow-up");
                let follow_up: AccountForAnswer =
                    answer(&runner, ops::ROUTE_ACCOUNT_FOR, &params, "fork affinity").await?;
                if follow_up.account != parent.account {
                    return Err("fork did not establish its own conversation affinity".into());
                }
            }
            "fork_of_blocked_parent" | "fork_of_unknown_retry" => {
                let reset = (name == "fork_of_blocked_parent").then(|| now_ms() + 60_000);
                self.subject
                    .block_account(&parent.account.account_id, reset)
                    .await?;
                let before = now_ms();
                let refusal = refused(
                    &runner,
                    ops::ROUTE_ACCOUNT_FOR,
                    &params,
                    RefusalCode::AccountHold,
                    "unavailable fork parent",
                )
                .await?;
                let after = now_ms();
                if refusal.detail.get("reason") != Some(&json!("fork_parent_account_unavailable")) {
                    return Err("fork parent hold has the wrong reason".into());
                }
                let until = refusal
                    .detail
                    .get("until_ms")
                    .and_then(Value::as_u64)
                    .ok_or("fork parent hold must have until_ms")?;
                let earliest = reset.unwrap_or(before + 900_000).saturating_sub(1_000);
                let latest = reset.unwrap_or(after + 900_000) + 1_000;
                if until < earliest || until > latest {
                    return Err(format!(
                        "fork parent hold recheck {until} is outside {earliest}..={latest}"
                    ));
                }
            }
            "fork_of_private_parent" => {
                let other = self.route_as(fixture::RUNNER_TWO).await?;
                parent_params["admission_id"] = json!("fork-other-principal-parent");
                parent_params["conversation_key"] = json!("other-principal-conversation");
                let _: AccountForAnswer = answer(
                    &other,
                    ops::ROUTE_ACCOUNT_FOR,
                    &parent_params,
                    "other principal's admission",
                )
                .await?;
                params["fork_of"]["admission_id"] = json!("fork-unknown-parent");
                let unknown = fork_invalid(&runner, &params).await?;
                params["admission_id"] = json!("fork-private-question");
                params["fork_of"]["admission_id"] = parent_params["admission_id"].clone();
                let private = fork_invalid(&runner, &params).await?;
                if unknown.code != private.code
                    || unknown.message != private.message
                    || unknown.detail != private.detail
                {
                    return Err("unknown and other-principal fork refusals differ".into());
                }
            }
            "fork_of_non_admit_refused" => {
                for mode in ["renew", "reassign", "future-mode", "continue"] {
                    params["mode"] = json!(mode);
                    params["expected_pin"] =
                        json!({"id":parent.pin.id, "generation":parent.pin.generation});
                    fork_invalid(&runner, &params).await?;
                }
            }
            "fork_of_admission_conflict" => {
                let _: AccountForAnswer =
                    answer(&runner, ops::ROUTE_ACCOUNT_FOR, &params, "first fork").await?;
                parent_params["admission_id"] = json!("fork-second-parent");
                parent_params["conversation_key"] = json!("second-parent-conversation");
                let _: AccountForAnswer = answer(
                    &runner,
                    ops::ROUTE_ACCOUNT_FOR,
                    &parent_params,
                    "second parent",
                )
                .await?;
                let mut changed = params.clone();
                changed["fork_of"]["admission_id"] = parent_params["admission_id"].clone();
                refused(
                    &runner,
                    ops::ROUTE_ACCOUNT_FOR,
                    &changed,
                    RefusalCode::AdmissionConflict,
                    "changed fork parent under same admission id",
                )
                .await?;
                // Same-body replay stability has its own admission check;
                // this case isolates comparison of a changed fork parent.
            }
            _ if name.starts_with("fork_of_excludes_") => {
                let field = name.strip_prefix("fork_of_excludes_").unwrap();
                params[field] = match field {
                    "requested_account" => json!({"account_id":sibling.account_id}),
                    "warm_credential" => json!(sibling.credential_id),
                    "parent_conversation_key" => parent_params["conversation_key"].clone(),
                    "candidates" => parent_params["candidates"].clone(),
                    _ => return Err(format!("unknown fork exclusion {field}")),
                };
                fork_invalid(&runner, &params).await?;
            }
            _ => return Err(format!("unknown fork check {name}")),
        }
        Ok(Ending::Passed)
    }
}

async fn fork_invalid<R: RoutingRoute>(runner: &R, params: &Value) -> Result<Refusal, String> {
    let refusal = refused(
        runner,
        ops::ROUTE_ACCOUNT_FOR,
        params,
        RefusalCode::InvalidRequest,
        "invalid fork reference or combination",
    )
    .await?;
    if refusal.detail.get("field") != Some(&json!("fork_of")) {
        return Err(format!(
            "fork refusal names the wrong field: {:?}",
            refusal.detail
        ));
    }
    Ok(refusal)
}
