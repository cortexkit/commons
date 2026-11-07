use super::*;
use crate::{
    fork_fake::{Defects, FakeSubject, ForkDefect},
    CASES,
};
use std::collections::BTreeMap;

const CONTRACT: &str = include_str!("../../../cortexkit-role-llm-runner/CONTRACT.md");
const MAP: &str = include_str!("../../rule-map.json");
const OUTSIDE_SECTION: &[&str] = &[
    "fork_answer.describe_missing",
    "fork_answer.describe_undeclared",
    "fork_answer.describe_invalid",
    "fork_answer.code_unknown_fork",
    "fork_answer.code_fork_ended",
    "fork_answer.code_fork_busy",
    "fork_answer.code_ask_id_reuse",
    "fork_answer.code_fork_base_unavailable",
    "fork_answer.code_fork_unsupported",
    "fork_answer.code_fork_account_held",
    "fork_answer.code_fork_account_blocked",
    "fork_answer.retryable",
];

fn validate_map(contract: &str, map: &Value) -> Check {
    let map = map.as_object().ok_or("rule map is not an object")?;
    let mut ids = BTreeSet::new();
    for line in contract.lines() {
        if let Some(rest) = line.strip_prefix("- [pinned] `fork_answer.") {
            let (tail, _) = rest.split_once("`:").ok_or("invalid pinned marker")?;
            let id = format!("fork_answer.{tail}");
            if !ids.insert(id) {
                return Err("duplicate pinned id".into());
            }
        }
    }
    let keys: BTreeSet<_> = map
        .keys()
        .filter(|id| id.starts_with("fork_answer."))
        .cloned()
        .collect();
    if keys != ids {
        return Err("mapped and pinned id sets differ".into());
    }
    let section = contract
        .split("### 9.20 `fork_answer`")
        .nth(1)
        .ok_or("no fork section")?
        .split("## 10.")
        .next()
        .unwrap();
    for line in section.lines().filter(|l| l.starts_with("- [pinned]")) {
        if !line.starts_with("- [pinned] `fork_answer.") || !line.contains("`:") {
            return Err("fork rule has no id".into());
        }
    }
    for id in OUTSIDE_SECTION {
        if !ids.contains(*id) {
            return Err(format!("missing additional rule {id}"));
        }
    }
    let mut unchecked = BTreeMap::new();
    for (id, entry) in map {
        if !entry["bug_shaped"].is_boolean() {
            return Err(format!("{id} has no bug_shaped flag"));
        }
        match (entry.get("case"), entry.get("not_kit_checked")) {
            (Some(case), None) => {
                let name = case.as_str().ok_or("case is not a string")?;
                if !CASES.iter().any(|c| c.name == name) {
                    return Err(format!("unknown mapped case {name}"));
                }
            }
            (None, Some(reason)) => {
                let reason = reason.as_str().ok_or("reason is not a string")?;
                if !matches!(
                    reason,
                    "caller_side" | "not_observable" | "informative_dependency"
                ) {
                    return Err("unknown not-kit-checked reason".into());
                }
                unchecked.insert(id.clone(), reason.to_owned());
            }
            _ => return Err("entry must map exactly one case or exception".into()),
        }
    }
    let mut listed = BTreeMap::new();
    for tail in contract
        .split("The following rule is not kit-checked: `")
        .skip(1)
    {
        let (id, tail) = tail.split_once('`').ok_or("bad unchecked id")?;
        let reason = tail.split('`').nth(1).ok_or("bad unchecked reason")?;
        listed.insert(id.to_owned(), reason.to_owned());
    }
    if unchecked != listed {
        return Err("map and contract not-kit-checked lists differ".into());
    }
    Ok(())
}

#[test]
fn fork_answer_rule_map_matches_every_pinned_rule() {
    let map: Value = serde_json::from_str(MAP).unwrap();
    validate_map(CONTRACT, &map).unwrap();
    let mut missing = map.clone();
    missing.as_object_mut().unwrap().remove("fork_answer.base");
    assert!(validate_map(CONTRACT, &missing).is_err());
    let mut extra = map.clone();
    extra["fork_answer.not_a_rule"] =
        json!({"case":"fork_answer_base_matches_digest","bug_shaped":false});
    assert!(validate_map(CONTRACT, &extra).is_err());
    let mut reason = map.clone();
    reason["fork_answer.compaction_isolation"]["not_kit_checked"] = json!("caller_side");
    assert!(validate_map(CONTRACT, &reason).is_err());
    let no_id = CONTRACT.replace("- [pinned] `fork_answer.base`:", "- [pinned] Base:");
    assert!(validate_map(&no_id, &map).is_err());
    let informative = CONTRACT.replace(
        "- [pinned] `fork_answer.base`:",
        "- [informative] `fork_answer.base`:",
    );
    assert!(validate_map(&informative, &map).is_err());
}

