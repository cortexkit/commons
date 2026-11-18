use std::collections::BTreeSet;
use std::time::Duration;

use cortexkit_bus_naming::federation::*;
use cortexkit_bus_naming::{
    box_account_denies, bus_permissions, delivery_authority_permissions, federation_consumer_specs,
    flow_engine_permissions, participant_permissions, shipped_streams, system_permissions,
    validate_consumer, validate_streams, AccountNames, ConsumerSpec, DiscardPolicy, Operation,
    StorageType, StreamKind, StreamSpec, TokenKind, EVENT_CONSUMERS, GIB, HOUR, LIVE_HINT_READERS,
    MIB, MODULE_BUCKETS,
};
use serde_json::{json, Value};

const SAMPLE_BOX: &str = "0123456789abcdef0123456789abcdef";

fn sample_box() -> BoxId {
    BoxId::parse(SAMPLE_BOX).unwrap()
}

#[test]
fn federation_names_and_module_prefixes_are_exact() {
    let box_id = sample_box();
    assert_eq!(
        hub_inbox_stream_name(&box_id),
        "HUB_INBOX_0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        inbox_stream_name(&box_id),
        "INBOX_0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        deliver_prefix(&box_id),
        "ckdel.0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        module_deliver_prefix(&box_id, "example-module").unwrap(),
        "ckdel.0123456789abcdef0123456789abcdef.example-module"
    );
    for module in ["", "other.module", "*", ">", "module "] {
        assert!(
            module_deliver_prefix(&box_id, module).is_err(),
            "{module:?}"
        );
    }
}

#[test]
fn federation_hub_streams_keep_their_bounds_and_do_not_enter_local_rows() {
    let box_id = sample_box();
    let local = shipped_streams(&AccountNames::derive("box_example").unwrap());
    for hub in [HubKind::SelfHosted, HubKind::Hosted] {
        let specs = hub_streams(&box_id, hub);
        assert_eq!(specs.len(), 6);
        assert!(specs.contains(&hub_memory_stream()));
        let streams: Vec<_> = specs.iter().map(|spec| spec.stream.clone()).collect();
        validate_streams(&streams).unwrap();
        for spec in &specs {
            assert!(!local
                .iter()
                .any(|row| row.name == spec.stream.name || row.kind == spec.stream.kind));
            assert!(!spec.stream.work_queue);
        }
    }
    assert_eq!(local.len(), 11);
    assert!(!local.iter().any(|row| matches!(
        row.kind,
        StreamKind::HubInbox
            | StreamKind::HubIdlog
            | StreamKind::HubRooms
            | StreamKind::HubMemory
            | StreamKind::KvResidence
            | StreamKind::ObjCuts
            | StreamKind::Fedout
    )));

    let inbox = hub_inbox_stream(&box_id).stream;
    assert_eq!(inbox.max_age, HOUR * 24 * 7);
    assert_eq!(inbox.discard, DiscardPolicy::New);
    assert_eq!(inbox.subjects.len(), 4);
    let idlog = hub_idlog_stream();
    assert_eq!(idlog.stream.subjects, ["ck.fleet.idlog"]);
    assert_eq!(idlog.stream.max_age, Duration::ZERO);
    assert_eq!(idlog.stream.max_bytes, 0);
    assert!(idlog.allow_direct);

    for (hub, replicas) in [(HubKind::SelfHosted, 1), (HubKind::Hosted, 3)] {
        let kv = residence_bucket(hub);
        assert_eq!(kv.history, 1);
        assert_eq!(kv.backing_stream.replicas, Some(replicas));
        let stream = kv.backing_stream.stream;
        assert_eq!(stream.max_msgs_per_subject, Some(1));
        assert_eq!(stream.max_message_size, Some(16 * 1024));
        assert_eq!(stream.max_bytes, 64 * MIB);
        assert_eq!(stream.max_age, Duration::ZERO);
        assert_eq!(stream.discard, DiscardPolicy::New);
        assert!(kv.backing_stream.allow_rollup_hdrs);
        assert!(kv.backing_stream.allow_direct);
        assert!(kv.backing_stream.deny_delete);
    }
    let cuts = cuts_object_store();
    assert_eq!(cuts.stream.max_bytes, 4 * GIB);
    assert_eq!(cuts.stream.max_age, HOUR * 24 * 7);
    assert_eq!(cuts.stream.discard, DiscardPolicy::New);
    assert!(cuts.allow_rollup_hdrs);
    assert!(cuts.allow_direct);
}

