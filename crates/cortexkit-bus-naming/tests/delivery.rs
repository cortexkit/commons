use std::time::Duration;

use cortexkit_bus_naming::{
    bus_permissions, delivery_authority_permissions, delivery_consumer_specs,
    flow_engine_permissions, participant_permissions, shipped_streams, validate_body_retention,
    validate_consumer, validate_consumers, validate_store_retention, AccountNames, AckPolicy,
    BodyRetention, DeliverPolicy, DiscardPolicy, LimitError, Operation, StreamKind, HOUR, MIB,
};

#[test]
fn delivery_builders_validate_closed_families_and_preserve_identity() {
    let names = AccountNames::derive("box_local").unwrap();
    assert_eq!(names.streams().inbox, "CK_BOX_LOCAL_INBOX");
    assert_eq!(names.streams().core_work, "CK_BOX_LOCAL_CORE_WORK");
    assert_eq!(names.streams().delivery_dead, "CK_BOX_LOCAL_DELIVERY_DEAD");
    for kind in ["head", "worker"] {
        assert_eq!(
            names.inbox_ready(kind, "a-b_9").unwrap(),
            format!("ck.box_local.inbox.{kind}.a-b_9.ready")
        );
    }
    for family in ["effect", "consult", "work", "campaign", "hire", "spec"] {
        assert_eq!(
            names.core_work_run(family, "a-b_9").unwrap(),
            format!("ck.box_local.core_work.{family}.a-b_9.run")
        );
    }
    for bad in ["", "A", "a.b", "a*", ">", "two words", &"a".repeat(64)] {
        assert!(names.inbox_ready("head", bad).is_err(), "{bad}");
        assert!(names.core_work_run("work", bad).is_err(), "{bad}");
    }
    for bad in ["", "reader", "HEAD", "*", "head.extra"] {
        assert!(names.inbox_ready(bad, "id").is_err());
    }
    for bad in ["", "inbox", "custom", "Effect", "*", "work.extra"] {
        assert!(names.core_work_run(bad, "id").is_err());
    }
    assert_eq!(names.delivery_dead(), "ck.box_local.delivery.dead");
}

#[test]
fn delivery_existing_six_stream_names_and_subjects_are_wire_compatible() {
    let names = AccountNames::derive("box_local").unwrap();
    let streams = shipped_streams(&names);
    let expected = [
        ("CK_BOX_LOCAL_ROOM", "ck.box_local.room.*.post"),
        ("CK_BOX_LOCAL_WAKE", "ck.box_local.wake.*.fire"),
        ("CK_BOX_LOCAL_PEER", "ck.box_local.peer.*.*.deliver"),
        ("CK_BOX_LOCAL_EFFECT", "ck.box_local.effect.*.*.intent"),
        ("CK_BOX_LOCAL_EFFECT_DEAD", "ck.box_local.effect.dead"),
        ("CK_BOX_LOCAL_EVENT", "ck.box_local.event.>"),
    ];
    for (spec, (name, subject)) in streams.iter().zip(expected) {
        assert_eq!(spec.name, name);
        assert_eq!(spec.subjects, [subject]);
    }
}

#[test]
fn delivery_command_limits_and_pinned_body_retention_are_explicit() {
    let names = AccountNames::derive("box_local").unwrap();
    for spec in shipped_streams(&names) {
        if matches!(spec.kind, StreamKind::EffectDead | StreamKind::DeliveryDead) {
            assert_eq!(
                cortexkit_bus_naming::DEAD_LETTER_DUPLICATE_WINDOW,
                Duration::from_secs(120)
            );
            assert_eq!(
                spec.duplicate_window,
                Some(cortexkit_bus_naming::DEAD_LETTER_DUPLICATE_WINDOW)
            );
        } else {
            assert_eq!(spec.duplicate_window, None);
        }
        if matches!(
            spec.kind,
            StreamKind::Effect | StreamKind::Inbox | StreamKind::CoreWork
        ) {
            assert!(spec.work_queue);
            assert_eq!(spec.discard, DiscardPolicy::New);
            assert_eq!(spec.max_age, Duration::ZERO);
            assert_eq!(spec.max_bytes, 256 * MIB);
            assert_eq!(spec.max_message_size, Some(16_384));
            assert!(matches!(
                validate_store_retention(&spec, HOUR * 24 * 365),
                Err(LimitError::BodyMustBePinned { .. })
            ));
            validate_body_retention(&spec, BodyRetention::PinnedUntilTerminal).unwrap();
        } else {
            assert_eq!(spec.max_message_size, None);
        }
        if spec.kind == StreamKind::DeliveryDead {
            assert!(!spec.work_queue);
            assert_eq!(spec.discard, DiscardPolicy::Old);
            assert_eq!(spec.max_age, HOUR * 24 * 7);
            assert_eq!(spec.max_bytes, 64 * MIB);
        }
    }
}

