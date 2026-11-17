use std::collections::BTreeSet;
use std::time::Duration;

use cortexkit_bus_naming::federation::*;
use cortexkit_bus_naming::{
    shipped_streams, validate_streams, AccountNames, DiscardPolicy, StreamKind, GIB, HOUR, MIB,
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
        assert_eq!(specs.len(), 4);
        let streams: Vec<_> = specs.iter().map(|spec| spec.stream.clone()).collect();
        validate_streams(&streams).unwrap();
        for spec in &specs {
            assert!(!local
                .iter()
                .any(|row| row.name == spec.stream.name || row.kind == spec.stream.kind));
            assert!(!spec.stream.work_queue);
        }
    }
    assert_eq!(local.len(), 9);
    assert!(!local.iter().any(|row| matches!(
        row.kind,
        StreamKind::HubInbox | StreamKind::HubIdlog | StreamKind::KvResidence | StreamKind::ObjCuts
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
fn federation_leaf_grant_is_unique_scoped_and_has_no_effect_subject() {
    for box_text in [SAMPLE_BOX, "fedcba9876543210fedcba9876543210"] {
        let box_id = BoxId::parse(box_text).unwrap();
        let grant = leaf_user_grant(&box_id);
        assert_eq!(grant.publish.len(), 41);
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
        for stream in [HUB_IDLOG_STREAM, RESIDENCE_STREAM, CUTS_STREAM] {
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
    }
}

fn stream_json(spec: &HubStreamSpec) -> Value {
    let stream = &spec.stream;
    json!({
        "kind": match stream.kind {
            StreamKind::HubInbox => "hub_inbox",
            StreamKind::HubIdlog => "hub_idlog",
            StreamKind::KvResidence => "kv_residence",
            StreamKind::ObjCuts => "obj_cuts",
            _ => panic!("not a hub stream"),
        },
        "name": stream.name,
        "subjects": stream.subjects,
        "max_age_seconds": stream.max_age.as_secs(),
        "max_bytes": stream.max_bytes,
        "discard": match stream.discard { DiscardPolicy::Old => "old", DiscardPolicy::New => "new" },
        "work_queue": stream.work_queue,
        "max_msgs_per_subject": stream.max_msgs_per_subject,
        "max_message_size": stream.max_message_size,
        "duplicate_window_seconds": stream.duplicate_window.map(|duration| duration.as_secs()),
        "replicas": spec.replicas,
        "allow_rollup_hdrs": spec.allow_rollup_hdrs,
        "allow_direct": spec.allow_direct,
        "deny_delete": spec.deny_delete,
    })
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
        },
        "grant": { "publish": grant.publish, "subscribe": grant.subscribe },
        "streams": {
            "hub_inbox": stream_json(&hub_inbox_stream(&box_id)),
            "hub_idlog": stream_json(&hub_idlog_stream()),
            "residence_self_hosted": residence_json(HubKind::SelfHosted),
            "residence_hosted": residence_json(HubKind::Hosted),
            "cuts": stream_json(&cuts_object_store()),
        },
    });
    let actual = format!("{}\n", serde_json::to_string_pretty(&document).unwrap());
    assert_eq!(
        actual.as_bytes(),
        include_bytes!("../test-vectors/federation/hub.json")
    );
}
