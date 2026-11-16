use std::{future::Future, time::Duration};

use async_nats::jetstream::{
    self,
    consumer::{push, DeliverPolicy},
    kv,
    stream::ConsumerErrorKind,
};
use bytes::Bytes;
use cortexkit_bus_naming::{ModuleBucket, MODULE_BUCKETS};
use cortexkit_bus_nats::NatsConnection;
use futures_util::StreamExt;

use crate::{binding::ConnectionBinding, BusClient, Error, Result};

/// A current value and its bucket-wide JetStream revision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BucketEntry {
    pub value: Bytes,
    pub revision: u64,
}

/// A snapshot/live change, or the boundary after the initial snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BucketItem {
    Put {
        key: String,
        revision: u64,
        value: Bytes,
    },
    Delete {
        key: String,
        revision: u64,
    },
    CaughtUp,
}

/// A declared module bucket. Keep its BusClient alive for credential upkeep.
#[derive(Clone)]
pub struct Bucket {
    binding: ConnectionBinding,
    name: String,
    declaration: &'static ModuleBucket,
    module: String,
}

impl Bucket {
    pub(crate) fn attach(client: &BusClient, owner: &str, bucket: &str) -> Result<Self> {
        let declaration = MODULE_BUCKETS
            .iter()
            .find(|entry| entry.owner == owner && entry.name == bucket)
            .ok_or_else(|| Error::BucketNotDeclared {
                owner: owner.into(),
                bucket: bucket.into(),
            })?;
        let name = client.names.module_bucket_name(owner, bucket)?;
        if client.module != declaration.owner
            && !declaration.readers.contains(&client.module.as_str())
        {
            return Err(Error::BucketNotReadable {
                module: client.module.clone(),
                bucket: name,
            });
        }
        Ok(Self {
            binding: ConnectionBinding::new(client),
            name,
            declaration,
            module: client.module.clone(),
        })
    }

    fn require_owner(&self) -> Result<()> {
        if self.module != self.declaration.owner {
            return Err(Error::BucketNotOwned {
                module: self.module.clone(),
                bucket: self.name.clone(),
            });
        }
        Ok(())
    }

    async fn store(&self) -> Result<kv::Store> {
        jetstream::new(self.binding.snapshot().client())
            .get_key_value(&self.name)
            .await
            .map_err(|error| Error::Nats(error.to_string()))
    }

    /// Gets the latest value; absent and deleted keys both return None.
    pub async fn get(&self, key: &str) -> Result<Option<BucketEntry>> {
        Ok(self
            .store()
            .await?
            .entry(key)
            .await
            .map_err(|error| Error::Nats(error.to_string()))?
            .filter(|entry| entry.operation == kv::Operation::Put)
            .map(|entry| BucketEntry {
                value: entry.value,
                revision: entry.revision,
            }))
    }

    /// Writes as the declared owner, refusing oversized values before any request.
    pub async fn put(&self, key: &str, value: impl Into<Bytes>) -> Result<u64> {
        self.require_owner()?;
        let value = value.into();
        if value.len() > self.declaration.max_value_bytes as usize {
            return Err(Error::BucketValueTooLarge {
                size: value.len(),
                cap: self.declaration.max_value_bytes,
            });
        }
        self.store()
            .await?
            .put(key, value)
            .await
            .map_err(|error| Error::Nats(error.to_string()))
    }

    /// Stores a retained delete marker and returns its publish acknowledgement's revision.
    pub async fn delete(&self, key: &str) -> Result<u64> {
        self.require_owner()?;
        validate_key(key)?;
        let mut headers = async_nats::HeaderMap::new();
        headers.insert("KV-Operation", "DEL");
        // Store::delete discards the acknowledgement; publishing the same marker
        // directly preserves its revision without a racing read-back.
        let ack = jetstream::new(self.binding.snapshot().client())
            .publish_with_headers(format!("$KV.{}.{key}", self.name), headers, Bytes::new())
            .await
            .map_err(|error| Error::Nats(error.to_string()))?
            .await
            .map_err(|error| Error::Nats(error.to_string()))?;
        Ok(ack.sequence)
    }

