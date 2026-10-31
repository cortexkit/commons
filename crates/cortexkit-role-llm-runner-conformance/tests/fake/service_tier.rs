use super::*;
use cortexkit_role_compaction_provider::ride::{RideCause, RideRequest};
use cortexkit_role_llm_runner::service_tier::{
    check_send, Served, ServiceTier, ServiceTiers, Support,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ServiceTierDefect {
    #[default]
    None,
    CallBeforeUnsupportedRefusal,
    ChangeRunTier,
    LateRide,
    EchoServed,
    NoFixture,
}

#[derive(Default)]
pub(super) struct State {
    applied: ServiceTier,
    previous: Option<ServiceTier>,
    rides: Vec<RideRequest>,
}

pub(super) fn support() -> cortexkit_role_llm_runner::service_tier::ServiceTiers {
    [(
        "openai.responses".into(),
        Support::new(vec![ServiceTier::Fast], Served::Unknown),
    )]
    .into()
}

impl Module {
    pub(super) fn tier_send(&self, session: &str, params: &Value) -> Option<Reply> {
        let active = self
            .sessions
            .lock()
            .unwrap()
            .get(session)
            .is_some_and(|sess| sess.runs.iter().any(|run| run.state == "active"));
        let mut states = self.world.service_tiers.lock().unwrap();
        let state = states.get_mut(session)?;
        let requested = match cortexkit_role_llm_runner::service_tier::parse_request(
            params.get("service_tier"),
        ) {
            Ok(tier) => tier,
            Err(problem) => return Some(refuse(errors::INVALID_PARAMS, Some(problem.detail()))),
        };
        let declared: ServiceTiers = support();
        match check_send(
            requested,
            active.then_some(state.applied),
            "openai.responses",
            Some(&declared),
        ) {
            Ok(applied) => state.applied = applied,
            Err(problem) => {
                let defect = self.world.defects.service_tier;
                if problem.reason() == "unsupported_tier"
                    && defect == ServiceTierDefect::CallBeforeUnsupportedRefusal
                {
                    self.world.model_calls.lock().unwrap().insert((
                        session.into(),
                        "refused-tier".into(),
                        0,
                    ));
                }
                let ignore = problem.reason() == "run_tier_fixed"
                    && defect == ServiceTierDefect::ChangeRunTier;
                if !ignore {
                    return Some(refuse(errors::INVALID_PARAMS, Some(problem.detail())));
                }
                state.applied = requested.unwrap_or_default();
            }
        }
        None
    }

    pub(super) fn tier_model_ingress(
        &self,
        session: &str,
        observation: &mut ModelRequestObservation,
    ) {
        let mut states = self.world.service_tiers.lock().unwrap();
        let Some(state) = states.get_mut(session) else {
            return;
        };
        let changed = state
            .previous
            .is_some_and(|previous| previous != state.applied);
        let ride = || {
            RideRequest::new("a".repeat(32), RideCause::ServiceTierChange, 1000)
                .with_service_tier(state.applied)
        };
        if changed && self.world.defects.service_tier != ServiceTierDefect::LateRide {
            state.rides.push(ride());
        }
        observation.service_tier_applied = Some(state.applied.as_str().into());
        observation.service_tier_change_rides = Some(state.rides.len() as u64);
        if changed && self.world.defects.service_tier == ServiceTierDefect::LateRide {
            state.rides.push(ride());
        }
        state.previous = Some(state.applied);
    }

    pub(super) fn tier_usage(&self, session: &str, usage: &mut RunUsage) {
        if let Some(state) = self.world.service_tiers.lock().unwrap().get(session) {
            usage.service_tier_applied =
                (!state.applied.is_standard()).then(|| state.applied.as_str().into());
            if self.world.defects.service_tier == ServiceTierDefect::EchoServed {
                usage.service_tier_served = Some(state.applied.as_str().into());
            }
        }
    }
}