#[test]
fn delivery_bootstrap_readers_are_disjoint_and_subset_checked() {
    let names = AccountNames::derive("box_local").unwrap();
    let streams = shipped_streams(&names);
    let specs = delivery_consumer_specs(&names).unwrap();
    assert_eq!(specs.len(), 4);
    for (index, spec) in specs.iter().enumerate() {
        assert!(spec.pull);
        assert_eq!(spec.ack_policy, AckPolicy::Explicit);
        assert_eq!(spec.deliver_policy, DeliverPolicy::All);
        assert_eq!(spec.ack_wait, Duration::from_secs(30));
        assert_eq!(spec.max_deliver, -1);
        assert!(!names
            .streams()
            .agent_streams()
            .contains(&spec.consumer.stream.as_str()));
        validate_consumer(&spec.consumer, &streams).unwrap();
        if index < 2 {
            assert_eq!(spec.consumer.durable, "m_prefrontal-core");
            assert_eq!(spec.max_ack_pending, 1);
        } else {
            assert_eq!(spec.consumer.durable, "c_ckbus_dead");
        }
    }
    assert_eq!(specs[0].consumer.stream, names.streams().inbox);
    assert_eq!(specs[1].consumer.stream, names.streams().core_work);
    let mut consumers = specs
        .iter()
        .map(|spec| spec.consumer.clone())
        .collect::<Vec<_>>();
    validate_consumers(&consumers, &streams).unwrap();
    consumers.push(consumers[0].clone());
    assert!(matches!(
        validate_consumers(&consumers, &streams),
        Err(LimitError::OverlappingConsumerFilters { .. })
    ));
    consumers.pop();
    consumers[0].filter_subjects = vec!["ck.box_local.inbox.>".into()];
    assert!(matches!(
        validate_consumers(&consumers, &streams),
        Err(LimitError::ConsumerFilterNotSubset { .. })
    ));
    consumers[0] = specs[0].consumer.clone();
    let duplicate = consumers[0].filter_subjects[0].clone();
    consumers[0].filter_subjects.push(duplicate);
    assert!(matches!(
        validate_consumer(&consumers[0], &streams),
        Err(LimitError::OverlappingConsumerFilters { .. })
    ));
    for stream in streams.iter().filter(|spec| {
        matches!(
            spec.kind,
            StreamKind::Wake | StreamKind::Peer | StreamKind::Effect
        )
    }) {
        let mut consumer = specs[0].consumer.clone();
        consumer.stream = stream.name.clone();
        consumer.filter_subjects = stream.subjects.clone();
        assert!(matches!(
            validate_consumer(&consumer, &streams),
            Err(LimitError::InvalidDurableOwner { .. })
        ));
    }
}

#[test]
fn delivery_grants_admit_only_core_and_diagnostics() {
    let names = AccountNames::derive("box_local").unwrap();
    let authority =
        delivery_authority_permissions(&names, "UCORE", "prefrontal-core", &[]).unwrap();
    let participant = participant_permissions(&names, "UOTHER", "other", &[]).unwrap();
    let flow = flow_engine_permissions(&names, "UFLOW", "basal").unwrap();
    let bus = bus_permissions(&names, "UBUS").unwrap();
    for subject in names
        .inbox_bindings()
        .into_iter()
        .chain(names.core_work_bindings())
        .chain([names.delivery_dead()])
    {
        assert!(authority
            .iter()
            .any(|entry| entry.operation == Operation::Publish && entry.subject == subject));
        for excluded in [&participant, &flow, &bus] {
            assert!(!excluded.iter().any(|entry| entry.subject == subject));
        }
    }
    for stream in [&names.streams().inbox, &names.streams().core_work] {
        for subject in [
            format!("$JS.API.CONSUMER.MSG.NEXT.{stream}.m_prefrontal-core"),
            format!("$JS.API.CONSUMER.INFO.{stream}.m_prefrontal-core"),
            format!("$JS.ACK.{stream}.m_prefrontal-core.>"),
        ] {
            assert!(authority.iter().any(|entry| entry.subject == subject));
            for excluded in [&participant, &flow] {
                assert!(!excluded.iter().any(|entry| entry.subject == subject));
            }
        }
        assert!(!authority.iter().any(
            |entry| entry.subject.contains("CONSUMER.CREATE") && entry.subject.contains(stream)
        ));
    }
    for stream in [&names.streams().effect_dead, &names.streams().delivery_dead] {
        assert!(bus.iter().any(
            |entry| entry.subject == format!("$JS.API.CONSUMER.MSG.NEXT.{stream}.c_ckbus_dead")
        ));
        for excluded in [&authority, &participant, &flow] {
            assert!(!excluded
                .iter()
                .any(|entry| entry.subject
                    == format!("$JS.API.CONSUMER.MSG.NEXT.{stream}.c_ckbus_dead")));
        }
    }
}
