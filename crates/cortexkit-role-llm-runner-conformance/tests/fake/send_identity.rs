//! Keep caller-visible send ids separate from the fake's internal queue keys.

use super::*;

const UNKEYED: &str = "submission-unkeyed-";
const UNKEYED_NO_ID: &str = "unkeyed-no-id-";

impl Module {
    pub(super) fn unkeyed_send(
        &self,
        stamp: &RouteStamp,
        session: &str,
        mut params: Value,
    ) -> Result<Reply, Killed> {
        let sessions = self.sessions.lock().unwrap();
        let Some(sess) = sessions.get(session) else {
            return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
        };
        if sess.owner == stamp.principal || self.world.defects.send_id_unkeyed == Some("refused") {
            return Ok(refuse(errors::SCOPE_OWNER_MISMATCH, None));
        }
        let prefix = if self.world.defects.send_id_unkeyed == Some("no_id") {
            UNKEYED_NO_ID
        } else {
            UNKEYED
        };
        let id = format!(
            "{prefix}{}",
            sess.sends.len() + sess.ordered.len() + sess.messages.len()
        );
        drop(sessions);
        params["send_id"] = json!(id);
        let mut reply = self.send(stamp, session, params)?;
        if let Reply::Response(body) = &mut reply {
            if prefix == UNKEYED {
                body["submission_id"] = json!(id);
            } else {
                body.as_object_mut().unwrap().remove("submission_id");
            }
        }
        Ok(reply)
    }

    pub(super) fn send_id_fact(&self, message: &Msg, history: bool) -> Option<String> {
        if self.world.defects.send_id_missing {
            return None;
        }
        let id = message.send_id.as_deref()?;
        if id.starts_with(UNKEYED_NO_ID) {
            return None;
        }
        if self.world.defects.send_id_unkeyed_internal && id.starts_with(UNKEYED) {
            return Some("internal-unkeyed-id".into());
        }
        if (history && self.world.defects.send_id_history_wrong)
            || (!history && self.world.defects.send_id_read_wrong)
        {
            return Some(
                if history {
                    "wrong-history-id"
                } else {
                    "wrong-read-id"
                }
                .into(),
            );
        }
        Some(id.into())
    }
}
