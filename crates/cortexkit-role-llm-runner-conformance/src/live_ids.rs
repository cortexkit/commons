//! Provisional display ids, checked against an adapter's native storage boundary.

use cortexkit_role_llm_runner::{
    live::{LiveEvent, LiveSubscribeRequest},
    subscribe::{kinds, SubscribeEvent},
};
use serde_json::Value;

use crate::{
    cases::{Case, Ending},
    subject::LlmRunnerSubject,
    RunnerRoute,
};

pub(crate) async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let opt_in = name != "live_ids_not_sent_without_opt_in";
    let store = name != "live_ids_never_stored_resets";
    let request = LiveSubscribeRequest::default().with_live_ids(opt_in);
    let Some(observed) = case
        .subject
        .live_ids_observation(case.handle.inner(), request, store)
        .await
        .map_err(|e| format!("live_ids_observation: {e}"))?
    else {
        return Ok(Ending::Inapplicable(
            "live_ids_observation: the adapter cannot drive a message before native storage".into(),
        ));
    };
    if !opt_in
        && observed
            .before_storage
            .iter()
            .chain(&observed.after_storage)
            .any(|frame| {
                frame
                    .get("event")
                    .and_then(|event| event.get("live_id"))
                    .is_some()
            })
    {
        return Err("live_id was sent without subscriber opt-in".into());
    }
    let before = events(&observed.before_storage)?;
    let after = events(&observed.after_storage)?;
    let mut live_id = None;
    for event in &before {
        if let Some((mid, id, part)) = delta(event) {
            if !opt_in {
                return Err(
                    "an unstored delta was not held for a subscriber without opt-in".into(),
                );
            }
            provisional(mid, id, part, &mut live_id)?;
        } else if !matches!(event, LiveEvent::DisplayReset) {
            return Err("a control event was sent before native storage".into());
        }
    }
    if opt_in && live_id.is_none() {
        return Err("no provisional delta streamed before native storage".into());
    }
    if !store {
        if observed.stored_mid.is_some() {
            return Err("the aborted message was stored".into());
        }
        for event in &after {
            if let Some((mid, id, part)) = delta(event) {
                provisional(mid, id, part, &mut live_id)?;
            } else if !matches!(event, LiveEvent::DisplayReset) {
                return Err("a never-stored message emitted a control event".into());
            }
        }
        if after.last() != Some(&LiveEvent::DisplayReset) {
            return Err("a never-stored message did not end with display_reset".into());
        }
        return Ok(Ending::Passed);
    }
    let mid = observed
        .stored_mid
        .as_deref()
        .ok_or("the fixture did not store the message")?;
    if live_id.as_deref() == Some(mid) {
        return Err("live_id equals the stored mid".into());
    }
    let mut started = false;
    let mut finished = false;
    for event in &after {
        match event {
            LiveEvent::MessageStarted(message) => {
                if started || message.mid != mid || message.live_id != live_id {
                    return Err("message_started must link the deltas' same live_id to the stored mid exactly once".into());
                }
                started = true;
            }
            LiveEvent::MessageFinished(message) => {
                if !started || finished || message.mid != mid {
                    return Err(
                        "message_finished must follow message_started with the stored mid".into(),
                    );
                }
                finished = true;
            }
            LiveEvent::DisplayReset => {}
            _ => {
                let Some((delta_mid, id, part)) = delta(event) else {
                    return Err("the fixture included an unrelated control event".into());
                };
                if !started || finished || delta_mid != Some(mid) || id.is_some() || part.is_none()
                {
                    return Err(
                        "deltas after message_started must carry mid and part, never live_id"
                            .into(),
                    );
                }
            }
        }
    }
    if !started || !finished {
        return Err("storage must emit message_started and message_finished".into());
    }
    Ok(Ending::Passed)
}

fn events(frames: &[Value]) -> Result<Vec<LiveEvent>, String> {
    frames
        .iter()
        .map(|raw| {
            let frame: SubscribeEvent =
                serde_json::from_value(raw.clone()).map_err(|e| format!("stream frame: {e}"))?;
            let wire = frame
                .body
                .get("event")
                .ok_or("a stream frame has no event")?;
            let event = LiveEvent::decode_strict(wire)?;
            let display = delta(&event).is_some() || matches!(event, LiveEvent::DisplayReset);
            if frame.kind
                != if display {
                    kinds::DISPLAY
                } else {
                    kinds::CONTROL
                }
            {
                return Err("an event was sent on the wrong lane".into());
            }
            if !display && frame.cursor.is_none() {
                return Err("a durable control event has no cursor".into());
            }
            Ok(event)
        })
        .collect()
}

fn delta(event: &LiveEvent) -> Option<(Option<&str>, Option<&str>, Option<&str>)> {
    match event {
        LiveEvent::TextDelta(d) | LiveEvent::ReasoningDelta(d) => {
            Some((d.mid.as_deref(), d.live_id.as_deref(), d.part.as_deref()))
        }
        LiveEvent::ToolInputDelta(d) => {
            Some((d.mid.as_deref(), d.live_id.as_deref(), d.part.as_deref()))
        }
        _ => None,
    }
}

fn provisional(
    mid: Option<&str>,
    id: Option<&str>,
    part: Option<&str>,
    live_id: &mut Option<String>,
) -> Result<(), String> {
    let id = id.ok_or("an unstored delta has no live_id")?;
    if mid.is_some() || part.is_none() {
        return Err("an unstored delta must carry live_id and part, not mid".into());
    }
    if live_id.get_or_insert_with(|| id.into()) != id {
        return Err("an unstored message changed live_id".into());
    }
    Ok(())
}
