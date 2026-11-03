//! Pure checks for session creation under a daemon-attested scope.
//!
//! These helpers implement scoped-start, frozen-send and plan-sender checks.
//! Route and opener identities must come from an already-validated bind stamp
//! and the session's recorded freeze, or from the validated creation decision
//! before its first write. Plan-sender identities come from the supplied plan
//! and are compared with that trusted opener; they are not evidence of the
//! caller's identity. The helpers do not validate stamps, perform other plan
//! admission checks, persist the freeze or select execution authority. The
//! runner must enforce their results before any write. Unscoped sends bypass
//! the start and frozen-send checks; a plan without a frozen opener is
//! unaffected.

use crate::{
    delivery_order::{PlanSender, PLAN_SENDERS_FIELD},
    errors::{self, scope_mismatch_fields as fields},
};

/// The kind and identifying attribute of the route's attested session scope.
/// A scope never carries both `flow_id` and `run_id`: the daemon refuses such
/// a record as `scope_run_id_with_flow_id`, so there are only three variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScopeKind<'a> {
    Plain,
    Flow { flow_id: &'a str },
    Run { run_id: &'a str },
}

/// Whether the runner admits the optional flow and run creation paths.
/// A runner may further restrict callers before invoking [`check_start`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StartPolicy {
    pub admit_flow: bool,
    pub admit_run: bool,
}

impl StartPolicy {
    pub fn new(admit_flow: bool, admit_run: bool) -> Self {
        Self {
            admit_flow,
            admit_run,
        }
    }
}

/// The sender and attributes frozen when a flow or run scope creates a session.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrozenSender {
    pub sender: String,
    pub flow_id: Option<String>,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
}

impl FrozenSender {
    pub fn new(opener: &str, kind: ScopeKind<'_>, agent_id: Option<&str>) -> Self {
        let (flow_id, run_id) = attributes(kind);
        Self {
            sender: opener.to_owned(),
            flow_id: flow_id.map(str::to_owned),
            run_id: run_id.map(str::to_owned),
            agent_id: agent_id.map(str::to_owned),
        }
    }
}

/// A successful creation check. The runner records a freeze with its first write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartDecision {
    Admit,
    AdmitAndFreeze(FrozenSender),
}

/// Check a session-creating send from an opener already admitted by the daemon.
/// Plain scopes always require the owner; flow/run admission is opt-in, and a
/// run scope never admits its owner. The returned pair is a refusal code and
/// optional `detail.field`, like [`crate::delivery_order::admit`].
/// This pure function performs no writes; the caller must honor a refusal
/// before writing session state and persist any admitted freeze itself.
pub fn check_start(
    scope_owner: &str,
    opener: &str,
    kind: ScopeKind<'_>,
    agent_id: Option<&str>,
    policy: StartPolicy,
) -> Result<StartDecision, (&'static str, Option<&'static str>)> {
    let admitted = match kind {
        ScopeKind::Plain => opener == scope_owner,
        ScopeKind::Flow { .. } => policy.admit_flow,
        ScopeKind::Run { .. } => policy.admit_run && opener != scope_owner,
    };
    if !admitted {
        return Err((errors::SCOPE_OWNER_MISMATCH, None));
    }
    Ok(match kind {
        ScopeKind::Plain => StartDecision::Admit,
        _ => StartDecision::AdmitAndFreeze(FrozenSender::new(opener, kind, agent_id)),
    })
}

/// Compare a later send against a frozen sender. The caller must call this
/// function and honor any refusal before writing session state, even for the
/// scope owner or a plan-named sender. This function performs no writes.
/// Mismatches are reported in sender, flow id, run id, agent id order.
pub fn check_frozen(
    frozen: &FrozenSender,
    opener: &str,
    kind: ScopeKind<'_>,
    agent_id: Option<&str>,
) -> Result<(), (&'static str, Option<&'static str>)> {
    let (flow_id, run_id) = attributes(kind);
    for (differs, code, field) in [
        (
            frozen.sender != opener,
            errors::SCOPE_SENDER_MISMATCH,
            fields::SENDER,
        ),
        (
            frozen.flow_id.as_deref() != flow_id,
            errors::SCOPE_FLOW_ID_MISMATCH,
            fields::FLOW_ID,
        ),
        (
            frozen.run_id.as_deref() != run_id,
            errors::SCOPE_RUN_ID_MISMATCH,
            fields::RUN_ID,
        ),
        (
            frozen.agent_id.as_deref() != agent_id,
            errors::SCOPE_AGENT_ID_MISMATCH,
            fields::AGENT_ID,
        ),
    ] {
        if differs {
            return Err((code, Some(field)));
        }
    }
    Ok(())
}

/// Check a plan's senders against the opener frozen by a flow or run scope.
/// An absent freeze leaves ordinary sessions unaffected. An absent or empty
/// sender list grants nobody else access. The runner must refuse with
/// `invalid_params` naming the returned field before writing any state, on
/// both send admission and refresh, whether or not it uses this helper.
pub fn check_plan_senders(
    frozen_opener: Option<&str>,
    senders: &[PlanSender],
) -> Result<(), &'static str> {
    if let Some(opener) = frozen_opener {
        if senders.iter().any(|sender| sender.module != opener) {
            return Err(PLAN_SENDERS_FIELD);
        }
    }
    Ok(())
}

