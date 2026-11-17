use std::time::Duration;

use cortexkit_bus_naming::{
    event_consumer_specs, flow_engine_consumer_spec, shipped_streams, validate_consumer,
    AccountNames, AckPolicy, DeliverPolicy, EVENT_CONSUMERS,
};

#[test]
fn reviewed_readers_and_basal_share_the_bootstrap_pull_configuration() {
    for account in ["box_readers", "box_second"] {
        let names = AccountNames::derive(account).unwrap();
        let specs = event_consumer_specs(&names).unwrap();
        assert_eq!(specs.len(), EVENT_CONSUMERS.len());
        let basal = flow_engine_consumer_spec(&names, "basal").unwrap();
        assert_eq!(basal.consumer.durable, "m_basal");
        assert_eq!(
            basal.consumer.filter_subjects,
            [format!("ck.{account}.event.>")]
        );
        for spec in specs.iter().chain(std::iter::once(&basal)) {
            assert!(spec.pull);
            assert_eq!(spec.ack_policy, AckPolicy::Explicit);
            assert_eq!(spec.deliver_policy, DeliverPolicy::All);
            assert_eq!(spec.ack_wait, Duration::from_secs(30));
            assert_eq!(spec.max_ack_pending, 1_000);
            assert_eq!(spec.consumer.stream, names.streams().event);
            validate_consumer(&spec.consumer, &shipped_streams(&names)).unwrap();
        }
        let reader = specs
            .iter()
            .find(|spec| spec.consumer.durable == "m_prefrontal-core")
            .unwrap();
        assert_eq!(
            reader.consumer.filter_subjects,
            [
                format!("ck.{account}.event.broca.>"),
                format!("ck.{account}.event.basal.flow_instance_created.v1"),
                format!("ck.{account}.event.basal.flow_instance_extended.v1"),
                format!("ck.{account}.event.basal.flow_instance_ended.v1"),
                format!("ck.{account}.event.basal.flow_instance_expired.v1"),
                format!("ck.{account}.event.basal.flow_instance_rescheduled.v1"),
                format!("ck.{account}.event.aft.late_result_ready.v1.prefrontal-core"),
                format!("ck.{account}.event.basal.late_result_ready.v1.prefrontal-core"),
                format!("ck.{account}.event.cerebellum.late_result_ready.v1.prefrontal-core"),
                format!("ck.{account}.event.fusiform.catalog_changed.v1"),
                format!("ck.{account}.event.basal.flow_owner_notice.v1"),
                format!("ck.{account}.event.cingulate.card_settled.v1.prefrontal-core"),
            ]
        );
        let astrocyte = specs
            .iter()
            .find(|spec| spec.consumer.durable == "m_astrocyte")
            .unwrap();
        assert_eq!(
            astrocyte.consumer.filter_subjects,
            [
                format!("ck.{account}.event.insula.>"),
                format!("ck.{account}.event.broca.step.finished.v1"),
                format!("ck.{account}.event.thalamus-harness.step.finished.v1"),
                format!("ck.{account}.event.thalamus.step.finished.v1"),
            ]
        );
        let mc = specs
            .iter()
            .find(|spec| spec.consumer.durable == "m_magic-context")
            .unwrap();
        assert_eq!(
            mc.consumer.filter_subjects,
            [
                format!("ck.{account}.event.thalamus-harness.compaction.>"),
                format!("ck.{account}.event.thalamus.compaction.>"),
                format!("ck.{account}.event.broca.compaction.>"),
            ]
        );
        for durable in ["m_thalamus-harness", "m_thalamus"] {
            let reader = specs
                .iter()
                .find(|spec| spec.consumer.durable == durable)
                .unwrap();
            assert_eq!(
                reader.consumer.filter_subjects,
                [format!("ck.{account}.event.magic-context.compaction.>")]
            );
        }
        let cerebellum = specs
            .iter()
            .find(|spec| spec.consumer.durable == "m_cerebellum")
            .unwrap();
        assert_eq!(
            cerebellum.consumer.filter_subjects,
            [
                format!("ck.{account}.event.thalamus-harness.turn_started.v1"),
                format!("ck.{account}.event.thalamus-harness.turn_ended.v1"),
            ]
        );
    }
}

