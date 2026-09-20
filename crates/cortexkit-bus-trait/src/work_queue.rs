use std::time::Duration;

use async_trait::async_trait;

use crate::{BusResult, ContentDigest, Headers, Message, PublishAck, Stream};

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

/// Publishes a persistent dead-letter record before terminating the exhausted delivery.
pub async fn terminally_dispose<S, Q>(
    stream: &S,
    queue: &Q,
    account: &str,
    exhausted: &MaxDeliveriesExceeded,
    dead_letter_digest: ContentDigest,
) -> BusResult<PublishAck>
where
    S: Stream,
    Q: WorkQueue,
{
    let record = DeadLetterRecord::from_exhaustion(exhausted);
    let ack = stream
        .publish(
            &effect_dead_subject(account),
            &record.message_id,
            dead_letter_digest,
            record.headers(),
        )
        .await?;
    queue.term(exhausted.item.token).await?;
    Ok(ack)
}
