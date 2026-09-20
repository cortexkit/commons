use async_trait::async_trait;
use cortexkit_bus_trait::{
    BusError, BusResult, Register, RegisterEntry, RegisterUpdate, RegisterValue, RegisterWatch,
    Revision, Tombstone, WatchEvent,
};
use tokio::sync::mpsc;

use crate::backend::{InMemoryBus, Watcher};

pub struct InMemoryRegisterWatch {
    snapshot: Option<WatchEvent>,
    updates: mpsc::UnboundedReceiver<RegisterUpdate>,
    has_snapshot: bool,
}

impl InMemoryRegisterWatch {
    /// Simulates losing the transport before a pending snapshot can arrive.
    pub fn disconnect(&mut self) {
        self.snapshot = None;
        self.updates.close();
        self.has_snapshot = false;
    }
}

#[async_trait]
impl Register for InMemoryBus {
    type Watch = InMemoryRegisterWatch;

    async fn put(&self, key: &str, value: Vec<u8>) -> BusResult<Revision> {
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        let revision = state.next_revision();
        state.register_values.insert(
            key.into(),
            RegisterValue {
                value: value.clone(),
                revision,
            },
        );
        state.notify_watchers(RegisterUpdate::Put(RegisterEntry {
            key: key.into(),
            value,
            revision,
        }));
        Ok(revision)
    }

    async fn get(&self, key: &str) -> BusResult<RegisterValue> {
        self.inner
            .lock()
            .expect("in-memory bus poisoned")
            .register_values
            .get(key)
            .cloned()
            .ok_or_else(|| BusError::absent(format!("register key {key}")))
    }

    async fn delete(&self, key: &str) -> BusResult<Tombstone> {
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        if state.register_values.remove(key).is_none() {
            return Err(BusError::absent(format!("register key {key}")));
        }
        let tombstone = Tombstone {
            key: key.into(),
            revision: state.next_revision(),
        };
        state.notify_watchers(RegisterUpdate::Delete(tombstone.clone()));
        Ok(tombstone)
    }

    async fn watch(&self, prefix: &str) -> BusResult<Self::Watch> {
        let (sender, updates) = mpsc::unbounded_channel();
        let mut state = self.inner.lock().expect("in-memory bus poisoned");
        let entries = state
            .register_values
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, value)| RegisterEntry {
                key: key.clone(),
                value: value.value.clone(),
                revision: value.revision,
            })
            .collect();
        let revision = Revision(state.register_revision);
        state.watchers.push(Watcher {
            prefix: prefix.into(),
            sender,
        });
        Ok(InMemoryRegisterWatch {
            snapshot: Some(WatchEvent::Snapshot { entries, revision }),
            updates,
            has_snapshot: false,
        })
    }
}

#[async_trait]
impl RegisterWatch for InMemoryRegisterWatch {
    async fn next(&mut self) -> BusResult<WatchEvent> {
        if let Some(snapshot) = self.snapshot.take() {
            self.has_snapshot = true;
            return Ok(snapshot);
        }
        self.updates
            .recv()
            .await
            .map(WatchEvent::Update)
            .ok_or_else(|| BusError::unavailable("register watch connection dropped"))
    }

    fn has_snapshot(&self) -> bool {
        self.has_snapshot
    }
}
