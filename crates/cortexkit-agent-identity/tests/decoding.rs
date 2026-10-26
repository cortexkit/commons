use cortexkit_agent_identity::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn round_trip<T: DeserializeOwned + Serialize>(value: Value) -> T {
    let decoded: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), value);
    decoded
}

#[test]
fn resolve_hand_built_round_trip() {
    // The merged case remains unsampled; these constructed values are not handler captures.
    let request = Request::new(
        "agent.resolve",
        AgentResolveParams::new("agent_12345678".into()),
    );
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "method": "agent.resolve", "params": {"agent_id": "agent_12345678"}
        })
    );
    let core = CoreAgentResolution::new(
        "agent_12345678".into(),
        AgentStatus::Merged,
        Some("agent_87654321".into()),
        Some(AgentGone::new(
            GoneReason::Merged,
            123,
            Presence::new("agent_87654321".into()),
        )),
        42,
    );
    let reply = Outcome::success(AgentResolveReply::new(core, "instance".into()));
    let expected = json!({"result": {
        "agent_id": "agent_12345678", "status": "merged", "merged_into": "agent_87654321",
        "gone": {"reason": "merged", "at": 123, "into_agent_id": "agent_87654321"},
        "generation": 42, "incarnation": "instance"
    }});
    assert_eq!(serde_json::to_value(reply).unwrap(), expected);
    round_trip::<Outcome<AgentResolveReply>>(expected);

    for status in ["unknown", "live", "retired"] {
        let gone = if status == "retired" {
            json!({"reason":"deleted", "at":123})
        } else {
            Value::Null
        };
        round_trip::<Outcome<AgentResolveReply>>(json!({"result": {
            "agent_id":"agent_12345678", "status":status, "merged_into":null,
            "gone":gone, "generation":42, "incarnation":"instance"
        }}));
    }
}

#[test]
fn unknown_enum_spellings() {
    macro_rules! vocabulary {
        ($ty:ty, [$($known:literal),* $(,)?]) => {{
            $(round_trip::<$ty>(json!($known));)*
            let value: $ty = round_trip(json!("future-spelling"));
            assert_eq!(value, <$ty>::Other("future-spelling".into()));
            assert_eq!(<$ty>::new("future-spelling"), value);
        }};
    }
    vocabulary!(AgentStatus, ["unknown", "live", "retired", "merged"]);
    vocabulary!(GoneReason, ["deleted", "merged"]);
    vocabulary!(CatalogGenerationState, ["ready", "active", "degraded"]);
    for excluded in ["installed", "approved", "superseded"] {
        assert_eq!(
            CatalogGenerationState::new(excluded),
            CatalogGenerationState::Other(excluded.into())
        );
    }
    vocabulary!(PinKind, ["memory-profile", "persona"]);
    vocabulary!(WhereStatus, ["unclaimed", "held_here", "held_elsewhere"]);
    vocabulary!(MoveSide, ["source", "receiver"]);
    vocabulary!(
        MoveStep,
        [
            "preparing",
            "exported",
            "sealed",
            "published",
            "releasing",
            "picked_up",
            "imported",
            "staged",
            "claimed",
            "activating",
            "activated",
        ]
    );
    vocabulary!(MoveReason, ["diverged"]);
    vocabulary!(
        MoveStatus,
        [
            "waiting_for_target",
            "abort_stuck",
            "held_as_fork",
            "contract_violation"
        ]
    );
    vocabulary!(MoveErrorCode, ["cut_accept_pending"]);
    vocabulary!(MoveWarningCode, ["release_not_enforced"]);
    vocabulary!(SessionNetwork, ["yes", "no", "unknown"]);
    vocabulary!(
        FleetAgentRole,
        ["assistant", "workspace_head", "head", "hiree"]
    );
    vocabulary!(FleetAvatarType, ["creature.classic", "creature.wild"]);
    vocabulary!(Via, ["root", "containment", "worktree", "implicit", "walk"]);
    vocabulary!(
        RefusalCode,
        [
            "invalid_params",
            "invalid_request",
            "unknown_agent",
            "gone",
            "claim_integrity_failed",
            "store_corrupt",
            "storage_unavailable",
            "storage_error",
            "encode_failed",
            "store_unavailable",
            "module_draining",
            "not_found",
        ]
    );
    for spelling in [
        "available",
        "withdrawn",
        "tombstoned",
        "revoked",
        "future-spelling",
    ] {
        round_trip::<PinStatusState>(json!(spelling));
    }
    assert_eq!(
        PinStatusState::new(json!("future-spelling")),
        PinStatusState::Other("future-spelling".into())
    );
}

