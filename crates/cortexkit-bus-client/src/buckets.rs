use std::{future::Future, time::Duration};

use async_nats::jetstream::{
    self,
    consumer::{push, AckPolicy, DeliverPolicy},
    context::GetStreamErrorKind,
    kv,
    stream::ConsumerErrorKind,
    ErrorCode,
};
use bytes::Bytes;
use cortexkit_bus_naming::{ModuleBucket, MODULE_BUCKETS};
use cortexkit_bus_nats::NatsConnection;
use futures_util::StreamExt;

use crate::{binding::ConnectionBinding, BusClient, Error, Result};

const WATCH_IDLE_HEARTBEAT: Duration = Duration::from_secs(5);

/// A current value and its bucket-wide JetStream revision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BucketEntry {
    pub value: Bytes,
    pub revision: u64,
}

/// A snapshot/live change, or the boundary after a snapshot.
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
    /// After reconnecting to the same bucket, delivers each key whose latest
    /// retained revision is newer than the last delivered revision.
    /// Intermediate values overwritten while disconnected are not delivered;
    /// readers needing every change must use an event stream, not a bucket.
    /// Delete markers remain the key's latest entry and survive reconnects.
    /// Owners must use delete, never purge: removing a marker hides the deletion.
    /// If the bucket is replaced, see [`Error::BucketReset`].
    /// Keep the BusClient alive; terminal renewal ends next() with Error::Transport,
    /// just as EventReader does.
    pub async fn watch(&self) -> Result<BucketWatch> {
        BucketWatch::open(self.binding.clone(), self.name.clone(), None).await
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

fn watch_key<'a>(subject: &'a str, bucket: &str) -> Option<&'a str> {
    let key = subject.strip_prefix(&format!("$KV.{bucket}."))?;
    validate_key(key).ok()?;
    Some(key)
}

struct WatchSubscription {
    messages: push::Messages,
    stream: String,
    consumer: String,
}

struct BoundWatch {
    subscription: WatchSubscription,
    empty: bool,
    created_at: i128,
    reset: bool,
}

/// A revision-resuming snapshot and live watch. Cancellation of next() is safe.
/// Origin and subject checks run before sequence bookkeeping so forged pushes
/// cannot change the resume point or inject bucket changes.
pub struct BucketWatch {
    binding: ConnectionBinding,
    name: String,
    domain: Option<&'static str>,
    subscription: Option<WatchSubscription>,
    stream_created_at: Option<i128>,
    consumer_sequence: u64,
    revision: u64,
    caught_up: bool,
    marker_pending: bool,
    forged_warning_logged: bool,
    missing_retry: Option<(tokio::time::Instant, Duration)>,
}

impl BucketWatch {
    pub(crate) async fn open(
        binding: ConnectionBinding,
        name: String,
        domain: Option<&'static str>,
    ) -> Result<Self> {
        let mut watch = Self {
            binding,
            name,
            domain,
            subscription: None,
            stream_created_at: None,
            consumer_sequence: 0,
            revision: 0,
            caught_up: false,
            marker_pending: false,
            forged_warning_logged: false,
            missing_retry: None,
        };
        watch.bind().await?;
        Ok(watch)
    }

    async fn bind(&mut self) -> Result<()> {
        // Drop the old inbox before opening another consumer. Module grants do
        // not include consumer deletion; the server reaps inactive consumers.
        self.subscription = None;
        let name = self.name.as_str();
        let (domain, created_at, revision) = (self.domain, self.stream_created_at, self.revision);
        let opened = created_at.is_some();
        let bound = loop {
            if let Some((deadline, _)) = self.missing_retry {
                tokio::select! {
                    biased;
                    changed = self.binding.changed() => { changed?; }
                    () = tokio::time::sleep_until(deadline) => {}
                }
            }
            let bound = bind_watch(&mut self.binding, name, opened, |connection| {
                open_bucket_subscription(connection, name, domain, created_at, revision)
            })
            .await?;
            if let Some(bound) = bound {
                break bound;
            }
            let delay = if let Some((_, delay)) = self.missing_retry {
                (delay * 2).min(Duration::from_secs(5))
            } else {
                tracing::warn!(
                    bucket = name,
                    "bucket watch bucket is missing; waiting for it to return"
                );
                Duration::from_secs(1)
            };
            self.missing_retry = Some((tokio::time::Instant::now() + delay, delay));
        };
        self.missing_retry = None;
        self.stream_created_at = Some(bound.created_at);
        self.subscription = Some(bound.subscription);
        self.consumer_sequence = 0;
        if bound.reset {
            self.revision = 0;
            self.caught_up = false;
        }
        self.marker_pending = !self.caught_up && bound.empty;
        if bound.reset {
            return Err(Error::BucketReset {
                bucket: name.into(),
            });
        }
        Ok(())
    }