fn attributes(kind: ScopeKind<'_>) -> (Option<&str>, Option<&str>) {
    match kind {
        ScopeKind::Plain => (None, None),
        ScopeKind::Flow { flow_id } => (Some(flow_id), None),
        ScopeKind::Run { run_id } => (None, Some(run_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_plan_senders_admit_only_the_opener() {
        let opener = vec![PlanSender::new("flow-carrier")];
        let other = vec![PlanSender::new("owner-module")];
        let mixed = vec![
            PlanSender::new("flow-carrier"),
            PlanSender::new("owner-module"),
        ];
        for senders in [[].as_slice(), opener.as_slice()] {
            assert_eq!(check_plan_senders(Some("flow-carrier"), senders), Ok(()));
        }
        for senders in [&other, &mixed] {
            assert_eq!(
                check_plan_senders(Some("flow-carrier"), senders),
                Err("plan.senders")
            );
            assert_eq!(check_plan_senders(None, senders), Ok(()));
        }
    }

    #[test]
    fn scoped_start_obeys_owner_and_optional_policy_before_freezing() {
        for policy in [StartPolicy::default(), StartPolicy::new(true, true)] {
            assert_eq!(
                check_start("owner", "owner", ScopeKind::Plain, None, policy),
                Ok(StartDecision::Admit)
            );
            assert_eq!(
                check_start("owner", "carrier", ScopeKind::Plain, None, policy),
                Err((errors::SCOPE_OWNER_MISMATCH, None))
            );
        }
        for (kind, enabled) in [
            (
                ScopeKind::Flow { flow_id: "f" },
                StartPolicy::new(true, false),
            ),
            (
                ScopeKind::Run { run_id: "r" },
                StartPolicy::new(false, true),
            ),
        ] {
            assert_eq!(
                check_start("owner", "carrier", kind, None, StartPolicy::default()),
                Err((errors::SCOPE_OWNER_MISMATCH, None))
            );
            let decision = check_start("owner", "carrier", kind, Some("agent"), enabled).unwrap();
            let StartDecision::AdmitAndFreeze(frozen) = decision else {
                panic!("missing freeze")
            };
            assert_eq!(frozen.sender, "carrier");
            assert_eq!(frozen.agent_id.as_deref(), Some("agent"));
            assert_eq!(
                check_frozen(&frozen, "carrier", kind, Some("agent")),
                Ok(())
            );
            match kind {
                ScopeKind::Flow { .. } => {
                    assert_eq!(frozen.flow_id.as_deref(), Some("f"));
                    assert_eq!(frozen.run_id, None);
                    assert!(matches!(
                        check_start("owner", "owner", kind, None, enabled),
                        Ok(StartDecision::AdmitAndFreeze(_))
                    ));
                }
                ScopeKind::Run { .. } => {
                    assert_eq!(frozen.flow_id, None);
                    assert_eq!(frozen.run_id.as_deref(), Some("r"));
                    assert_eq!(
                        check_start("owner", "owner", kind, None, enabled),
                        Err((errors::SCOPE_OWNER_MISMATCH, None))
                    );
                }
                ScopeKind::Plain => unreachable!(),
            }
        }
    }

    #[test]
    fn scoped_frozen_sender_mismatches_use_contract_precedence() {
        let flow = ScopeKind::Flow { flow_id: "f" };
        let frozen = FrozenSender::new("sender", flow, Some("agent"));
        let run = ScopeKind::Run { run_id: "r" };
        assert_eq!(
            check_frozen(&frozen, "other", run, None),
            Err((errors::SCOPE_SENDER_MISMATCH, Some("sender")))
        );
        assert_eq!(
            check_frozen(&frozen, "sender", run, None),
            Err((errors::SCOPE_FLOW_ID_MISMATCH, Some("flow_id")))
        );
        assert_eq!(
            check_frozen(
                &frozen,
                "sender",
                ScopeKind::Flow { flow_id: "other" },
                None
            ),
            Err((errors::SCOPE_FLOW_ID_MISMATCH, Some("flow_id")))
        );
        let frozen_run = FrozenSender::new("sender", run, Some("agent"));
        assert_eq!(
            check_frozen(
                &frozen_run,
                "sender",
                ScopeKind::Run { run_id: "other" },
                None
            ),
            Err((errors::SCOPE_RUN_ID_MISMATCH, Some("run_id")))
        );
        assert_eq!(
            check_frozen(&frozen_run, "sender", ScopeKind::Plain, None),
            Err((errors::SCOPE_RUN_ID_MISMATCH, Some("run_id")))
        );
        assert_eq!(
            check_frozen(&frozen, "sender", flow, None),
            Err((errors::SCOPE_AGENT_ID_MISMATCH, Some("agent_id")))
        );
        assert_eq!(
            check_frozen(&frozen, "sender", flow, Some("other")),
            Err((errors::SCOPE_AGENT_ID_MISMATCH, Some("agent_id")))
        );
        let absent_agent = FrozenSender::new("sender", flow, None);
        assert_eq!(
            check_frozen(&absent_agent, "sender", flow, Some("agent")),
            Err((errors::SCOPE_AGENT_ID_MISMATCH, Some("agent_id")))
        );
        assert_eq!(check_frozen(&absent_agent, "sender", flow, None), Ok(()));
    }
}