fn movement() -> Value {
    json!({"cut_id":"cut", "side":"source", "step":"published", "peer":"peer", "peer_name":null})
}

#[test]
fn optional_payload_presence() {
    let mut value = movement();
    let absent: AgentMoving = round_trip(value.clone());
    assert!(absent.hold.is_missing());
    assert!(absent.detail.is_missing());
    assert!(absent.diagnostic.is_missing());
    value["hold"] = Value::Null;
    let null: AgentMoving = round_trip(value.clone());
    assert_eq!(null.hold, Presence::new(Value::Null));
    assert_ne!(null.hold, absent.hold);
    value["hold"] = json!({"participant":{"future":[null, 1, "x"]}});
    value["detail"] = Value::Null;
    value["diagnostic"] = json!([true, {"unfamiliar":2}]);
    round_trip::<AgentMoving>(value);
}

#[test]
fn projected_pin_state_accepts_non_string_json() {
    for state in [
        Value::Null,
        json!(false),
        json!(23),
        json!(["x"]),
        json!({"future":true}),
    ] {
        let decoded: PinStatus = round_trip(json!({"state":state, "blocked":false}));
        assert_eq!(decoded.state, PinStatusState::Json(state));
        assert!(!decoded.blocked);
    }
}

#[test]
fn open_movement_payloads_survive() {
    let mut movement = movement();
    movement["reason"] = json!("diverged");
    movement["detail"] = json!({"participant":{"state":"future", "cut":null}});
    movement["hold"] = json!(null);
    movement["status"] = json!("contract_violation");
    movement["message"] = json!("participant notice");
    movement["waiting_since_ms"] = json!(-3);
    movement["waiting_ms"] = json!(3);
    movement["diagnostics"] = json!([{"participant":"x", "payload":[false]}, 2, null]);
    movement["diagnostic"] = json!({"opaque":{"nested":true}});
    let value = json!({
        "known":true, "status":"held_here", "holder":"author", "holder_name":null,
        "epoch":7, "cut_id":null, "read_at_ms":-2, "stale":false, "moving":movement,
        "moveErrors":[{"cut_id":"older", "code":"cut_accept_pending"}],
        "moveWarnings":[{"cut_id":"older", "module_id":"participant", "code":"release_not_enforced", "message":"notice"}],
        "moveDiagnostics":[{"cut_id":"older", "participant":{"future":[true, null]}, "code":17}]
    });
    let decoded: AgentWhere = round_trip(value);
    match decoded.move_diagnostics {
        Presence::Present(values) => assert_eq!(
            values[0].payload["participant"],
            json!({"future":[true,null]})
        ),
        _ => panic!("expected diagnostics"),
    }
}

fn ignores<T: DeserializeOwned + Serialize>(known: Value) {
    let mut extended = known.clone();
    extended["future_member"] = json!({"nested":[1, 2]});
    let decoded: T = serde_json::from_value(extended).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), known);
}

