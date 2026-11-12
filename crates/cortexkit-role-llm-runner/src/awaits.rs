//! Reader-facing waits, independent of a runner's reason vocabulary.

use serde::{Deserialize, Deserializer, Serialize};

use crate::run::RunReason;

macro_rules! open_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
        #[serde(from = "String", into = "String")]
        #[non_exhaustive]
        pub enum $name {
            $($variant,)+
            Unknown(String),
        }
        impl From<String> for $name {
            fn from(value: String) -> Self {
                match value.as_str() {
                    $($wire => Self::$variant,)+
                    _ => Self::Unknown(value),
                }
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                match value {
                    $($name::$variant => $wire.into(),)+
                    $name::Unknown(value) => value,
                }
            }
        }
    };
}

open_enum!(AwaitParty { Runner => "runner", Caller => "caller", Person => "person", Nobody => "nobody" });
open_enum!(AwaitAction { SignIn => "sign_in", Approve => "approve", Answer => "answer" });
open_enum!(AwaitMessageFrom { Host => "host", Provider => "provider", Plugin => "plugin", Runner => "runner" });

/// Who ends a wait, with only the facts the runner can observe. Unknown parties
/// mean no information; readers retain their existing behaviour.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunAwaits {
    pub party: AwaitParty,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<RunReason>,
    /// Unix-epoch milliseconds when the wait began.
    pub since_ms: u64,
    /// Expected automatic movement time. Omit when unknown or already past.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_ms: Option<u64>,
    /// Latest time before the runner gives up; equals the top-level
    /// `RunStatus.deadline_ms` when both fields are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "attempt"
    )]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<AwaitPerson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<AwaitMessage>,
}

impl RunAwaits {
    pub fn new(party: AwaitParty, since_ms: u64) -> Self {
        Self {
            party,
            since_ms,
            reason: None,
            next_ms: None,
            deadline_ms: None,
            attempt: None,
            person: None,
            message: None,
        }
    }

    /// A false answer means malformed data, not a refusal. Also checks values
    /// constructed in memory; decoding refuses the same bounded-field violations.
    pub fn well_formed(&self) -> bool {
        self.attempt != Some(0)
            && self.person.as_ref().is_none_or(AwaitPerson::well_formed)
            && self.message.as_ref().is_none_or(AwaitMessage::well_formed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AwaitPerson {
    pub action: AwaitAction,
    /// Display label, not an account identifier or a credential (1–120 characters).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "label"
    )]
    pub on: Option<String>,
}

impl AwaitPerson {
    pub fn new(action: AwaitAction) -> Self {
        Self { action, on: None }
    }
    pub fn well_formed(&self) -> bool {
        self.on.as_ref().is_none_or(|on| bounded(on, 120))
    }
}

/// Cleaned, quoted information, never instructions or the person's own words.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AwaitMessage {
    #[serde(deserialize_with = "text")]
    pub text: String,
    pub from: AwaitMessageFrom,
}

impl AwaitMessage {
    pub fn new(text: impl Into<String>, from: AwaitMessageFrom) -> Self {
        Self {
            text: text.into(),
            from,
        }
    }
    pub fn well_formed(&self) -> bool {
        bounded(&self.text, 300)
    }
}

fn bounded(value: &str, max: usize) -> bool {
    (1..=max).contains(&value.chars().count())
}

/// Check a declaring runner's ending. Unknown lifecycle states remain open;
/// unknown parties are no information, so readers keep their existing behaviour.
pub fn ending_consistent(
    state: &str,
    error: Option<&crate::run::RunError>,
    wait: Option<&RunAwaits>,
) -> bool {
    use crate::run::states;
    if state == states::ERROR {
        return match (error, wait) {
            (Some(_), Some(wait)) if matches!(wait.party, AwaitParty::Unknown(_)) => {
                wait.well_formed()
            }
            (Some(error), Some(wait)) => {
                wait.well_formed()
                    && wait.party == crate::errors::expected_party(error)
                    && (wait.party != AwaitParty::Person
                        || wait
                            .person
                            .as_ref()
                            .is_some_and(|p| p.action == AwaitAction::SignIn))
                    && (!crate::errors::is_account_wait(&error.provider_code)
                        || wait.next_ms.is_none()
                        || wait.next_ms
                            == crate::errors::account_wait_next_ms(error, wait.since_ms))
            }
            _ => false,
        };
    }
    if crate::run::RunState::parse(state).is_terminal()
        || matches!(state, states::ACTIVE | states::PAUSED)
    {
        wait.is_none()
    } else {
        wait.is_none_or(RunAwaits::well_formed)
    }
}

