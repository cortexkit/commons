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
            [format!("ck.{account}.event.broca.>")]
        );
        let astrocyte = specs
            .iter()
            .find(|spec| spec.consumer.durable == "m_astrocyte")
            .unwrap();
        assert_eq!(
            astrocyte.consumer.filter_subjects,
            [format!("ck.{account}.event.insula.>")]
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
                format!("ck.{account}.event.broca.compaction.>"),
            ]
        );
        for durable in ["m_thalamus-harness", "m_broca"] {
            let reader = specs
                .iter()
                .find(|spec| spec.consumer.durable == durable)
                .unwrap();
            assert_eq!(
                reader.consumer.filter_subjects,
                [format!("ck.{account}.event.magic-context.compaction.>")]
            );
        }
    }
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
