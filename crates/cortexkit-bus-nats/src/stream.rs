use std::time::Duration;

use async_nats::jetstream::consumer::{pull, Consumer};
use async_nats::jetstream::message::AckKind;
use async_trait::async_trait;
use bytes::Bytes;
use cortexkit_bus_naming::AccountNames;
use cortexkit_bus_trait::{
    BusError, BusResult, ContentDigest, DurableOwner, Headers, PublishReceipt, Stream,
    StreamCursor, StreamDelivery,
};

use crate::codec::{decode_message, encode_headers};
use crate::error::map_operation_error;
use crate::pull_delivery::{fetch_genuine, PullOrigin};
use crate::NatsConnection;

#[derive(Clone)]
pub struct NatsStream {
    connection: NatsConnection,
    stream: String,
    pull_expires: Duration,
}

impl NatsStream {
    pub(crate) fn new(connection: NatsConnection, stream: String, pull_expires: Duration) -> Self {
        Self {
            connection,
            stream,
            pull_expires,
        }
    }

    pub fn stream_name(&self) -> &str {
        &self.stream
    }
}

pub struct NatsStreamCursor {
    connection: NatsConnection,
    consumer: Consumer<pull::Config>,
    origin: PullOrigin,
    pull_expires: Duration,
    in_flight: Option<async_nats::jetstream::Message>,
}

/// A message pulled from this cursor's durable by [`NatsStreamCursor::next_or_undecodable`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Pulled {
    /// A successfully decoded delivery, in flight until settled.
    Delivery(StreamDelivery),
    /// A message from this cursor's own durable whose bus headers could not be
    /// decoded. It stays in flight: settle it with `ack`, `nak`, or `term`
    /// before pulling again.
    Undecodable(Undecodable),
}

/// Metadata and the bus header error for an undecodable message from this cursor's durable.
#[derive(Debug)]
#[non_exhaustive]
pub struct Undecodable {
    pub stream_seq: u64,
    pub delivery_count: u64,
    pub subject: String,
    pub error: BusError,
}

#[async_trait]
impl Stream for NatsStream {
    type Cursor = NatsStreamCursor;

    async fn publish(
        &self,
        subject: &str,
        id: &str,
        digest: ContentDigest,
        headers: Headers,
    ) -> BusResult<PublishReceipt> {
        let headers = encode_headers(id, digest, headers)?;
        let mut events = self.connection.event_receiver();
        let ack = self
            .connection
            .jetstream()
            .publish_with_headers(subject.to_owned(), headers, Bytes::new())
            .await
            .map_err(|error| map_operation_error(subject, error, &mut events))?
            .await
            .map_err(|error| map_operation_error(subject, error, &mut events))?;
        Ok(PublishReceipt::new(ack.sequence, ack.duplicate))
    }

    // Binding is independent of retention: WorkQueue streams may use this
    // cursor with unlimited server delivery and application-owned termination.
    async fn consumer(&self, owner: DurableOwner<'_>) -> BusResult<Self::Cursor> {
        let durable = durable_name(owner)?;
        let mut events = self.connection.event_receiver();
        let consumer = self
            .connection
            .jetstream()
            .get_consumer_from_stream::<pull::Config, _, _>(&durable, &self.stream)
            .await
            .map_err(|error| map_operation_error(&durable, error, &mut events))?;
        let origin = PullOrigin::bind(&self.connection, &consumer, &self.stream, &durable).await?;
        Ok(NatsStreamCursor {
            connection: self.connection.clone(),
            consumer,
            origin,
            pull_expires: self.pull_expires,
            in_flight: None,
        })
    }
}

/// The owner's durable name, `c_{agent_id}` or `m_{module_id}`, from
/// `AccountNames::consumer_name` and `AccountNames::module_consumer_name`.
/// Both refuse an id that is not one plain subject token, since a dot or a
/// wildcard in it would address some other consumer.
fn durable_name(owner: DurableOwner<'_>) -> BusResult<String> {
    let (id, name) = match owner {
        DurableOwner::Agent(agent_id) => (agent_id, AccountNames::consumer_name(agent_id)),
        DurableOwner::Module(module_id) => {
            (module_id, AccountNames::module_consumer_name(module_id))
        }
    };
    name.map_err(|error| BusError::denied(id, error.to_string()))
}

#[async_trait]
impl StreamCursor for NatsStreamCursor {
    async fn next(&mut self) -> BusResult<Option<StreamDelivery>> {
        match self.next_or_undecodable().await? {
            Some(Pulled::Delivery(delivery)) => Ok(Some(delivery)),
            Some(Pulled::Undecodable(undecodable)) => {
                // Leave the message unsettled so a reader that understands its
                // headers can process it on redelivery.
                self.in_flight = None;
                Err(undecodable.error)
            }
            None => Ok(None),
        }
    }

