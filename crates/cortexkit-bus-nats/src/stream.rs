use std::time::Duration;

use async_nats::jetstream::consumer::{pull, Consumer};
use async_nats::jetstream::message::AckKind;
use async_trait::async_trait;
use bytes::Bytes;
use cortexkit_bus_trait::{
    BusError, BusResult, ContentDigest, Headers, PublishAck, Stream, StreamCursor, StreamDelivery,
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
    ) -> BusResult<PublishAck> {
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
        Ok(PublishAck {
            stream_seq: ack.sequence,
        })
    }

    async fn consumer(&self, durable: &str, identity: &str) -> BusResult<Self::Cursor> {
        let expected = format!("c_{identity}");
        if durable != expected {
            return Err(BusError::denied(
                durable,
                format!("durable for identity {identity} must be named {expected}"),
            ));
        }
        let mut events = self.connection.event_receiver();
        let consumer = self
            .connection
            .jetstream()
            .get_consumer_from_stream::<pull::Config, _, _>(durable, &self.stream)
            .await
            .map_err(|error| map_operation_error(durable, error, &mut events))?;
        Ok(NatsStreamCursor {
            connection: self.connection.clone(),
            consumer,
            pull_expires: self.pull_expires,
            in_flight: None,
        })
    }
}

#[async_trait]
impl StreamCursor for NatsStreamCursor {
    async fn next(&mut self) -> BusResult<Option<StreamDelivery>> {
        if self.in_flight.is_some() {
            return Err(BusError::unavailable(
                "the current delivery must be acknowledged or negatively acknowledged",
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
}