#[test]
fn routing_live_hint_subscriptions_include_insula_and_exact_step_hints() {
    use cortexkit_bus_naming::{live_hint_subscriptions, LIVE_HINT_READERS};
    let account = AccountNames::derive("box_hints").unwrap();
    let subscriptions = live_hint_subscriptions(&account, "striatum").unwrap();
    for subject in [
        "ck.box_hints.live.broca.step.finished.v1",
        "ck.box_hints.live.insula.>",
        "ck.box_hints.live.thalamus-harness.step.finished.v1",
    ] {
        assert!(subscriptions.iter().any(|actual| actual == subject));
    }
    assert_eq!(subscriptions.len(), 10);
    let routing = LIVE_HINT_READERS
        .iter()
        .find(|reader| reader.module_id == "striatum")
        .unwrap();
    assert_eq!(
        routing.subjects,
        [
            "event.broca.step.finished.v1",
            "event.thalamus-harness.step.finished.v1"
        ]
    );
}

#[test]
fn striatum_preserves_insula_and_live_hints_while_adding_route_facts_and_catalog() {
    use cortexkit_bus_naming::LIVE_HINT_READERS;
    let striatum_events = EVENT_CONSUMERS
        .iter()
        .find(|reader| reader.module_id == "striatum")
        .unwrap();
    assert_eq!(striatum_events.durable, "m_striatum");
    assert_eq!(
        striatum_events.filter_subjects,
        [
            "event.insula.>",
            "event.broca.route_fact.v1",
            "event.thalamus-harness.route_fact.v1",
            "event.colliculus.route_fact.v1",
            "event.prefrontal-core.route_fact.v1",
            "event.prefrontal.route_fact.v1",
            "event.plexus.route_fact.v1",
            "event.fusiform.catalog_changed.v1",
        ]
    );
    let striatum_hints = LIVE_HINT_READERS
        .iter()
        .find(|reader| reader.module_id == "striatum")
        .unwrap();
    assert_eq!(
        striatum_hints.subjects,
        [
            "event.broca.step.finished.v1",
            "event.thalamus-harness.step.finished.v1"
        ]
    );
}

#[test]
fn other_modules_read_both_prefrontal_publishers_in_event_and_live_subjects() {
    use cortexkit_bus_naming::live_hint_subscriptions;

    let account = AccountNames::derive("box_rename").unwrap();
    let specs = event_consumer_specs(&account).unwrap();
    for (module, durable, suffix) in [
        ("striatum", "m_striatum", "route_fact.v1"),
        ("broca", "m_broca", "late_result_ready.v1.broca"),
        (
            "thalamus-harness",
            "m_thalamus-harness_late_results",
            "late_result_ready.v1.thalamus-harness",
        ),
        (
            "astrocyte",
            "m_astrocyte_attribution",
            "attribution_fact.v1",
        ),
        ("plexus", "m_plexus", "agent_changed.v1"),
        ("wernicke", "m_wernicke_agents", "agent_changed.v1"),
        ("colliculus", "m_colliculus_events", "board_changed.v1"),
    ] {
        let spec = specs
            .iter()
            .find(|spec| spec.consumer.durable == durable)
            .unwrap();
        let live = live_hint_subscriptions(&account, module).unwrap();
        for publisher in ["prefrontal-core", "prefrontal"] {
            assert!(
                spec.consumer
                    .filter_subjects
                    .contains(&format!("ck.box_rename.event.{publisher}.{suffix}")),
                "{durable} must read {publisher}.{suffix}"
            );
            assert!(
                live.contains(&format!("ck.box_rename.live.{publisher}.{suffix}")),
                "{module} must observe live hints from {publisher}.{suffix}"
            );
        }
    }
}