/// Check a declaring runner's status/head/control wait without interpreting
/// unknown reason strings. Compare the wait with the top-level `reason`,
/// `deadline_ms` and `resume_not_before_ms` fields, which remain beside it.
pub fn status_consistent(
    state: &str,
    reason: Option<&RunReason>,
    deadline_ms: Option<u64>,
    resume_not_before_ms: Option<u64>,
    error: Option<&crate::run::RunError>,
    wait: Option<&RunAwaits>,
) -> bool {
    if !matches!(state, "active" | "paused") {
        return ending_consistent(state, error, wait);
    }
    let Some(wait) = wait else {
        return true;
    };
    wait.well_formed()
        && (wait.party != AwaitParty::Runner
            || wait.deadline_ms.is_none()
            || deadline_ms.is_none()
            || wait.deadline_ms == deadline_ms)
        && (reason != Some(&RunReason::AccountLimit)
            || wait.party != AwaitParty::Runner
            || wait.next_ms.is_none()
            || wait.next_ms == resume_not_before_ms)
}
fn attempt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let value = Option::<u32>::deserialize(d)?;
    if value == Some(0) {
        return Err(serde::de::Error::custom("attempt must be at least 1"));
    }
    Ok(value)
}
fn label<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(d)?;
    if value.as_ref().is_some_and(|v| !bounded(v, 120)) {
        return Err(serde::de::Error::custom(
            "person.on must be 1..=120 characters",
        ));
    }
    Ok(value)
}
fn text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    if !bounded(&value, 300) {
        return Err(serde::de::Error::custom(
            "message.text must be 1..=300 characters",
        ));
    }
    Ok(value)
}