fn subject_matches(pattern: &str, subject: &str) -> bool {
    let pattern: Vec<_> = pattern.split('.').collect();
    let subject: Vec<_> = subject.split('.').collect();
    for (index, token) in pattern.iter().enumerate() {
        match *token {
            ">" => return subject.len() > index,
            "*" if index < subject.len() => {}
            literal if subject.get(index) == Some(&literal) => {}
            _ => return false,
        }
    }
    pattern.len() == subject.len()
}

#[test]
fn federation_streams_route_local_and_inbound_publishes_to_distinct_bindings() {
    let account = AccountNames::from_roster_host_id(SAMPLE_BOX).unwrap();
    let origin = BoxId::parse("fedcba9876543210fedcba9876543210").unwrap();
    let streams = shipped_streams(&account);
    let routes = [
        (
            StreamKind::Room,
            "room",
            account.room_post("room_a").unwrap(),
            &[&["room_a", "post"][..]][..],
        ),
        (
            StreamKind::Wake,
            "wake",
            account.wake_fire("agent_a").unwrap(),
            &[&["agent_a", "handoff"][..]][..],
        ),
        (
            StreamKind::Peer,
            "peer",
            account.peer_delivery("agent_a", "session_a").unwrap(),
            &[&["agent_a", "handoff"][..], &["custody"][..]][..],
        ),
    ];
    for (kind, family, local, destinations) in routes {
        let stream = streams.iter().find(|stream| stream.kind == kind).unwrap();
        let matching_bindings = |subject: &str| {
            stream
                .subjects
                .iter()
                .enumerate()
                .filter_map(|(index, binding)| subject_matches(binding, subject).then_some(index))
                .collect::<Vec<_>>()
        };
        assert_eq!(matching_bindings(&local), [0], "{local}");
        for (index, dest) in destinations.iter().enumerate() {
            let inbound = account.inbound_federated(&origin, family, dest).unwrap();
            assert_eq!(matching_bindings(&inbound), [index + 1], "{inbound}");
        }
    }
}