#[test]
fn unknown_members_are_ignored() {
    ignores::<ContextStamp>(json!({"incarnation":"i", "generation":1, "logPosition":2}));
    ignores::<ContextDocument>(
        json!({"revision":2, "sha256":"digest", "charter_bytes":3, "document_bytes":4}),
    );
    ignores::<ContextProjectReply>(json!({
        "project_id":null, "workspace_id":null, "domain":null, "via":"implicit", "document":null,
        "members_digest":null, "stamp":{"incarnation":"i", "generation":1, "logPosition":0}
    }));
    ignores::<AgentGone>(json!({"reason":"deleted", "at":-1}));
    let core = json!({"agent_id":"agent_12345678", "status":"live", "merged_into":null, "gone":null, "generation":1});
    ignores::<CoreAgentResolution>(core.clone());
    let mut direct = core.clone();
    direct["incarnation"] = json!("i");
    ignores::<AgentResolveReply>(direct);
    ignores::<CatalogGeneration>(json!({"generation_id":"g", "state":"degraded"}));
    ignores::<PinStatus>(json!({"state":"available", "blocked":true}));
    ignores::<ContextPin>(json!({
        "name":"@example/persona", "version":"1", "kind":"persona", "manifest_sha256":"a",
        "archive_sha256":"b", "publisher_fingerprint":"c", "status":null
    }));
    let location = json!({"known":true, "status":"unclaimed", "holder":null, "holder_name":null,
        "epoch":0, "cut_id":null, "read_at_ms":null, "stale":false});
    ignores::<AgentWhere>(location.clone());
    let mut entry = location.clone();
    entry["agent_id"] = json!("agent_12345678");
    ignores::<AgentWhereEntry>(entry.clone());
    ignores::<AgentWhereManyReply>(json!({"agents":[entry]}));
    ignores::<ContextSessionReply>(json!({"agent":core, "catalog_generations":[], "pins":[],
        "where":location, "stamp":{"incarnation":"i", "generation":1, "logPosition":0}}));
    ignores::<AgentMoving>(movement());
    ignores::<MoveError>(json!({"cut_id":"c", "code":"cut_accept_pending"}));
    ignores::<MoveWarning>(
        json!({"cut_id":"c", "module_id":"participant", "code":"release_not_enforced", "message":"m"}),
    );
    ignores::<Request<AgentResolveParams>>(
        json!({"method":"agent.resolve", "params":{"agent_id":"agent_12345678"}}),
    );
    ignores::<Outcome<AgentResolveReply>>(
        json!({"code":"gone", "message":"gone", "detail":{"future":true}}),
    );
}

/// Params mirror the handler's admission: an unknown member is refused, while
/// replies and the request envelope stay open to additions.
#[test]
fn params_refuse_unknown_members() {
    fn refuses<T: DeserializeOwned>(known: Value) {
        assert!(serde_json::from_value::<T>(known.clone()).is_ok());
        let mut extended = known;
        extended["unknown"] = json!(true);
        assert!(serde_json::from_value::<T>(extended).is_err());
    }
    refuses::<ContextProjectParams>(json!({"folder":"/folder"}));
    refuses::<AgentIdParams>(json!({"agent_id":"agent_12345678"}));
    refuses::<AgentWhereManyParams>(json!({"agentIds":["agent_12345678"]}));
    refuses::<AgentSessionForRefParams>(json!({"ref_kind":"future-kind", "ref":"r"}));
    refuses::<AgentSessionForRunnerParams>(json!({"runner":"runner", "session":"s"}));
    refuses::<AgentFleetIdentityParams>(json!({}));
    assert!(serde_json::from_value::<Request<AgentResolveParams>>(
        json!({"method":"agent.resolve", "params":{"agent_id":"agent_12345678", "unknown":true}}),
    )
    .is_err());
}

/// The agent reads' refusal codes decode as named variants.
#[test]
fn agent_read_refusal_codes_are_named() {
    for (wire, code) in [
        ("unknown_agent", RefusalCode::UnknownAgent),
        ("gone", RefusalCode::Gone),
        ("claim_integrity_failed", RefusalCode::ClaimIntegrityFailed),
        ("store_corrupt", RefusalCode::StoreCorrupt),
        ("not_found", RefusalCode::NotFound),
    ] {
        let outcome: Outcome<AgentResolveReply> =
            serde_json::from_value(json!({"code":wire, "message":"m", "detail":null})).unwrap();
        let Outcome::Refusal(refusal) = outcome else {
            panic!("{wire} must decode as a refusal")
        };
        assert_eq!(refusal.code, code);
    }
}

