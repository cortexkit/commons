use cortexkit_agent_identity::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use sha2::{Digest, Sha256};

const PROJECT: &str = include_str!("../test-vectors/memory-context-v1/context-project.json");
const SESSION: &str = include_str!("../test-vectors/memory-context-v1/context-session.json");
const POPULATED: &str =
    include_str!("../test-vectors/memory-context-v1/context-session-populated.json");
const WHERE: &str = include_str!("../test-vectors/agent-sessions-v1/where.json");
const MANY: &str = include_str!("../test-vectors/agent-sessions-v1/where-many.json");
const SOURCE: &str = include_str!("../test-vectors/agent-sessions-v1/where-moving-source.json");
const RECEIVER: &str = include_str!("../test-vectors/agent-sessions-v1/where-moving-receiver.json");
const SHARING_OFF: &str = include_str!("../test-vectors/agent-sessions-v1/where-sharing-off.json");
const RESOLVE: &str = include_str!("../test-vectors/agent-identity-reads-v1/agent-resolve.json");
const RESOLVE_MERGED: &str =
    include_str!("../test-vectors/agent-identity-reads-v1/agent-resolve-merged.json");

#[derive(Deserialize)]
struct Record<'a> {
    op: Option<String>,
    request: Value,
    #[serde(borrow)]
    reply: &'a RawValue,
}

// Sort explicitly: a downstream serde_json feature may enable insertion-order maps.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut pairs: Vec<_> = map.into_iter().collect();
            pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
            Value::Object(pairs.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        value => value,
    }
}

fn check_reply<T: DeserializeOwned + Serialize>(raw: &str) -> Outcome<T> {
    let reply: Outcome<T> = serde_json::from_str(raw).unwrap();
    let encoded = serde_json::to_string(&sorted(serde_json::to_value(&reply).unwrap())).unwrap();
    assert_eq!(encoded.as_bytes(), raw.as_bytes());
    reply
}

fn sample<P: DeserializeOwned + Serialize, T: DeserializeOwned + Serialize>(
    bytes: &str,
    index: usize,
    method: &str,
) -> Outcome<T> {
    let records: Vec<Record<'_>> = serde_json::from_str(bytes).unwrap();
    let record = &records[index];
    if let Some(op) = &record.op {
        assert_eq!(op, method);
        let params: P = serde_json::from_value(record.request.clone()).unwrap();
        assert_eq!(serde_json::to_value(params).unwrap(), record.request);
    } else {
        let request: Request<P> = serde_json::from_value(record.request.clone()).unwrap();
        assert_eq!(request.method, method);
        assert_eq!(serde_json::to_value(request).unwrap(), record.request);
    }
    check_reply(record.reply.get())
}

fn success<T>(reply: Outcome<T>) -> T {
    match reply {
        Outcome::Success(reply) => reply.result,
        _ => panic!("expected success"),
    }
}

fn moving(reply: &AgentWhere) -> &AgentMoving {
    match &reply.moving {
        Presence::Present(value) => value,
        _ => panic!("expected moving"),
    }
}

fn project(index: usize) -> ContextProjectReply {
    success(sample::<ContextProjectParams, _>(
        PROJECT,
        index,
        "context.project",
    ))
}

fn session(bytes: &str, index: usize) -> ContextSessionReply {
    let reply: ContextSessionReply = success(sample::<ContextSessionParams, _>(
        bytes,
        index,
        "context.session",
    ));
    assert_eq!(reply.agent.status, AgentStatus::Live);
    assert!(reply.agent.gone.is_none());
    assert!(reply.agent.merged_into.is_none());
    assert_eq!(reply.agent.generation, reply.stamp.generation);
    reply
}

fn whereabouts(bytes: &str, index: usize) -> AgentWhere {
    success(sample::<AgentWhereParams, _>(bytes, index, "agent.where"))
}

#[test]
fn project_before_document() {
    let reply = project(0);
    assert_eq!(reply.via, Via::Root);
    assert!(reply.document.is_none());
    assert_eq!(reply.domain.as_deref(), Some("@example/software"));
}