#[test]
fn live_hint_subscriptions_preserve_event_only_readers_and_validate_modules() {
    use cortexkit_bus_naming::{live_hint_subscriptions, TokenKind};
    let account = AccountNames::derive("box_hints").unwrap();
    assert_eq!(
        live_hint_subscriptions(&account, "astrocyte").unwrap(),
        [
            "ck.box_hints.live.basal.attribution_fact.v1",
            "ck.box_hints.live.broca.step.finished.v1",
            "ck.box_hints.live.colliculus.usage_recorded.v1",
            "ck.box_hints.live.entorhinal.placement_fact.v1",
            "ck.box_hints.live.fusiform.catalog_changed.v1",
            "ck.box_hints.live.insula.>",
            "ck.box_hints.live.magic-context.attribution_fact.v1",
            "ck.box_hints.live.prefrontal-core.attribution_fact.v1",
            "ck.box_hints.live.prefrontal.attribution_fact.v1",
            "ck.box_hints.live.thalamus-harness.step.finished.v1",
            "ck.box_hints.live.thalamus.step.finished.v1",
        ]
    );
    for module in ["", "*", ">", "reader.module"] {
        assert_eq!(
            live_hint_subscriptions(&account, module)
                .unwrap_err()
                .kind(),
            TokenKind::ModuleId
        );
    }
}

#[test]
fn live_hint_subscriptions_are_empty_for_an_unlisted_module() {
    use cortexkit_bus_naming::live_hint_subscriptions;
    let account = AccountNames::derive("box_hints").unwrap();
    assert!(live_hint_subscriptions(&account, "unlisted")
        .unwrap()
        .is_empty());
}

#[test]
fn retired_runner_is_absent_from_reader_tables_and_permission_golden() {
    use cortexkit_bus_naming::LIVE_HINT_READERS;
    const RETIRED: &str = "thalamus-pi";
    for reader in EVENT_CONSUMERS {
        assert!(!reader.module_id.contains(RETIRED));
        assert!(!reader.durable.contains(RETIRED));
        assert!(reader.filter_subjects.iter().all(|s| !s.contains(RETIRED)));
    }
    for reader in LIVE_HINT_READERS {
        assert!(!reader.module_id.contains(RETIRED));
        assert!(reader.subjects.iter().all(|s| !s.contains(RETIRED)));
    }
    assert!(!include_str!("grants/permission_golden.txt").contains(RETIRED));
}

#[test]
fn retired_payer_reader_is_absent() {
    assert!(EVENT_CONSUMERS
        .iter()
        .all(|reader| reader.durable != "m_astrocyte_payer"));
}

#[test]
fn table_entries_never_share_a_durable_name() {
    use std::collections::BTreeSet;
    assert!(EVENT_CONSUMERS.len() >= 2);
    let durables = EVENT_CONSUMERS
        .iter()
        .map(|entry| entry.durable)
        .collect::<BTreeSet<_>>();
    assert_eq!(durables.len(), EVENT_CONSUMERS.len());
    for entry in EVENT_CONSUMERS {
        assert_eq!(entry.durable_name().unwrap(), entry.durable);
        for other in EVENT_CONSUMERS
            .iter()
            .filter(|other| other.module_id != entry.module_id)
        {
            let base = format!("m_{}", other.module_id);
            assert_ne!(entry.durable, base);
            assert!(
                !entry.durable.starts_with(&format!("{base}_")),
                "{} names {}'s durable",
                entry.module_id,
                other.module_id
            );
        }
    }
}