#[test]
fn federation_leaf_grant_is_unique_scoped_and_has_no_effect_subject() {
    for box_text in [SAMPLE_BOX, "fedcba9876543210fedcba9876543210"] {
        let box_id = BoxId::parse(box_text).unwrap();
        let grant = leaf_user_grant(&box_id);
        assert_eq!(grant.publish.len(), 59);
        assert_eq!(grant.subscribe.len(), 5);
        for entries in [&grant.publish, &grant.subscribe] {
            assert_eq!(entries.iter().collect::<BTreeSet<_>>().len(), entries.len());
            assert!(entries
                .iter()
                .all(|subject| !subject.split('.').any(|token| token == "effect")));
            for subject in entries {
                for token in subject.split('.') {
                    if let Some(suffix) = token
                        .strip_prefix("HUB_INBOX_")
                        .or_else(|| token.strip_prefix("INBOX_"))
                    {
                        assert_eq!(suffix, box_text, "{subject}");
                    }
                }
            }
        }
        let may_publish = |subject: &str| {
            grant
                .publish
                .iter()
                .any(|pattern| subject_matches(pattern, subject))
        };
        let foreign = if box_text == SAMPLE_BOX {
            "fedcba9876543210fedcba9876543210"
        } else {
            SAMPLE_BOX
        };
        for stream in [format!("HUB_INBOX_{foreign}"), format!("INBOX_{foreign}")] {
            for suffix in [
                "CONSUMER.CREATE",
                "CONSUMER.INFO",
                "CONSUMER.DELETE",
                "CONSUMER.RESET",
                "STREAM.INFO",
                "STREAM.MSG.GET",
                "STREAM.PURGE",
                "STREAM.DELETE",
            ] {
                for tail in ["", ".consumer", ".consumer.filter"] {
                    assert!(!may_publish(&format!(
                        "$JS.hub.API.{suffix}.{stream}{tail}"
                    )));
                }
            }
        }
        for target in [box_text, foreign] {
            assert!(!may_publish(&format!("ck.{target}.effect.intent")));
            assert!(may_publish(&format!("ck.{target}.room.post")));
        }
        for stream in [
            HUB_IDLOG_STREAM,
            HUB_ROOMS_STREAM,
            HUB_MEMORY_STREAM,
            RESIDENCE_STREAM,
            CUTS_STREAM,
        ] {
            assert!(may_publish(&format!(
                "$JS.hub.API.CONSUMER.CREATE.{stream}"
            )));
            assert!(!may_publish(&format!("$JS.hub.API.STREAM.CREATE.{stream}")));
            assert!(!may_publish(&format!("$JS.hub.API.STREAM.DELETE.{stream}")));
            assert_eq!(
                may_publish(&format!("$JS.hub.API.STREAM.PURGE.{stream}")),
                stream == CUTS_STREAM
            );
        }
        assert!(may_publish(&format!(
            "$JS.hub.API.CONSUMER.CREATE.HUB_INBOX_{box_id}"
        )));
        assert!(!may_publish("$KV.residence.key"));
        assert!(!may_publish("$JS.hub.API.$KV.other.key"));
        assert!(!may_publish("$O.other.C.chunk"));
        assert!(may_publish("ck.fleet.room.room_a"));
        assert!(may_publish("ck.fleet.memory.project.pj-example"));
        assert!(!may_publish("ck.fleet.memory.project"));
        assert!(!may_publish("ck.fleet.memory.project.pj-example.extra"));
        for suffix in [
            "STREAM.INFO.HUB_MEMORY",
            "STREAM.MSG.GET.HUB_MEMORY",
            "DIRECT.GET.HUB_MEMORY",
            "DIRECT.GET.HUB_MEMORY.ck.fleet.memory.project.pj-example",
            "CONSUMER.CREATE.HUB_MEMORY.example",
            "CONSUMER.INFO.HUB_MEMORY.example",
        ] {
            assert!(may_publish(&format!("$JS.hub.API.{suffix}")), "{suffix}");
        }
        assert!(!may_publish("$JS.hub.API.STREAM.MSG.DELETE.HUB_MEMORY"));
        for subject in ["fedout.frame", "fedout.box.peer.agent.handoff"] {
            assert!(!may_publish(subject));
            assert!(!grant
                .subscribe
                .iter()
                .any(|pattern| subject_matches(pattern, subject)));
        }
        for subject in [
            format!("ck.{foreign}.peer.agent.handoff"),
            format!("ckdel.{foreign}.prefrontal.reply"),
        ] {
            assert!(!grant
                .subscribe
                .iter()
                .any(|pattern| subject_matches(pattern, &subject)));
        }
    }
}

fn base_stream_json(stream: &StreamSpec) -> Value {
    json!({
        "kind": match stream.kind {
            StreamKind::Room => "room",
            StreamKind::Wake => "wake",
            StreamKind::Peer => "peer",
            StreamKind::Fedsend => "fedsend",
            StreamKind::FedsendDead => "fedsend_dead",
            StreamKind::Fedout => "fedout",
            StreamKind::HubInbox => "hub_inbox",
            StreamKind::HubIdlog => "hub_idlog",
            StreamKind::HubRooms => "hub_rooms",
            StreamKind::HubMemory => "hub_memory",
            StreamKind::KvResidence => "kv_residence",
            StreamKind::ObjCuts => "obj_cuts",
            _ => panic!("not a hub stream"),
        },
        "name": stream.name,
        "subjects": stream.subjects,
        "storage": match stream.storage { StorageType::File => "file", _ => panic!("unknown storage") },
        "max_age_seconds": stream.max_age.as_secs(),
        "max_bytes": stream.max_bytes,
        "discard": match stream.discard { DiscardPolicy::Old => "old", DiscardPolicy::New => "new" },
        "work_queue": stream.work_queue,
        "max_msgs_per_subject": stream.max_msgs_per_subject,
        "max_message_size": stream.max_message_size,
        "duplicate_window_seconds": stream.duplicate_window.map(|duration| duration.as_secs()),
    })
}