/// Conservatively prepare untrusted wait text for quotation. Drop a whole
/// message with credential markers, addresses or opaque identifiers rather
/// than risk disclosing part of a secret. Remove URL tokens and controls,
/// normalize whitespace and cap the result at 300 Unicode characters.
/// This is not a wait recognizer: callers must first recognize the whole
/// message under its error type, and must not use the cleaned text as a code.
pub fn clean_await_message(input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    if [
        "secret",
        "password",
        "token",
        "bearer",
        "authorization",
        "api_key",
        "api-key",
        "sk-",
        "key=",
        "akia",
        "ghp_",
        "github_pat_",
        "xoxb-",
        "-----begin",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return None;
    }
    let plain: String = input
        .chars()
        .map(|c| if c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}') { ' ' } else { c })
        .collect();
    let mut words = Vec::new();
    for word in plain.split_whitespace() {
        let lower = word.to_lowercase();
        if lower.contains("://")
            || lower.contains("www.")
            || lower.starts_with("mailto:")
            || lower.starts_with("data:")
        {
            continue;
        }
        if word.contains('@')
            || word.chars().filter(|c| c.is_alphanumeric()).count() >= 32
            || (word.len() >= 16
                && word.chars().any(|c| c.is_ascii_digit())
                && word.chars().any(|c| c.is_ascii_alphabetic()))
        {
            return None;
        }
        if word.split_once('.').is_some_and(|(a, b)| {
            a.chars().last().is_some_and(char::is_alphanumeric)
                && b.chars().next().is_some_and(char::is_alphabetic)
        }) {
            continue;
        }
        words.push(word);
    }
    let result: String = words.join(" ").chars().take(300).collect();
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_awaits_vectors_cover_every_surface() {
        use crate::{
            live::RunEnded,
            read::LastRunState,
            run::{RunResult, RunStatus},
            subscribe::SubscribeEvent,
            vectors,
        };
        let file = vectors::load("run-awaits.json");
        for case in vectors::cases(&file, "cases") {
            let name = case["name"].as_str().unwrap();
            let status: RunStatus = vectors::round_trip(name, &case["status"]);
            let head: LastRunState = vectors::round_trip(name, &case["head"]);
            let event: SubscribeEvent = vectors::round_trip(name, &case["subscribe"]);
            assert_eq!(status.awaits, head.awaits, "{name}");
            assert_eq!(status.awaits, event.awaits().unwrap(), "{name}");
            assert!(status.awaits_consistent(), "{name}");
            let absent: SubscribeEvent = serde_json::from_value(json!({"kind":"control"})).unwrap();
            assert_eq!(absent.awaits().unwrap(), None);
            let null: SubscribeEvent =
                serde_json::from_value(json!({"kind":"control","awaits":null})).unwrap();
            assert_eq!(null.awaits().unwrap(), None);
            if let Some(wait) = &status.awaits {
                assert_eq!(
                    absent.with_awaits(wait.clone()).awaits().unwrap().as_ref(),
                    Some(wait)
                );
            }
            if case.get("result").is_some() {
                let result: RunResult = vectors::round_trip(name, &case["result"]);
                assert_eq!(status.awaits, result.awaits);
                assert!(result.awaits_consistent(), "{name}");
            }
            if case.get("run_paused").is_some() {
                let paused: RunEnded = vectors::round_trip(name, &case["run_paused"]);
                assert_eq!(status.awaits, paused.awaits, "{name}");
            }
        }
        for case in vectors::cases(&file, "malformed") {
            let wait = &case["awaits"];
            let name = case["name"].as_str().unwrap();
            assert!(
                serde_json::from_value::<RunAwaits>(wait.clone()).is_err(),
                "{name}"
            );
            assert!(
                serde_json::from_value::<RunStatus>(json!({"state":"active","awaits":wait}))
                    .is_err(),
                "{name}"
            );
            assert!(
                serde_json::from_value::<LastRunState>(
                    json!({"run_id":"r","state":"active","awaits":wait})
                )
                .is_err(),
                "{name}"
            );
            assert!(
                serde_json::from_value::<RunResult>(
                    json!({"run_id":"r","state":"error","awaits":wait})
                )
                .is_err(),
                "{name}"
            );
            assert!(
                serde_json::from_value::<RunEnded>(
                    json!({"run_id":"r","reason":"auth_required","awaits":wait})
                )
                .is_err(),
                "{name}"
            );
            let event: SubscribeEvent =
                serde_json::from_value(json!({"kind":"control","awaits":wait})).unwrap();
            assert!(event.awaits().is_err(), "{name}");
        }
    }

    #[test]
    fn ending_mapping_and_consistency_are_independent_of_diagnostics() {
        use crate::{
            errors,
            run::{RunError, RunResult, RunStatus},
        };
        for (code, party) in [
            ("account_limit_exhausted", AwaitParty::Caller),
            ("account_selection_refused", AwaitParty::Caller),
            ("transient", AwaitParty::Caller),
            ("auth_required", AwaitParty::Person),
            ("request_invalid", AwaitParty::Nobody),
            ("unknown_code", AwaitParty::Nobody),
        ] {
            let error = RunError::new("auth_required", "transient", code)
                .with_retry_after_secs(1.5)
                .with_resets_at_ms(9999)
                .with_status(429);
            let wait = errors::ending_awaits(&error, 1000);
            assert_eq!(wait.party, party, "{code}");
            assert_eq!(wait.next_ms, errors::is_account_wait(code).then_some(2500));
            let mut result = RunResult::new("r", "error")
                .with_error(error)
                .with_awaits(wait);
            assert!(result.awaits_consistent(), "{code}");
            result.awaits.as_mut().unwrap().party = AwaitParty::Runner;
            assert!(!result.awaits_consistent(), "{code}");
        }
        let error =
            RunError::new("account", "limited", "account_limit_exhausted").with_resets_at_ms(3000);
        assert_eq!(errors::ending_awaits(&error, 1000).next_ms, Some(3000));
        assert_eq!(errors::ending_awaits(&error, 4000).next_ms, None);
        let future = RunAwaits::new(AwaitParty::Unknown("future_party".into()), 1000);
        assert!(RunResult::new("r", "error")
            .with_error(error.clone())
            .with_awaits(future.clone())
            .awaits_consistent());
        assert!(RunResult::new("r", "future_state")
            .with_awaits(future)
            .awaits_consistent());
        let mut wait = RunAwaits::new(AwaitParty::Runner, 1);
        wait.deadline_ms = Some(100);
        let mut status = RunStatus::new("active")
            .with_deadline_ms(100)
            .with_awaits(wait);
        assert!(status.awaits_consistent());
        status.deadline_ms = Some(101);
        assert!(!status.awaits_consistent());
        for state in [
            "completed",
            "cancelled",
            "interrupted",
            "context_ceiling",
            "max_steps",
            "budget_exhausted",
            "active",
            "paused",
        ] {
            assert!(!RunResult::new("r", state)
                .with_awaits(RunAwaits::new(AwaitParty::Nobody, 1))
                .awaits_consistent());
        }
        for reason in [
            "tool_held",
            "provider_retry",
            "account_free",
            "rate_limited",
            "server_busy",
            "quota_reading",
            "routing_hold",
        ] {
            let decoded: RunReason = serde_json::from_value(json!(reason)).unwrap();
            assert!(!matches!(decoded, RunReason::Unknown(_)));
            assert_eq!(String::from(decoded), reason);
        }
    }

    #[test]
    fn persisting_account_pause_can_omit_a_past_floor_without_reading_a_clock() {
        use crate::run::RunStatus;
        let mut wait = RunAwaits::new(AwaitParty::Runner, 1000);
        wait.reason = Some(RunReason::AccountLimit);
        let mut status = RunStatus::new("paused")
            .with_reason(RunReason::AccountLimit)
            .with_resume_not_before_ms(2000)
            .with_awaits(wait);
        assert!(status.awaits_consistent());
        status.awaits.as_mut().unwrap().next_ms = Some(3000);
        assert!(!status.awaits_consistent());
        status.resume_not_before_ms = Some(3000);
        assert!(status.awaits_consistent());
    }

    #[test]
    fn bounds_and_open_values() {
        let value = json!({"party":"future", "since_ms":10, "person":{"action":"future_action","on":"work"}});
        let wait: RunAwaits = serde_json::from_value(value.clone()).unwrap();
        assert!(wait.well_formed());
        assert_eq!(serde_json::to_value(wait).unwrap(), value);
        for extra in [
            json!({"attempt":0}),
            json!({"message":{"text":"x".repeat(301),"from":"host"}}),
            json!({"person":{"action":"answer","on":"x".repeat(121)}}),
            json!({"message":{"text":"","from":"host"}}),
            json!({"person":{"action":"answer","on":""}}),
        ] {
            let mut body = json!({"party":"runner","since_ms":10});
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(serde_json::from_value::<RunAwaits>(body).is_err());
        }
        let boundary = json!({"party":"runner","since_ms":10,"attempt":1,
            "person":{"action":"answer","on":"é".repeat(120)},
            "message":{"text":"é".repeat(300),"from":"future"}});
        assert!(serde_json::from_value::<RunAwaits>(boundary)
            .unwrap()
            .well_formed());
    }

    #[test]
    fn cleans_untrusted_messages() {
        assert_eq!(
            clean_await_message("Retry\nsoon\u{1b} https://example.com/private?q=1"),
            Some("Retry soon".into())
        );
        assert_eq!(
            clean_await_message("see www.example.com"),
            Some("see".into())
        );
        for value in [
            "Bearer abc",
            "api_key=abc",
            "password: abc",
            "sk-1234",
            "contact me@example.com",
            "abc012345678901234567890123456789012345",
            "\n\0",
            "https://example.com",
            "AKIA1234567890ABCDEF",
            "ghp_abcdef",
            "abcdef0123456789abcd",
        ] {
            assert_eq!(clean_await_message(value), None, "{value}");
        }
        assert_eq!(
            clean_await_message(&"é ".repeat(301))
                .unwrap()
                .chars()
                .count(),
            300
        );
        assert_eq!(
            clean_await_message("Retry in 2 seconds."),
            Some("Retry in 2 seconds.".into())
        );
        assert_eq!(
            clean_await_message("See example.com/help or mailto:a@example.com"),
            Some("See or".into())
        );
        assert_eq!(
            clean_await_message("Retry\u{202e} soon"),
            Some("Retry soon".into())
        );
    }
}