    async fn ack(&mut self) -> BusResult<()> {
        self.ack_with(AckKind::Ack).await
    }

    async fn nak(&mut self, delay: Duration) -> BusResult<()> {
        self.ack_with(AckKind::Nak(Some(delay))).await
    }

    async fn term(&mut self, reason: Option<&str>) -> BusResult<()> {
        match reason {
            None | Some("") => self.ack_with(AckKind::Term).await,
            // async-nats 0.50 has no `+TERM <reason>` ack kind, so the reason
            // form is sent on the delivery's reply subject directly, the same
            // request-and-wait exchange `double_ack_with` performs.
            Some(reason) => self.term_with_reason(reason).await,
        }
    }

    async fn in_progress(&mut self) -> BusResult<()> {
        let message = self
            .in_flight
            .as_ref()
            .ok_or_else(|| BusError::absent("in-flight stream delivery"))?;
        let mut events = self.connection.event_receiver();
        // The delivery stays in flight: a progress report only restarts the
        // ack wait, and the handler still has to settle it.
        message
            .double_ack_with(AckKind::Progress)
            .await
            .map_err(|error| map_operation_error("delivery progress", error, &mut events))
    }
}

impl NatsStreamCursor {
    /// Sets the expiry of each server fetch on this cursor, overriding the
    /// connection's request timeout as the default. Zero is clamped to one
    /// nanosecond, the smallest positive expiry, so the fetch stays bounded.
    pub fn with_pull_expires(mut self, expires: Duration) -> Self {
        self.pull_expires = expires.max(Duration::from_nanos(1));
        self
    }

    /// Pulls a verified message from this cursor's durable, or returns `None`
    /// if the fetch expires. Messages from other origins are dropped without
    /// a reply, just as in [`StreamCursor::next`].
    ///
    /// Unlike `next`, this method keeps a message with undecodable bus headers
    /// in flight. Both returned variants must be settled with `ack`, `nak`,
    /// or `term` before another pull. A pull while a message is in flight
    /// returns an error.
    pub async fn next_or_undecodable(&mut self) -> BusResult<Option<Pulled>> {
        if self.in_flight.is_some() {
            return Err(BusError::unavailable(
                "the current delivery must be acknowledged, negatively acknowledged or terminated",
            ));
        }
        let Some(genuine) = fetch_genuine(
            &self.connection,
            &self.consumer,
            &self.origin,
            self.pull_expires,
            "consumer pull",
        )
        .await?
        else {
            return Ok(None);
        };
        let message = genuine.message;
        let stream_seq = genuine.stream_sequence;
        // The server counts deliveries from 1, so a negative count cannot
        // occur on a real delivery.
        let delivery_count = u64::try_from(genuine.delivered).unwrap_or(0);
        let pulled = match decode_message(&message) {
            Ok(message) => Pulled::Delivery(StreamDelivery {
                stream_seq,
                delivery_count,
                message,
            }),
            Err(error) => Pulled::Undecodable(Undecodable {
                stream_seq,
                delivery_count,
                subject: message.subject.to_string(),
                error,
            }),
        };
        self.in_flight = Some(message);
        Ok(Some(pulled))
    }

    async fn ack_with(&mut self, kind: AckKind) -> BusResult<()> {
        let message = self
            .in_flight
            .as_ref()
            .ok_or_else(|| BusError::absent("in-flight stream delivery"))?;
        let mut events = self.connection.event_receiver();
        // Wait for the server's reply, not just the client's send queue, so the
        // acknowledgement survives the caller exiting right after it.
        message
            .double_ack_with(kind)
            .await
            .map_err(|error| map_operation_error("delivery acknowledgement", error, &mut events))?;
        self.in_flight = None;
        Ok(())
    }

    async fn term_with_reason(&mut self, reason: &str) -> BusResult<()> {
        let message = self
            .in_flight
            .as_ref()
            .ok_or_else(|| BusError::absent("in-flight stream delivery"))?;
        let reply = message
            .reply
            .clone()
            .ok_or_else(|| BusError::absent("reply subject of the in-flight stream delivery"))?;
        let mut events = self.connection.event_receiver();
        self.connection
            .client()
            .request(reply, Bytes::from(format!("+TERM {reason}")))
            .await
            .map_err(|error| map_operation_error("delivery termination", error, &mut events))?;
        self.in_flight = None;
        Ok(())
    }
}