#[test]
fn project_with_document() {
    let reply = project(1);
    let document = reply.document.unwrap();
    assert_eq!(document.revision, 2);
    assert_eq!(document.charter_bytes, 17);
    assert_eq!(document.document_bytes, 16);
    assert!(reply.members_digest.is_some());
}

#[test]
fn project_without_workspace() {
    let reply = project(2);
    assert_eq!(reply.project_id.as_deref(), Some("agentless"));
    assert!(reply.workspace_id.is_none());
    assert!(reply.domain.is_none());
    assert!(reply.members_digest.is_none());
}

#[test]
fn project_implicit() {
    let reply = project(3);
    assert_eq!(reply.via, Via::Implicit);
    assert!(reply.project_id.is_none());
}

#[test]
fn project_worktree() {
    let reply = project(4);
    assert_eq!(reply.via, Via::Worktree);
    assert_eq!(reply.document.unwrap().revision, 2);
}

#[test]
fn session_pin_without_status() {
    let reply = session(SESSION, 0);
    assert_eq!(reply.pins.len(), 1);
    assert_eq!(reply.pins[0].kind, PinKind::MemoryProfile);
    assert!(reply.pins[0].status.is_none());
}

#[test]
fn session_without_pins() {
    let reply = session(SESSION, 1);
    assert!(reply.pins.is_empty());
    assert!(reply.catalog_generations.is_empty());
    assert_eq!(reply.whereabouts.status, WhereStatus::Unclaimed);
}

#[test]
fn session_revoked_pin() {
    let reply = session(SESSION, 2);
    let status = reply.pins[0].status.as_ref().unwrap();
    assert_eq!(status.state, PinStatusState::Revoked);
    assert!(status.blocked);
}

#[test]
fn session_populated() {
    let reply = session(POPULATED, 0);
    assert_eq!(reply.catalog_generations.len(), 2);
    assert_eq!(
        reply.catalog_generations[0].state,
        CatalogGenerationState::Active
    );
    assert_eq!(
        reply.catalog_generations[1].state,
        CatalogGenerationState::Ready
    );
    let location = reply.whereabouts;
    assert_eq!(location.holder_name.as_deref(), Some("Juniper laptop"));
    assert_eq!(location.status, WhereStatus::HeldElsewhere);
    assert_eq!(location.cut_id.as_deref(), Some("cut-context-remote"));
    assert_eq!(location.read_at_ms, Some(700));
    assert_eq!(moving(&location).side, MoveSide::Source);
    assert_eq!(moving(&location).step, MoveStep::Releasing);
    assert!(moving(&location).waiting_ms.is_missing());
}

#[test]
fn session_retired_refusal() {
    match sample::<ContextSessionParams, ContextSessionReply>(POPULATED, 1, "context.session") {
        Outcome::Refusal(error) => {
            assert_eq!(error.code, RefusalCode::new("gone"));
            let gone: AgentGone = serde_json::from_value(error.detail.unwrap()).unwrap();
            assert_eq!(gone.reason, GoneReason::Deleted);
            assert_eq!(gone.at, 10);
            assert!(gone.into_agent_id.is_missing());
        }
        _ => panic!("expected refusal"),
    }
}

#[test]
fn where_unclaimed() {
    let reply = whereabouts(WHERE, 0);
    assert_eq!(reply.status, WhereStatus::Unclaimed);
    assert!(!reply.known);
    assert!(reply.stale);
}

#[test]
fn where_held_here() {
    let reply = whereabouts(WHERE, 1);
    assert_eq!(reply.status, WhereStatus::HeldHere);
    assert_eq!(reply.holder_name.as_deref(), Some("This machine"));
}

#[test]
fn where_held_elsewhere() {
    let reply = whereabouts(WHERE, 2);
    assert_eq!(reply.status, WhereStatus::HeldElsewhere);
    assert_eq!(reply.holder_name.as_deref(), Some("Other machine"));
}

