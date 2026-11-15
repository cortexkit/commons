use super::*;

const ENROLLED: &str = "reserved:prefrontal-core";

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn pin_origin_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(ENROLLED).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        if !describe.serves_select_pin_origin() {
            return Ok(Ending::NotApplicable(
                "select_pin_origin capability gate".into(),
            ));
        }
        let pin = format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER);
        let plain = with(base(), "pin", json!(pin));
        let marked = with(plain.clone(), "pin_origin", json!("user"));
        match name {
            "select_pin_origin_enrolled" | "select_pin_origin_unenrolled" => {
                // Establish eligibility before reserve facts can mask another refusal.
                let baseline = select(&route, &plain, "pin before reserve").await?;
                if baseline.route.route_key() != pin {
                    return Err("baseline did not serve the pinned subscription model".into());
                }
                self.subject
                    .configure_pin_origin_reserve(
                        fixture::SUBSCRIPTION_PROVIDER,
                        &[ENROLLED.into()],
                    )
                    .await?;
                Self::pin_origin_limited(&route, &plain).await?;
                if name == "select_pin_origin_enrolled" {
                    let served =
                        select(&route, &marked, "enrolled user pin inside reserve").await?;
                    if served.route.route_key() != pin {
                        return Err("user pin changed the pinned model".into());
                    }
                    let excluded = with(marked, "exclude", json!([pin]));
                    let refusal = refused(
                        &route,
                        ops::ROUTE_SELECT,
                        &excluded,
                        RefusalCode::PinUnavailable,
                        "excluded user pin",
                    )
                    .await?;
                    if refusal.detail.get("reason") != Some(&json!("excluded")) {
                        return Err(format!("user pin bypassed exclusion: {refusal}"));
                    }
                } else {
                    for principal in [CONSUMER, "reserved:prefrontal-core-other"] {
                        let other = self
                            .route(RouteStamp::scoped(
                                principal,
                                ScopeStamp {
                                    owner: ENROLLED.into(),
                                    scope_ref: "pin-origin".into(),
                                    scope_epoch: 1,
                                },
                            ))
                            .await?;
                        Self::pin_origin_limited(&other, &plain).await?;
                        Self::pin_origin_limited(&other, &marked).await?;
                    }
                }
            }
            "select_pin_origin_without_pin" | "select_pin_origin_unknown" => {
                let invalid = if name == "select_pin_origin_without_pin" {
                    with(base(), "pin_origin", json!("user"))
                } else {
                    with(plain, "pin_origin", json!("agent"))
                };
                for principal in [ENROLLED, CONSUMER] {
                    let caller = self.route_as(principal).await?;
                    let refusal = refused(
                        &caller,
                        ops::ROUTE_SELECT,
                        &invalid,
                        RefusalCode::InvalidRequest,
                        name,
                    )
                    .await?;
                    if refusal.detail.get("field") != Some(&json!("pin_origin")) {
                        return Err(format!("{name}: expected field pin_origin, got {refusal}"));
                    }
                }
            }
            _ => return Err(format!("unknown pin origin check {name}")),
        }
        Ok(Ending::Passed)
    }

    async fn pin_origin_limited<R: RoutingRoute>(route: &R, params: &Value) -> Check {
        let refusal = refused(
            route,
            ops::ROUTE_SELECT,
            params,
            RefusalCode::PinUnavailable,
            "pin inside reserve",
        )
        .await?;
        if refusal.detail.get("reason") != Some(&json!("quota_exhausted"))
            || refusal.detail.get("pin") != params.get("pin")
            || refusal.detail.contains_key("reset_at_ms")
            || refusal
                .detail
                .get("cause")
                .is_some_and(|cause| cause != &json!("subscription_reserve"))
        {
            return Err(format!(
                "expected quota_exhausted with the pinned model, no unknown reset and absent or subscription_reserve cause, got {refusal}"
            ));
        }
        Ok(())
    }
}