fn stream_json(spec: &HubStreamSpec) -> Value {
    let mut document = base_stream_json(&spec.stream);
    let object = document.as_object_mut().unwrap();
    object.insert("replicas".into(), json!(spec.replicas));
    object.insert("allow_rollup_hdrs".into(), json!(spec.allow_rollup_hdrs));
    object.insert("allow_direct".into(), json!(spec.allow_direct));
    object.insert("deny_delete".into(), json!(spec.deny_delete));
    document
}

fn residence_json(hub: HubKind) -> Value {
    let bucket = residence_bucket(hub);
    json!({
        "bucket": bucket.bucket,
        "history": bucket.history,
        "backing_stream": stream_json(&bucket.backing_stream),
    })
}

#[test]
fn federation_golden_matches_bytes() {
    let box_id = sample_box();
    let grant = leaf_user_grant(&box_id);
    let document = json!({
        "box_id": box_id.as_str(),
        "domain": HUB_DOMAIN,
        "api_prefix": HUB_API_PREFIX,
        "families": FEDERATED_FAMILIES,
        "names": {
            "inbox_stream": inbox_stream_name(&box_id),
            "deliver_prefix": deliver_prefix(&box_id),
            "module_deliver_prefix": module_deliver_prefix(&box_id, "example-module").unwrap(),
            "cuts_bucket": CUTS_BUCKET,
            "hub_room": hub_room_subject("room_a").unwrap(),
            "hub_memory": hub_memory_subject(MEMORY_KIND_PROJECT, "pj-example").unwrap(),
        },
        "grant": { "publish": grant.publish, "subscribe": grant.subscribe },
        "streams": {
            "hub_inbox": stream_json(&hub_inbox_stream(&box_id)),
            "hub_idlog": stream_json(&hub_idlog_stream()),
            "hub_rooms": stream_json(&hub_rooms_stream()),
            "hub_memory": stream_json(&hub_memory_stream()),
            "residence_self_hosted": residence_json(HubKind::SelfHosted),
            "residence_hosted": residence_json(HubKind::Hosted),
            "cuts": stream_json(&cuts_object_store()),
        },
    });
    assert_golden(
        "hub.json",
        &document,
        include_str!("../test-vectors/federation/hub.json"),
    );
}

