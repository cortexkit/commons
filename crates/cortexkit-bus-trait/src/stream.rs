use std::time::Duration;

use async_trait::async_trait;

use crate::{BusResult, ContentDigest, Headers, Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishAck {
    pub stream_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamDelivery {
    pub stream_seq: u64,
    pub message: Message,
}

/// A cursor for an already-created durable, selected by its associated registry identity.
#[async_trait]
pub trait StreamCursor: Send {
    async fn next(&mut self) -> BusResult<Option<StreamDelivery>>;
    async fn ack(&mut self) -> BusResult<()>;
    async fn nak(&mut self, delay: Duration) -> BusResult<()>;
}

#[async_trait]
pub trait Stream: Send + Sync {
    type Cursor: StreamCursor;

    async fn publish(
        &self,
        subject: &str,
        id: &str,
        digest: ContentDigest,
        headers: Headers,
    ) -> BusResult<PublishAck>;

    /// Attaches to a durable created separately; this client cannot create workload consumers.
    async fn consumer(&self, durable: &str, identity: &str) -> BusResult<Self::Cursor>;
}
