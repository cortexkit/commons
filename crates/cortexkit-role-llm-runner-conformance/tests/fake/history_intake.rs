//! A held import whose stored prefix is independent of a queued user turn.

use super::*;

pub struct Intake {
    texts: Vec<String>,
    stored: usize,
    queued: Option<(RouteStamp, Value)>,
}

pub fn state(world: &World, session: &str) -> Option<String> {
    let intakes = world.intakes.lock().unwrap();
    let intake = intakes.get(session)?;
    if world.defects.history_intake == Some("early_marker") && intake.stored > 1 {
        None
    } else {
        Some("running".into())
    }
}

pub fn intercept_send(
    module: &Module,
    stamp: &RouteStamp,
    session: &str,
    params: &Value,
) -> Option<Reply> {
    let mut intakes = module.world.intakes.lock().unwrap();
    let intake = intakes.get_mut(session)?;
    match module.world.defects.history_intake {
        Some("send_refused") => Some(invalid("history_intake")),
        Some("run_early") => None,
        _ => {
            intake.queued = Some((stamp.clone(), params.clone()));
            Some(Reply::Response(
                json!({"state":"pending","submission_id":params["send_id"]}),
            ))
        }
    }
}

fn module(subject: &FakeSubject) -> Arc<Module> {
    subject.modules.lock().unwrap().last().unwrap().clone()
}

pub fn begin(subject: &FakeSubject, session: &str, texts: &[String]) -> Result<bool, HarnessError> {
    if !subject.defects.history_intake_fixture {
        return Ok(false);
    }
    let module = module(subject);
    module
        .commit(
            session,
            json!({"kind":"start", "owner":subject.owner_stamp().principal,
        "lineage":format!("intake-{session}"), "tools":[SCRIPTED_TOOL]}),
        )
        .map_err(|_| HarnessError::new("killed beginning intake"))?;
    module.world.intakes.lock().unwrap().insert(
        session.into(),
        Intake {
            texts: texts.to_vec(),
            stored: 0,
            queued: None,
        },
    );
    append(&module, session, &texts[0], false)?;
    module
        .world
        .intakes
        .lock()
        .unwrap()
        .get_mut(session)
        .unwrap()
        .stored = 1;
    Ok(true)
}

fn append(
    module: &Module,
    session: &str,
    text: &str,
    bump_version: bool,
) -> Result<(), HarnessError> {
    module.commit(session, json!({"kind":"intake_chunk","body":{"role":"user","text":text},"bump_version":bump_version}))
        .map_err(|_| HarnessError::new("killed storing intake chunk"))
}

pub fn advance(subject: &FakeSubject, session: &str, finish: bool) -> Result<bool, HarnessError> {
    let module = module(subject);
    let (texts, start) = {
        let intakes = module.world.intakes.lock().unwrap();
        let Some(intake) = intakes.get(session) else {
            return Ok(false);
        };
        (intake.texts.clone(), intake.stored)
    };
    let end = if finish { texts.len() } else { start + 1 };
    for text in &texts[start..end] {
        append(
            &module,
            session,
            text,
            !finish && subject.defects.history_intake == Some("version"),
        )?;
    }
    if finish {
        let intake = module
            .world
            .intakes
            .lock()
            .unwrap()
            .remove(session)
            .unwrap();
        if let Some((stamp, params)) = intake.queued {
            module
                .send(&stamp, session, params)
                .map_err(|_| HarnessError::new("killed releasing intake send"))?;
        }
    } else {
        module
            .world
            .intakes
            .lock()
            .unwrap()
            .get_mut(session)
            .unwrap()
            .stored = end;
    }
    Ok(true)
}
