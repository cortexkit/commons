//! Opt-in plan-change notes (contract §10.2).
//!
//! `notes_until_rebuild` is a policy, not an ordered cost rung. Notes are
//! durable runner records, never transcript entries or tool-result rides.
//! Their delivery and reconciliation are the runner's responsibility.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{capabilities, session_change::RefreshRequest};

pub const NOTES_UNTIL_REBUILD: &str = "notes_until_rebuild";
pub const SYSTEM_NOTE_MAX: usize = 8_192;

/// Additional native-provider forms in the notice form vocabulary.
pub mod forms {
    pub const TOOL_ADDITION: &str = "tool_addition";
    pub const TOOL_REMOVAL: &str = "tool_removal";
}

/// Fixed text for an addition, using the exact offered tool name.
pub fn tool_addition(name: &str) -> String {
    format!("Tool \"{name}\" is now available.")
}

/// Fixed text for a removal, also used as the per-call refusal message.
pub fn tool_removal(name: &str) -> String {
    format!("Tool \"{name}\" is no longer available. Do not call it.")
}

fn declares<S: AsRef<str>>(groups: &[S], group: &str) -> bool {
    groups.iter().any(|value| value.as_ref() == group)
}

/// Validate a refresh policy before fetching or writing. Old runners keep
/// using `RefreshRequest::check_policy`, which rejects the opt-in policy.
pub fn check_policy<S: AsRef<str>>(policy: &str, groups: &[S]) -> Result<(), &'static str> {
    if crate::rungs::ALL.contains(&policy)
        || (policy == NOTES_UNTIL_REBUILD && declares(groups, capabilities::PLAN_NOTES))
    {
        Ok(())
    } else {
        Err("policy")
    }
}

/// Validate the note against the fetched applied/pending system-text diff.
/// The owner supplies a short change description, never replacement text.
/// Validation failures name the `invalid_params` field and write nothing.
pub fn check_refresh<S: AsRef<str>>(
    policy: &str,
    system_note: Option<&str>,
    system_text_changed: bool,
    groups: &[S],
) -> Result<(), &'static str> {
    if system_note.is_some()
        && (!declares(groups, capabilities::PLAN_NOTES) || policy != NOTES_UNTIL_REBUILD)
    {
        return Err("system_note");
    }
    check_policy(policy, groups)?;
    if policy == NOTES_UNTIL_REBUILD
        && (system_note.is_some() != system_text_changed
            || system_note.is_some_and(|note| note.len() > SYSTEM_NOTE_MAX))
    {
        return Err("system_note");
    }
    Ok(())
}

/// The `session.refresh` body for a runner declaring `plan_notes`, with or
/// without `refresh_relaunch`. Decode `RefreshRequest` with neither group,
/// or `RelaunchRefreshRequest` with only `refresh_relaunch`. These are strict
/// views of one wire body, not separate operations. This additive type keeps
/// exhaustive `RefreshRequest` struct literals source-compatible.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PlanNotesRefreshRequest {
    pub plan: Map<String, Value>,
    pub policy: String,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_note: Option<String>,
    /// Accepted only when `refresh_relaunch` is also declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

impl PlanNotesRefreshRequest {
    pub fn new(plan: Map<String, Value>, policy: impl Into<String>, generation: u64) -> Self {
        RefreshRequest::new(plan, policy, generation).into()
    }

    pub fn with_system_note(mut self, note: impl Into<String>) -> Self {
        self.system_note = Some(note.into());
        self
    }

    pub fn with_required(mut self, required: bool) -> Self {
        self.required = Some(required);
        self
    }

    /// Validate declarations and the fetched surface diff before acceptance.
    pub fn check<S: AsRef<str>>(
        &self,
        groups: &[S],
        system_text_changed: bool,
    ) -> Result<(), &'static str> {
        if self.required.is_some() && !declares(groups, capabilities::REFRESH_RELAUNCH) {
            return Err("required");
        }
        check_refresh(
            &self.policy,
            self.system_note.as_deref(),
            system_text_changed,
            groups,
        )
    }
}

impl From<RefreshRequest> for PlanNotesRefreshRequest {
    fn from(request: RefreshRequest) -> Self {
        Self {
            plan: request.plan,
            policy: request.policy,
            generation: request.generation,
            system_note: None,
            required: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_notes_refresh_preserves_legacy_bytes_and_strict_decoding() {
        let old = RefreshRequest::new(Map::new(), crate::rungs::IMMEDIATELY, 7);
        let new = PlanNotesRefreshRequest::from(old.clone());
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
        let with_note = PlanNotesRefreshRequest::new(Map::new(), NOTES_UNTIL_REBUILD, 8)
            .with_system_note("Use the new response style.");
        let bytes = serde_json::to_vec(&with_note).unwrap();
        assert!(serde_json::from_slice::<RefreshRequest>(&bytes).is_err());
        let relaunch: crate::session_change::RelaunchRefreshRequest =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            relaunch.check_plan_notes(&[capabilities::PLAN_NOTES], true),
            Ok(())
        );
        assert_eq!(
            relaunch.check_plan_notes::<&str>(&[], true),
            Err("system_note")
        );
        let mut legacy_relaunch = relaunch;
        legacy_relaunch.policy = crate::rungs::IMMEDIATELY.into();
        assert_eq!(legacy_relaunch.check_policy(), Err("system_note"));
    }

    #[test]
    fn plan_notes_system_note_matches_diff_policy_and_utf8_limit() {
        let groups = [capabilities::PLAN_NOTES];
        let request = PlanNotesRefreshRequest::new(Map::new(), NOTES_UNTIL_REBUILD, 1);
        assert_eq!(request.check(&groups, false), Ok(()));
        assert_eq!(request.check(&groups, true), Err("system_note"));
        assert_eq!(request.check::<&str>(&[], false), Err("policy"));
        let mut note = request.with_system_note("é".repeat(SYSTEM_NOTE_MAX / 2));
        assert_eq!(note.check(&groups, true), Ok(()));
        assert_eq!(note.check(&groups, false), Err("system_note"));
        note.system_note.as_mut().unwrap().push('x');
        assert_eq!(note.check(&groups, true), Err("system_note"));
        note.system_note = Some("short".into());
        note.policy = crate::rungs::IMMEDIATELY.into();
        assert_eq!(note.check(&groups, true), Err("system_note"));
        assert_eq!(note.check::<&str>(&[], true), Err("system_note"));
        note.system_note = None;
        note.required = Some(false);
        assert_eq!(note.check(&groups, false), Err("required"));
        assert_eq!(
            note.check(
                &[capabilities::PLAN_NOTES, capabilities::REFRESH_RELAUNCH],
                false
            ),
            Ok(())
        );
        assert!(!crate::rungs::ALL.contains(&NOTES_UNTIL_REBUILD));
    }

    #[test]
    fn plan_notes_fixed_tool_strings_are_exact() {
        assert_eq!(tool_addition("lookup"), "Tool \"lookup\" is now available.");
        assert_eq!(
            tool_removal("lookup"),
            "Tool \"lookup\" is no longer available. Do not call it."
        );
    }
}
