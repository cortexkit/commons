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
use crate::kv_watch::{KvChange, KvDeliveries};
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

/// A JetStream register watch. Use [`Self::next_change`] to distinguish a
/// replacement bucket from a transient outage. [`RegisterWatch::next`] reports
/// a reset as unavailable with the reason `register watch bucket {bucket} reset`.
pub struct NatsRegisterWatch {
    deliveries: KvDeliveries,
    first: Option<WatchEvent>,
    buffered: VecDeque<RegisterUpdate>,
    has_snapshot: bool,
}

/// An entry or snapshot from a register watch, or notice of a new bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NatsRegisterWatchChange {
    Event(WatchEvent),
    /// Discard previously held entries and revisions. The following updates
    /// start with the replacement bucket's current entries, then live changes.
    BucketReset {
        bucket: String,
    },
}

fn reset_unavailable(bucket: &str) -> BusError {
    BusError::unavailable(format!("register watch bucket {bucket} reset"))
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
        // The same consumer filters async-nats's `watch_many_with_history`
        // and `watch_with_history` create, delivering the latest revision of
        // every matching key first.
        let opened = if prefix.is_empty() {
            KvDeliveries::open(self.connection.clone(), &self.store, ">".to_owned(), true)
                .await
                .map_err(|error| map_operation_error(&self.bucket, error, &mut events))
        } else {
            let pattern = if prefix.ends_with('.') {
                format!("{prefix}>")
            } else {
                prefix.to_owned()
            };
            KvDeliveries::open(self.connection.clone(), &self.store, pattern, false)
                .await
                .map_err(|error| map_operation_error(prefix, error, &mut events))
        };
        let mut watch = opened?;

        let mut snapshot = Vec::new();
        let mut buffered = VecDeque::new();
        let mut revision = 0;
        loop {
            let next = tokio::time::timeout(EMPTY_SNAPSHOT_QUIET_PERIOD, watch.next()).await;
            let Ok(entry) = next else {
                break;
            };
            let entry = match entry? {
                Some(KvChange::Entry(entry)) => entry,
                Some(KvChange::BucketReset { bucket }) => return Err(reset_unavailable(&bucket)),
                None => break,
            };
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
            deliveries: watch,
            first: Some(WatchEvent::Snapshot {
                entries: snapshot,
                revision: Revision(revision),
            }),
            buffered,
            has_snapshot: false,
        })
    }
}

impl NatsRegisterWatch {
    /// Reads the next snapshot, update or reset without ending the watch on
    /// reset. Resume reading on the same watch after clearing the old state.
    pub async fn next_change(&mut self) -> BusResult<NatsRegisterWatchChange> {
        if let Some(first) = self.first.take() {
            self.has_snapshot = true;
            return Ok(NatsRegisterWatchChange::Event(first));
        }
        if let Some(update) = self.buffered.pop_front() {
            return Ok(NatsRegisterWatchChange::Event(WatchEvent::Update(update)));
        }
        let entry = match self.deliveries.next().await? {
            Some(KvChange::Entry(entry)) => entry,
            Some(KvChange::BucketReset { bucket }) => {
                return Ok(NatsRegisterWatchChange::BucketReset { bucket });
            }
            None => return Err(BusError::unavailable("register watch ended")),
        };
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
        Ok(NatsRegisterWatchChange::Event(WatchEvent::Update(update)))
    }
}

#[async_trait]
impl RegisterWatch for NatsRegisterWatch {
    async fn next(&mut self) -> BusResult<WatchEvent> {
        match self.next_change().await? {
            NatsRegisterWatchChange::Event(event) => Ok(event),
            NatsRegisterWatchChange::BucketReset { bucket } => Err(reset_unavailable(&bucket)),
        }
    }

    fn has_snapshot(&self) -> bool {
        self.has_snapshot
    }
}
