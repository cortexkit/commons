use super::*;
use cortexkit_role_routing::account_continue::{
    ContinueAnswer, ContinueOffer, ContinueRequest, ExhaustedAccount,
};

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn continue_check(&self, name: &str) -> Result<Ending, String> {
        let consumer = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&consumer, ops::ROLE_DESCRIBE, &json!({}), "role.describe").await?;
        let undeclared = name == "continue_undeclared_group_refuses_mode";
        if describe.serves(capabilities::ACCOUNT_CONTINUE) == undeclared {
            return Ok(Ending::NotApplicable(format!(
                "role.describe {} account_continue",
                if undeclared {
                    "declares"
                } else {
                    "does not declare"
                }
            )));
        }
        let signed_out = name.starts_with("continue_signed_out");
        let signed_out_undeclared = name == "continue_signed_out_undeclared_refuses_window";
        if signed_out
            && describe.serves(capabilities::ACCOUNT_CONTINUE_SIGNED_OUT) == signed_out_undeclared
        {
            return Ok(Ending::NotApplicable(format!(
                "role.describe {} account_continue_signed_out",
                if signed_out_undeclared {
                    "declares"
                } else {
                    "does not declare"
                }
            )));
        }
        let mut request = ContinueRequest::new(
            "continue-run",
            "continue-admission",
            "continue-conversation",
            "previous-decision",
            ExhaustedAccount::new("credential-1", "account-1", "five_hour", now_ms()),
            "continue-selection",
        );
        if signed_out {
            request.exhausted.limit_window = "signed_out".into();
        }
        if undeclared || signed_out_undeclared {
            let runner = self.route_as(fixture::RUNNER_ONE).await?;
            let refusal = refused(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &serde_json::to_value(&request).unwrap(),
                RefusalCode::InvalidRequest,
                "undeclared continuation capability",
            )
            .await?;
            let field = if signed_out_undeclared {
                "exhausted.limit_window"
            } else {
                "mode"
            };
            if refusal.detail.get("field") != Some(&json!(field)) {
                return Err(format!(
                    "undeclared capability refusal did not name {field}: {refusal}"
                ));
            }
            return Ok(Ending::Passed);
        }

        if matches!(
            name,
            "continue_account_decision_preserves_route"
                | "continue_account_decision_wrong_admission_refused"
                | "continue_account_decision_chain"
        ) {
            return self.continue_account_decision_check(name).await;
        }

        let mut catalog = fixture::continue_catalog();
        // Admit before adding spares, without a caller pin that would forbid failover.
        catalog
            .accounts
            .retain(|account| account.account_id == "account-1");
        self.subject.configure_catalog(&catalog).await?;
        let credentials = fixture::continue_credentials();
        let selected = select(
            &consumer,
            &with(base(), "pin", json!(fixture::HOLD_FIRST)),
            "continuation selection",
        )
        .await?;
        let runner = self.route_as(&selected.runner.module).await?;
        let mut admission =
            Self::account_for("continue-admission", "continue-conversation", "model-c");
        admission["decision_id"] = json!(selected.decision_id);
        let admitted: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &admission,
            "original admission",
        )
        .await?;
        let original = credentials
            .iter()
            .find(|c| {
                c.account.credential_id == admitted.account.credential_id
                    && c.account.account_id == admitted.account.account_id
            })
            .ok_or("original admission is not a seeded credential/account pair")?;
        if original.account.provider != fixture::SUBSCRIPTION_PROVIDER
            || original.account.method != "oauth"
            || !original.models.iter().any(|m| m == "model-c")
        {
            return Err("original admission is not compatible with the previous decision".into());
        }
        let _: AccountObserveAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_OBSERVE,
            &json!({
                "event_id":"continue-started", "kind":"admission_started",
                "admission_id":request.admission_id, "run_id":request.run_id,
                "served_account_id":admitted.account.account_id,
                "credential_id":admitted.account.credential_id, "observed_at_ms":now_ms()
            }),
            "started run",
        )
        .await?;
        request.previous_decision_id = selected.decision_id;
        request.exhausted = ExhaustedAccount::new(
            &admitted.account.credential_id,
            &admitted.account.account_id,
            "five_hour",
            now_ms(),
        )
        .with_resets_at_ms(now_ms() + 300_000);
        if signed_out {
            request.exhausted.limit_window = "signed_out".into();
            request.exhausted.resets_at_ms = None;
        }
        if name == "continue_credit_window_refused" {
            request.exhausted.limit_window = "credit".into();
            request.exhausted.resets_at_ms = None;
            let refusal = refused(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &serde_json::to_value(&request).unwrap(),
                RefusalCode::InvalidRequest,
                "credit is not a continuation quota window",
            )
            .await?;
            if refusal.detail.get("field") != Some(&json!("exhausted.limit_window")) {
                return Err(format!(
                    "credit refusal did not name exhausted.limit_window: {refusal}"
                ));
            }
            return Ok(Ending::Passed);
        }
        self.subject
            .configure_continue_credentials(&credentials)
            .await?;
        self.subject
            .block_account("account-6", Some(now_ms() + 300_000))
            .await?;
        if name == "continue_hold_unknown_until_stays_unknown" {
            // Leave no compatible spare, and deliberately provide no reset time.
            for credential in &credentials {
                self.subject
                    .block_account(&credential.account.account_id, None)
                    .await?;
            }
            request.exhausted.resets_at_ms = None;
        }
        let params = serde_json::to_value(&request).unwrap();
        let first_value: Value =
            answer(&runner, ops::ROUTE_ACCOUNT_FOR, &params, "continue").await?;
        let first: ContinueAnswer = serde_json::from_value(first_value.clone())
            .map_err(|e| format!("continuation answer does not decode: {e}"))?;
        if name == "continue_hold_unknown_until_stays_unknown" {
            match first {
                ContinueAnswer::Hold(hold) if hold.until_ms.is_none() => return Ok(Ending::Passed),
                other => {
                    return Err(format!(
                        "unknown reset must remain an untimed hold, got {other:?}"
                    ))
                }
            }
        }
        let offer = match &first {
            ContinueAnswer::Offer(offer) => offer,
            other => {
                return Err(format!(
                    "a compatible spare exists, expected offer, got {other:?}"
                ))
            }
        };
        let offered = credentials
            .iter()
            .find(|c| {
                c.account.credential_id == offer.credential_id
                    && c.account.account_id == offer.account
            })
            .ok_or("offer is not a seeded credential/account pair")?;
        if signed_out {
            if offer.account == request.exhausted.account {
                return Err("offer names the signed-out account".into());
            }
            if offered.account.provider != original.account.provider
                || offered.account.method != original.account.method
                || !offered.models.contains(&selected.route.model)
                || offer.account == "account-6"
            {
                return Err("signed-out offer changes provider, auth method or model, or names a held account".into());
            }
            if offer.decision_id.is_empty()
                || offer.decision_id == request.previous_decision_id
                || offer.valid_until_ms <= now_ms()
            {
                return Err("signed-out offer must name a new decision and future deadline".into());
            }
            // A new conversation avoids replaying the original admission or its affinity.
            let fresh: AccountForAnswer = answer(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &Self::account_for("signed-out-fresh", "signed-out-fresh", "model-c"),
                "fresh admission after signed-out continuation",
            )
            .await?;
            if fresh.account.account_id == request.exhausted.account
                || fresh.account.credential_id == request.exhausted.credential_id
            {
                return Err("fresh admission names the signed-out credential".into());
            }
            // Remove every spare without pre-blocking the signed-out account:
            // the signed-out evidence must exclude that account.
            self.subject
                .configure_continue_credentials(std::slice::from_ref(original))
                .await?;
            request.selection_id = "signed-out-no-spare".into();
            let no_spare: ContinueAnswer = answer(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &serde_json::to_value(&request).unwrap(),
                "signed-out continuation without a compatible spare",
            )
            .await?;
            if matches!(no_spare, ContinueAnswer::Offer(_)) {
                return Err("signed-out continuation without a spare must hold or refuse".into());
            }
            return Ok(Ending::Passed);
        }
        match name {
            "continue_offer_excludes_exhausted_account"
                if offer.account == request.exhausted.account =>
            {
                return Err("offer names the exhausted account".into())
            }
            "continue_offer_preserves_provider"
                if offered.account.provider != original.account.provider =>
            {
                return Err("offer changes provider".into())
            }
            "continue_offer_preserves_auth_method"
                if offered.account.method != original.account.method =>
            {
                return Err("offer changes auth method".into())
            }
            "continue_offer_preserves_model"
                if !offered.models.iter().any(|m| m == &selected.route.model) =>
            {
                return Err("offer changes model eligibility".into())
            }
            "continue_offer_excludes_known_hold" if offer.account == "account-6" => {
                return Err("offer names an account under a known hold".into())
            }
            "continue_same_id_returns_identical_offer" => {
                let retry: Value =
                    answer(&runner, ops::ROUTE_ACCOUNT_FOR, &params, "same-id retry").await?;
                if first_value != retry {
                    return Err(format!(
                        "same-id retry changed offer: {first:?} -> {retry:?}"
                    ));
                }
            }
            "continue_changed_question_refused_by_selection_id" => {
                let mut changed_questions = Vec::new();
                for field in [
                    "run_id",
                    "admission_id",
                    "conversation_key",
                    "previous_decision_id",
                ] {
                    let mut changed = params.clone();
                    changed[field] = json!("different");
                    changed_questions.push((field.to_owned(), changed));
                }
                for (field, value) in [
                    ("credential_id", json!("different-credential")),
                    ("account", json!("different-account")),
                    ("limit_window", json!("seven_day")),
                    ("pool", json!("different-pool")),
                    (
                        "resets_at_ms",
                        json!(request.exhausted.resets_at_ms.unwrap() + 1),
                    ),
                    ("retry_after_s", json!(1)),
                    (
                        "observed_at_ms",
                        json!(request.exhausted.observed_at_ms + 1),
                    ),
                ] {
                    let mut changed = params.clone();
                    changed["exhausted"][field] = value;
                    changed_questions.push((format!("exhausted.{field}"), changed));
                }
                for (field, changed) in changed_questions {
                    let refusal = refused(
                        &runner,
                        ops::ROUTE_ACCOUNT_FOR,
                        &changed,
                        RefusalCode::InvalidRequest,
                        &format!("same selection id with changed {field}"),
                    )
                    .await?;
                    if refusal.detail.get("field") != Some(&json!("selection_id")) {
                        return Err(format!(
                            "changed {field} did not name selection_id: {refusal}"
                        ));
                    }
                    let retry: Value = answer(
                        &runner,
                        ops::ROUTE_ACCOUNT_FOR,
                        &params,
                        "original question after rejected change",
                    )
                    .await?;
                    if retry != first_value {
                        return Err(format!(
                            "changed {field} replaced the first answer: {retry:?}"
                        ));
                    }
                }
            }
            "continue_offer_excludes_exhausted_account"
            | "continue_offer_preserves_provider"
            | "continue_offer_preserves_auth_method"
            | "continue_offer_preserves_model"
            | "continue_offer_excludes_known_hold" => {}
            _ => return Err(format!("unknown continuation check {name}")),
        }
        if offer.decision_id.is_empty()
            || offer.decision_id == request.previous_decision_id
            || offer.valid_until_ms <= now_ms()
        {
            return Err("offer must name a new decision and a future validity deadline".into());
        }
        Ok(Ending::Passed)
    }

    async fn continue_account_decision_check(&self, name: &str) -> Result<Ending, String> {
        let mut catalog = fixture::continue_catalog();
        catalog.accounts.retain(|a| a.account_id == "account-1");
        self.subject.configure_catalog(&catalog).await?;
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        // No selection is made: the model is pinned directly by this admission.
        let admitted: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::account_for(
                "account-decision-admission",
                "account-decision-conversation",
                "model-c",
            ),
            "select-less admission",
        )
        .await?;
        if admitted.reason.decision_id.is_empty()
            || admitted.account.provider != fixture::SUBSCRIPTION_PROVIDER
            || admitted.account.method != "oauth"
            || admitted.account.account_id != "account-1"
            || admitted.account.credential_id != "credential-1"
        {
            return Err(
                "select-less admission did not name the seeded account and its decision".into(),
            );
        }
        let mut request = ContinueRequest::new(
            "account-decision-run",
            "account-decision-admission",
            "account-decision-conversation",
            &admitted.reason.decision_id,
            ExhaustedAccount::new(
                &admitted.account.credential_id,
                &admitted.account.account_id,
                "five_hour",
                now_ms(),
            )
            .with_resets_at_ms(now_ms() + 300_000),
            "account-decision-selection",
        );
        let _: AccountObserveAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_OBSERVE,
            &json!({
                "event_id":"account-decision-started", "kind":"admission_started",
                "admission_id":request.admission_id, "run_id":request.run_id,
                "served_account_id":admitted.account.account_id,
                "credential_id":admitted.account.credential_id, "observed_at_ms":now_ms()
            }),
            "select-less run started",
        )
        .await?;
        if name == "continue_account_decision_wrong_admission_refused" {
            // Both admissions use the same account, so account identity cannot
            // substitute for checking which admission owns the decision.
            let foreign: AccountForAnswer = answer(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &Self::account_for("foreign-admission", "foreign-conversation", "model-c"),
                "other select-less admission",
            )
            .await?;
            let mut mismatched = request.clone();
            mismatched.previous_decision_id = foreign.reason.decision_id;
            mismatched.selection_id = "foreign-account-decision-selection".into();
            let refusal: ContinueAnswer = answer(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &serde_json::to_value(mismatched).unwrap(),
                "account decision from another admission",
            )
            .await?;
            if !matches!(refusal, ContinueAnswer::Refuse(ref r) if r.routing_code == "unknown_decision")
            {
                return Err(format!("another admission's account decision must refuse unknown_decision, got {refusal:?}"));
            }
        }
        let mut credentials = fixture::continue_credentials();
        if name == "continue_account_decision_chain" {
            let mut third = credentials[1].clone();
            third.account.account_id = "account-7".into();
            third.account.credential_id = "credential-7".into();
            credentials.push(third);
        }
        self.subject
            .configure_continue_credentials(&credentials)
            .await?;
        self.subject
            .block_account("account-6", Some(now_ms() + 300_000))
            .await?;
        // A same-admission offer is also required after the foreign-id refusal;
        // refusing every account decision must not pass the isolation check.
        let first: ContinueAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &serde_json::to_value(&request).unwrap(),
            "continue from admission account decision",
        )
        .await?;
        let offer = account_decision_offer(&first, &request, &credentials)?;
        if name == "continue_account_decision_chain" {
            let _: AccountObserveAnswer = answer(
                &runner,
                ops::ROUTE_ACCOUNT_OBSERVE,
                &json!({
                    "event_id":"account-decision-switched", "kind":"admission_started",
                    "admission_id":request.admission_id, "run_id":request.run_id,
                    "served_account_id":offer.account, "credential_id":offer.credential_id,
                    "observed_at_ms":now_ms()
                }),
                "first account switch recorded",
            )
            .await?;
            request.previous_decision_id = offer.decision_id;
            request.exhausted =
                ExhaustedAccount::new(offer.credential_id, offer.account, "seven_day", now_ms())
                    .with_resets_at_ms(now_ms() + 300_000);
            request.selection_id = "account-decision-second-selection".into();
            let second: ContinueAnswer = answer(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &serde_json::to_value(&request).unwrap(),
                "continue from offered account decision",
            )
            .await?;
            let second = account_decision_offer(&second, &request, &credentials)?;
            if second.decision_id == admitted.reason.decision_id
                || second.account == admitted.account.account_id
            {
                return Err(
                    "second continuation reused the original decision or exhausted account".into(),
                );
            }
        }
        Ok(Ending::Passed)
    }
}

fn account_decision_offer(
    answer: &ContinueAnswer,
    request: &ContinueRequest,
    credentials: &[crate::ContinueCredential],
) -> Result<ContinueOffer, String> {
    let ContinueAnswer::Offer(offer) = answer else {
        return Err(format!(
            "a compatible spare exists for the account decision, expected offer, got {answer:?}"
        ));
    };
    let offered = credentials
        .iter()
        .find(|c| {
            c.account.credential_id == offer.credential_id && c.account.account_id == offer.account
        })
        .ok_or("account-decision offer is not a seeded credential/account pair")?;
    if offered.account.provider != fixture::SUBSCRIPTION_PROVIDER
        || offered.account.method != "oauth"
        || !offered.models.iter().any(|m| m == "model-c")
    {
        return Err(
            "account-decision offer changes provider, auth method or model eligibility".into(),
        );
    }
    if offer.account == request.exhausted.account || offer.account == "account-6" {
        return Err("account-decision offer names an exhausted or held account".into());
    }
    if offer.decision_id.is_empty()
        || offer.decision_id == request.previous_decision_id
        || offer.valid_until_ms <= now_ms()
    {
        return Err("account-decision offer must name a new decision and future deadline".into());
    }
    Ok(offer.clone())
}
