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
    /// Binds a claimant to an existing durable. The claimant's cap is derived
    /// from the consumer's own `max_deliver`, one below it: the claimant decides
    /// to dead-letter on a delivery that still has one server redelivery after it,
    /// so a claimant that dies between publishing the record and `term()` gets
    /// that spare delivery to finish the job. At the consumer's own limit the
    /// server would never redeliver, and the item would stay unterminated for
    /// good. Deriving the cap, rather than taking it as an argument, keeps it
    /// from drifting away from `max_deliver`.
    pub(crate) async fn bind(
        connection: NatsConnection,
        stream: String,
        durable: String,
        pull_expires: Duration,
    ) -> BusResult<Self> {
        let mut events = connection.event_receiver();
        let consumer = connection
            .jetstream()
            .get_consumer_from_stream::<pull::Config, _, _>(&durable, &stream)
            .await
            .map_err(|error| map_operation_error(&durable, error, &mut events))?;
        let max_deliver = consumer.cached_info().config.max_deliver;
        let max_deliveries = claimant_cap(max_deliver).ok_or_else(|| {
            BusError::denied(
                durable.clone(),
                format!(
                    "work-queue consumer max_deliver must be at least 2, so the dead-letter \
                     decision has a spare redelivery after it; it is {max_deliver}"
                ),
            )
        })?;
        Ok(Self {
            connection,
            consumer,
            max_deliveries,
            pull_expires,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// The delivery on which this claimant reports `MaxDeliveriesExceeded`.
    pub fn max_deliveries(&self) -> u32 {
        self.max_deliveries
    }

    async fn settle(&self, token: DeliveryToken, kind: AckKind) -> BusResult<()> {
        let message = self
            .in_flight
            .lock()
            .await
            .remove(&token)
            .ok_or_else(|| BusError::absent(format!("in-flight work delivery {}", token.0)))?;
        let mut events = self.connection.event_receiver();
        // A plain ack is only queued in the client; a claimant that exits right
        // after it can lose it. The double ack waits for the server's reply.
        if let Err(error) = message.double_ack_with(kind).await {
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

/// A claimant's cap from its consumer's `max_deliver`: one below it, or `None`
/// when that leaves no spare redelivery (unlimited, zero or one).
fn claimant_cap(max_deliver: i64) -> Option<u32> {
    if max_deliver < 2 {
        return None;
    }
    u32::try_from(max_deliver - 1).ok()
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

#[cfg(test)]
mod cap_tests {
    use super::claimant_cap;

    #[test]
    fn the_cap_leaves_one_spare_redelivery() {
        assert_eq!(claimant_cap(5), Some(4));
        assert_eq!(claimant_cap(2), Some(1));
        assert_eq!(claimant_cap(1), None);
        assert_eq!(claimant_cap(0), None);
        assert_eq!(claimant_cap(-1), None);
    }
}