#[test]
fn where_unknown_refusal() {
    match sample::<AgentWhereParams, AgentWhere>(WHERE, 3, "agent.where") {
        Outcome::Refusal(error) => assert_eq!(error.code, RefusalCode::new("unknown_agent")),
        _ => panic!("expected refusal"),
    }
}

#[test]
fn where_sharing_off() {
    let reply = whereabouts(SHARING_OFF, 0);
    assert!(reply.known);
    assert!(!reply.stale);
    assert_eq!(reply.status, WhereStatus::Unclaimed);
    assert_eq!(reply.epoch, 0);
    assert!(reply.read_at_ms.is_none());
    assert!(reply.holder.is_none());
}

#[test]
fn where_moving_source() {
    let reply = whereabouts(SOURCE, 0);
    let movement = moving(&reply);
    assert_eq!(movement.side, MoveSide::Source);
    assert_eq!(movement.step, MoveStep::Published);
    assert_eq!(movement.status, Presence::new(MoveStatus::WaitingForTarget));
    assert_eq!(movement.waiting_since_ms, Presence::new(650));
    assert_eq!(movement.waiting_ms, Presence::new(50));
    assert!(reply.cut_id.is_none());
}

#[test]
fn where_moving_receiver() {
    let reply = whereabouts(RECEIVER, 0);
    assert_eq!(moving(&reply).side, MoveSide::Receiver);
    assert_eq!(moving(&reply).step, MoveStep::Claimed);
    assert_eq!(reply.cut_id.as_deref(), Some("cut-moving-agent"));
    assert!(moving(&reply).waiting_ms.is_missing());
    assert!(moving(&reply).status.is_missing());
}

#[test]
fn where_many_preserves_order_skips_unknown() {
    let reply: AgentWhereManyReply = success(sample::<AgentWhereManyParams, _>(
        MANY,
        0,
        "agent.where_many",
    ));
    assert_eq!(reply.agents.len(), 2);
    assert_eq!(reply.agents[0].agent_id, "agent_0000000000000107");
    assert_eq!(reply.agents[1].agent_id, "agent_0000000000000106");
    assert_eq!(
        reply.agents[0].whereabouts.status,
        WhereStatus::HeldElsewhere
    );
    assert_eq!(reply.agents[1].whereabouts.status, WhereStatus::HeldHere);
}

#[test]
fn resolve_live_head() {
    let reply = resolution(0);
    assert_eq!(reply.agent.agent_id, "agent_0000000000000001");
    assert_eq!(reply.agent.status, AgentStatus::Live);
    assert!(reply.agent.gone.is_none());
    assert!(reply.agent.merged_into.is_none());
}

#[test]
fn resolve_live_hire() {
    let reply = resolution(1);
    assert_eq!(reply.agent.agent_id, "agent_0000000000000005");
    assert_eq!(reply.agent.status, AgentStatus::Live);
    assert!(reply.agent.gone.is_none());
    assert!(reply.agent.merged_into.is_none());
}

#[test]
fn resolve_retired_success() {
    let reply = resolution(2);
    assert_eq!(reply.agent.status, AgentStatus::Retired);
    assert!(reply.agent.merged_into.is_none());
    let gone = reply.agent.gone.unwrap();
    assert_eq!(gone.at, 10);
    assert_eq!(gone.reason, GoneReason::Deleted);
    assert!(gone.into_agent_id.is_missing());
}

#[test]
fn resolve_unknown_success() {
    let reply = resolution(3);
    assert_eq!(reply.agent.agent_id, "agent_ffffffffffffffff");
    assert_eq!(reply.agent.status, AgentStatus::Unknown);
    assert!(reply.agent.gone.is_none());
    assert!(reply.agent.merged_into.is_none());
}

