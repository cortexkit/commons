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
        let routing = specs
            .iter()
            .find(|spec| spec.consumer.durable == "m_prefrontal-routing")
            .unwrap();
        assert_eq!(
            routing.consumer.filter_subjects,
            [format!("ck.{account}.event.insula.>")]
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
        for durable in ["m_thalamus-harness", "m_thalamus", "m_broca"] {
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
fn routing_durable_filters_are_only_insula() {
    let routing = EVENT_CONSUMERS
        .iter()
        .find(|reader| reader.module_id == "prefrontal-routing")
        .unwrap();
    assert_eq!(routing.filter_subjects, ["event.insula.>"]);
}

#[test]
fn routing_live_hint_subscriptions_include_insula_and_exact_step_hints() {
    use cortexkit_bus_naming::{live_hint_subscriptions, LIVE_HINT_READERS};
    let account = AccountNames::derive("box_hints").unwrap();
    assert_eq!(
        live_hint_subscriptions(&account, "prefrontal-routing").unwrap(),
        [
            "ck.box_hints.live.broca.step.finished.v1",
            "ck.box_hints.live.insula.>",
            "ck.box_hints.live.thalamus-harness.step.finished.v1",
        ]
    );
    let routing = LIVE_HINT_READERS
        .iter()
        .find(|reader| reader.module_id == "prefrontal-routing")
        .unwrap();
    assert_eq!(
        routing.subjects,
        [
            "event.broca.step.finished.v1",
            "event.thalamus-harness.step.finished.v1",
        ]
    );
}

#[test]
fn live_hint_subscriptions_preserve_event_only_readers_and_validate_modules() {
    use cortexkit_bus_naming::{live_hint_subscriptions, TokenKind};
    let account = AccountNames::derive("box_hints").unwrap();
    assert_eq!(
        live_hint_subscriptions(&account, "astrocyte").unwrap(),
        [
            "ck.box_hints.live.broca.step.finished.v1",
            "ck.box_hints.live.insula.>",
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
fn table_entries_never_share_a_durable_name() {
    use std::collections::BTreeSet;
    assert!(EVENT_CONSUMERS.len() >= 2);
    let durables = EVENT_CONSUMERS
        .iter()
        .map(|entry| entry.durable)
        .collect::<BTreeSet<_>>();
    assert_eq!(durables.len(), EVENT_CONSUMERS.len());
}
