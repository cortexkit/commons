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
use futures_util::StreamExt;

use crate::codec::{decode_message, encode_headers};
use crate::error::map_operation_error;
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
    pull_expires: Duration,
    in_flight: Option<async_nats::jetstream::Message>,
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

    async fn consumer(&self, owner: DurableOwner<'_>) -> BusResult<Self::Cursor> {
        let durable = durable_name(owner)?;
        let mut events = self.connection.event_receiver();
        let consumer = self
            .connection
            .jetstream()
            .get_consumer_from_stream::<pull::Config, _, _>(&durable, &self.stream)
            .await
            .map_err(|error| map_operation_error(&durable, error, &mut events))?;
        Ok(NatsStreamCursor {
            connection: self.connection.clone(),
            consumer,
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
        if self.in_flight.is_some() {
            return Err(BusError::unavailable(
                "the current delivery must be acknowledged, negatively acknowledged or terminated",
            ));
        }
        let mut events = self.connection.event_receiver();
        let mut messages = self
            .consumer
            .fetch()
            .max_messages(1)
            .expires(self.pull_expires)
            .messages()
            .await
            .map_err(|error| map_operation_error("consumer pull", error, &mut events))?;
        let Some(message) = messages.next().await else {
            return Ok(None);
        };
        let message =
            message.map_err(|error| map_operation_error("consumer pull", error, &mut events))?;
        let info = message.info().map_err(|error| {
            map_operation_error("consumer delivery metadata", error, &mut events)
        })?;
        let delivery = StreamDelivery {
            stream_seq: info.stream_sequence,
            // `num_delivered` from the delivery's metadata; the server counts
            // from 1, so a negative value cannot occur on a real delivery.
            delivery_count: u64::try_from(info.delivered).unwrap_or(0),
            message: decode_message(&message)?,
        };
        self.in_flight = Some(message);
        Ok(Some(delivery))
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
