//! Successful retests are scoped to the block captured by an admission.

use super::*;

fn success(admission: &str, account: &str, credential: &str) -> Value {
    json!({"event_id":"success", "kind":"attempt", "admission_id":admission,
        "served_account_id":account, "credential_id":credential,
        "attempt_outcome":{"status":"served"}})
}

impl<S: RoutingSubject> Suite<'_, S> {
    async fn success_admit(
        &self,
        id: &str,
        account: &str,
        credential: &str,
    ) -> Result<AccountForAnswer, String> {
        let route = self.route_as(fixture::RUNNER_ONE).await?;
        let params = Self::success_admission(id, account, credential);
        let answer: AccountForAnswer =
            answer(&route, ops::ROUTE_ACCOUNT_FOR, &params, "retest admission").await?;
        if answer.account.account_id != account || answer.account.credential_id != credential {
            return Err(
                "admission answered outside the exact account and credential candidate".into(),
            );
        }
        Ok(answer)
    }

    fn success_admission(id: &str, account: &str, credential: &str) -> Value {
        let mut params = Self::account_for(id, id, "model-c");
        params["candidates"] = json!({"view_digest":"success-fixture", "accounts":[{
            "account_id":account,"credential_id":credential}]});
        params
    }

    async fn success_blocked(&self, id: &str, account: &str, credential: &str) -> Check {
        let route = self.route_as(fixture::RUNNER_ONE).await?;
        match call::<_, AccountForAnswer>(
            &route,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::success_admission(id, account, credential),
        )
        .await?
        {
            Err(refusal)
                if matches!(
                    refusal.code,
                    RefusalCode::AccountPoolExhausted
                        | RefusalCode::AccountHold
                        | RefusalCode::AuthRequired
                ) =>
            {
                Ok(())
            }
            other => Err(format!(
                "block did not survive for {account}/{credential}: {other:?}"
            )),
        }
    }

    async fn success_seed(
        &self,
        id: &str,
        account: &str,
        credential: &str,
        kind: &str,
        time: u64,
    ) -> Check {
        self.subject
            .configure_observe_retest_block(account, credential, kind, time)
            .await?;
        self.success_admit(id, account, credential).await?;
        Ok(())
    }

    pub(super) async fn observe_success_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(fixture::RUNNER_ONE).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        let undeclared = name == "account_observe_success_undeclared";
        if describe.serves(capabilities::ACCOUNT_OBSERVE_SUCCESS) == undeclared {
            return Ok(Ending::NotApplicable(
                "account_observe_success capability gate".into(),
            ));
        }
        let mut catalog = fixture::catalog();
        let first = catalog
            .accounts
            .iter()
            .find(|a| a.provider == fixture::SUBSCRIPTION_PROVIDER)
            .unwrap()
            .clone();
        let mut sibling = first.clone();
        sibling.credential_id = "success-sibling".into();
        let mut other = first.clone();
        other.account_id = "success-other-account".into();
        other.credential_id = "success-other-credential".into();
        catalog.accounts = vec![first.clone(), sibling.clone(), other.clone()];
        self.subject.configure_catalog(&catalog).await?;
        let account = first.account_id.as_str();
        let credential = first.credential_id.as_str();
        let mut params = success("retest", account, credential);
        match name {
            "account_observe_success_undeclared" => {
                Self::invalid_field(
                    &route,
                    ops::ROUTE_ACCOUNT_OBSERVE,
                    &params,
                    "attempt_outcome",
                )
                .await?
            }
            "account_observe_success_status_refusals" => {
                for status in [
                    "quota",
                    "throttled",
                    "credential",
                    "unavailable",
                    "rejected",
                ] {
                    params["attempt_outcome"]["status"] = json!(status);
                    Self::invalid_field(
                        &route,
                        ops::ROUTE_ACCOUNT_OBSERVE,
                        &params,
                        "attempt_outcome.status",
                    )
                    .await?;
                }
            }
            "account_observe_success_companions" => {
                for field in ["admission_id", "served_account_id", "credential_id"] {
                    let mut missing = params.clone();
                    missing.as_object_mut().unwrap().remove(field);
                    Self::invalid_field(&route, ops::ROUTE_ACCOUNT_OBSERVE, &missing, field)
                        .await?;
                }
            }
            "account_observe_success_kind" => {
                for kind in [
                    "admission_started",
                    "run_finished",
                    "account_limit_exhausted",
                    "credential_refused",
                    "transient_backoff",
                    "future_kind",
                ] {
                    params["kind"] = json!(kind);
                    Self::invalid_field(
                        &route,
                        ops::ROUTE_ACCOUNT_OBSERVE,
                        &params,
                        "attempt_outcome",
                    )
                    .await?;
                }
            }
            "account_observe_success_retry" => {
                params["attempt_outcome"]["retry_after_ms"] = json!(0);
                Self::invalid_field(
                    &route,
                    ops::ROUTE_ACCOUNT_OBSERVE,
                    &params,
                    "attempt_outcome.retry_after_ms",
                )
                .await?;
            }
            "account_observe_success_quota_shared" => {
                for (index, status) in ["served", "served_empty"].iter().enumerate() {
                    self.fresh().await?;
                    self.subject.configure_catalog(&catalog).await?;
                    let route = self.route_as(fixture::RUNNER_ONE).await?;
                    let id = format!("quota-retest-{index}");
                    self.success_seed(
                        &format!("quota-other-{index}"),
                        &other.account_id,
                        &other.credential_id,
                        "quota",
                        now_ms().saturating_sub(1000),
                    )
                    .await?;
                    self.success_seed(
                        &id,
                        account,
                        credential,
                        "quota",
                        now_ms().saturating_sub(1000),
                    )
                    .await?;
                    self.success_blocked(
                        &format!("quota-sibling-blocked-{index}"),
                        account,
                        &sibling.credential_id,
                    )
                    .await?;
                    params = success(&id, account, credential);
                    params["event_id"] = json!(format!("quota-success-{index}"));
                    params["attempt_outcome"]["status"] = json!(status);
                    let observed: AccountObserveAnswer = answer(
                        &route,
                        ops::ROUTE_ACCOUNT_OBSERVE,
                        &params,
                        "successful quota retest",
                    )
                    .await?;
                    if !observed.observation.bound || observed.observation.effects.is_empty() {
                        return Err("successful quota retest did not name its ended block".into());
                    }
                    // Two fresh admissions distinguish a cleared block from another retest slot.
                    for probe in 0..2 {
                        self.success_admit(
                            &format!("quota-sibling-cleared-{index}-{probe}"),
                            account,
                            &sibling.credential_id,
                        )
                        .await?;
                    }
                    self.success_blocked(
                        &format!("quota-other-blocked-{index}"),
                        &other.account_id,
                        &other.credential_id,
                    )
                    .await?;
                }
            }
            _ => {
                for (index, kind) in ["quota", "credit", "credential_refused", "transient_backoff"]
                    .iter()
                    .enumerate()
                {
                    // Surviving blocks of one kind must not affect the next kind's case.
                    self.fresh().await?;
                    self.subject.configure_catalog(&catalog).await?;
                    let route = self.route_as(fixture::RUNNER_ONE).await?;
                    let independent = if *kind == "quota" { &other } else { &sibling };
                    let id = format!("retest-{index}");
                    self.success_seed(
                        &id,
                        account,
                        credential,
                        kind,
                        now_ms().saturating_sub(1000),
                    )
                    .await?;
                    params = success(&id, account, credential);
                    params["event_id"] = json!(format!("success-{index}"));
                    if index % 2 == 1 {
                        params["attempt_outcome"]["status"] = json!("served_empty");
                    }
                    match name {
                        "account_observe_success_newer_block" => {
                            self.success_seed(
                                &format!("newer-{index}"),
                                account,
                                credential,
                                kind,
                                now_ms() + 1,
                            )
                            .await?
                        }
                        "account_observe_success_other_credential" => {
                            self.success_seed(
                                &format!("sibling-{index}"),
                                &independent.account_id,
                                &independent.credential_id,
                                kind,
                                now_ms(),
                            )
                            .await?
                        }
                        "account_observe_success_other_account" => {
                            self.success_seed(
                                &format!("other-{index}"),
                                &other.account_id,
                                &other.credential_id,
                                kind,
                                now_ms(),
                            )
                            .await?
                        }
                        "account_observe_success_unbound" => {
                            params["admission_id"] = json!("unknown-admission")
                        }
                        "account_observe_success_unknown" => {
                            params["attempt_outcome"]["status"] = json!("future_status")
                        }
                        _ => {}
                    }
                    let observed: AccountObserveAnswer = answer(
                        &route,
                        ops::ROUTE_ACCOUNT_OBSERVE,
                        &params,
                        "successful retest",
                    )
                    .await?;
                    let unchanged = matches!(
                        name,
                        "account_observe_success_newer_block"
                            | "account_observe_success_unbound"
                            | "account_observe_success_unknown"
                    );
                    if name == "account_observe_success_unbound" && observed.observation.bound {
                        return Err("unknown admission was bound".into());
                    }
                    if unchanged {
                        if !observed.observation.effects.is_empty() {
                            return Err("observational success returned effects".into());
                        }
                        self.success_blocked(
                            &format!("still-blocked-{index}"),
                            account,
                            credential,
                        )
                        .await?;
                    } else {
                        if !observed.observation.bound || observed.observation.effects.is_empty() {
                            return Err("successful retest did not name its ended block".into());
                        }
                        self.success_admit(&format!("cleared-{index}"), account, credential)
                            .await?;
                        if name == "account_observe_success_other_credential" {
                            self.success_blocked(
                                &format!("sibling-probe-{index}"),
                                &independent.account_id,
                                &independent.credential_id,
                            )
                            .await?;
                        }
                        if name == "account_observe_success_other_account" {
                            self.success_blocked(
                                &format!("other-probe-{index}"),
                                &other.account_id,
                                &other.credential_id,
                            )
                            .await?;
                        }
                        if name == "account_observe_success_duplicate" {
                            self.success_seed(
                                &format!("duplicate-reblock-{index}"),
                                account,
                                credential,
                                kind,
                                now_ms() + 1,
                            )
                            .await?;
                            let duplicate: AccountObserveAnswer = answer(
                                &route,
                                ops::ROUTE_ACCOUNT_OBSERVE,
                                &params,
                                "duplicate success",
                            )
                            .await?;
                            if !duplicate.observation.duplicate
                                || duplicate.observation.id != observed.observation.id
                                || !duplicate.observation.effects.is_empty()
                            {
                                return Err("duplicate success applied again".into());
                            }
                            self.success_blocked(
                                &format!("duplicate-probe-{index}"),
                                account,
                                credential,
                            )
                            .await?;
                        }
                        params["event_id"] = json!(format!("unblocked-{index}"));
                        params["admission_id"] = json!(format!("cleared-{index}"));
                        let unblocked: AccountObserveAnswer = answer(
                            &route,
                            ops::ROUTE_ACCOUNT_OBSERVE,
                            &params,
                            "unblocked success",
                        )
                        .await?;
                        if !unblocked.observation.effects.is_empty() {
                            return Err("unblocked success was not observational".into());
                        }
                    }
                }
            }
        }
        Ok(Ending::Passed)
    }
}
