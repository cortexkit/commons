use std::time::Duration;

use cortexkit_bus_naming::{
    shipped_streams, validate_consumer, validate_store_retention, validate_streams, AccountNames,
    ConsumerSpec, DiscardPolicy, LimitError, StreamKind, StreamSpec, GIB, HOUR, MIB,
};

#[test]
fn shipped_stream_bindings_are_pairwise_disjoint() {
    let names = AccountNames::derive("box_limits").unwrap();
    validate_streams(&shipped_streams(&names)).unwrap();
}

#[test]
fn overlapping_stream_filters_are_refused_with_both_filters_named() {
    let names = AccountNames::derive("box_limits").unwrap();
    let mut streams = shipped_streams(&names);
    streams[1].subjects = vec![names.room_binding()];

    let error = validate_streams(&streams).expect_err("duplicate capture must be refused");
    match error {
        LimitError::OverlappingStreamFilters {
            left_stream,
            left_filter,
            right_stream,
            right_filter,
            involves_work_queue,
        } => {
            assert_eq!(left_stream, names.streams().room);
            assert_eq!(left_filter, names.room_binding());
            assert_eq!(right_stream, names.streams().wake);
            assert_eq!(right_filter, names.room_binding());
            assert!(!involves_work_queue);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn wider_work_queue_filter_is_refused_against_dead_letter_stream() {
    let names = AccountNames::derive("box_limits").unwrap();
    let mut streams = shipped_streams(&names);
    let effect = streams
        .iter_mut()
        .find(|stream| stream.name == names.streams().effect)
        .unwrap();
    effect.subjects = vec![format!("ck.{}.effect.>", names.account())];

    let error = validate_streams(&streams).expect_err("work queue must not capture dead letters");
    let message = error.to_string();
    assert!(message.contains(&names.streams().effect));
    assert!(message.contains(&names.streams().effect_dead));
    assert!(message.contains("work-queue"));
}

#[test]
fn consumer_filter_must_be_a_subset_of_its_stream_binding() {
    let names = AccountNames::derive("box_limits").unwrap();
    let streams = shipped_streams(&names);
    let invalid_filter = format!("ck.{}.peer.agent_a.>", names.account());
    let consumer = ConsumerSpec {
        durable: "c_agent_a".to_owned(),
        stream: names.streams().peer.clone(),
        filter_subjects: vec![invalid_filter.clone()],
    };

    let error = validate_consumer(&consumer, &streams).expect_err("trailing > is too broad");
    let message = error.to_string();
    assert!(message.contains(&invalid_filter));
    assert!(message.contains(&names.peer_binding()));
}

#[test]
fn store_retention_cannot_be_shorter_than_stream_max_age() {
    let names = AccountNames::derive("box_limits").unwrap();
    let streams = shipped_streams(&names);
    let peer = streams
        .iter()
        .find(|stream| stream.name == names.streams().peer)
        .unwrap();

    let error = validate_store_retention(peer, HOUR * 23)
        .expect_err("23h store retention cannot cover a 24h stream");
    assert!(matches!(error, LimitError::StoreRetentionTooShort { .. }));
    validate_store_retention(peer, HOUR * 48).expect("the shipped 48h peer retention is valid");
}

#[test]
fn six_shipped_bindings_validate_and_include_the_event_stream() {
    let names = AccountNames::derive("box_limits").unwrap();
    let streams = shipped_streams(&names);
    assert_eq!(
        streams.iter().map(|stream| stream.kind).collect::<Vec<_>>(),
        vec![
            StreamKind::Room,
            StreamKind::Wake,
            StreamKind::Peer,
            StreamKind::Effect,
            StreamKind::EffectDead,
            StreamKind::Event,
        ]
    );
    validate_streams(&streams).unwrap();
}

#[test]
fn only_the_event_stream_caps_messages_per_subject() {
    let names = AccountNames::derive("box_limits").unwrap();
    let streams = names.streams();
    let spec =
        |kind, name: &str, subject: String, hours: u32, max_bytes, discard, queue| StreamSpec {
            kind,
            name: name.to_owned(),
            subjects: vec![subject],
            max_age: HOUR * hours,
            max_bytes,
            discard,
            work_queue: queue,
            max_msgs_per_subject: None,
        };
    // The five older streams keep exactly the values they had before the event
    // stream existed, with no per-subject cap, so ck-bus emits the same
    // configuration for them as before.
    let mut expected = vec![
        spec(
            StreamKind::Room,
            &streams.room,
            names.room_binding(),
            24,
            GIB,
            DiscardPolicy::Old,
            false,
        ),
        spec(
            StreamKind::Wake,
            &streams.wake,
            names.wake_binding(),
            24,
            GIB,
            DiscardPolicy::Old,
            false,
        ),
        spec(
            StreamKind::Peer,
            &streams.peer,
            names.peer_binding(),
            24,
            GIB,
            DiscardPolicy::Old,
            false,
        ),
        spec(
            StreamKind::Effect,
            &streams.effect,
            names.effect_binding(),
            24,
            256 * MIB,
            DiscardPolicy::New,
            true,
        ),
        spec(
            StreamKind::EffectDead,
            &streams.effect_dead,
            names.effect_dead(),
            24 * 7,
            64 * MIB,
            DiscardPolicy::Old,
            false,
        ),
    ];
    // The event stream: a week, 1 GiB, discard old, at most 10,000 messages on
    // any one subject, and not a work queue.
    let mut event = spec(
        StreamKind::Event,
        &streams.event,
        names.event_binding(),
        24 * 7,
        GIB,
        DiscardPolicy::Old,
        false,
    );
    event.max_msgs_per_subject = Some(10_000);
    expected.push(event);

    assert_eq!(shipped_streams(&names), expected);
}

#[test]
fn an_event_binding_overlapping_the_effect_binding_is_refused() {
    let names = AccountNames::derive("box_limits").unwrap();
    let mut streams = shipped_streams(&names);
    let overlapping = format!("ck.{}.*.*.*.intent", names.account());
    streams
        .iter_mut()
        .find(|stream| stream.kind == StreamKind::Event)
        .unwrap()
        .subjects = vec![overlapping.clone()];

    match validate_streams(&streams).expect_err("overlap with effect intents must be refused") {
        LimitError::OverlappingStreamFilters {
            left_stream,
            left_filter,
            right_stream,
            right_filter,
            involves_work_queue,
        } => {
            assert_eq!(left_stream, names.streams().effect);
            assert_eq!(left_filter, names.effect_binding());
            assert_eq!(right_stream, names.streams().event);
            assert_eq!(right_filter, overlapping);
            assert!(involves_work_queue);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn resolved_event_stores_must_outlive_the_week_long_event_stream() {
    let names = AccountNames::derive("box_limits").unwrap();
    let streams = shipped_streams(&names);
    let event = streams
        .iter()
        .find(|stream| stream.kind == StreamKind::Event)
        .unwrap();

    let error = validate_store_retention(event, HOUR * 24 * 6)
        .expect_err("a 6-day store cannot cover a 7-day event stream");
    assert!(matches!(error, LimitError::StoreRetentionTooShort { .. }));
    validate_store_retention(event, HOUR * 24 * 8).expect("an 8-day store covers it");
}

#[test]
fn partial_token_wildcards_are_not_subject_patterns() {
    let names = AccountNames::derive("box_limits").unwrap();
    let mut streams = shipped_streams(&names);
    streams[0].subjects = vec!["ck.box_limits.room.room*.post".to_owned()];
    assert!(matches!(
        validate_streams(&streams),
        Err(LimitError::InvalidSubject { .. })
    ));

    assert_eq!(HOUR, Duration::from_secs(3600));
}
