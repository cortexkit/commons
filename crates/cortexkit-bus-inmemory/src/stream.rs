use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortexkit_bus_trait::{
    BusError, BusResult, ContentDigest, DurableOwner, Headers, Message, PublishReceipt, Stream,
    StreamCursor, StreamDelivery,
};

use crate::backend::{
    durable_name, BackendEvent, DurableState, InMemoryBus, Lease, QueueEntry, StoredMessage,
};

/// A cursor on one durable. Several cursors may share a durable, as several
/// pullers may share a JetStream durable: an unsettled delivery is withheld
/// from the others until its ack wait expires, then redelivered to whichever
/// pulls next.
pub struct InMemoryStreamCursor {
    bus: InMemoryBus,
    /// The durable's name.
    key: String,
    id: u64,
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
    ) -> BusResult<PublishReceipt> {
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

        let now = (self.deduplication_clock)();
        let window = state.config.deduplication_window;
        state
            .seen_ids
            .retain(|_, (_, stored_at)| now.duration_since(*stored_at) < window);
        if let Some(&(stream_seq, _)) = state.seen_ids.get(id) {
            return Ok(PublishReceipt::new(stream_seq, true));
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
        state.seen_ids.insert(id.into(), (stream_seq, now));
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
        Ok(PublishReceipt::new(stream_seq, false))
    }

    async fn consumer(&self, owner: DurableOwner<'_>) -> BusResult<Self::Cursor> {
        let key = durable_name(owner)?;
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        let Some(durable_state) = state.durable_cursors.get_mut(&key) else {
            return Err(BusError::absent(format!("durable consumer {key}")));
        };
        durable_state.next_cursor_id += 1;
        let id = durable_state.next_cursor_id;
        Ok(InMemoryStreamCursor {
            bus: self.clone(),
            key,
            id,
            in_flight_index: None,
        })
    }
}

#[async_trait]
impl StreamCursor for InMemoryStreamCursor {
    async fn next(&mut self) -> BusResult<Option<StreamDelivery>> {
        if self.in_flight_index.is_some() {
            return Err(BusError::unavailable(
                "the current delivery must be acknowledged, negatively acknowledged or terminated",
            ));
        }

        let mut guard = self.bus.inner.lock().expect("in-memory bus poisoned");
        let state = &mut *guard;
        let ack_wait = state.config.stream_ack_wait;
        let durable = state
            .durable_cursors
            .get_mut(&self.key)
            .expect("durable disappeared after binding");
        let now = Instant::now();
        // Another cursor holds the head message and its ack wait has not run out.
        if durable
            .lease
            .as_ref()
            .is_some_and(|lease| lease.deadline > now)
        {
            return Ok(None);
        }
        if durable
            .available_at
            .is_some_and(|available_at| available_at > now)
        {
            return Ok(None);
        }
        let Some(stored) = state.stream_messages.get(durable.next_index) else {
            return Ok(None);
        };
        durable.delivery_count += 1;
        durable.available_at = None;
        durable.lease = Some(Lease {
            cursor_id: self.id,
            deadline: now + ack_wait,
        });
        self.in_flight_index = Some(durable.next_index);
        Ok(Some(StreamDelivery {
            stream_seq: stored.stream_seq,
            delivery_count: durable.delivery_count,
            message: stored.message.clone(),
        }))
    }

    async fn ack(&mut self) -> BusResult<()> {
        let index = self.take_in_flight()?;
        let mut state = self.bus.inner.lock().expect("in-memory bus poisoned");
        let durable = durable_mut(&mut state.durable_cursors, &self.key);
        // A late ack, after the ack wait expired and another cursor settled
        // the message, finds the head already moved on and changes nothing.
        if durable.next_index == index {
            advance(durable);
        }
        Ok(())
    }

    async fn nak(&mut self, delay: Duration) -> BusResult<()> {
        let index = self.take_in_flight()?;
        let mut state = self.bus.inner.lock().expect("in-memory bus poisoned");
        let durable = durable_mut(&mut state.durable_cursors, &self.key);
        if durable.next_index == index && holds_lease(durable, self.id) {
            durable.lease = None;
            durable.available_at = Some(Instant::now() + delay);
        }
        Ok(())
    }