#[test]
fn resolve_merged_success() {
    let reply = resolution_from(RESOLVE_MERGED, 0, 5);
    assert_eq!(reply.agent.agent_id, "agent_0000000000000601");
    assert_eq!(reply.agent.status, AgentStatus::Merged);
    assert_eq!(
        reply.agent.merged_into.as_deref(),
        Some("agent_0000000000000602")
    );
    let gone = reply.agent.gone.unwrap();
    assert_eq!(gone.at, 700);
    assert_eq!(gone.reason, GoneReason::Merged);
    assert_eq!(
        gone.into_agent_id,
        Presence::Present("agent_0000000000000602".to_owned())
    );
}

#[test]
fn resolve_merge_survivor_is_live() {
    let reply = resolution_from(RESOLVE_MERGED, 1, 5);
    assert_eq!(reply.agent.agent_id, "agent_0000000000000602");
    assert_eq!(reply.agent.status, AgentStatus::Live);
    assert!(reply.agent.gone.is_none());
    assert!(reply.agent.merged_into.is_none());
}

fn resolution(index: usize) -> AgentResolveReply {
    resolution_from(RESOLVE, index, 7)
}

fn resolution_from(source: &str, index: usize, generation: i64) -> AgentResolveReply {
    let reply: AgentResolveReply = success(sample::<AgentResolveParams, _>(
        source,
        index,
        "agent.resolve",
    ));
    let encoded = serde_json::to_value(&reply).unwrap();
    let mut keys: Vec<_> = encoded
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "agent_id",
            "generation",
            "gone",
            "incarnation",
            "merged_into",
            "status"
        ]
    );
    assert_eq!(reply.agent.generation, generation);
    assert_eq!(reply.incarnation, "0123456789abcdef");
    reply
}

// Malformed handler requests are fixture data, not valid typed params to round-trip.
fn resolve_refusal(index: usize) -> Value {
    let records: Vec<Record<'_>> = serde_json::from_str(RESOLVE).unwrap();
    let record = &records[index];
    assert_eq!(record.op.as_deref(), Some("agent.resolve"));
    match check_reply::<AgentResolveReply>(record.reply.get()) {
        Outcome::Refusal(error) => {
            assert_eq!(error.code, RefusalCode::InvalidRequest);
            assert!(error.detail.is_none());
        }
        _ => panic!("expected invalid_request refusal"),
    }
    record.request.clone()
}

#[test]
fn resolve_missing_agent_id_refusal() {
    let request = resolve_refusal(4);
    assert_eq!(request, serde_json::json!({}));
    assert!(serde_json::from_value::<AgentResolveParams>(request).is_err());
}

#[test]
fn resolve_non_string_agent_id_refusal() {
    let request = resolve_refusal(5);
    assert_eq!(request, serde_json::json!({"agent_id":42}));
    assert!(serde_json::from_value::<AgentResolveParams>(request).is_err());
}

#[test]
fn resolve_extra_param_refusal() {
    let request = resolve_refusal(6);
    assert_eq!(
        request,
        serde_json::json!({"agent_id":"agent_0000000000000001", "unknown":true})
    );
}

#[test]
fn vector_checksums() {
    let sets: [(&str, &[(&str, &str)]); 3] = [
        (
            include_str!("../test-vectors/agent-identity-reads-v1/SHA256SUMS"),
            &[
                ("agent-resolve-merged.json", RESOLVE_MERGED),
                ("agent-resolve.json", RESOLVE),
            ],
        ),
        (
            include_str!("../test-vectors/memory-context-v1/SHA256SUMS"),
            &[
                ("context-project.json", PROJECT),
                ("context-session-populated.json", POPULATED),
                ("context-session.json", SESSION),
            ],
        ),
        (
            include_str!("../test-vectors/agent-sessions-v1/SHA256SUMS"),
            &[
                ("where-many.json", MANY),
                ("where-moving-receiver.json", RECEIVER),
                ("where-moving-source.json", SOURCE),
                ("where-sharing-off.json", SHARING_OFF),
                ("where.json", WHERE),
            ],
        ),
    ];
    for (manifest, files) in sets {
        let expected = files
            .iter()
            .map(|(name, bytes)| format!("{:x}  {name}\n", Sha256::digest(bytes.as_bytes())))
            .collect::<String>();
        assert_eq!(manifest, expected);
    }
}
