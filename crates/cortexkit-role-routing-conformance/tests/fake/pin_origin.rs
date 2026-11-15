use super::*;

impl FakeRoute {
    pub(super) fn select_pin_origin(
        &self,
        state: &mut State,
        params: &Value,
    ) -> Result<Value, Refusal> {
        if params.get("pin_origin").is_none() && state.pin_origin_reserve.is_none() {
            return self.select_placement(state, params);
        }
        let mut params = params.clone();
        if (self.fault("select_pin_origin_without_pin") && params.get("pin").is_none())
            || (self.fault("select_pin_origin_unknown")
                && params.get("pin_origin") == Some(&json!("agent")))
        {
            params.as_object_mut().unwrap().remove("pin_origin");
        }
        let describe: RoleDescribe = serde_json::from_value(self.describe(state)?).unwrap();
        let request = parse_select_request_serving(&params, &describe.capabilities)?;
        let marked = request.pin_origin
            == Some(cortexkit_role_routing_conformance::wire::select::PinOrigin::User);
        if self.fault("select_pin_origin_unenrolled_refused")
            && marked
            && !state
                .pin_origin_reserve
                .as_ref()
                .is_some_and(|(_, enrolled)| enrolled.contains(&self.stamp.principal))
        {
            return Err(Refusal::invalid_request("pin_origin", "not enrolled"));
        }
        params.as_object_mut().unwrap().remove("pin_origin");
        let answer = self.select_placement(state, &params)?;
        if let Some((provider, enrolled)) = &state.pin_origin_reserve {
            let honoured = marked
                && !self.fault("select_pin_origin_enrolled")
                && (enrolled.contains(&self.stamp.principal)
                    || self.fault("select_pin_origin_unenrolled"));
            if let Some(pin) = request.pin.filter(|_| {
                answer.pointer("/route/provider") == Some(&json!(provider)) && !honoured
            }) {
                let reason = if marked && self.fault("select_pin_origin_wrong_reason") {
                    PinUnavailableReason::parse("subscription_reserve")
                } else {
                    PinUnavailableReason::QuotaExhausted
                };
                let mut refusal = Refusal::pin_unavailable(pin, reason, None);
                if !self.fault("select_pin_origin_without_cause") {
                    let cause = if marked && self.fault("select_pin_origin_wrong_cause") {
                        "other"
                    } else {
                        "subscription_reserve"
                    };
                    refusal.detail.insert("cause".into(), json!(cause));
                }
                return Err(refusal);
            }
        }
        Ok(answer)
    }
}
