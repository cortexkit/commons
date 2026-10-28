//! Deliberately faulty checkpoint behavior, isolated from legacy fixtures.

use super::*;
use cortexkit_role_llm_runner::{
    content_digest::content_digest, read::HeadCut, tags::SetupCheckpoint,
};
use cortexkit_role_llm_runner_conformance::message_checkpoint::{Fixture, Scenario};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Defect {
    #[default]
    None,
    NumberingFromZero,
    HeadMismatch,
    RemovedCutFromStart,
    MarkerMissingCounters,
    MissingCounterFromZero,
    ServesSummaryRow,
    SetupHeadFromStart,
    SetupHeadWaitsForSetup,
    SetupReadWaitsForSetup,
    SetupReadBelowFirstOrdinal,
}

#[derive(Clone)]
pub(super) struct Seed {
    scenario: Scenario,
    checkpoint: SetupCheckpoint,
    marker_key: Option<cortexkit_role_llm_runner::tags::MarkerCheckpoint>,
    setup_completed: Arc<AtomicBool>,
}

pub(super) fn fixture(subject: &FakeSubject, session: &str, scenario: Scenario) -> Fixture {
    let body = json!({"role":"user", "content":[{"type":"text", "text":"Folded history."}]});
    let checkpoint = SetupCheckpoint::new(500, format!("{session}.retained"))
        .with_message_checkpoint(
            42,
            HeadCut::new(
                format!("{session}.folded"),
                content_digest(&body),
                1_793_145_600_123,
            ),
        );
    let initial = cortexkit_role_llm_runner_conformance::CompactionMessage {
        compaction_id: format!("{session}.head"),
        version: 1,
        from: 42,
        to: 42,
        replacement: vec!["folded prefix".into()],
    };
    let setup = if matches!(
        scenario,
        Scenario::MissingMessageCounter | Scenario::MissingTagCounter
    ) {
        cortexkit_role_llm_runner_conformance::CompactionSetup::Ready(initial)
    } else {
        cortexkit_role_llm_runner_conformance::CompactionSetup::ReadyWithMessageCheckpoint {
            initial,
            checkpoint: checkpoint.clone(),
        }
    };
    subject.install_compaction(session, setup);
    subject.world().message_checkpoints.lock().unwrap().insert(
        session.into(),
        Seed {
            scenario,
            checkpoint: checkpoint.clone(),
            marker_key: None,
            setup_completed: Arc::new(AtomicBool::new(false)),
        },
    );
    Fixture {
        checkpoint,
        plan_fields: Map::new(),
    }
}

pub(super) fn marker_setup_outstanding(world: &World, session: &str) -> bool {
    world.setup_holds.lock().unwrap().get(session) == Some(&history::SetupHold::Held)
        && world
            .message_checkpoints
            .lock()
            .unwrap()
            .get(session)
            .is_some_and(|seed| seed.marker_key.is_some())
}

pub(super) fn head_wait(world: &World, session: &str) -> Option<Arc<AtomicBool>> {
    checkpoint_wait(world, session, Defect::SetupHeadWaitsForSetup)
}

pub(super) fn read_wait(world: &World, session: &str) -> Option<Arc<AtomicBool>> {
    checkpoint_wait(world, session, Defect::SetupReadWaitsForSetup)
}

fn checkpoint_wait(world: &World, session: &str, defect: Defect) -> Option<Arc<AtomicBool>> {
    if world.defects.message_checkpoint != defect || !marker_setup_outstanding(world, session) {
        return None;
    }
    world
        .message_checkpoints
        .lock()
        .unwrap()
        .get(session)
        .map(|seed| seed.setup_completed.clone())
}

pub(super) fn inject_below_first(
    sess: &Sess,
    world: &World,
    request: &ReadRequest,
    page: &mut ReadPage,
) {
    if world.defects.message_checkpoint != Defect::SetupReadBelowFirstOrdinal
        || !marker_setup_outstanding(world, &sess.name)
        || request.from_ordinal != Some(sess.first_ordinal)
    {
        return;
    }
    let (Some(cut), Some(ordinal)) = (&sess.moved_head, sess.first_ordinal.checked_sub(1)) else {
        return;
    };
    let mut row = ReadMessage::new(
        ordinal,
        &cut.mid,
        json!({"role":"user", "content":[{"type":"text", "text":"Folded history."}]}),
    );
    row.digest = Some(cut.digest.clone());
    row.written_at = Some(cut.written_at);
    row.role = Some("user".into());
    page.messages.insert(0, row);
}

pub(super) fn setup_returned(world: &World, session: &str) {
    if let Some(seed) = world.message_checkpoints.lock().unwrap().get(session) {
        seed.setup_completed.store(true, Ordering::SeqCst);
    }
}

