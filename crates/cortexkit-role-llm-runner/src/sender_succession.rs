//! One-hop retry and status lookup across a change of session sender.
//!
//! Check the raw plan before decoding its senders, fetching items or writing
//! admission state, so a refusal names the particular `succeeds` member.

use crate::{capabilities, delivery_order::PlanSender};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// Validate the opt-in and every supplied `succeeds`. Returns the field an
/// `invalid_params` refusal names. A predecessor need not be a listed sender;
/// only one successor may name it, and a module cannot succeed itself.
pub fn check_plan<S: AsRef<str>>(plan: &Map<String, Value>, groups: &[S]) -> Result<(), String> {
    let Some(senders) = plan.get("senders").and_then(Value::as_array) else {
        // The ordinary sender-plan check owns the enclosing member's shape.
        return Ok(());
    };
    let declared = groups
        .iter()
        .any(|group| group.as_ref() == capabilities::SENDER_SUCCESSION);
    let mut predecessors = BTreeSet::new();
    for (index, sender) in senders.iter().enumerate() {
        let Some(raw) = sender.get("succeeds") else {
            continue;
        };
        let field = || format!("plan.senders[{index}].succeeds");
        let predecessor = raw.as_str().filter(|id| !id.is_empty()).ok_or_else(field)?;
        if !declared
            || sender.get("module").and_then(Value::as_str) == Some(predecessor)
            || !predecessors.insert(predecessor)
        {
            return Err(field());
        }
    }
    Ok(())
}

/// The predecessor whose send records the caller checks first for a keyed
/// retry or `send.status`, from the current admission plan's senders. It
/// only selects the module, one hop, never following that module's own
/// succession. Looking up the record, comparing payloads and falling back to
/// the caller's own records (CONTRACT §9.7) are the caller's steps. It grants
/// no send, stop or withdrawal on the predecessor's behalf.
pub fn predecessor<'a>(senders: &'a [PlanSender], caller: &str) -> Option<&'a str> {
    senders
        .iter()
        .find(|sender| sender.module == caller)
        .and_then(|sender| sender.succeeds.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{send::SendRequest, session_change::RefreshRequest};
    use serde_json::json;

    #[test]
    fn plan_and_refresh_share_indexed_shape_and_opt_in_refusals() {
        for (senders, index) in [
            (json!([{"module":"new", "succeeds":null}]), 0),
            (json!([{"module":"new", "succeeds":12}]), 0),
            (json!([{"module":"new", "succeeds":""}]), 0),
            (json!([{"module":"new", "succeeds":"new"}]), 0),
            (
                json!([{"module":"a", "succeeds":"old"}, {"module":"b", "succeeds":"old"}]),
                1,
            ),
        ] {
            let plan = json!({"senders":senders}).as_object().unwrap().clone();
            let send = SendRequest::new("hello", "send-1").with_plan(plan.clone());
            let refresh = RefreshRequest::new(plan, "immediately", 1);
            let expected = Err(format!("plan.senders[{index}].succeeds"));
            assert_eq!(
                send.check_sender_succession(&[capabilities::SENDER_SUCCESSION]),
                expected
            );
            assert_eq!(
                refresh.check_sender_succession(&[capabilities::SENDER_SUCCESSION]),
                expected
            );
        }
        let plan = json!({"senders":[{"module":"new", "succeeds":"old"}]})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            check_plan(&plan, &[] as &[&str]),
            Err("plan.senders[0].succeeds".into())
        );
        assert_eq!(
            check_plan(&plan, &[capabilities::SENDER_SUCCESSION]),
            Ok(())
        );
    }

    #[test]
    fn lookup_is_one_hop_and_needs_no_predecessor_entry() {
        let senders = vec![
            PlanSender::new("new").with_succeeds("old"),
            PlanSender::new("old").with_succeeds("older"),
        ];
        assert_eq!(predecessor(&senders, "new"), Some("old"));
        assert_eq!(predecessor(&senders[..1], "new"), Some("old"));
        assert_eq!(predecessor(&senders, "third"), None);
    }
}