async fn case_result(name: &str, defect: ForkDefect) -> Check {
    let mut fake = FakeSubject::new(Defects {
        fork: defect,
        ..Defects::default()
    });
    fake.capabilities.extend([
        Capability::ForkAnswer,
        Capability::SessionChange,
        Capability::RoutedAccounts,
        Capability::Streaming,
        Capability::TranscriptReads,
        Capability::Queue,
        Capability::RunOps,
        Capability::RenderedModelInput,
        Capability::ServiceTier,
    ]);
    let root = tempfile::tempdir().unwrap();
    let declared = fake.capabilities().into_iter().collect();
    let mint = Mint::default();
    let mut driver = CrashDriver::new(&fake);
    let result = if matches!(
        name,
        "fork_answer_ask_id_replays" | "fork_answer_survives_restart"
    ) {
        restart(&fake, &mut driver, root.path(), &declared, &mint, name).await
    } else {
        let handle = driver.spawn(&root.path().join("main")).await.unwrap();
        run(
            &Case {
                subject: &fake,
                handle: &handle,
                declared: &declared,
                mint: &mint,
            },
            name,
        )
        .await
    };
    match result {
        Ok(Ending::Passed) => Ok(()),
        Ok(_) => Err(format!("{name}: unexpectedly skipped or inapplicable")),
        Err(reason) => Err(format!("{name}: {reason}")),
    }
}

fn defect(name: &str) -> ForkDefect {
    match name {
        "fork_answer_base_matches_digest" => ForkDefect::BaseBytes,
        "fork_answer_refuses_mismatched_base" => ForkDefect::Base,
        "fork_answer_refuses_tool_call" => ForkDefect::Tools,
        "fork_answer_model_failure_finishes" => ForkDefect::Failure,
        "fork_answer_ask_id_replays" => ForkDefect::Replay,
        "fork_answer_idle_expiry" => ForkDefect::Expiry,
        "fork_answer_never_writes_head" => ForkDefect::Head,
        "fork_answer_admits_only_fork_callers" => ForkDefect::Callers,
        "fork_answer_account_for_pins_head_credential" => ForkDefect::Account,
        "fork_answer_account_held_refusal" => ForkDefect::HeldMapping,
        "fork_answer_account_blocked_refusal" => ForkDefect::BlockedMapping,
        "fork_answer_conversation_key_bounded" => ForkDefect::Key,
        "fork_answer_survives_restart" => ForkDefect::Restart,
        _ => panic!("no faulty fake for {name}"),
    }
}

#[tokio::test]
async fn fork_answer_rule_map_meta_test_rejects_faulty_fakes_by_name() {
    let map: Value = serde_json::from_str(MAP).unwrap();
    let names: BTreeSet<_> = map
        .as_object()
        .unwrap()
        .iter()
        .filter(|(id, _)| id.starts_with("fork_answer."))
        .filter_map(|(_, entry)| entry["case"].as_str())
        .collect();
    for name in names {
        case_result(name, ForkDefect::None).await.unwrap();
        let failure = case_result(name, defect(name)).await.expect_err(name);
        assert!(failure.starts_with(&format!("{name}:")), "{failure}");
    }
    let bugs: BTreeSet<_> = map
        .as_object()
        .unwrap()
        .iter()
        .filter(|(id, v)| id.starts_with("fork_answer.") && v["bug_shaped"] == true)
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(
        bugs,
        BTreeSet::from([
            "fork_answer.base",
            "fork_answer.no_tools",
            "fork_answer.account_pin"
        ])
    );
}

macro_rules! conforming_case {
    ($($name:ident),* $(,)?)=>{$(
        #[tokio::test]
        async fn $name() { case_result(stringify!($name),ForkDefect::None).await.unwrap(); }
    )*};
}
conforming_case!(
    fork_answer_base_matches_digest,
    fork_answer_refuses_mismatched_base,
    fork_answer_refuses_tool_call,
    fork_answer_model_failure_finishes,
    fork_answer_ask_id_replays,
    fork_answer_idle_expiry,
    fork_answer_never_writes_head,
    fork_answer_admits_only_fork_callers,
    fork_answer_account_for_pins_head_credential,
    fork_answer_conversation_key_bounded,
    fork_answer_survives_restart,
    fork_answer_account_held_refusal,
    fork_answer_account_blocked_refusal,
);

#[tokio::test]
async fn fork_answer_base_allows_only_cache_marker_relocation() {
    case_result(
        "fork_answer_base_matches_digest",
        ForkDefect::RelocatedMarker,
    )
    .await
    .unwrap();
    let error = case_result("fork_answer_base_matches_digest", ForkDefect::BaseBytes)
        .await
        .unwrap_err();
    assert!(
        error.starts_with("fork_answer_base_matches_digest:"),
        "{error}"
    );
    assert!(error.contains("first differing byte offset"), "{error}");
    assert!(error.contains("head") && error.contains("fork"), "{error}");
}

#[tokio::test]
async fn fork_answer_optional_hooks_are_inapplicable_not_passed() {
    for (name, hook) in [
        (
            "fork_answer_conversation_key_bounded",
            "configure_fork_head_key",
        ),
        (
            "fork_answer_account_held_refusal",
            "routing_peer_error_replies",
        ),
        (
            "fork_answer_account_blocked_refusal",
            "routing_peer_error_replies",
        ),
    ] {
        let mut fake = FakeSubject::new(Defects::default());
        fake.fork_peer_errors = false;
        fake.fork_head_key_fixture = false;
        fake.capabilities
            .extend([Capability::ForkAnswer, Capability::RoutedAccounts]);
        let root = tempfile::tempdir().unwrap();
        let mut driver = CrashDriver::new(&fake);
        let handle = driver.spawn(root.path()).await.unwrap();
        let mint = Mint::default();
        let declared = fake.capabilities().into_iter().collect();
        match run(
            &Case {
                subject: &fake,
                handle: &handle,
                declared: &declared,
                mint: &mint,
            },
            name,
        )
        .await
        {
            Ok(Ending::Inapplicable(reason)) => assert!(reason.contains(hook), "{reason}"),
            _ => panic!("{name} did not name its unsupported hook"),
        }
    }
}
