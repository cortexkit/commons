use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cortexkit_bus_inmemory::{InMemoryBus, InMemoryConfig, DEFAULT_DEDUPLICATION_WINDOW};
use cortexkit_bus_trait::{ContentDigest, DurableOwner, Headers, Stream, StreamCursor};

const SUBJECT: &str = "ck.box.peer.agent.s.deliver";
const OWNER: DurableOwner<'static> = DurableOwner::Agent("agent");

fn fixture(window: Duration) -> (InMemoryBus, Arc<Mutex<Instant>>) {
    let clock = Arc::new(Mutex::new(Instant::now()));
    let read_clock = clock.clone();
    let bus = InMemoryBus::new_with_deduplication_clock(
        InMemoryConfig::default()
            .with_durable(OWNER)
            .with_deduplication_window(window),
        move || *read_clock.lock().expect("clock poisoned"),
    );
    (bus, clock)
}

#[tokio::test]
async fn duplicate_id_returns_original_sequence_and_stores_only_once() {
    let (bus, _) = fixture(DEFAULT_DEDUPLICATION_WINDOW);
    let first_digest = ContentDigest::of_bytes(b"first");
    let first = bus
        .publish(SUBJECT, "one", first_digest, Headers::new())
        .await
        .unwrap();
    let duplicate = bus
        .clone()
        .publish(
            "ck.box.peer.agent.other.deliver",
            "one",
            ContentDigest::of_bytes(b"different message"),
            Headers::new(),
        )
        .await
        .unwrap();
    assert!(!first.duplicate());
    assert!(duplicate.duplicate());
    assert_eq!(first.stream_sequence(), duplicate.stream_sequence());
    let mut cursor = bus.consumer(OWNER).await.unwrap();
    let delivered = cursor.next().await.unwrap().unwrap();
    assert_eq!(delivered.stream_seq, first.stream_sequence());
    assert_eq!(delivered.message.digest, first_digest);
    cursor.ack().await.unwrap();
    assert!(cursor.next().await.unwrap().is_none());
}

#[tokio::test]
async fn distinct_ids_are_stored_with_distinct_sequences() {
    let (bus, _) = fixture(DEFAULT_DEDUPLICATION_WINDOW);
    let digest = ContentDigest::of_bytes(b"same content");
    let first = bus
        .publish(SUBJECT, "one", digest, Headers::new())
        .await
        .unwrap();
    let second = bus
        .publish(SUBJECT, "two", digest, Headers::new())
        .await
        .unwrap();
    assert!(!first.duplicate());
    assert!(!second.duplicate());
    assert!(second.stream_sequence() > first.stream_sequence());
    let mut cursor = bus.consumer(OWNER).await.unwrap();
    for id in ["one", "two"] {
        assert_eq!(cursor.next().await.unwrap().unwrap().message.id, id);
        cursor.ack().await.unwrap();
    }
    assert!(cursor.next().await.unwrap().is_none());
}

#[tokio::test]
async fn configured_window_expires_without_sleeping() {
    let window = Duration::from_secs(7);
    let (bus, clock) = fixture(window);
    let digest = ContentDigest::of_bytes(b"content");
    for id in ["one", "two"] {
        let receipt = bus
            .publish(SUBJECT, id, digest, Headers::new())
            .await
            .unwrap();
        assert!(!receipt.duplicate());
    }
    *clock.lock().unwrap() += window;
    for (index, id) in ["one", "two"].into_iter().enumerate() {
        let receipt = bus
            .publish(SUBJECT, id, digest, Headers::new())
            .await
            .unwrap();
        assert!(!receipt.duplicate());
        assert_eq!(receipt.stream_sequence(), 3 + index as u64);
    }
    let mut cursor = bus.consumer(OWNER).await.unwrap();
    for id in ["one", "two", "one", "two"] {
        assert_eq!(cursor.next().await.unwrap().unwrap().message.id, id);
        cursor.ack().await.unwrap();
    }
    assert!(cursor.next().await.unwrap().is_none());
}

#[tokio::test]
async fn default_window_is_two_minutes_and_duplicates_do_not_extend_it() {
    let clock = Arc::new(Mutex::new(Instant::now()));
    let read_clock = clock.clone();
    let bus = InMemoryBus::new_with_deduplication_clock(InMemoryConfig::default(), move || {
        *read_clock.lock().unwrap()
    });
    let digest = ContentDigest::of_bytes(b"content");
    let first = bus
        .publish(SUBJECT, "one", digest, Headers::new())
        .await
        .unwrap();
    assert!(!first.duplicate());
    *clock.lock().unwrap() += Duration::from_secs(119);
    let duplicate = bus
        .publish(SUBJECT, "one", digest, Headers::new())
        .await
        .unwrap();
    assert!(duplicate.duplicate());
    assert_eq!(duplicate.stream_sequence(), first.stream_sequence());
    *clock.lock().unwrap() += Duration::from_secs(1);
    let fresh = bus
        .publish(SUBJECT, "one", digest, Headers::new())
        .await
        .unwrap();
    assert!(!fresh.duplicate());
    assert!(fresh.stream_sequence() > first.stream_sequence());
}

#[tokio::test]
async fn separate_streams_do_not_share_seen_ids() {
    let (first_stream, _) = fixture(DEFAULT_DEDUPLICATION_WINDOW);
    let (second_stream, _) = fixture(DEFAULT_DEDUPLICATION_WINDOW);
    let digest = ContentDigest::of_bytes(b"content");
    for bus in [first_stream, second_stream] {
        let receipt = bus
            .publish(SUBJECT, "one", digest, Headers::new())
            .await
            .unwrap();
        assert!(!receipt.duplicate());
        assert_eq!(receipt.stream_sequence(), 1);
    }
}