#[test]
fn new_reader_rows_pin_exact_subjects_and_explicit_durable_names() {
    let expected: &[(&str, &str, &[&str])] = &[
        (
            "thalamus-harness",
            "m_thalamus-harness_late_results",
            &[
                "event.aft.late_result_ready.v1.thalamus-harness",
                "event.cerebellum.late_result_ready.v1.thalamus-harness",
                "event.basal.late_result_ready.v1.thalamus-harness",
                "event.prefrontal-core.late_result_ready.v1.thalamus-harness",
                "event.prefrontal.late_result_ready.v1.thalamus-harness",
            ],
        ),
        (
            "astrocyte",
            "m_astrocyte_media_usage",
            &["event.colliculus.usage_recorded.v1"],
        ),
        (
            "astrocyte",
            "m_astrocyte_catalog",
            &["event.fusiform.catalog_changed.v1"],
        ),
        (
            "astrocyte",
            "m_astrocyte_attribution",
            &[
                "event.prefrontal-core.attribution_fact.v1",
                "event.prefrontal.attribution_fact.v1",
                "event.magic-context.attribution_fact.v1",
                "event.entorhinal.placement_fact.v1",
                "event.basal.attribution_fact.v1",
            ],
        ),
        (
            "aft",
            "m_aft_sendstate",
            &[
                "event.thalamus-harness.send_state.v1.aft",
                "event.broca.send_state.v1.aft",
            ],
        ),
        (
            "aft",
            "m_aft_cards",
            &["event.cingulate.card_settled.v1.aft"],
        ),
        (
            "plexus",
            "m_plexus",
            &[
                "event.prefrontal-core.agent_changed.v1",
                "event.prefrontal.agent_changed.v1",
                "event.cingulate.card_settled.v1.plexus",
            ],
        ),
        (
            "wernicke",
            "m_wernicke_agents",
            &[
                "event.prefrontal-core.agent_changed.v1",
                "event.prefrontal.agent_changed.v1",
            ],
        ),
        (
            "colliculus",
            "m_colliculus_events",
            &[
                "event.fusiform.catalog_changed.v1",
                "event.prefrontal-core.board_changed.v1",
                "event.prefrontal.board_changed.v1",
                "event.magic-context.memory_changed.v1",
                "event.thalamus-harness.turn_ended.v1",
                "event.broca.run.ended.v1",
            ],
        ),
        (
            "occipital",
            "m_occipital",
            &[
                "event.thalamus-harness.send_state.v1.occipital",
                "event.broca.send_state.v1.occipital",
                "event.cerebellum.display_record_ready.v1",
                "event.aft.display_record_ready.v1",
                "event.fusiform.catalog_changed.v1",
            ],
        ),
        (
            "broca",
            "m_broca",
            &[
                "event.magic-context.compaction.>",
                "event.aft.late_result_ready.v1.broca",
                "event.cerebellum.late_result_ready.v1.broca",
                "event.prefrontal-core.late_result_ready.v1.broca",
                "event.prefrontal.late_result_ready.v1.broca",
                "event.basal.late_result_ready.v1.broca",
                "event.fusiform.catalog_changed.v1",
                "event.cingulate.card_settled.v1.broca",
            ],
        ),
        (
            "magic-context",
            "m_magic-context_runs",
            &[
                "event.thalamus-harness.turn_ended.v1",
                "event.broca.run.ended.v1",
            ],
        ),
    ];
    let names = AccountNames::derive("box_readers").unwrap();
    let specs = event_consumer_specs(&names).unwrap();
    assert_eq!(EVENT_CONSUMERS.len(), 19);
    for (module, durable, filters) in expected {
        let row = EVENT_CONSUMERS
            .iter()
            .find(|row| row.durable == *durable)
            .unwrap();
        assert_eq!(row.module_id, *module);
        assert_eq!(row.filter_subjects, *filters);
        let spec = specs
            .iter()
            .find(|spec| spec.consumer.durable == *durable)
            .unwrap();
        assert_eq!(
            spec.consumer.filter_subjects,
            filters
                .iter()
                .map(|filter| format!("ck.box_readers.{filter}"))
                .collect::<Vec<_>>()
        );
    }
    for row in EVENT_CONSUMERS {
        for filter in row.filter_subjects {
            if !filter.contains(['*', '>']) {
                names
                    .parse_event_subject(&format!("ck.box_readers.{filter}"))
                    .unwrap();
            }
        }
    }
}

#[test]
fn steps_committed_is_a_versioned_live_only_hint() {
    use cortexkit_bus_naming::{live_hint_subscriptions, LIVE_HINT_READERS};
    let names = AccountNames::derive("box_hints").unwrap();
    let reader = LIVE_HINT_READERS
        .iter()
        .find(|row| row.module_id == "magic-context")
        .unwrap();
    assert_eq!(reader.subjects, ["event.astrocyte.steps_committed.v1"]);
    assert!(live_hint_subscriptions(&names, "magic-context")
        .unwrap()
        .contains(&"ck.box_hints.live.astrocyte.steps_committed.v1".to_owned()));
    for row in LIVE_HINT_READERS {
        for subject in row.subjects {
            names
                .parse_event_subject(&format!("ck.box_hints.{subject}"))
                .unwrap();
        }
    }
    assert!(!EVENT_CONSUMERS
        .iter()
        .filter(|row| row.module_id == "magic-context")
        .flat_map(|row| row.filter_subjects)
        .any(|subject| subject.contains("steps_committed")));
}
