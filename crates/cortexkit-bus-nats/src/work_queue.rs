use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::consumer::{pull, Consumer};
use async_nats::jetstream::message::AckKind;
use async_trait::async_trait;
use cortexkit_bus_trait::{
    BusError, BusResult, ClaimOutcome, DeliveryToken, MaxDeliveriesExceeded, WorkItem, WorkQueue,
};
use futures_util::StreamExt;
use tokio::sync::Mutex;

use crate::codec::decode_message;
use crate::error::map_operation_error;
use crate::NatsConnection;

#[derive(Clone)]
pub struct NatsWorkQueue {
    connection: NatsConnection,
    consumer: Consumer<pull::Config>,
    max_deliveries: u32,
    pull_expires: Duration,
    in_flight: Arc<Mutex<HashMap<DeliveryToken, async_nats::jetstream::Message>>>,
}

impl NatsWorkQueue {
    pub(crate) async fn bind(
        connection: NatsConnection,
        stream: String,
        durable: String,
        max_deliveries: u32,
        pull_expires: Duration,
    ) -> BusResult<Self> {
        if max_deliveries == 0 {
            return Err(BusError::denied(
                durable,
                "work-queue max deliveries must be greater than zero",
            ));
        }
        let mut events = connection.event_receiver();
        let consumer = connection
            .jetstream()
            .get_consumer_from_stream::<pull::Config, _, _>(&durable, &stream)
            .await
            .map_err(|error| map_operation_error(&durable, error, &mut events))?;
        Ok(Self {
            connection,
            consumer,
            max_deliveries,
            pull_expires,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn settle(&self, token: DeliveryToken, kind: AckKind) -> BusResult<()> {
        let message = self
            .in_flight
            .lock()
            .await
            .remove(&token)
            .ok_or_else(|| BusError::absent(format!("in-flight work delivery {}", token.0)))?;
        let mut events = self.connection.event_receiver();
        if let Err(error) = message.ack_with(kind).await {
            self.in_flight.lock().await.insert(token, message);
            return Err(map_operation_error(
                "work delivery acknowledgement",
                error,
                &mut events,
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl WorkQueue for NatsWorkQueue {
    async fn claim(&self) -> BusResult<ClaimOutcome> {
        let mut events = self.connection.event_receiver();
        let mut messages = self
            .consumer
            .fetch()
            .max_messages(1)
            .expires(self.pull_expires)
            .messages()
            .await
            .map_err(|error| map_operation_error("work queue pull", error, &mut events))?;
        let Some(message) = messages.next().await else {
            return Ok(ClaimOutcome::Empty);
        };
        let message =
            message.map_err(|error| map_operation_error("work queue pull", error, &mut events))?;
        let info = message
            .info()
            .map_err(|error| map_operation_error("work delivery metadata", error, &mut events))?;
        let delivery_count = u32::try_from(info.delivered)
            .map_err(|_| BusError::unavailable("NATS returned a negative delivery count"))?;
        let token = DeliveryToken(info.stream_sequence);
        let item = WorkItem {
            token,
            message: decode_message(&message)?,
            delivery_count,
        };
        self.in_flight.lock().await.insert(token, message);
        if delivery_count >= self.max_deliveries {
            Ok(ClaimOutcome::MaxDeliveriesExceeded(MaxDeliveriesExceeded {
                item,
                max_deliveries: self.max_deliveries,
            }))
        } else {
            Ok(ClaimOutcome::Item(item))
        }
    }

    async fn ack(&self, token: DeliveryToken) -> BusResult<()> {
        self.settle(token, AckKind::Ack).await
    }

    async fn nak(&self, token: DeliveryToken, delay: Duration) -> BusResult<()> {
        self.settle(token, AckKind::Nak(Some(delay))).await
    }

    async fn term(&self, token: DeliveryToken) -> BusResult<()> {
        self.settle(token, AckKind::Term).await
    }
}
