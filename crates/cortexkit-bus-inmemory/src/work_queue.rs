use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortexkit_bus_trait::{
    AsyncPermissionViolation, BusError, BusResult, ClaimOutcome, MaxDeliveriesExceeded,
    PermissionViolationSource, WorkItem, WorkQueue,
};

use crate::backend::{BackendEvent, InMemoryBus};

#[async_trait]
impl WorkQueue for InMemoryBus {
    async fn claim(&self) -> BusResult<ClaimOutcome> {
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        let max_deliveries = state
            .config
            .work_queue
            .as_ref()
            .ok_or_else(|| BusError::absent("configured work queue"))?
            .max_deliveries;
        let now = Instant::now();
        let Some(entry) = state
            .queue
            .iter_mut()
            .find(|entry| !entry.in_flight && entry.available_at <= now)
        else {
            return Ok(ClaimOutcome::Empty);
        };

        entry.in_flight = true;
        if entry.delivery_count >= max_deliveries {
            return Ok(ClaimOutcome::MaxDeliveriesExceeded(MaxDeliveriesExceeded {
                item: WorkItem {
                    token: entry.token,
                    message: entry.message.clone(),
                    delivery_count: entry.delivery_count,
                },
                max_deliveries,
            }));
        }

        entry.delivery_count += 1;
        Ok(ClaimOutcome::Item(WorkItem {
            token: entry.token,
            message: entry.message.clone(),
            delivery_count: entry.delivery_count,
        }))
    }

    async fn ack(&self, token: cortexkit_bus_trait::DeliveryToken) -> BusResult<()> {
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        remove_in_flight(&mut state.queue, token)?;
        state.events.push(BackendEvent::Acknowledged { token });
        Ok(())
    }

    async fn nak(
        &self,
        token: cortexkit_bus_trait::DeliveryToken,
        delay: Duration,
    ) -> BusResult<()> {
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        let max_delay = state
            .config
            .work_queue
            .as_ref()
            .ok_or_else(|| BusError::absent("configured work queue"))?
            .max_nak_delay;
        let entry = state
            .queue
            .iter_mut()
            .find(|entry| entry.token == token && entry.in_flight)
            .ok_or_else(|| BusError::absent(format!("in-flight work delivery {}", token.0)))?;
        let applied_delay = delay.min(max_delay);
        entry.available_at = Instant::now() + applied_delay;
        entry.in_flight = false;
        state
            .events
            .push(BackendEvent::NegativelyAcknowledged { token });
        if applied_delay != delay {
            return Err(BusError::clamped(
                format!("nak delay reduced from {delay:?} to {max_delay:?}"),
                Some(max_delay),
            ));
        }
        Ok(())
    }

    async fn term(&self, token: cortexkit_bus_trait::DeliveryToken) -> BusResult<()> {
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        remove_in_flight(&mut state.queue, token)?;
        state.events.push(BackendEvent::Terminated { token });
        Ok(())
    }
}

fn remove_in_flight(
    queue: &mut std::collections::VecDeque<crate::backend::QueueEntry>,
    token: cortexkit_bus_trait::DeliveryToken,
) -> BusResult<()> {
    let position = queue
        .iter()
        .position(|entry| entry.token == token && entry.in_flight)
        .ok_or_else(|| BusError::absent(format!("in-flight work delivery {}", token.0)))?;
    queue.remove(position);
    Ok(())
}

#[async_trait]
impl PermissionViolationSource for InMemoryBus {
    async fn next_violation(&mut self) -> BusResult<Option<AsyncPermissionViolation>> {
        Ok(self
            .inner
            .lock()
            .expect("in-memory bus poisoned")
            .permission_violations
            .pop_front())
    }
}
