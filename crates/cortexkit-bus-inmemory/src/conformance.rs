use std::time::Duration;

use async_trait::async_trait;
use cortexkit_bus_trait::{
    map_connection_failure, map_credential_sign_failure, terminally_dispose, BackendKind, BusError,
    ClaimOutcome, ConformanceBackend, ConnectionFailure, ContentDigest, CredentialSignFailure,
    Headers, PropertyOutcome, PropertySpec, Register, RegisterUpdate, RegisterWatch, Stream,
    StreamCursor, WatchEvent, WorkQueue,
};

use crate::{BackendEvent, InMemoryBus, InMemoryConfig, WorkQueueConfig};

pub struct InMemoryConformance;

#[async_trait]
impl ConformanceBackend for InMemoryConformance {
    fn backend_kind(&self) -> BackendKind {
        BackendKind::InMemory
    }

    async fn probe(&self, property: PropertySpec) -> PropertyOutcome {
        let result = match property.name {
            "publish_ack_monotonic_sequence" => publish_ack_sequence().await,
            "consumer_replay_within_process_lifetime" => replay_from_cursor().await,
            "register_revision" => register_revision().await,
            "register_absent_vs_empty" => register_absent_vs_empty().await,
            "watch_snapshot_then_updates_with_connection_flag" => register_watch().await,
            "work_redelivery_count" => work_redelivery().await,
            "work_cap_exhaustion_and_dead_letter" => cap_exhaustion().await,
            "full_work_queue_names_exhausted_limit" => full_queue().await,
            "error_taxonomy_and_boundary_mapping" => error_taxonomy().await,
            other => Err(format!("in-memory has no parity probe named {other}")),
        };
        match result {
            Ok(()) => PropertyOutcome::Passed,
            Err(reason) => PropertyOutcome::Failed { reason },
        }
    }
}

async fn publish_ack_sequence() -> Result<(), String> {
    let bus = InMemoryBus::new(InMemoryConfig::default());
    let first = bus
        .publish("ck.box.room.a.post", "one", digest("one"), Headers::new())
        .await
        .map_err(debug)?;
    let second = bus
        .publish("ck.box.room.a.post", "two", digest("two"), Headers::new())
        .await
        .map_err(debug)?;
    require(
        second.stream_seq > first.stream_seq,
        "publish sequence did not increase",
    )
}

async fn replay_from_cursor() -> Result<(), String> {
    let bus = InMemoryBus::new(InMemoryConfig::default().with_durable("c_agent", "agent"));
    bus.publish(
        "ck.box.peer.agent.s.deliver",
        "m1",
        digest("m1"),
        Headers::new(),
    )
    .await
    .map_err(debug)?;
    let expected = bus
        .consumer("c_agent", "agent")
        .await
        .map_err(debug)?
        .next()
        .await
        .map_err(debug)?
        .ok_or("first cursor saw no message")?;
    let mut reopened = bus.consumer("c_agent", "agent").await.map_err(debug)?;
    let replayed = reopened
        .next()
        .await
        .map_err(debug)?
        .ok_or("reopened cursor saw no replay")?;
    require(
        expected.stream_seq == replayed.stream_seq,
        "cursor did not replay",
    )?;
    reopened.ack().await.map_err(debug)
}

async fn register_revision() -> Result<(), String> {
    let bus = InMemoryBus::new(InMemoryConfig::default());
    let first = bus.put("module", b"one".to_vec()).await.map_err(debug)?;
    let second = bus.put("module", b"two".to_vec()).await.map_err(debug)?;
    let read = bus.get("module").await.map_err(debug)?;
    require(second > first, "register revision did not increase")?;
    require(read.revision == second, "get returned the wrong revision")
}

async fn register_absent_vs_empty() -> Result<(), String> {
    let bus = InMemoryBus::new(InMemoryConfig::default());
    bus.put("empty", Vec::new()).await.map_err(debug)?;
    let empty = bus.get("empty").await.map_err(debug)?;
    require(empty.value.is_empty(), "empty value was not preserved")?;
    require(
        matches!(bus.get("missing").await, Err(BusError::Absent { .. })),
        "missing key was not Absent",
    )
}

