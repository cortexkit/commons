//! Keeps the row's runner-step members in line with the runner step event
//! (`step.finished.v1` from `cortexkit-role-llm-runner`), so neither side can
//! rename or retype a shared member without this test going red.
//!
//! Members the row carries unchanged are checked by name, type and value. The
//! row's identity (`harness`, `step_ref`) is a rule rather than a rename, so it
//! is checked through `live_row_identity` against real events.

use std::collections::BTreeSet;

use cortexkit_role_llm_runner::step_events::StepFinished;
use cortexkit_usage_steps::{live_row_identity, StepRow};

/// An event with every member present, optional ones included.
fn full_event() -> StepFinished {
    serde_json::from_value(serde_json::json!({
        "runner": "example-runner",
        "session_ref": "sref_ex",
        "log_incarnation": "inc_ex",
        "seq": 7,
        "run_id": "run_ex",
        "step_id": "step_ex",
        "attempt": 1,
        "attempt_id": "att_ex",
        "terminal_status": "completed",
        "harness_step_ref": "hstep_ex",
        "provider_request_id": "preq_ex",
        "harness": "example-harness",
        "session": "ses_ex",
        "work_class": "interactive",
        "provider": "example-provider",
        "model": "example-model",
        "input_tokens": 11,
        "cached_input_tokens": 12,
        "cache_write_tokens": 13,
        "output_tokens": 14,
        "reasoning_tokens": 10,
        "cache_write_5m_tokens": 16,
        "cache_write_1h_tokens": 17,
        "reasoning_included_in_output": true,
        "request_body_bytes": 18,
        "context_limit": 19,
        "finish_reason": "stop",
        "started_at_ms": 20,
        "finished_at_ms": 21
    }))
    .unwrap()
}

