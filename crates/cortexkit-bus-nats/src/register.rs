use std::collections::VecDeque;
use std::time::Duration;

use async_nats::jetstream::kv::{self, Operation};
use async_trait::async_trait;
use bytes::Bytes;
use cortexkit_bus_trait::{
    BusError, BusResult, Register, RegisterEntry, RegisterUpdate, RegisterValue, RegisterWatch,
    Revision, Tombstone, WatchEvent,
};
use futures_util::StreamExt;

use crate::error::map_operation_error;
use crate::NatsConnection;

const EMPTY_SNAPSHOT_QUIET_PERIOD: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct NatsRegister {
    connection: NatsConnection,
    bucket: String,
    store: kv::Store,
}

impl NatsRegister {
    pub(crate) async fn bind(connection: NatsConnection, bucket: String) -> BusResult<Self> {
        let mut events = connection.event_receiver();
        let store = connection
            .jetstream()
            .get_key_value(bucket.clone())
            .await
            .map_err(|error| map_operation_error(&bucket, error, &mut events))?;
        Ok(Self {
            connection,
            bucket,
            store,
        })
    }

    pub fn bucket_name(&self) -> &str {
        &self.bucket
    }
}

pub struct NatsRegisterWatch {
    connection: NatsConnection,
    watch: kv::Watch,
    first: Option<WatchEvent>,
    buffered: VecDeque<RegisterUpdate>,
    has_snapshot: bool,
}

#[async_trait]
impl Register for NatsRegister {
    type Watch = NatsRegisterWatch;

    async fn put(&self, key: &str, value: Vec<u8>) -> BusResult<Revision> {
        let mut events = self.connection.event_receiver();
        self.store
            .put(key, Bytes::from(value))
            .await
            .map(Revision)
            .map_err(|error| map_operation_error(key, error, &mut events))
    }

    async fn get(&self, key: &str) -> BusResult<RegisterValue> {
        let mut events = self.connection.event_receiver();
        let entry = self
            .store
            .entry(key.to_owned())
            .await
            .map_err(|error| map_operation_error(key, error, &mut events))?
            .filter(|entry| entry.operation == Operation::Put)
            .ok_or_else(|| BusError::absent(format!("register key {key}")))?;
        Ok(RegisterValue {
            value: entry.value.to_vec(),
            revision: Revision(entry.revision),
        })
    }

    async fn delete(&self, key: &str) -> BusResult<Tombstone> {
        let mut events = self.connection.event_receiver();
        self.store
            .delete(key)
            .await
            .map_err(|error| map_operation_error(key, error, &mut events))?;
        let mut history = self
            .store
            .history(key)
            .await
            .map_err(|error| map_operation_error(key, error, &mut events))?;
        let mut revision = None;
        while let Some(entry) = history.next().await {
            let entry = entry.map_err(|error| map_operation_error(key, error, &mut events))?;
            revision = Some(entry.revision);
        }
        Ok(Tombstone {
            key: key.to_owned(),
            revision: Revision(revision.ok_or_else(|| {
                BusError::unavailable("delete succeeded but its tombstone revision was absent")
            })?),
        })
    }

    async fn watch(&self, prefix: &str) -> BusResult<Self::Watch> {
        let mut events = self.connection.event_receiver();
        let mut watch = if prefix.is_empty() {
            self.store
                .watch_many_with_history([">"])
                .await
                .map_err(|error| map_operation_error(&self.bucket, error, &mut events))?
        } else {
            let pattern = if prefix.ends_with('.') {
                format!("{prefix}>")
            } else {
                prefix.to_owned()
            };
            self.store
                .watch_with_history(pattern)
                .await
                .map_err(|error| map_operation_error(prefix, error, &mut events))?
        };

        let mut snapshot = Vec::new();
        let mut buffered = VecDeque::new();
        let mut revision = 0;
        loop {
            let next = tokio::time::timeout(EMPTY_SNAPSHOT_QUIET_PERIOD, watch.next()).await;
            let Some(entry) = next.unwrap_or_default() else {
                break;
            };
            let entry = entry.map_err(|error| map_operation_error(prefix, error, &mut events))?;
            revision = revision.max(entry.revision);
            let seen_current = entry.seen_current;
            match entry.operation {
                Operation::Put => snapshot.push(RegisterEntry {
                    key: entry.key,
                    value: entry.value.to_vec(),
                    revision: Revision(entry.revision),
                }),
                Operation::Delete | Operation::Purge => {
                    buffered.push_back(RegisterUpdate::Delete(Tombstone {
                        key: entry.key,
                        revision: Revision(entry.revision),
                    }));
                }
            }
            if seen_current {
                break;
            }
        }

        Ok(NatsRegisterWatch {
            connection: self.connection.clone(),
            watch,
            first: Some(WatchEvent::Snapshot {
                entries: snapshot,
                revision: Revision(revision),
            }),
            buffered,
            has_snapshot: false,
        })
    }
}

#[async_trait]
impl RegisterWatch for NatsRegisterWatch {
    async fn next(&mut self) -> BusResult<WatchEvent> {
        if let Some(first) = self.first.take() {
            self.has_snapshot = true;
            return Ok(first);
        }
        if let Some(update) = self.buffered.pop_front() {
            return Ok(WatchEvent::Update(update));
        }
        let mut events = self.connection.event_receiver();
        let entry = self
            .watch
            .next()
            .await
            .ok_or_else(|| BusError::unavailable("register watch ended"))?
            .map_err(|error| map_operation_error("register watch", error, &mut events))?;
        let update = match entry.operation {
            Operation::Put => RegisterUpdate::Put(RegisterEntry {
                key: entry.key,
                value: entry.value.to_vec(),
                revision: Revision(entry.revision),
            }),
            Operation::Delete | Operation::Purge => RegisterUpdate::Delete(Tombstone {
                key: entry.key,
                revision: Revision(entry.revision),
            }),
        };
        Ok(WatchEvent::Update(update))
    }

    fn has_snapshot(&self) -> bool {
        self.has_snapshot
    }
}
