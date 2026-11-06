use std::time::Duration;

use async_trait::async_trait;

use crate::{BusResult, ContentDigest, Headers, Message, PublishReceipt, Stream};

pub const DEAD_LETTER_REASON_MAX_DELIVERIES: &str = "max_deliveries_exceeded";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeliveryToken(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItem {
    pub token: DeliveryToken,
    pub message: Message,
    pub delivery_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaxDeliveriesExceeded {
    pub item: WorkItem,
    pub max_deliveries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Item(WorkItem),
    Empty,
    MaxDeliveriesExceeded(MaxDeliveriesExceeded),
}

/// A claimant on a bounded work queue.
///
/// `ack`, `nak` and `term` return `Ok` only once the server has recorded the
/// decision, so a claimant may exit straight after one without losing it.
#[async_trait]
pub trait WorkQueue: Send + Sync {
    async fn claim(&self) -> BusResult<ClaimOutcome>;
    async fn ack(&self, token: DeliveryToken) -> BusResult<()>;
    async fn nak(&self, token: DeliveryToken, delay: Duration) -> BusResult<()>;
    async fn term(&self, token: DeliveryToken) -> BusResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterRecord {
    pub original_subject: String,
    pub message_id: String,
    pub digest: ContentDigest,
    pub delivery_count: u32,
    pub reason: String,
}

impl DeadLetterRecord {
    pub fn from_exhaustion(exhausted: &MaxDeliveriesExceeded) -> Self {
        Self {
            original_subject: exhausted.item.message.subject.clone(),
            message_id: exhausted.item.message.id.clone(),
            digest: exhausted.item.message.digest,
            delivery_count: exhausted.item.delivery_count,
            reason: DEAD_LETTER_REASON_MAX_DELIVERIES.into(),
        }
    }

    pub fn headers(&self) -> Headers {
        Headers::from([
            ("original-subject".into(), self.original_subject.clone()),
            ("message-id".into(), self.message_id.clone()),
            ("content-digest".into(), self.digest.to_string()),
            ("delivery-count".into(), self.delivery_count.to_string()),
            ("reason".into(), self.reason.clone()),
        ])
    }
}

pub fn effect_dead_subject(account: &str) -> String {
    format!("ck.{account}.effect.dead")
}

/// The business family, not the transport's retention policy, selects the
/// diagnostic stream. Non-session effects still use the effect dead-letter lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeliveryFamily {
    Effect,
    Inbox,
    Room,
    Wake,
    Peer,
    Consult,
    Work,
    Campaign,
    Hire,
    Spec,
    Push,
}

impl DeliveryFamily {
    /// `account` must already satisfy the bus naming account-token rules.
    pub fn dead_subject(self, account: &str) -> String {
        match self {
            Self::Effect => effect_dead_subject(account),
            _ => format!("ck.{account}.delivery.dead"),
        }
    }
}

/// Persistently publishes a cursor's terminal diagnostic before terminating it.
///
/// Call only after recording terminal business disposition. On redelivery, check
/// that disposition first and repeat this helper, never the work. The diagnostic
/// id includes the original subject and message id to avoid cross-family id
/// collisions in the shared stream. Broker deduplication applies within the
/// configured duplicate window; lifetime idempotency belongs to the business store.
///
/// With unlimited server delivery and a terminal receipt checked before work,
/// repeated crashes cannot exhaust terminalization retries or rerun the work.
/// Retry this helper until term succeeds: a crash gap longer than the 120-second
/// EFFECT_DEAD / DELIVERY_DEAD window (`DEAD_LETTER_DUPLICATE_WINDOW` in bus-naming)
/// may append a second diagnostic with the same deterministic id. Command-stream
/// relay republishes use the server default window; durable acceptance records
/// keyed by command id and checked before work, not broker duplicate history,
/// are the authoritative guard against duplicate execution.
pub async fn terminally_dispose_cursor<S, C>(
    stream: &S,
    cursor: &mut C,
    account: &str,
    family: DeliveryFamily,
    delivery: &crate::StreamDelivery,
    reason: &str,
    dead_letter_digest: ContentDigest,
) -> BusResult<PublishReceipt>
where
    S: Stream,
    C: crate::StreamCursor,
{
    let message = &delivery.message;
    let id = format!(
        "{}:{}:{}",
        message.subject.len(),
        message.subject,
        message.id
    );
    let headers = Headers::from([
        ("original-subject".into(), message.subject.clone()),
        ("message-id".into(), message.id.clone()),
        ("content-digest".into(), message.digest.to_string()),
        ("delivery-count".into(), delivery.delivery_count.to_string()),
        ("reason".into(), reason.into()),
    ]);
    let receipt = stream
        .publish(
            &family.dead_subject(account),
            &id,
            dead_letter_digest,
            headers,
        )
        .await?;
    cursor.term(Some(reason)).await?;
    Ok(receipt)
}

/// Publishes a persistent dead-letter record before terminating the exhausted delivery.
/// Returns the publish receipt, including duplicates, as described by [`Stream::publish`].
/// A duplicate is also followed by termination; use a separate dead-letter stream
/// so its message ids do not collide with the original work items.
pub async fn terminally_dispose<S, Q>(
    stream: &S,
    queue: &Q,
    account: &str,
    exhausted: &MaxDeliveriesExceeded,
    dead_letter_digest: ContentDigest,
) -> BusResult<PublishReceipt>
where
    S: Stream,
    Q: WorkQueue,
{
    let record = DeadLetterRecord::from_exhaustion(exhausted);
    let receipt = stream
        .publish(
            &effect_dead_subject(account),
            &record.message_id,
            dead_letter_digest,
            record.headers(),
        )
        .await?;
    queue.term(exhausted.item.token).await?;
    Ok(receipt)
}

#[cfg(test)]
mod cursor_disposal_tests {
    use super::*;
    use crate::{BusError, DurableOwner, StreamCursor, StreamDelivery};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingCursor(Arc<Mutex<Vec<String>>>);

    #[async_trait]
    impl StreamCursor for RecordingCursor {
        async fn next(&mut self) -> BusResult<Option<StreamDelivery>> {
            panic!("disposal must not pull work")
        }
        async fn ack(&mut self) -> BusResult<()> {
            panic!("disposal must not ack work")
        }
        async fn nak(&mut self, _: Duration) -> BusResult<()> {
            panic!("disposal must not retry work")
        }
        async fn in_progress(&mut self) -> BusResult<()> {
            panic!("disposal must not prolong work")
        }
        async fn term(&mut self, reason: Option<&str>) -> BusResult<()> {
            self.0
                .lock()
                .unwrap()
                .push(format!("term:{}", reason.unwrap()));
            Ok(())
        }
    }

    struct RecordingStream {
        events: Arc<Mutex<Vec<String>>>,
        fail_publish: bool,
    }

    #[async_trait]
    impl Stream for RecordingStream {
        type Cursor = RecordingCursor;
        async fn consumer(&self, _: DurableOwner<'_>) -> BusResult<Self::Cursor> {
            panic!("disposal must not bind work")
        }
        async fn publish(
            &self,
            subject: &str,
            id: &str,
            _: ContentDigest,
            headers: Headers,
        ) -> BusResult<PublishReceipt> {
            self.events.lock().unwrap().push(format!(
                "publish:{subject}:{id}:{}",
                headers["delivery-count"]
            ));
            if self.fail_publish {
                Err(BusError::unavailable("publish failed"))
            } else {
                Ok(PublishReceipt::new(42, true))
            }
        }
    }

    fn delivery(subject: &str) -> StreamDelivery {
        StreamDelivery {
            stream_seq: 1,
            delivery_count: u64::from(u32::MAX) + 1,
            message: Message {
                subject: subject.into(),
                id: "same-id".into(),
                digest: ContentDigest::of_bytes(b"body"),
                headers: Headers::new(),
            },
        }
    }

    #[tokio::test]
    async fn cursor_disposal_does_not_term_when_publish_fails() {
        let mut cursor = RecordingCursor::default();
        let stream = RecordingStream {
            events: cursor.0.clone(),
            fail_publish: true,
        };
        assert!(terminally_dispose_cursor(
            &stream,
            &mut cursor,
            "box",
            DeliveryFamily::Inbox,
            &delivery("ck.box.inbox.head.target.ready"),
            "terminal",
            ContentDigest::of_bytes(b"record")
        )
        .await
        .is_err());
        let events = cursor.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].starts_with("publish:ck.box.delivery.dead:"));
    }

    #[tokio::test]
    async fn cursor_disposal_routes_before_term_and_preserves_unlimited_counts() {
        for family in [
            DeliveryFamily::Effect,
            DeliveryFamily::Inbox,
            DeliveryFamily::Room,
            DeliveryFamily::Wake,
            DeliveryFamily::Peer,
            DeliveryFamily::Consult,
            DeliveryFamily::Work,
            DeliveryFamily::Campaign,
            DeliveryFamily::Hire,
            DeliveryFamily::Spec,
            DeliveryFamily::Push,
        ] {
            let mut cursor = RecordingCursor::default();
            let stream = RecordingStream {
                events: cursor.0.clone(),
                fail_publish: false,
            };
            let receipt = terminally_dispose_cursor(
                &stream,
                &mut cursor,
                "box",
                family,
                &delivery("original"),
                "terminal",
                ContentDigest::of_bytes(b"record"),
            )
            .await
            .unwrap();
            assert!(receipt.duplicate());
            assert_eq!(receipt.stream_sequence(), 42);
            let events = cursor.0.lock().unwrap();
            let expected_subject = if family == DeliveryFamily::Effect {
                "ck.box.effect.dead"
            } else {
                "ck.box.delivery.dead"
            };
            assert_eq!(
                *events,
                [
                    format!("publish:{expected_subject}:8:original:same-id:4294967296"),
                    "term:terminal".into()
                ]
            );
        }
    }

    #[tokio::test]
    async fn cursor_disposal_ids_are_stable_and_distinguish_source_subjects() {
        let mut cursor = RecordingCursor::default();
        let stream = RecordingStream {
            events: cursor.0.clone(),
            fail_publish: false,
        };
        for subject in ["source-one", "source-two", "source-one"] {
            let receipt = terminally_dispose_cursor(
                &stream,
                &mut cursor,
                "box",
                DeliveryFamily::Inbox,
                &delivery(subject),
                "terminal",
                ContentDigest::of_bytes(b"record"),
            )
            .await
            .unwrap();
            assert!(receipt.duplicate());
        }
        let events = cursor.0.lock().unwrap();
        assert_eq!(events[0], events[4]);
        assert_ne!(events[0], events[2]);
    }
}