fn member_names(value: serde_json::Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

/// Every event member, and what the row does with it. A new event member
/// fails here until someone decides whether the row carries it.
const EVENT_MEMBERS: &[&str] = &[
    "attempt",
    "attempt_id",
    "cache_write_1h_tokens",
    "cache_write_5m_tokens",
    "cache_write_tokens",
    "cached_input_tokens",
    "context_limit",
    "finish_reason",
    "finished_at_ms",
    "harness",
    "harness_step_ref",
    "input_tokens",
    "log_incarnation",
    "model",
    "output_tokens",
    "provider",
    "provider_request_id",
    "reasoning_included_in_output",
    "reasoning_tokens",
    "request_body_bytes",
    "run_id",
    "runner",
    "seq",
    "session",
    "session_ref",
    "started_at_ms",
    "step_id",
    "terminal_status",
    "work_class",
];

/// Members both shapes carry under the same name and with the same meaning:
/// the row carries the event's value unchanged.
const SAME_NAME: &[&str] = &["context_limit", "model", "provider", "run_id", "session"];

/// Members both shapes carry under the same name but not always with the same
/// meaning. The row's `harness` is the event's `harness` only when the event
/// carries a host record; otherwise it is the event's `runner`. See
/// `live_row_identity`.
const SAME_NAME_NOT_ALWAYS_SAME_MEANING: &[&str] = &["harness"];

/// Row member and the event member whose value it carries under another name.
const RENAMED: &[(&str, &str)] = &[
    ("input", "input_tokens"),
    ("cache_read", "cached_input_tokens"),
    ("cache_write", "cache_write_tokens"),
    ("cache_write_5m", "cache_write_5m_tokens"),
    ("cache_write_1h", "cache_write_1h_tokens"),
    ("output", "output_tokens"),
    ("reasoning", "reasoning_tokens"),
    ("request_body_len", "request_body_bytes"),
    ("finish", "finish_reason"),
    ("occurred_at_ms", "finished_at_ms"),
];

fn row_members() -> BTreeSet<String> {
    StepRow::REQUIRED_MEMBERS
        .iter()
        .chain(StepRow::OPTIONAL_MEMBERS)
        .map(|m| m.to_string())
        .collect()
}

#[test]
fn shared_member_names_match_the_runner_step_event() {
    let event = member_names(serde_json::to_value(full_event()).unwrap());
    let pinned: BTreeSet<String> = EVENT_MEMBERS.iter().map(|m| m.to_string()).collect();
    assert_eq!(event, pinned, "the runner step event's members changed");

    let row = row_members();
    let full_row: StepRow = serde_json::from_str(include_str!(
        "../test-vectors/usage-steps-v1/read/row-every-member.json"
    ))
    .unwrap();
    // Tier fields are row-only until a readers-first change to the closed bus
    // event. Preserve the legacy fixture and fill the newer members here.
    let full_row = full_row
        .with_service_tier_applied(
            cortexkit_role_llm_runner::service_tier::ServiceTier::Fast.as_str(),
        )
        .with_service_tier_served("fast");
    assert_eq!(member_names(serde_json::to_value(full_row).unwrap()), row);

    let shared: BTreeSet<String> = event.intersection(&row).cloned().collect();
    let same_meaning: BTreeSet<String> = SAME_NAME.iter().map(|m| m.to_string()).collect();
    let different_meaning: BTreeSet<String> = SAME_NAME_NOT_ALWAYS_SAME_MEANING
        .iter()
        .map(|m| m.to_string())
        .collect();
    assert!(
        same_meaning.is_disjoint(&different_meaning),
        "a member is listed both as carried unchanged and as meaning something else: {:?}",
        same_meaning
            .intersection(&different_meaning)
            .collect::<Vec<_>>()
    );
    let expected: BTreeSet<String> = same_meaning.union(&different_meaning).cloned().collect();
    assert_eq!(shared, expected, "the members shared by name changed");
    for (row_member, event_member) in RENAMED {
        assert!(row.contains(*row_member), "row lost {row_member}");
        assert!(event.contains(*event_member), "event lost {event_member}");
    }
    for member in &different_meaning {
        assert!(
            !RENAMED.iter().any(|(row_member, _)| row_member == member),
            "{member} comes from the identity rule, not from one event member"
        );
    }
}

fn identity_of(event: &StepFinished) -> Option<(String, String)> {
    live_row_identity(
        &event.runner,
        &event.harness,
        event.harness_step_ref.as_deref(),
        event.attempt_id.as_deref(),
    )
}

/// `full_event()` without its host record, at the given attempt number and
/// attempt id.
fn event_without_host_record(attempt: u64, attempt_id: &str) -> StepFinished {
    let mut value = serde_json::to_value(full_event()).unwrap();
    let members = value.as_object_mut().unwrap();
    members.remove("harness_step_ref");
    members.insert("attempt".to_owned(), attempt.into());
    members.insert("attempt_id".to_owned(), attempt_id.into());
    serde_json::from_value(value).unwrap()
}

#[test]
fn live_row_identity_uses_the_host_record_when_the_event_carries_one() {
    let event = full_event();
    assert_eq!(event.harness_step_ref.as_deref(), Some("hstep_ex"));
    assert_eq!(
        identity_of(&event),
        Some(("example-harness".to_owned(), "hstep_ex".to_owned()))
    );
}

#[test]
fn live_row_identity_uses_the_runner_and_attempt_id_without_a_host_record() {
    let event = event_without_host_record(1, "att_first");
    assert_eq!(event.harness_step_ref, None);
    assert_eq!(
        identity_of(&event),
        Some(("example-runner".to_owned(), "att_first".to_owned()))
    );
}

#[test]
fn live_row_identity_gives_each_attempt_of_a_step_its_own_row() {
    let first = identity_of(&event_without_host_record(1, "att_first"));
    let retry = identity_of(&event_without_host_record(2, "att_retry"));
    assert_ne!(first, retry, "two attempts of one step share an identity");
    assert_eq!(retry.unwrap().1, "att_retry");
}

/// After a session restore a runner can number a step and attempt the same
/// as an earlier one. Their attempt ids differ, so their rows must too.
#[test]
fn live_row_identity_keeps_restored_attempts_apart_when_the_triple_repeats() {
    let before = identity_of(&event_without_host_record(1, "att_before_restore"));
    let after = identity_of(&event_without_host_record(1, "att_after_restore"));
    assert_ne!(
        before, after,
        "a repeated run, step and attempt merged two attempts into one row"
    );
}

#[test]
fn live_row_identity_gives_none_without_host_record_or_attempt_id() {
    assert_eq!(
        live_row_identity("example-runner", "example-harness", None, None),
        None
    );
}

#[test]
fn shared_member_values_move_from_event_to_row_unchanged() {
    let event = serde_json::to_value(full_event()).unwrap();
    let mut row: serde_json::Value = serde_json::from_str(include_str!(
        "../test-vectors/usage-steps-v1/read/row-every-member.json"
    ))
    .unwrap();
    let pairs = SAME_NAME
        .iter()
        .map(|name| (*name, *name))
        .chain(RENAMED.iter().copied());
    for (row_member, event_member) in pairs.clone() {
        row[row_member] = event[event_member].clone();
    }
    let decoded: StepRow = serde_json::from_value(row).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    for (row_member, event_member) in pairs {
        assert_eq!(
            encoded[row_member], event[event_member],
            "{row_member} does not carry {event_member} unchanged"
        );
    }
}

/// Compiles only while each shared member has the same Rust type on both
/// sides (an event's required member is optional on the row).
#[test]
fn shared_member_types_match_at_compile_time() {
    fn same<T>(_: &T, _: &T) {}
    let event = full_event();
    let row: StepRow = serde_json::from_str(include_str!(
        "../test-vectors/usage-steps-v1/read/row-every-member.json"
    ))
    .unwrap();
    // Identity inputs (see live_row_identity): with `harness_step_ref`, the
    // row's `harness` is the event's `harness` and its `step_ref` is
    // `harness_step_ref`; without it, `harness` is the event's `runner` and
    // `step_ref` is `attempt_id`.
    same(&row.harness, &event.harness);
    same(&row.harness, &event.runner);
    same(&Some(row.step_ref.clone()), &event.harness_step_ref);
    same(&Some(row.step_ref.clone()), &event.attempt_id);
    same(&row.session, &event.session);
    same(&row.run_id, &Some(event.run_id.clone()));
    same(&row.provider, &Some(event.provider.clone()));
    same(&row.model, &Some(event.model.clone()));
    same(&row.context_limit, &event.context_limit);
    same(&row.input, &event.input_tokens);
    same(&row.cache_read, &event.cached_input_tokens);
    same(&row.cache_write, &event.cache_write_tokens);
    same(&row.cache_write_5m, &event.cache_write_5m_tokens);
    same(&row.cache_write_1h, &event.cache_write_1h_tokens);
    same(&row.output, &event.output_tokens);
    same(&row.reasoning, &event.reasoning_tokens);
    same(&row.request_body_len, &event.request_body_bytes);
    same(&row.finish, &event.finish_reason);
    same(&row.occurred_at_ms, &event.finished_at_ms);
}