async fn register_watch() -> Result<(), String> {
    let bus = InMemoryBus::new(InMemoryConfig::default());
    bus.put("module.a", b"v1".to_vec()).await.map_err(debug)?;
    let mut watch = bus.watch("module.").await.map_err(debug)?;
    require(
        !watch.has_snapshot(),
        "watch claimed a snapshot before delivery",
    )?;
    let snapshot = watch.next().await.map_err(debug)?;
    require(
        matches!(snapshot, WatchEvent::Snapshot { ref entries, .. } if entries.len() == 1),
        "watch did not deliver the complete initial snapshot",
    )?;
    require(
        watch.has_snapshot(),
        "watch did not record snapshot receipt",
    )?;
    bus.put("module.a", b"v2".to_vec()).await.map_err(debug)?;
    require(
        matches!(
            watch.next().await.map_err(debug)?,
            WatchEvent::Update(RegisterUpdate::Put(_))
        ),
        "watch did not distinguish an update",
    )
}

async fn work_redelivery() -> Result<(), String> {
    let bus = queue_bus(3, 4_096);
    bus.publish(
        "ck.box.effect.a.s.intent",
        "m1",
        digest("m1"),
        Headers::new(),
    )
    .await
    .map_err(debug)?;
    let first = item(bus.claim().await.map_err(debug)?)?;
    bus.nak(first.token, Duration::ZERO).await.map_err(debug)?;
    let second = item(bus.claim().await.map_err(debug)?)?;
    require(
        first.delivery_count == 1,
        "first delivery count was not one",
    )?;
    require(second.delivery_count == 2, "redelivery count was not two")
}

async fn cap_exhaustion() -> Result<(), String> {
    let bus = queue_bus(1, 4_096);
    bus.publish(
        "ck.box.effect.a.s.intent",
        "m1",
        digest("m1"),
        Headers::new(),
    )
    .await
    .map_err(debug)?;
    let first = item(bus.claim().await.map_err(debug)?)?;
    bus.nak(first.token, Duration::ZERO).await.map_err(debug)?;
    let exhausted = match bus.claim().await.map_err(debug)? {
        ClaimOutcome::MaxDeliveriesExceeded(exhausted) => exhausted,
        other => {
            return Err(format!(
                "cap did not report MaxDeliveriesExceeded: {other:?}"
            ))
        }
    };
    terminally_dispose(&bus, &bus, "box", &exhausted, digest("dead-record"))
        .await
        .map_err(debug)?;
    let events = bus.events();
    let published = events.iter().position(|event| {
        matches!(event, BackendEvent::Published { subject, .. } if subject == "ck.box.effect.dead")
    });
    let terminated = events
        .iter()
        .position(|event| matches!(event, BackendEvent::Terminated { .. }));
    require(
        published
            .zip(terminated)
            .is_some_and(|(publish, term)| publish < term),
        "dead-letter publish did not precede term",
    )
}

async fn full_queue() -> Result<(), String> {
    let bus = queue_bus(5, 150);
    bus.publish(
        "ck.box.effect.a.s.intent",
        "first",
        digest("first"),
        Headers::new(),
    )
    .await
    .map_err(debug)?;
    let error = bus
        .publish(
            "ck.box.effect.a.s.intent",
            "second",
            digest("second"),
            Headers::new(),
        )
        .await
        .expect_err("second publish should exceed max-bytes");
    require(
        matches!(error, BusError::Unavailable { exhausted_limit: Some(ref limit), .. } if limit == "max_bytes"),
        "full queue did not name max_bytes in Unavailable",
    )
}

async fn error_taxonomy() -> Result<(), String> {
    let denied_subject = "ck.box.room.denied.post";
    let bus = InMemoryBus::new(
        InMemoryConfig::default()
            .deny_subject(denied_subject)
            .with_work_queue(WorkQueueConfig {
                subject: "ck.box.effect.a.s.intent".into(),
                max_bytes: 4_096,
                max_deliveries: 5,
                max_nak_delay: Duration::from_millis(1),
            }),
    );
    require(
        matches!(bus.publish(denied_subject, "m", digest("m"), Headers::new()).await, Err(BusError::Denied { subject, .. }) if subject == denied_subject),
        "fixture denial was not Denied",
    )?;
    require(
        matches!(bus.get("missing").await, Err(BusError::Absent { .. })),
        "missing register key was not Absent",
    )?;
    bus.publish(
        "ck.box.effect.a.s.intent",
        "work",
        digest("work"),
        Headers::new(),
    )
    .await
    .map_err(debug)?;
    let claimed = item(bus.claim().await.map_err(debug)?)?;
    require(
        matches!(
            bus.nak(claimed.token, Duration::from_secs(1)).await,
            Err(BusError::Clamped {
                retry_after: Some(_),
                ..
            })
        ),
        "oversized delay was not reported Clamped",
    )?;
    require(
        matches!(
            map_connection_failure(ConnectionFailure::WriteOnSeveredSocket),
            BusError::Unavailable { .. }
        ),
        "severed write was not Unavailable",
    )?;
    require(
        matches!(
            map_connection_failure(ConnectionFailure::ServerClosedThenReauthenticationRefused {
                subject: denied_subject.into(),
                reason: "revoked".into()
            }),
            BusError::Denied { .. }
        ),
        "reconnect refusal was not Denied",
    )?;
    require(
        matches!(
            map_credential_sign_failure(CredentialSignFailure::RecordMissing {
                record: "nats.module.g1.e0".into()
            }),
            BusError::Absent { .. }
        ),
        "missing signing record was not Absent",
    )
}