    /// Returns one change or a snapshot boundary, never acknowledging writes.
    /// On [`Error::BucketReset`], clear the old state and call `next` again.
    /// Current entries from the replacement bucket come before a fresh
    /// [`BucketItem::CaughtUp`], even when the new bucket is empty.
    pub async fn next(&mut self) -> Result<BucketItem> {
        loop {
            if self.binding.has_changed() || self.subscription.is_none() {
                self.bind().await?;
            }
            tokio::select! {
                biased;
                changed = self.binding.changed() => { changed?; self.subscription = None; }
                () = std::future::ready(()), if self.marker_pending => {
                    self.marker_pending = false;
                    self.caught_up = true;
                    return Ok(BucketItem::CaughtUp);
                }
                message = self.subscription.as_mut().expect("bound watch").messages.next() => {
                    let message = match message {
                        Some(Ok(message)) => message,
                        Some(Err(error)) if error.kind() == push::MessagesErrorKind::MissingHeartbeat => {
                            // Rebind on the same connection; the stream info tells us
                            // whether the last verified revision is still usable.
                            self.subscription = None;
                            continue;
                        }
                        Some(Err(error)) => {
                            let error = Error::Nats(error.to_string());
                            self.binding.recover(error).await?;
                            self.subscription = None;
                            continue;
                        }
                        None => {
                            // A connection that closed for good ends the watch;
                            // resume on its replacement after the last revision.
                            self.binding.recover(Error::Nats("bucket watch closed".into())).await?;
                            self.subscription = None;
                            continue;
                        }
                    };
                    let verified = message.info().ok().and_then(|info| {
                        let key = watch_key(message.subject.as_str(), &self.name)?;
                        let subscription = self.subscription.as_ref().expect("bound watch");
                        if info.stream != subscription.stream || info.consumer != subscription.consumer {
                            return None;
                        }
                        Some((info, key.to_owned()))
                    });
                    let Some((info, key)) = verified else {
                        self.binding.drop_forged(
                            &mut self.forged_warning_logged,
                            &format!("$KV.{}.<valid-key>", self.name),
                            message.subject.as_str(),
                        );
                        continue;
                    };
                    if info.consumer_sequence != self.consumer_sequence + 1 {
                        self.subscription = None;
                        continue;
                    }
                    self.consumer_sequence = info.consumer_sequence;
                    if !self.caught_up && info.pending == 0 { self.marker_pending = true; }
                    if !advance_revision(&mut self.revision, info.stream_sequence) { continue; }
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

async fn open_bucket_subscription(
    connection: NatsConnection,
    name: &str,
    domain: Option<&str>,
    previous_created_at: Option<i128>,
    revision: u64,
) -> Result<Option<BoundWatch>> {
    let client = connection.client();
    let context = match domain {
        Some(domain) => jetstream::with_domain(client.clone(), domain),
        None => jetstream::new(client.clone()),
    };
    let stream_name = format!("KV_{name}");
    let stream = match context.get_stream(&stream_name).await {
        Ok(stream) => stream,
        Err(error) => {
            if previous_created_at.is_some()
                && matches!(error.kind(), GetStreamErrorKind::JetStream(server)
                    if server.error_code() == ErrorCode::STREAM_NOT_FOUND)
            {
                return Ok(None);
            }
            return Err(Error::Nats(error.to_string()));
        }
    };
    let info = stream.cached_info();
    let created_at = info.created.unix_timestamp_nanos();
    let reset = previous_created_at.is_some_and(|previous| previous != created_at)
        || info.state.last_sequence < revision;
    let consumer = context
        .create_consumer_on_stream(
            push::Config {
                deliver_subject: client.new_inbox(),
                filter_subject: format!("$KV.{name}.>"),
                deliver_policy: if reset || revision == 0 {
                    DeliverPolicy::LastPerSubject
                } else {
                    DeliverPolicy::ByStartSequence {
                        start_sequence: revision + 1,
                    }
                },
                ack_policy: AckPolicy::None,
                max_deliver: 1,
                max_ack_pending: 0,
                flow_control: true,
                idle_heartbeat: WATCH_IDLE_HEARTBEAT,
                inactive_threshold: Duration::from_secs(30),
                num_replicas: 1,
                memory_storage: true,
                ..Default::default()
            },
            stream_name,
        )
        .await
        .map_err(|error| consumer_error(name, error))?;
    let info = consumer.cached_info();
    Ok(Some(BoundWatch {
        empty: info.num_pending == 0,
        created_at,
        reset,
        subscription: WatchSubscription {
            stream: info.stream_name.clone(),
            consumer: info.name.clone(),
            messages: consumer
                .messages()
                .await
                .map_err(|error| Error::Nats(error.to_string()))?,
        },
    }))
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