    /// Delivers current entries, one CaughtUp marker (even when empty), then changes.
    /// Uses only unnamed ephemeral consumers; dropped watches release their slots
    /// after the server's 30-second inactivity threshold, not immediately.
    /// New watches fail immediately with WatchCapacity when all slots are full.
    /// An existing watch rebinding after credential reissue logs once and retries
    /// capacity with 1-to-5-second backoff for at most 60 seconds, then returns
    /// WatchCapacity if no slot became available. Terminal maintenance interrupts
    /// that wait with the same transport error as EventReader.
    ///
    /// After reconnecting, delivers each key whose latest retained revision is
    /// newer than the last delivered revision, without repeating a revision.
    /// Intermediate values overwritten while disconnected are not delivered;
    /// readers needing every change must use an event stream, not a bucket.
    /// Delete markers remain the key's latest entry and survive reconnects.
    /// Owners must use delete, never purge: removing a marker hides the deletion.
    /// Keep the BusClient alive; terminal renewal ends next() with Error::Transport,
    /// just as EventReader does.
    pub async fn watch(&self) -> Result<BucketWatch> {
        let mut watch = BucketWatch {
            bucket: self.clone(),
            subscription: None,
            revision: 0,
            caught_up: false,
            marker_pending: false,
            opened: false,
        };
        watch.bind().await?;
        Ok(watch)
    }
}

// Match async-nats KV's key grammar when publishing delete markers directly.
fn validate_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.starts_with('.')
        || key.ends_with('.')
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-/_=.".contains(&byte))
    {
        return Err(Error::Nats("invalid KV key".into()));
    }
    Ok(())
}

/// A revision-resuming snapshot and live watch. Cancellation of next() is safe.
pub struct BucketWatch {
    bucket: Bucket,
    subscription: Option<push::Ordered>,
    revision: u64,
    caught_up: bool,
    marker_pending: bool,
    opened: bool,
}

impl BucketWatch {
    async fn bind(&mut self) -> Result<()> {
        // Drop the old inbox before opening another consumer. Module grants do
        // not include consumer deletion; the server reaps inactive consumers.
        self.subscription = None;
        let name = &self.bucket.name;
        let revision = self.revision;
        let (subscription, empty) = bind_watch(
            &mut self.bucket.binding,
            name,
            self.opened,
            |connection| async move {
                let context = jetstream::new(connection.client());
                let consumer = context
                    .create_consumer_on_stream(
                        push::OrderedConfig {
                            deliver_subject: connection.client().new_inbox(),
                            filter_subject: format!("$KV.{name}.>"),
                            deliver_policy: if revision == 0 {
                                DeliverPolicy::LastPerSubject
                            } else {
                                DeliverPolicy::ByStartSequence {
                                    start_sequence: revision + 1,
                                }
                            },
                            ..Default::default()
                        },
                        format!("KV_{name}"),
                    )
                    .await
                    .map_err(|error| consumer_error(name, error))?;
                let empty = consumer.cached_info().num_pending == 0;
                Ok((
                    consumer
                        .messages()
                        .await
                        .map_err(|error| Error::Nats(error.to_string()))?,
                    empty,
                ))
            },
        )
        .await?;
        self.subscription = Some(subscription);
        self.opened = true;
        self.marker_pending = !self.caught_up && empty;
        Ok(())
    }

