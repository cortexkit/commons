use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortexkit_bus_trait::{
    BusError, BusResult, ContentDigest, Headers, Message, PublishAck, Stream, StreamCursor,
    StreamDelivery,
};

use crate::backend::{BackendEvent, InMemoryBus, QueueEntry, StoredMessage};

pub struct InMemoryStreamCursor {
    bus: InMemoryBus,
    key: (String, String),
    in_flight_index: Option<usize>,
}

#[async_trait]
impl Stream for InMemoryBus {
    type Cursor = InMemoryStreamCursor;

    async fn publish(
        &self,
        subject: &str,
        id: &str,
        digest: ContentDigest,
        headers: Headers,
    ) -> BusResult<PublishAck> {
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        if state.config.denied_subjects.contains(subject) {
            state
                .permission_violations
                .push_back(cortexkit_bus_trait::AsyncPermissionViolation {
                    subject: subject.into(),
                    reason: "fixture-configured publish denial".into(),
                });
            return Err(BusError::denied(
                subject,
                "fixture-configured publish denial",
            ));
        }

        let message = Message {
            subject: subject.into(),
            id: id.into(),
            digest,
            headers,
        };
        if let Some(config) = &state.config.work_queue {
            if subject == config.subject {
                let queued_bytes = state
                    .queue
                    .iter()
                    .map(|entry| entry.message.wire_size())
                    .sum::<usize>();
                if queued_bytes + message.wire_size() > config.max_bytes {
                    return Err(BusError::unavailable_limit(
                        "max_bytes",
                        format!("work queue {} is full", config.subject),
                    ));
                }
            }
        }

        let stream_seq = state.next_stream_seq;
        state.next_stream_seq += 1;
        state.stream_messages.push(StoredMessage {
            stream_seq,
            message: message.clone(),
        });
        if state
            .config
            .work_queue
            .as_ref()
            .is_some_and(|config| subject == config.subject)
        {
            let token = cortexkit_bus_trait::DeliveryToken(state.next_delivery_token);
            state.next_delivery_token += 1;
            state.queue.push_back(QueueEntry {
                token,
                message: message.clone(),
                delivery_count: 0,
                available_at: Instant::now(),
                in_flight: false,
            });
        }
        state.events.push(BackendEvent::Published {
            subject: subject.into(),
            id: id.into(),
        });
        Ok(PublishAck { stream_seq })
    }

    async fn consumer(&self, durable: &str, identity: &str) -> BusResult<Self::Cursor> {
        let key = (durable.to_owned(), identity.to_owned());
        if !self
            .inner
            .lock()
            .expect("in-memory bus poisoned")
            .durable_cursors
            .contains_key(&key)
        {
            return Err(BusError::absent(format!(
                "durable consumer {durable} for identity {identity}"
            )));
        }
        Ok(InMemoryStreamCursor {
            bus: self.clone(),
            key,
            in_flight_index: None,
        })
    }
}

#[async_trait]
impl StreamCursor for InMemoryStreamCursor {
    async fn next(&mut self) -> BusResult<Option<StreamDelivery>> {
        if self.in_flight_index.is_some() {
            return Err(BusError::unavailable(
                "the current delivery must be acknowledged or negatively acknowledged",
            ));
        }

        let state = self.bus.inner.lock().expect("in-memory bus poisoned");
        let durable = state
            .durable_cursors
            .get(&self.key)
            .expect("durable disappeared after binding");
        if durable
            .available_at
            .is_some_and(|available_at| available_at > Instant::now())
        {
            return Ok(None);
        }
        let Some(stored) = state.stream_messages.get(durable.next_index) else {
            return Ok(None);
        };
        self.in_flight_index = Some(durable.next_index);
        Ok(Some(StreamDelivery {
            stream_seq: stored.stream_seq,
            message: stored.message.clone(),
        }))
    }

    async fn ack(&mut self) -> BusResult<()> {
        let index = self
            .in_flight_index
            .take()
            .ok_or_else(|| BusError::absent("in-flight stream delivery"))?;
        let mut state = self.bus.inner.lock().expect("in-memory bus poisoned");
        let durable = state
            .durable_cursors
            .get_mut(&self.key)
            .expect("durable disappeared after binding");
        durable.next_index = index + 1;
        durable.available_at = None;
        Ok(())
    }

    async fn nak(&mut self, delay: Duration) -> BusResult<()> {
        self.in_flight_index
            .take()
            .ok_or_else(|| BusError::absent("in-flight stream delivery"))?;
        let mut state = self.bus.inner.lock().expect("in-memory bus poisoned");
        state
            .durable_cursors
            .get_mut(&self.key)
            .expect("durable disappeared after binding")
            .available_at = Some(Instant::now() + delay);
        Ok(())
    }
}
