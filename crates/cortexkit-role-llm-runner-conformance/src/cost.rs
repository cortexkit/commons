//! `per_step_cost_scales_with_new_messages`: the timing half of the cost
//! rule (`cortexkit-role-llm-runner` CONTRACT §14.2).
//!
//! Two sessions, each seeded through the runner's own store with finished
//! messages behind a cut that leaves the same window, one with 1,000 and one
//! with 150,000. The case times one exchange at a time on each, alternating
//! between them, and requires the median step time of the large session to
//! be at most twice the small one's. Memory is not measured: it is part of
//! each runner's own acceptance benchmark, not of this case.

use std::time::{Duration, Instant};

use crate::{
    cases::{Case, Ending, Session},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};

/// The seeded sizes the rule compares.
const SMALL: u64 = 1_000;
const LARGE: u64 = 150_000;

/// The window both sessions keep since their latest cut.
const WINDOW: u64 = 20;

/// Timed exchanges per session, after one untimed warm-up exchange each.
const SAMPLES: usize = 9;

/// The largest median ratio the rule allows.
const MAX_RATIO: f64 = 2.0;

/// The timing resolution: a median below it counts as this long, so the
/// scheduling noise of a step that takes a few milliseconds cannot decide
/// the case either way.
const RESOLUTION: Duration = Duration::from_millis(5);

async fn seeded<S>(
    case: &Case<'_, S>,
    label: &str,
    messages: u64,
) -> Result<Result<Session<S::Route>, String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let script = Script {
        turns: (0..SAMPLES + 2)
            .map(|n| ScriptedTurn::new(vec![ScriptedPart::Text(format!("{label}-answer-{n}"))]))
            .collect(),
    };
    let mut session = case.open(label, script).await?;
    case.run_prompt(&mut session, &format!("{label}-first"))
        .await?;
    if let Err(e) = case
        .subject
        .seed_session_history(case.handle.inner(), &session.name, messages, WINDOW)
        .await
    {
        return Ok(Err(format!(
            "the adapter cannot seed finished messages through the runner's store: {e}"
        )));
    }
    case.run_prompt(&mut session, &format!("{label}-warm-up"))
        .await?;
    Ok(Ok(session))
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2].max(RESOLUTION)
}

pub async fn run<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut small = match seeded(case, &case.mint.next("cost-small"), SMALL).await? {
        Ok(session) => session,
        Err(reason) => return Ok(Ending::Inapplicable(reason)),
    };
    let mut large = match seeded(case, &case.mint.next("cost-large"), LARGE).await? {
        Ok(session) => session,
        Err(reason) => return Ok(Ending::Inapplicable(reason)),
    };
    let mut small_times = Vec::with_capacity(SAMPLES);
    let mut large_times = Vec::with_capacity(SAMPLES);
    for n in 0..SAMPLES {
        for (session, times) in [
            (&mut small, &mut small_times),
            (&mut large, &mut large_times),
        ] {
            let started = Instant::now();
            case.run_prompt(session, &format!("cost-step-{n}")).await?;
            times.push(started.elapsed());
        }
    }
    let (small_median, large_median) = (median(small_times), median(large_times));
    let ratio = large_median.as_secs_f64() / small_median.as_secs_f64();
    if ratio > MAX_RATIO {
        return Err(format!(
            "the median step took {large_median:?} with {LARGE} seeded messages and \
             {small_median:?} with {SMALL}, a ratio of {ratio:.2}, above {MAX_RATIO}: per-step \
             work grows with the session's length"
        ));
    }
    Ok(Ending::Passed)
}
