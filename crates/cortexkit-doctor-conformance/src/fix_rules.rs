use cortexkit_doctor::{runner, CheckOutput, Finding, FindingId, FixResult, Mode};

use crate::stand_in::StandIn;
use crate::{run_failure, verdict, Adapter, Outcome, RuleResult, Scratch};

#[derive(Clone, Copy)]
enum Scenario {
    StaleDigest,
    ModuleRunning,
    NoTerminal,
    FullRecheck,
}

pub(super) fn run(
    adapter: &mut impl Adapter,
    check_valid: bool,
    full_valid: bool,
) -> [RuleResult; 5] {
    let skip = if !adapter.non_prompting() {
        Some("non_prompting_not_declared")
    } else if !check_valid {
        Some("prerequisite_failed")
    } else {
        None
    };
    let mut comparisons = Vec::new();
    let [stale_digest, module_running, no_terminal, full_recheck] = [
        (Scenario::StaleDigest, "fix.stale_digest"),
        (Scenario::ModuleRunning, "fix.module_running"),
        (Scenario::NoTerminal, "fix.no_terminal"),
        (Scenario::FullRecheck, "fix.full_recheck"),
    ]
    .map(|(scenario, name)| RuleResult {
        name,
        outcome: {
            let skip = match scenario {
                Scenario::FullRecheck if adapter.full_fixable_id().is_none() => {
                    Some("missing_adapter_input")
                }
                Scenario::FullRecheck if !full_valid => skip.or(Some("prerequisite_failed")),
                _ => skip,
            };
            match skip {
                Some(reason) => Outcome::Skipped { reason },
                None => scenario
                    .run(adapter, &mut comparisons)
                    .unwrap_or_else(|reason| Outcome::Failed { reason }),
            }
        },
    });
    let no_change = if comparisons.is_empty() {
        Outcome::Skipped {
            reason: skip.unwrap_or("no_fix_run"),
        }
    } else {
        verdict(comparisons.into_iter().collect::<Result<(), _>>())
    };
    [
        stale_digest,
        module_running,
        no_terminal,
        full_recheck,
        RuleResult {
            name: "fix.no_change_on_refusal",
            outcome: no_change,
        },
    ]
}

impl Scenario {
    fn run(
        self,
        adapter: &mut impl Adapter,
        comparisons: &mut Vec<Result<(), &'static str>>,
    ) -> Result<Outcome, &'static str> {
        let running = matches!(self, Self::ModuleRunning);
        let (id, liveness, mode) = match self {
            Self::ModuleRunning => (
                adapter.needs_stop_id().cloned(),
                adapter.liveness().cloned(),
                Mode::Quick,
            ),
            Self::FullRecheck => (adapter.full_fixable_id().cloned(), None, Mode::Full),
            _ => (adapter.fixable_id().cloned(), None, Mode::Quick),
        };
        let Some(id) = id.filter(|_| !running || liveness.is_some()) else {
            return Ok(Outcome::Skipped {
                reason: "missing_adapter_input",
            });
        };

        let scratch = Scratch::new(adapter)?;
        let options = scratch.runner_options();
        let baseline = runner::check(adapter.program(), mode, &options).map_err(run_failure)?;
        let finding = find(&baseline, &id).ok_or("bad_adapter_input")?;
        let plan = finding.fix().plan().ok_or("bad_adapter_input")?;
        if running && !finding.fix().needs_stop() {
            return Err("bad_adapter_input");
        }
        if mode == Mode::Full {
            let quick =
                runner::check(adapter.program(), Mode::Quick, &options).map_err(run_failure)?;
            if find(&quick, &id).is_some() {
                return Err("bad_adapter_input");
            }
        }

        // Keep the signal active across the observation, fix, and both snapshots.
        let _stand_in = if let Some(liveness) = liveness {
            let stand_in = StandIn::start(scratch.data_home(), &liveness)?;
            let observed =
                runner::check(adapter.program(), Mode::Quick, &options).map_err(run_failure)?;
            let observed_plan = find(&observed, &id).and_then(|finding| finding.fix().plan());
            if !observed_plan.is_some_and(|observed| observed.digest() == plan.digest()) {
                return Ok(Outcome::Skipped {
                    reason: "setup_not_observable",
                });
            }
            Some(stand_in)
        } else {
            None
        };

        let (digest, expected) = match self {
            Self::StaleDigest => (stale_digest(plan.digest()), "plan_changed"),
            Self::ModuleRunning => (plan.digest().to_owned(), "module_running"),
            Self::NoTerminal | Self::FullRecheck => (plan.digest().to_owned(), "no_terminal"),
        };
        let before = scratch.snapshot()?;
        let result = runner::fix(
            adapter.program(),
            &id,
            &digest,
            plan.timeout_ms(),
            mode,
            &options.with_conformance_fix_marker(),
        );
        comparisons.push(scratch.snapshot().and_then(|after| {
            if before == after {
                Ok(())
            } else {
                Err("home_changed")
            }
        }));

        let output = result.map_err(run_failure)?;
        if output.result() == FixResult::Refused && output.code() == Some(expected) {
            Ok(Outcome::Passed)
        } else {
            Err("unexpected_fix_result")
        }
    }
}

fn find<'a>(check: &'a CheckOutput, id: &FindingId) -> Option<&'a Finding> {
    check.findings().iter().find(|finding| finding.id() == id)
}

fn stale_digest(digest: &str) -> String {
    // Change one validated hex digit so the stale digest still passes argv validation.
    let mut stale = digest.to_owned();
    let first = "sha256:".len();
    let replacement = if stale.as_bytes()[first] == b'0' {
        "1"
    } else {
        "0"
    };
    stale.replace_range(first..=first, replacement);
    stale
}