#[test]
fn request_spellings_and_required_fields() {
    assert!(serde_json::from_value::<ContextProjectParams>(json!({"path":"/folder"})).is_err());
    assert!(serde_json::from_value::<AgentIdParams>(json!({"agentId":"agent_12345678"})).is_err());
    assert!(serde_json::from_value::<AgentWhereManyParams>(json!({"agent_ids":[]})).is_err());
    for value in [json!({}), json!({"folder":null}), json!({"folder":1})] {
        assert!(serde_json::from_value::<ContextProjectParams>(value).is_err());
    }
    for value in [json!({}), json!({"agent_id":null}), json!({"agent_id":1})] {
        assert!(serde_json::from_value::<AgentIdParams>(value).is_err());
    }
    assert!(serde_json::from_value::<AgentResolveReply>(json!({
        "agent_id":"a", "status":"live", "merged_into":null, "gone":null, "generation":1
    }))
    .is_err());
    // Constructors are data containers, not the handler's request admission layer.
    round_trip::<AgentWhereManyParams>(json!({"agentIds":[]}));
    round_trip::<AgentIdParams>(json!({"agent_id":"not-an-id"}));
}

#[test]
fn runner_params_refuse_session_id_alias() {
    assert!(serde_json::from_value::<AgentSessionForRunnerParams>(
        json!({"runner":"runner", "session_id":"session"})
    )
    .is_err());
    let params = AgentSessionForRunnerParams::new("runner".into(), "session".into());
    assert_eq!(
        serde_json::to_value(params).unwrap(),
        json!({"runner":"runner", "session":"session"})
    );
}

#[test]
fn session_lookup_required_params() {
    for params in [
        json!({}),
        json!({"ref_kind":"k"}),
        json!({"ref_kind":"k", "session_ref":"s"}),
        json!({"ref_kind":null, "ref":"s"}),
        json!({"ref_kind":"k", "ref":null}),
    ] {
        assert!(serde_json::from_value::<AgentSessionForRefParams>(params).is_err());
    }
    for params in [
        json!({}),
        json!({"runner":"r"}),
        json!({"runner":null, "session":"s"}),
        json!({"runner":"r", "session":null}),
    ] {
        assert!(serde_json::from_value::<AgentSessionForRunnerParams>(params).is_err());
    }
    let params = AgentSessionForRefParams::new("future-kind".into(), "opaque-ref".into());
    assert_eq!(
        serde_json::to_value(params).unwrap(),
        json!({"ref_kind":"future-kind", "ref":"opaque-ref"})
    );
}

#[test]
fn fleet_token_accepts_any_json_value() {
    let missing: AgentFleetIdentityParams = round_trip(json!({}));
    assert!(missing.token.is_missing());
    for token in [
        Value::Null,
        json!(7),
        json!(true),
        json!([]),
        json!({"nested":[false, 1]}),
        json!(""),
        json!("malformed"),
        json!("wrong-incarnation:1"),
    ] {
        let params: AgentFleetIdentityParams = round_trip(json!({"token":token}));
        assert_eq!(serde_json::to_value(params.token).unwrap(), token);
    }
}

fn ref_row() -> Value {
    json!({"agent_id":"a", "ref_kind":"future-kind", "ref":"opaque", "harness":"future-harness",
        "session_id":"s", "project_root":null, "incarnation":"i"})
}

fn runner_row() -> Value {
    json!({"agent_id":"a", "ref_kind":"future-kind", "ref":"opaque", "harness":"future-harness",
        "session":"s", "project_root":null, "runner":"future-runner", "ended_at_ms":null,
        "network":"future-network", "network_history":[{"network":"future-network", "since_ms":-1}],
        "incarnation":"i"})
}

#[test]
fn session_reply_spellings_are_not_aliases() {
    let mut runner = runner_row();
    let session = runner.as_object_mut().unwrap().remove("session").unwrap();
    runner["session_id"] = session;
    assert!(serde_json::from_value::<AgentSessionForRunnerReply>(runner).is_err());
    let mut reference = ref_row();
    let session = reference
        .as_object_mut()
        .unwrap()
        .remove("session_id")
        .unwrap();
    reference["session"] = session;
    // Nullable fields accept omission, but the other operation's spelling is not an alias.
    let decoded: AgentSessionForRefReply = serde_json::from_value(reference).unwrap();
    assert!(decoded.session_id.is_none());
}