fn queue_bus(max_deliveries: u32, max_bytes: usize) -> InMemoryBus {
    InMemoryBus::new(InMemoryConfig::default().with_work_queue(WorkQueueConfig {
        subject: "ck.box.effect.a.s.intent".into(),
        max_bytes,
        max_deliveries,
        max_nak_delay: Duration::from_secs(30),
    }))
}

fn item(outcome: ClaimOutcome) -> Result<cortexkit_bus_trait::WorkItem, String> {
    match outcome {
        ClaimOutcome::Item(item) => Ok(item),
        other => Err(format!("expected work item, got {other:?}")),
    }
}

fn digest(value: &str) -> ContentDigest {
    ContentDigest::of_bytes(value.as_bytes())
}

fn require(condition: bool, reason: &str) -> Result<(), String> {
    condition.then_some(()).ok_or_else(|| reason.into())
}

fn debug(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_bus_trait::{
        classify_properties, run_conformance, PropertyApplicability, PropertySet, SuiteBug,
        CONFORMANCE_PROPERTIES,
    };

    struct WeakenedNatsFixture;

    #[async_trait]
    impl ConformanceBackend for WeakenedNatsFixture {
        fn backend_kind(&self) -> BackendKind {
            BackendKind::Nats
        }

        async fn probe(&self, property: PropertySpec) -> PropertyOutcome {
            match property.set {
                PropertySet::Parity => InMemoryConformance.probe(property).await,
                PropertySet::NatsOnly => PropertyOutcome::Failed {
                    reason: "fixture deliberately has only process-local semantics".into(),
                },
            }
        }
    }

    #[tokio::test]
    async fn in_memory_passes_every_parity_property() {
        let report = run_conformance(&InMemoryConformance, PropertySet::Parity)
            .await
            .expect("parity is applicable to in-memory");

        assert_eq!(report.property_set_name, "parity");
        for (property, outcome) in &report.outcomes {
            assert_eq!(outcome, &PropertyOutcome::Passed, "{}", property.name);
        }
        assert!(report.passed());
    }

    #[tokio::test]
    async fn asserting_nats_only_against_in_memory_is_a_suite_bug() {
        let result = run_conformance(&InMemoryConformance, PropertySet::NatsOnly).await;
        assert_eq!(result, Err(SuiteBug::NatsOnlyAssertedAgainstInMemory));
    }

    #[tokio::test]
    async fn weakened_backend_passes_real_parity_probes_but_fails_nats_only() {
        let parity = run_conformance(&WeakenedNatsFixture, PropertySet::Parity)
            .await
            .expect("parity run is valid");
        let nats_only = run_conformance(&WeakenedNatsFixture, PropertySet::NatsOnly)
            .await
            .expect("NATS-only run is valid for a NATS-labelled fixture");

        assert!(parity.passed());
        assert!(!nats_only.passed());
    }

    #[test]
    fn every_nats_only_property_is_not_applicable_not_passed() {
        let classifications = classify_properties(BackendKind::InMemory);
        let nats_only = classifications
            .iter()
            .filter(|row| row.property.set == PropertySet::NatsOnly)
            .collect::<Vec<_>>();

        assert_eq!(
            nats_only.len(),
            CONFORMANCE_PROPERTIES
                .iter()
                .filter(|property| property.set == PropertySet::NatsOnly)
                .count()
        );
        assert!(nats_only.iter().all(|row| matches!(
            row.applicability,
            PropertyApplicability::NotApplicable { .. }
        )));
    }
}