    /// Returns one change or the initial snapshot boundary, never acknowledging writes.
    pub async fn next(&mut self) -> Result<BucketItem> {
        loop {
            if self.bucket.binding.has_changed() || self.subscription.is_none() {
                self.bind().await?;
            }
            tokio::select! {
                biased;
                changed = self.bucket.binding.changed() => { changed?; self.subscription = None; }
                () = std::future::ready(()), if self.marker_pending => {
                    self.marker_pending = false;
                    self.caught_up = true;
                    return Ok(BucketItem::CaughtUp);
                }
                message = self.subscription.as_mut().expect("bound watch").next() => {
                    let message = match message
                        .ok_or_else(|| Error::Nats("bucket watch closed".into()))
                        .and_then(|message| message.map_err(|error| Error::Nats(error.to_string())))
                    {
                        Ok(message) => message,
                        Err(error) => {
                            // A connection that closed for good ends the watch;
                            // resume on its replacement after the last revision.
                            self.bucket.binding.recover(error).await?;
                            self.subscription = None;
                            continue;
                        }
                    };
                    let info = message.info().map_err(|error| Error::Nats(error.to_string()))?;
                    if !self.caught_up && info.pending == 0 { self.marker_pending = true; }
                    if !advance_revision(&mut self.revision, info.stream_sequence) { continue; }
                    let key = message.subject.strip_prefix(&format!("$KV.{}.", self.bucket.name))
                        .ok_or(Error::Reply("bucket watch subject mismatch"))?.to_owned();
                    let revision = info.stream_sequence;
                    let deleted = message.headers.as_ref().and_then(|headers| headers.get("KV-Operation"))
                        .is_some_and(|operation| matches!(operation.as_str(), "DEL" | "PURGE"));
                    return Ok(if deleted { BucketItem::Delete { key, revision } } else {
                        BucketItem::Put { key, revision, value: message.payload.clone() }
                    });
                }
            }
        }
    }
}

pub(crate) async fn bind_watch<T, F, Fut>(
    binding: &mut ConnectionBinding,
    bucket: &str,
    retry_capacity: bool,
    mut open: F,
) -> Result<T>
where
    F: FnMut(NatsConnection) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut deadline = None;
    let mut delay = Duration::from_secs(1);
    loop {
        let attempt = binding.bind(&mut open);
        let result = if let Some(deadline) = deadline {
            tokio::time::timeout_at(deadline, attempt)
                .await
                .unwrap_or_else(|_| {
                    Err(Error::WatchCapacity {
                        bucket: bucket.into(),
                    })
                })
        } else {
            attempt.await
        };
        match result {
            Err(Error::WatchCapacity { .. }) if retry_capacity => {
                let deadline = *deadline.get_or_insert_with(|| {
                    #[cfg(test)]
                    {
                        binding.bucket_watch_capacity_warnings += 1;
                    }
                    tracing::warn!(
                        bucket,
                        "waiting for bucket watch consumer capacity during rebind"
                    );
                    tokio::time::Instant::now() + Duration::from_secs(60)
                });
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(Error::WatchCapacity {
                        bucket: bucket.into(),
                    });
                }
                tokio::select! {
                    biased;
                    changed = binding.changed() => { changed?; }
                    () = tokio::time::sleep(delay.min(remaining)) => {}
                }
                delay = (delay * 2).min(Duration::from_secs(5));
            }
            result => return result,
        }
    }
}

fn advance_revision(last: &mut u64, revision: u64) -> bool {
    if revision <= *last {
        return false;
    }
    *last = revision;
    true
}

fn consumer_error(bucket: &str, error: jetstream::stream::ConsumerError) -> Error {
    if matches!(error.kind(), ConsumerErrorKind::JetStream(ref server)
        if server.error_code() == jetstream::ErrorCode::MAXIMUM_CONSUMERS_LIMIT)
    {
        Error::WatchCapacity {
            bucket: bucket.into(),
        }
    } else {
        Error::Nats(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bucket_revision_deduplication() {
        let mut last = 12;
        assert!(!super::advance_revision(&mut last, 12));
        assert!(!super::advance_revision(&mut last, 11));
        assert_eq!(last, 12);
        assert!(super::advance_revision(&mut last, 15));
        assert_eq!(last, 15);
    }
}
