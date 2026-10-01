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
    /// How many times this message has been delivered to this durable,
    /// counting this delivery: 1 on the first delivery, and one more after
    /// every nak or ack-wait expiry. JetStream reports it as `num_delivered`.
    ///
    /// On a stream whose max-deliver is unlimited this is the only way a
    /// consumer can tell a message that keeps failing from a new one, so it is
    /// what a consumer uses to give up and [`StreamCursor::term`] it.
    pub delivery_count: u64,
    pub message: Message,
}

/// A cursor for an already-created durable, selected by its associated registry identity.
///
/// At most one delivery is in flight per cursor: `ack`, `nak` and `term` each
/// settle it, after which `next` may be called again. Each of them, and
/// `in_progress`, returns `Ok` only once the server has recorded it.
#[async_trait]
pub trait StreamCursor: Send {
    async fn next(&mut self) -> BusResult<Option<StreamDelivery>>;
    async fn ack(&mut self) -> BusResult<()>;
    async fn nak(&mut self, delay: Duration) -> BusResult<()>;

    /// Settles the current delivery terminally: it is not acknowledged as
    /// processed and it is never redelivered (JetStream `+TERM`). `reason`
    /// is recorded with the termination where the backend can carry one
    /// (JetStream's `+TERM <reason>`), for example `body_absent`.
    async fn term(&mut self, reason: Option<&str>) -> BusResult<()>;

    /// Reports that the current delivery is still being worked on (JetStream
    /// `+WPI`): the ack-wait clock restarts from now, so a handler slower than
    /// one ack wait is not redelivered under it. The delivery stays in flight
    /// and must still be settled with `ack`, `nak` or `term`.
    async fn in_progress(&mut self) -> BusResult<()>;
}

/// Whose durable a cursor binds: an agent or a module, with its id. The
/// backend derives the durable name from the kind and the id (`c_` or `m_`
/// followed by the id), so a cursor can bind only an agent's or a module's
/// durable and never an arbitrary consumer name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableOwner<'a> {
    /// An agent's durable on an agent stream (WAKE, PEER, EFFECT), named
    /// `c_{agent_id}`.
    Agent(&'a str),
    /// A module's durable on a non-agent stream (EVENT, or prefrontal-core's
    /// one durable on ROOM), named `m_{module_id}`.
    Module(&'a str),
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

    /// Attaches to the owner's durable, created separately; this client cannot
    /// create workload consumers.
    async fn consumer(&self, owner: DurableOwner<'_>) -> BusResult<Self::Cursor>;
}