pub(super) fn marker_fixture(subject: &FakeSubject, session: &str) -> Fixture {
    let mut fixture = fixture(subject, session, Scenario::Adoption);
    let key = cortexkit_role_llm_runner::tags::MarkerCheckpoint::new(500, session)
        .with_message_checkpoint(42, fixture.checkpoint.head_cut.clone().unwrap());
    subject
        .world()
        .message_checkpoints
        .lock()
        .unwrap()
        .get_mut(session)
        .unwrap()
        .marker_key = Some(key);
    subject.install_compaction(
        session,
        cortexkit_role_llm_runner_conformance::CompactionSetup::ReadyWithMessageCheckpoint {
            initial: cortexkit_role_llm_runner_conformance::CompactionMessage {
                compaction_id: format!("{session}.marker-head"),
                version: 1,
                from: 42,
                to: 42,
                replacement: vec!["folded native prefix".into()],
            },
            checkpoint: fixture.checkpoint.clone(),
        },
    );
    fixture.plan_fields = subject.send_fields(session, true);
    fixture.plan_fields.get_mut("plan").unwrap()["render_tags"] = json!(true);
    fixture
}

pub(super) fn refuse_missing_counter(world: &World, session: &str) -> bool {
    world
        .message_checkpoints
        .lock()
        .unwrap()
        .get(session)
        .is_some_and(|seed| {
            matches!(
                seed.scenario,
                Scenario::MissingMessageCounter | Scenario::MissingTagCounter
            ) && world.defects.message_checkpoint != Defect::MissingCounterFromZero
        })
}

pub(super) fn adopt(sess: &mut Sess, world: &World) {
    let seeds = world.message_checkpoints.lock().unwrap();
    let Some(seed) = seeds.get(&sess.name) else {
        return;
    };
    let zero = world.defects.message_checkpoint == Defect::NumberingFromZero
        || (seed.scenario == Scenario::CutRemoved
            && world.defects.message_checkpoint == Defect::RemovedCutFromStart)
        || matches!(
            seed.scenario,
            Scenario::MissingMessageCounter | Scenario::MissingTagCounter
        );
    sess.first_ordinal = if zero { 0 } else { 42 };
    sess.base = sess.first_ordinal;
    sess.moved_head = seed.checkpoint.head_cut.clone();
    if let Some(key) = &seed.marker_key {
        sess.first_ordinal = key.message_count.unwrap();
        sess.base = sess.first_ordinal;
        sess.moved_head = key.head_cut.clone();
        sess.planted_marker = Some(
            cortexkit_role_llm_runner::tags::Marker::new(&seed.checkpoint.cut, true)
                .with_checkpoint(key.clone()),
        );
    }
    if world.defects.message_checkpoint == Defect::HeadMismatch {
        sess.moved_head.as_mut().unwrap().digest = "0".repeat(64);
    }
    if seed.scenario == Scenario::Adoption {
        let ordinal = sess.push(
            json!({"role":"user", "content":[{"type":"text", "text":"retained boundary"}]}),
            "",
            Vec::new(),
        );
        sess.at_mut(ordinal).mid = seed.checkpoint.cut.clone();
    }
}

pub(super) fn write_native_key(sess: &mut Sess, record: &Value, defects: &Defects) {
    let first = record["first_ordinal"].as_u64().unwrap();
    let tag_checkpoint = sess
        .numbering
        .iter()
        .filter(|((mid, _), _)| {
            sess.messages
                .iter()
                .any(|m| &m.mid == mid && m.ordinal < first)
        })
        .map(|(_, n)| *n)
        .max()
        .unwrap_or(0);
    let mut key = json!({
        "session":sess.name, "tag_checkpoint":tag_checkpoint, "message_count": first,
        "head_cut":{"mid":record["through_mid"], "digest":record["through_digest"], "written_at":record["through_written_at"]}
    });
    if defects.message_checkpoint == Defect::MarkerMissingCounters {
        key.as_object_mut().unwrap().remove("message_count");
    }
    sess.native_message_key = Some(key);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_checkpoint_planted_marker_tags_are_available_before_setup_answer() {
        let mut sess = Sess {
            name: "s".into(),
            lineage: "line".into(),
            render_tags: true,
            first_ordinal: 42,
            base: 42,
            planted_marker: Some(
                cortexkit_role_llm_runner::tags::Marker::new("retained", true).with_checkpoint(
                    cortexkit_role_llm_runner::tags::MarkerCheckpoint::new(500, "s"),
                ),
            ),
            ..Sess::default()
        };
        let ordinal = sess.push(
            json!({"role":"user", "content":[{"type":"text", "text":"retained boundary"}]}),
            "",
            Vec::new(),
        );
        sess.at_mut(ordinal).mid = "retained".into();
        assert!(sess.compaction.checkpoint.is_none());
        let before = tags::fresh(&sess, &Defects::default(), true);
        assert_eq!(before.values().copied().collect::<Vec<_>>(), vec![501]);
        sess.compaction.checkpoint = Some((500, "retained".into()));
        assert_eq!(tags::fresh(&sess, &Defects::default(), true), before);
    }
}