#[test]
fn federation_hub_module_golden_matches_bytes() {
    let box_id = sample_box();
    let mut grants = serde_json::Map::new();
    for module in HUB_MODULES {
        assert!(is_hub_module(module));
        let grant = hub_module_grant(&box_id, module).unwrap();
        grants.insert(
            module.into(),
            json!({
                "publish": grant.publish,
                "subscribe": grant.subscribe,
                "denies": grant.denies.iter().map(|entry| json!({
                    "operation": entry.operation.as_str(), "subject": entry.subject,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    assert_golden(
        "hub-module-grants.json",
        &json!({ "box_id": box_id.as_str(), "grants": grants }),
        include_str!("../test-vectors/federation/hub-module-grants.json"),
    );
}

#[test]
fn federation_hub_module_refuses_other_ids() {
    let box_id = sample_box();
    for module in ["example-module", "Prefrontal", "prefrontal.core", "", "*"] {
        assert!(!is_hub_module(module), "{module:?}");
        assert_eq!(hub_module_grant(&box_id, module), None, "{module:?}");
    }
}

#[test]
fn federation_hub_module_alias_uses_attested_deliver_prefix() {
    let box_id = sample_box();
    assert!(is_hub_module("prefrontal-core"));
    let legacy = hub_module_grant(&box_id, "prefrontal-core").unwrap();
    let current = hub_module_grant(&box_id, "prefrontal").unwrap();
    assert_eq!(legacy.publish, current.publish);
    assert_eq!(legacy.denies, current.denies);
    assert_eq!(
        legacy.subscribe,
        [format!("ckdel.{SAMPLE_BOX}.prefrontal-core.>")]
    );
}

fn assert_golden(name: &str, document: &Value, expected: &str) {
    let actual = format!("{}\n", serde_json::to_string_pretty(document).unwrap());
    if std::env::var_os("UPDATE_FEDERATION_GOLDENS").as_deref() == Some(std::ffi::OsStr::new("1")) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-vectors/federation")
            .join(name);
        std::fs::write(path, actual).unwrap();
    } else {
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn federation_handoff_names_validate_families_and_destination_tokens() {
    let box_id = sample_box();
    let account = AccountNames::from_roster_host_id(SAMPLE_BOX).unwrap();
    for family in ["room", "wake", "peer", "sync"] {
        assert_eq!(
            account
                .fedsend(&box_id, family, &["agent_a", "handoff"])
                .unwrap(),
            format!("ck.box_{SAMPLE_BOX}.fedsend.{SAMPLE_BOX}.{family}.agent_a.handoff")
        );
        assert_eq!(
            account
                .inbound_federated(&box_id, family, &["agent_a", "handoff"])
                .unwrap(),
            format!("ck.box_{SAMPLE_BOX}.{family}.fed.{SAMPLE_BOX}.agent_a.handoff")
        );
    }
    assert_eq!(
        account
            .inbound_federated(&box_id, "peer", &["custody"])
            .unwrap(),
        format!("ck.box_{SAMPLE_BOX}.peer.fed.{SAMPLE_BOX}.custody")
    );
    for family in ["", "effect", "Peer", "peer.*", ">"] {
        assert!(account.fedsend(&box_id, family, &["custody"]).is_err());
        assert!(account
            .inbound_federated(&box_id, family, &["custody"])
            .is_err());
    }
    for tokens in [
        vec![],
        vec![""],
        vec!["*"],
        vec![">"],
        vec!["agent.a"],
        vec!["agent a"],
        vec!["Agent"],
        vec!["agent_a", "handoff.>"],
    ] {
        assert!(
            account.fedsend(&box_id, "peer", &tokens).is_err(),
            "{tokens:?}"
        );
        assert!(
            account.inbound_federated(&box_id, "peer", &tokens).is_err(),
            "{tokens:?}"
        );
    }
    assert_eq!(hub_room_subject("room_a").unwrap(), "ck.fleet.room.room_a");
    assert!(BoxId::parse("fleet").is_err());
    for room in ["", "room.a", "*", ">", "Room", "room "] {
        assert!(hub_room_subject(room).is_err());
    }
}

#[test]
fn federation_memory_subject_validates_kind_and_owner() {
    for (kind, owner, expected) in [
        (
            MEMORY_KIND_PROJECT,
            "pj-example",
            "ck.fleet.memory.project.pj-example",
        ),
        (
            MEMORY_KIND_WORKSPACE,
            "ws_example",
            "ck.fleet.memory.workspace.ws_example",
        ),
        (MEMORY_KIND_GLOBAL, "all", "ck.fleet.memory.global.all"),
        (
            MEMORY_KIND_OBSERVATION,
            "all",
            "ck.fleet.memory.observation.all",
        ),
        (
            MEMORY_KIND_PRIMER,
            "pj-example",
            "ck.fleet.memory.primer.pj-example",
        ),
    ] {
        assert_eq!(hub_memory_subject(kind, owner).unwrap(), expected);
    }
    for kind in ["", "fed", "Project", "projects", "project.extra", "*", ">"] {
        let error = hub_memory_subject(kind, "pj-example").unwrap_err();
        assert_eq!(error.kind(), TokenKind::MemoryKind);
        assert_eq!(error.token(), kind);
    }
    for kind in [
        MEMORY_KIND_PROJECT,
        MEMORY_KIND_WORKSPACE,
        MEMORY_KIND_PRIMER,
    ] {
        for owner in ["", "fed", "owner.extra", "WithCaps", "owner ", "*", ">"] {
            let error = hub_memory_subject(kind, owner).unwrap_err();
            assert_eq!(error.kind(), TokenKind::MemoryOwner);
            assert_eq!(error.token(), owner);
        }
    }
    for kind in [MEMORY_KIND_GLOBAL, MEMORY_KIND_OBSERVATION] {
        let error = hub_memory_subject(kind, "pj-example").unwrap_err();
        assert_eq!(error.kind(), TokenKind::MemoryOwner);
        assert_eq!(error.token(), "pj-example");
    }
}

#[test]
fn federation_memory_stream_preserves_history_and_matches_hub_placement() {
    let memory = hub_memory_stream();
    assert_eq!(memory.stream.kind, StreamKind::HubMemory);
    assert_eq!(memory.stream.name, "HUB_MEMORY");
    assert_eq!(memory.stream.subjects, ["ck.fleet.memory.*.*"]);
    assert_eq!(memory.stream.max_age, Duration::ZERO);
    assert_eq!(memory.stream.max_bytes, 1_073_741_824);
    assert_eq!(memory.stream.discard, DiscardPolicy::New);
    assert_eq!(memory.stream.max_message_size, Some(65_536));
    assert_eq!(memory.stream.max_msgs_per_subject, None);

    let rooms = hub_rooms_stream();
    assert_eq!(memory.stream.storage, rooms.stream.storage);
    assert_eq!(memory.stream.work_queue, rooms.stream.work_queue);
    assert_eq!(
        memory.stream.duplicate_window,
        rooms.stream.duplicate_window
    );
    assert_eq!(memory.replicas, rooms.replicas);
    assert_eq!(memory.allow_direct, rooms.allow_direct);
    assert_eq!(memory.allow_rollup_hdrs, rooms.allow_rollup_hdrs);
    assert_eq!(memory.deny_delete, rooms.deny_delete);
}

#[test]
fn federation_reserved_address_token_cannot_be_a_local_room_or_agent() {
    use cortexkit_bus_naming::{validate_token, TokenKind};
    let account = AccountNames::derive("box_reserved").unwrap();
    for kind in [TokenKind::RoomId, TokenKind::AgentId] {
        let error = validate_token(kind, "fed").unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.token(), "fed");
        assert!(error.reason().contains("reserved"));
    }
    assert!(account.room_post("fed").is_err());
    assert!(account.room_subscription("fed").is_err());
    assert!(account.wake_fire("fed").is_err());
    assert!(account.peer_delivery("fed", "session_a").is_err());
    assert!(account.peer_filter("fed").is_err());
    assert!(account.peer_subscription("fed").is_err());
    assert!(AccountNames::consumer_name("fed").is_err());
    assert!(account.peer_delivery("agent_a", "fed").is_ok());
    assert!(account
        .fedsend(&sample_box(), "peer", &["fed", "handoff"])
        .is_ok());
    assert!(validate_token(TokenKind::ModuleId, "fed").is_ok());
    assert!(validate_token(TokenKind::RoomId, "fed_room").is_ok());
    assert!(validate_token(TokenKind::AgentId, "fed_agent").is_ok());
}

#[test]
fn federation_handoff_limits_protect_stored_frames_and_bound_shared_rooms() {
    let account = AccountNames::from_roster_host_id(SAMPLE_BOX).unwrap();
    let streams = shipped_streams(&account);
    validate_streams(&streams).unwrap();
    for kind in [
        StreamKind::Room,
        StreamKind::Wake,
        StreamKind::Peer,
        StreamKind::Fedsend,
        StreamKind::FedsendDead,
    ] {
        let stream = streams.iter().find(|stream| stream.kind == kind).unwrap();
        assert_eq!(stream.max_age, Duration::from_secs(604_800));
        assert_eq!(stream.discard, DiscardPolicy::New);
        assert_eq!(stream.storage, StorageType::File);
        assert!(!stream.work_queue);
        if matches!(kind, StreamKind::Fedsend | StreamKind::FedsendDead) {
            assert_eq!(stream.max_message_size, Some(1_048_576));
            assert_eq!(stream.max_bytes, 0);
        } else {
            assert_eq!(stream.max_bytes, 1_073_741_824);
            assert_eq!(stream.max_message_size, None);
        }
    }
    let outbound = fedout_stream();
    assert_eq!(outbound.name, "FEDOUT");
    assert_eq!(outbound.subjects, ["fedout.>"]);
    assert_eq!(outbound.max_age, Duration::from_secs(604_800));
    assert_eq!(outbound.max_message_size, Some(1_048_576));
    assert_eq!(outbound.discard, DiscardPolicy::New);
    assert_eq!(outbound.storage, StorageType::File);
    let rooms = hub_rooms_stream();
    assert_eq!(rooms.stream.name, "HUB_ROOMS");
    assert_eq!(rooms.stream.subjects, ["ck.fleet.room.>"]);
    assert_eq!(rooms.stream.max_msgs_per_subject, Some(100_000));
    assert_eq!(rooms.stream.max_message_size, Some(262_144));
    assert_eq!(rooms.stream.max_age, Duration::ZERO);
    assert_eq!(rooms.stream.max_bytes, 0);
    assert_eq!(rooms.stream.discard, DiscardPolicy::Old);
    assert!(rooms.allow_direct);
}

#[test]
fn federation_module_readers_are_separate_and_can_defer_for_the_whole_retention() {
    let account = AccountNames::from_roster_host_id(SAMPLE_BOX).unwrap();
    let streams = shipped_streams(&account);
    for module in ["prefrontal-core", "prefrontal"] {
        let specs = federation_consumer_specs(&account, module).unwrap();
        assert_eq!(specs.len(), 2);
        let expected = [
            (
                account.streams().peer.clone(),
                vec![
                    format!("ck.box_{SAMPLE_BOX}.peer.fed.*.*.handoff"),
                    format!("ck.box_{SAMPLE_BOX}.peer.fed.*.custody"),
                ],
            ),
            (
                account.streams().wake.clone(),
                vec![format!("ck.box_{SAMPLE_BOX}.wake.fed.*.*.handoff")],
            ),
        ];
        for (spec, (stream, filters)) in specs.iter().zip(expected) {
            assert_eq!(spec.consumer.durable, format!("m_{module}"));
            assert_eq!(spec.consumer.stream, stream);
            assert_eq!(spec.consumer.filter_subjects, filters);
            assert_eq!(spec.max_deliver, -1);
            assert!(spec.pull);
            assert_eq!(spec.ack_policy, cortexkit_bus_naming::AckPolicy::Explicit);
            validate_consumer(&spec.consumer, &streams).unwrap();
        }
    }
    assert!(federation_consumer_specs(&account, "example-module").is_err());
    for stream in [
        &account.streams().peer,
        &account.streams().wake,
        &account.streams().effect,
    ] {
        let consumer = ConsumerSpec {
            durable: "m_other".into(),
            stream: stream.clone(),
            filter_subjects: vec![],
        };
        assert!(validate_consumer(&consumer, &streams).is_err());
    }
}

#[test]
fn federation_box_users_cannot_forge_origins_and_only_bus_publishes_dead_frames() {
    let account = AccountNames::from_roster_host_id(SAMPLE_BOX).unwrap();
    let denies = box_account_denies(&account);
    assert_eq!(denies.len(), 1);
    assert_eq!(denies[0].operation, Operation::Publish);
    assert_eq!(denies[0].subject, format!("ck.box_{SAMPLE_BOX}.*.fed.>"));
    let mut users = vec![
        bus_permissions(&account, "UBUS").unwrap(),
        system_permissions("USYSTEM").unwrap(),
        flow_engine_permissions(&account, "UFLOW", "basal").unwrap(),
    ];
    let modules =
        EVENT_CONSUMERS
            .iter()
            .map(|entry| entry.module_id)
            .chain(LIVE_HINT_READERS.iter().map(|entry| entry.module_id))
            .chain(MODULE_BUCKETS.iter().flat_map(|bucket| {
                std::iter::once(bucket.owner).chain(bucket.readers.iter().copied())
            }))
            .chain(["prefrontal-core", "prefrontal", "example-module"])
            .collect::<BTreeSet<_>>();
    for module in modules {
        let grants = participant_permissions(&account, "UMODULE", module, &[]).unwrap();
        for family in ["peer", "wake", "room", "sync"] {
            let subject = account
                .fedsend(&sample_box(), family, &["agent_a", "handoff"])
                .unwrap();
            assert_eq!(
                grants
                    .iter()
                    .any(|entry| entry.operation == Operation::Publish
                        && subject_matches(&entry.subject, &subject)),
                ["prefrontal-core", "prefrontal"].contains(&module)
                    && ["peer", "wake"].contains(&family),
                "{module} {family}"
            );
        }
        users.push(grants);
    }
    for module in ["prefrontal-core", "prefrontal"] {
        let grants = delivery_authority_permissions(&account, "UAUTHORITY", module, &[]).unwrap();
        for family in ["peer", "wake"] {
            let subject = account
                .fedsend(&sample_box(), family, &["custody"])
                .unwrap();
            assert!(grants
                .iter()
                .any(|entry| entry.operation == Operation::Publish
                    && subject_matches(&entry.subject, &subject)));
        }
        users.push(grants);
    }
    for (index, grants) in users.iter().enumerate() {
        for family in ["room", "wake", "peer", "sync", "future_family"] {
            let subject = format!("ck.box_{SAMPLE_BOX}.{family}.fed.{SAMPLE_BOX}.agent_a.handoff");
            // NATS evaluates explicit denies first, even if a future grant is broader.
            let may_publish = !denies.iter().any(|entry| {
                entry.operation == Operation::Publish && subject_matches(&entry.subject, &subject)
            }) && grants.iter().any(|entry| {
                entry.operation == Operation::Publish && subject_matches(&entry.subject, &subject)
            });
            assert!(!may_publish, "user {index}: {subject}");
        }
        let dead = format!("ck.box_{SAMPLE_BOX}.fedsend_dead.unknown.peer.agent_a.handoff");
        assert_eq!(
            grants
                .iter()
                .any(|entry| entry.operation == Operation::Publish
                    && subject_matches(&entry.subject, &dead)),
            index == 0
        );
    }
    assert!(!subject_matches(
        &denies[0].subject,
        &account.peer_delivery("agent_a", "session_a").unwrap()
    ));
    assert!(!subject_matches(
        &denies[0].subject,
        &account
            .fedsend(&sample_box(), "peer", &["custody"])
            .unwrap()
    ));
}

#[test]
fn federation_local_golden_matches_bytes() {
    let box_id = sample_box();
    let account = AccountNames::from_roster_host_id(SAMPLE_BOX).unwrap();
    let specs = shipped_streams(&account);
    let streams = specs
        .iter()
        .filter(|stream| {
            matches!(
                stream.kind,
                StreamKind::Room
                    | StreamKind::Wake
                    | StreamKind::Peer
                    | StreamKind::Fedsend
                    | StreamKind::FedsendDead
            )
        })
        .map(base_stream_json)
        .collect::<Vec<_>>();
    let mut readers = serde_json::Map::new();
    let mut grants = serde_json::Map::new();
    for module in ["prefrontal-core", "prefrontal"] {
        let specs = federation_consumer_specs(&account, module).unwrap();
        readers.insert(
            module.into(),
            json!(specs
                .iter()
                .map(|spec| json!({
                    "durable": spec.consumer.durable,
                    "stream": spec.consumer.stream,
                    "filter_subjects": spec.consumer.filter_subjects,
                    "pull": spec.pull,
                    "ack_policy": "explicit",
                    "deliver_policy": "all",
                    "ack_wait_seconds": spec.ack_wait.as_secs(),
                    "max_deliver": spec.max_deliver,
                    "max_ack_pending": spec.max_ack_pending,
                }))
                .collect::<Vec<_>>()),
        );
        grants.insert(
            module.into(),
            json!(
                delivery_authority_permissions(&account, "UAUTHORITY", module, &[])
                    .unwrap()
                    .iter()
                    .filter(|entry| entry.subject.contains(".fedsend."))
                    .map(|entry| entry.subject.clone())
                    .collect::<Vec<_>>()
            ),
        );
    }
    let names = [
        ("peer_handoff", "peer", vec!["agent_a", "handoff"]),
        ("wake_handoff", "wake", vec!["agent_a", "handoff"]),
        ("peer_custody", "peer", vec!["custody"]),
        ("room", "room", vec!["room_a", "post"]),
        ("sync", "sync", vec!["state"]),
    ]
    .into_iter()
    .map(|(name, family, dest)| {
        (
            name.to_owned(),
            json!({
                "outbound": account.fedsend(&box_id, family, &dest).unwrap(),
                "inbound": account.inbound_federated(&box_id, family, &dest).unwrap(),
            }),
        )
    })
    .collect::<serde_json::Map<_, _>>();
    let document = json!({
        "account": account.account(),
        "box_id": box_id.as_str(),
        "subjects": names,
        "box_streams": streams,
        "federation_account_stream": base_stream_json(&fedout_stream()),
        "readers": readers,
        "fedsend_publish": grants,
        "bus_only_publish": account.fedsend_dead_binding(),
        "all_box_users_deny": box_account_denies(&account).iter().map(|entry| json!({
            "operation": entry.operation.as_str(), "subject": entry.subject,
        })).collect::<Vec<_>>(),
    });
    assert_golden(
        "local.json",
        &document,
        include_str!("../test-vectors/federation/local.json"),
    );
}
