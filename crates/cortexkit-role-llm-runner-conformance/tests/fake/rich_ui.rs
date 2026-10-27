//! The `rich_ui_stamp` group in the durable-log fake: the scripted
//! system-text providers whose answers declare a rich UI stamp, admission's
//! agreement check, the stamp each run freezes from its plan, and the
//! surfaces that serve it, with the deliberate breaks the cases must catch.
//!
//! A session whose first send carries `plan.system_texts` was built by
//! `rich_ui_plan_fields`: at admission the fake "fetches" one answer per
//! item from the world's scripted providers and freezes the stamp they agree
//! on in its start record. Every run records the stamp its plan carries when
//! it starts, so a later replan changes only later runs.

use super::*;
use cortexkit_role_llm_runner::{display::DisplayRow, rich_ui::RichUiStamp};
use cortexkit_role_llm_runner_conformance::rich_ui_stamp::{conflicting_stamp, fixture_stamp};

/// The plan member naming the session's system-text items, one provider each.
pub(super) const PLAN_SYSTEM_TEXTS: &str = "system_texts";

fn decode(stamp: &Value) -> Option<RichUiStamp> {
    (!stamp.is_null())
        .then(|| serde_json::from_value(stamp.clone()).expect("recorded stamps decode"))
}

/// The stamp a run serves on its surfaces, with the breaks that change the
/// value itself. Only a session admitted with a rich UI plan has stamps.
pub(super) fn run_stamp(defects: &Defects, sess: &Sess, run_id: &str) -> Option<RichUiStamp> {
    if !sess.rich_ui_plan {
        return None;
    }
    let run = sess.run(run_id)?;
    match run.rich_ui.as_ref().and_then(decode) {
        Some(_) if defects.rich_ui_wrong_value => Some(conflicting_stamp()),
        Some(stamp) => Some(stamp),
        None if defects.rich_ui_stamps_unstamped_run => Some(fixture_stamp()),
        None => None,
    }
}

/// Whether `m` is the first of two or more assistant messages its run
/// wrote: the one `rich_ui_skips_an_assistant_row` leaves unstamped.
fn first_of_several(sess: &Sess, m: &Msg) -> bool {
    let mut assistants = sess
        .messages
        .iter()
        .filter(|other| other.run_id == m.run_id && other.body["role"] == "assistant");
    let first = assistants.next().map(|first| first.mid.clone());
    assistants.next().is_some() && first.as_deref() == Some(m.mid.as_str())
}

/// The stamp the row or entry of message `m` carries. `user_defect` is the
/// break that also stamps the user messages of a stamped run on this surface.
fn message_stamp(
    defects: &Defects,
    sess: &Sess,
    m: &Msg,
    user_defect: bool,
) -> Option<RichUiStamp> {
    if m.run_id.is_empty() {
        return None;
    }
    let stamp = run_stamp(defects, sess, &m.run_id)?;
    match m.body["role"].as_str() {
        Some("assistant")
            if defects.rich_ui_skips_an_assistant_row && first_of_several(sess, m) =>
        {
            None
        }
        Some("assistant") => Some(stamp),
        Some("user") if user_defect => Some(stamp),
        _ => None,
    }
}

impl FakeSubject {
    pub(super) fn rich_ui_plan(
        &self,
        session: &str,
        stamps: &[Option<RichUiStamp>],
    ) -> Option<Map<String, Value>> {
        if !self.rich_ui_plan_fixture || self.without_plans {
            return None;
        }
        self.world().rich_ui_answers.lock().unwrap().insert(
            session.into(),
            stamps
                .iter()
                .map(|stamp| serde_json::to_value(stamp).unwrap())
                .collect(),
        );
        let mut fields = self.send_fields(session, true);
        let items: Vec<Value> = (0..stamps.len())
            .map(|index| json!({ "provider": format!("fake.rich-ui-{index}") }))
            .collect();
        fields.get_mut("plan")?[PLAN_SYSTEM_TEXTS] = json!(items);
        Some(fields)
    }