#[test]
fn session_runner_and_held_on_never_accept_null() {
    let reference: AgentSessionForRefReply = round_trip(ref_row());
    assert!(reference.runner.is_missing());
    assert!(reference.held_on.is_missing());
    let runner: AgentSessionForRunnerReply = round_trip(runner_row());
    assert_eq!(
        runner.network,
        SessionNetwork::Other("future-network".into())
    );
    assert!(runner.network_history[0].network == runner.network);
    for field in ["runner", "heldOn"] {
        let mut reference = ref_row();
        reference[field] = Value::Null;
        assert!(serde_json::from_value::<AgentSessionForRefReply>(reference).is_err());
        let mut runner = runner_row();
        runner[field] = Value::Null;
        assert!(serde_json::from_value::<AgentSessionForRunnerReply>(runner).is_err());
    }
    let mut runner = runner_row();
    runner.as_object_mut().unwrap().remove("runner");
    assert!(serde_json::from_value::<AgentSessionForRunnerReply>(runner).is_err());
}

#[test]
fn fleet_empty_is_distinct_from_unchanged() {
    let mut value = json!({"generation":0, "incarnation":"i", "logPosition":0, "token":"opaque", "unchanged":true});
    let unchanged: AgentFleetIdentityReply = round_trip(value.clone());
    assert!(unchanged.agents.is_missing());
    value["unchanged"] = json!(false);
    value["agents"] = json!([]);
    let full: AgentFleetIdentityReply = round_trip(value.clone());
    assert_eq!(full.agents, Presence::new(vec![]));
    assert!(!full.unchanged);
    value["agents"] = Value::Null;
    assert!(serde_json::from_value::<AgentFleetIdentityReply>(value).is_err());
}

#[test]
fn new_read_reply_members_stay_open() {
    ignores::<AgentSessionForRefReply>(ref_row());
    ignores::<AgentSessionForRunnerReply>(runner_row());
    ignores::<SessionNetworkEntry>(json!({"network":"future", "since_ms":0}));
    ignores::<AgentFleetIdentityReply>(
        json!({"generation":0, "incarnation":"i", "logPosition":0, "token":"opaque", "unchanged":true}),
    );
    ignores::<FleetAvatar>(json!({"fingerprint":"opaque", "type":"future-avatar"}));
    ignores::<FleetProject>(json!({"id":"p", "name":"project"}));
    ignores::<FleetWorkspace>(json!({"id":"w", "name":"workspace"}));
    ignores::<FleetAgentIdentity>(
        json!({"agentId":"a", "displayName":"name", "tag":"", "labels":[], "role":"future-role"}),
    );
}

#[test]
fn fleet_optional_children_keep_present_empty_values() {
    let agent: FleetAgentIdentity = round_trip(
        json!({"agentId":"a", "displayName":"", "tag":"", "labels":[], "role":"future-role",
        "project":{"id":"p", "name":"", "canonicalRoot":""}, "workspace":{"id":"w", "name":""},
        "avatar":{"fingerprint":"", "type":"future-avatar", "version":0}}),
    );
    assert_eq!(agent.role, FleetAgentRole::Other("future-role".into()));
    let Presence::Present(avatar) = agent.avatar else {
        panic!("expected avatar")
    };
    assert_eq!(
        avatar.avatar_type,
        FleetAvatarType::Other("future-avatar".into())
    );
    assert_eq!(avatar.version, Presence::new(0));
    let empty =
        json!({"agentId":"a", "displayName":"n", "tag":"", "labels":[], "role":"assistant"});
    let absent: FleetAgentIdentity = round_trip(empty.clone());
    assert!(
        absent.avatar.is_missing() && absent.project.is_missing() && absent.workspace.is_missing()
    );
    for field in ["avatar", "project", "workspace"] {
        let mut null = empty.clone();
        null[field] = Value::Null;
        assert!(serde_json::from_value::<FleetAgentIdentity>(null).is_err());
    }
}