    async fn term(&mut self, reason: Option<&str>) -> BusResult<()> {
        let index = self.take_in_flight()?;
        let mut guard = self.bus.inner.lock().expect("in-memory bus poisoned");
        let state = &mut *guard;
        let durable = durable_mut(&mut state.durable_cursors, &self.key);
        if durable.next_index == index {
            // Terminating moves the durable past the message exactly as an ack
            // does, so it is never delivered again; only the record differs.
            advance(durable);
            state.events.push(BackendEvent::StreamTerminated {
                stream_seq: state.stream_messages[index].stream_seq,
                reason: reason.map(str::to_owned),
            });
        }
        Ok(())
    }

    async fn in_progress(&mut self) -> BusResult<()> {
        let index = self
            .in_flight_index
            .ok_or_else(|| BusError::absent("in-flight stream delivery"))?;
        let mut guard = self.bus.inner.lock().expect("in-memory bus poisoned");
        let state = &mut *guard;
        let ack_wait = state.config.stream_ack_wait;
        let durable = durable_mut(&mut state.durable_cursors, &self.key);
        // Once the message has been redelivered to another cursor, this cursor
        // no longer holds it and its report changes nothing.
        if durable.next_index == index && holds_lease(durable, self.id) {
            if let Some(lease) = durable.lease.as_mut() {
                lease.deadline = Instant::now() + ack_wait;
            }
        }
        Ok(())
    }
}

impl InMemoryStreamCursor {
    fn take_in_flight(&mut self) -> BusResult<usize> {
        self.in_flight_index
            .take()
            .ok_or_else(|| BusError::absent("in-flight stream delivery"))
    }
}

/// A dropped cursor gives its unsettled delivery back at once instead of after
/// the ack wait, so a reopened cursor in the same process replays it without
/// waiting (JetStream would wait for the ack wait to run out). The delivery
/// still counts towards the message's delivery count.
impl Drop for InMemoryStreamCursor {
    fn drop(&mut self) {
        let Some(index) = self.in_flight_index else {
            return;
        };
        let Ok(mut state) = self.bus.inner.lock() else {
            return;
        };
        if let Some(durable) = state.durable_cursors.get_mut(&self.key) {
            if durable.next_index == index && holds_lease(durable, self.id) {
                durable.lease = None;
            }
        }
    }
}

fn durable_mut<'a>(
    durables: &'a mut std::collections::HashMap<String, DurableState>,
    key: &str,
) -> &'a mut DurableState {
    durables
        .get_mut(key)
        .expect("durable disappeared after binding")
}

fn holds_lease(durable: &DurableState, cursor_id: u64) -> bool {
    durable
        .lease
        .as_ref()
        .is_some_and(|lease| lease.cursor_id == cursor_id)
}

/// Moves the durable past its head message, which is never delivered again.
fn advance(durable: &mut DurableState) {
    durable.next_index += 1;
    durable.available_at = None;
    durable.delivery_count = 0;
    durable.lease = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryConfig;

    #[tokio::test]
    async fn a_cursor_binds_the_durable_of_the_kind_its_caller_names() {
        let bus = InMemoryBus::new(
            InMemoryConfig::default().with_durable(DurableOwner::Module("prefrontal-core")),
        );
        assert!(!bus
            .publish(
                "ck.box.room.room_a.post",
                "room:room_a:1",
                ContentDigest::of_bytes(b"post"),
                Headers::new(),
            )
            .await
            .expect("publish")
            .duplicate());

        let mut cursor = bus
            .consumer(DurableOwner::Module("prefrontal-core"))
            .await
            .expect("the module durable binds");
        let delivery = cursor.next().await.expect("next").expect("a delivery");
        assert_eq!(delivery.message.id, "room:room_a:1");
        cursor.ack().await.expect("ack");

        // The same id as an agent names `c_prefrontal-core`, which was never
        // created: the kind, not the string, decides the durable.
        assert!(matches!(
            bus.consumer(DurableOwner::Agent("prefrontal-core")).await,
            Err(BusError::Absent { .. })
        ));
        // An id that is not a single subject token is refused before lookup.
        assert!(matches!(
            bus.consumer(DurableOwner::Module("prefrontal.core")).await,
            Err(BusError::Denied { .. })
        ));
    }
}