    pub(super) fn rich_ui_replan(&self, session: &str, stamp: Option<&RichUiStamp>) -> bool {
        if !self.rich_ui_replan_fixture {
            return false;
        }
        self.world()
            .rich_ui_replans
            .lock()
            .unwrap()
            .insert(session.into(), serde_json::to_value(stamp).unwrap());
        true
    }
}

impl Module {
    /// Admission's fetch of the plan's system-text answers: whether the plan
    /// names any, and the stamp they agree on, or the `plan_stale` refusal of
    /// two different stamps.
    pub(super) fn admit_rich_ui(
        &self,
        session: &str,
        plan: Option<&Map<String, Value>>,
    ) -> Result<(bool, Value), Reply> {
        let Some(items) = plan
            .and_then(|plan| plan.get(PLAN_SYSTEM_TEXTS))
            .and_then(Value::as_array)
            .filter(|_| self.world.serves(groups::RICH_UI_STAMP))
        else {
            return Ok((false, Value::Null));
        };
        let answers = self
            .world
            .rich_ui_answers
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_default();
        let fetched: Vec<(String, Option<RichUiStamp>)> = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let provider = item["provider"].as_str().unwrap_or_default().to_owned();
                (provider, answers.get(index).and_then(decode))
            })
            .collect();
        let stamp = match cortexkit_role_llm_runner::rich_ui::plan_stamp(
            fetched
                .iter()
                .map(|(provider, stamp)| (provider.as_str(), stamp.as_ref())),
        ) {
            Ok(stamp) => stamp,
            Err(_) if self.world.defects.rich_ui_conflict_accepted => {
                fetched.iter().find_map(|(_, stamp)| stamp.clone())
            }
            Err(conflict) => return Err(refuse(conflict.code(), Some(conflict.detail()))),
        };
        Ok((true, serde_json::to_value(stamp).unwrap()))
    }

    /// Apply a replan the adapter asked for, durably, before the next run
    /// starts, as a runner applies an accepted refresh.
    pub(super) fn apply_rich_ui_replan(&self, session: &str) -> Result<(), Killed> {
        let pending = self.world.rich_ui_replans.lock().unwrap().remove(session);
        match pending {
            Some(stamp) => self.commit(
                session,
                json!({ "kind": "rich_ui_replan", "rich_ui": stamp }),
            ),
            None => Ok(()),
        }
    }

    /// The `rich_ui` of a `session.read` entry for `m`.
    pub(super) fn read_stamp(&self, sess: &Sess, m: &Msg, opted: bool) -> Option<RichUiStamp> {
        let defects = &self.world.defects;
        if !self.world.serves(groups::RICH_UI_STAMP)
            || !(opted || defects.rich_ui_read_without_opt_in)
        {
            return None;
        }
        message_stamp(defects, sess, m, defects.rich_ui_read_on_user_entry)
    }

    /// Stamp the display row of the message `mid`.
    pub(super) fn stamp_row(&self, sess: &Sess, mid: &str, row: &mut DisplayRow) {
        if !self.world.serves(groups::RICH_UI_STAMP) {
            return;
        }
        let defects = &self.world.defects;
        if let Some(m) = sess.messages.iter().find(|m| m.mid == mid) {
            row.rich_ui = message_stamp(defects, sess, m, defects.rich_ui_on_user_rows);
        }
    }

    /// `session.head`'s `last_run_rich_ui` for the session's last run.
    pub(super) fn head_stamp(&self, sess: &Sess) -> Option<RichUiStamp> {
        let defects = &self.world.defects;
        if !self.world.serves(groups::RICH_UI_STAMP) || defects.rich_ui_head_omitted {
            return None;
        }
        if defects.rich_ui_head_leftover {
            // The latest stamp any run carried, whichever run is last.
            return sess
                .runs
                .iter()
                .rev()
                .find_map(|run| run_stamp(defects, sess, &run.run_id));
        }
        run_stamp(defects, sess, &sess.runs.last()?.run_id)
    }
}
