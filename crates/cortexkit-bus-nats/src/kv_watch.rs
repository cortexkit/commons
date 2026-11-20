//! An ordered push consumer over a KV bucket's stream that checks every
//! delivery's origin before trusting it.
//!
//! async-nats's own `kv::Watch` panics on a message whose subject is not under
//! `$KV.<bucket>.`, and its `push::Ordered` stream records a message's stream
//! sequence as the resume point before anyone can check where the message
//! came from. Such messages can reach a watch's delivery inbox: anyone allowed
//! to create a JetStream consumer may point a push consumer's delivery subject
//! at any subject, the server does not check that subject against their
//! permissions, and each delivered message keeps its original subject and the
//! foreign stream's acknowledgement reply. So this module reads the delivery
//! subscription itself and drops any message that did not come from the
//! consumer it created on the watched bucket, before the message can touch the
//! sequence bookkeeping.
//!
//! The consumer is created with async-nats's `push::OrderedConfig`, so the
//! server-side settings match the KV watch it replaces: no acknowledgements,
//! flow control, idle heartbeats, memory storage and an inactivity reaper. The
//! client-side bookkeeping mirrors `push::Ordered::poll_next` in async-nats
//! 0.50.0 (`src/jetstream/consumer/push.rs`, lines 550-763): idle heartbeats
//! whose `Nats-Last-Consumer` disagrees with the last delivered consumer
//! sequence, a consumer-sequence gap, a reconnect, and two missed heartbeat
//! intervals each recreate the consumer from the last delivered stream
//! sequence on a fresh inbox (`recreate_ephemeral_consumer`, lines 871-905),
//! and flow-control requests are answered with an empty reply.

use std::time::Duration;

use async_nats::jetstream::consumer::push::OrderedConfig;
use async_nats::jetstream::consumer::DeliverPolicy;
use async_nats::jetstream::kv::Operation;
use async_nats::{Message, StatusCode, Subscriber};
use bytes::Bytes;
use cortexkit_bus_trait::{BusError, BusResult};
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::{NatsConnection, NatsConnectionState};

/// The idle-heartbeat interval `push::OrderedConfig` gives the consumer.
const IDLE_HEARTBEAT: Duration = Duration::from_secs(5);
/// How long a consumer recreation may take before it is retried, as in
/// async-nats's ordered consumer.
const RECREATE_TIMEOUT: Duration = Duration::from_secs(5);
const KV_OPERATION: &str = "KV-Operation";
const NATS_MARKER_REASON: &str = "Nats-Marker-Reason";
const NATS_LAST_CONSUMER: &str = "Nats-Last-Consumer";

/// One accepted KV change, already checked against the watched bucket.
pub(crate) struct KvEntry {
    pub(crate) key: String,
    pub(crate) value: Bytes,
    pub(crate) revision: u64,
    pub(crate) operation: Operation,
    /// The watch has caught up with the bucket: no message was pending when
    /// this one, or an earlier one, was delivered.
    pub(crate) seen_current: bool,
}

/// What became of one data message read from the delivery inbox.
enum Delivery {
    Accepted(KvEntry),
    /// Our own consumer skipped a sequence, so the consumer is recreated.
    Gap,
    /// The message did not come from our consumer on the watched bucket.
    Foreign(String),
}

pub(crate) struct KvDeliveries {
    connection: NatsConnection,
    jetstream: async_nats::jetstream::Context,
    bucket: String,
    stream_name: String,
    /// `$KV.<bucket>.`: every genuine delivery's subject starts with it.
    key_prefix: String,
    /// The key pattern the watch filters on, such as `>` or `module.>`.
    key_filter: String,
    config: OrderedConfig,
    consumer_name: String,
    subscriber: Option<Subscriber>,
    stream_sequence: u64,
    consumer_sequence: u64,
    heartbeat_deadline: Option<Instant>,
    connection_state: watch::Receiver<NatsConnectionState>,
    last_state: NatsConnectionState,
    seen_current: bool,
    warned_foreign: bool,
}

impl KvDeliveries {
    /// Creates the watch consumer and subscribes to its delivery inbox.
    ///
    /// `key_filter` is a key pattern relative to the bucket; `many` selects
    /// the multi-filter consumer field, as `kv::Store::watch_many_with_history`
    /// does, instead of the single filter `kv::Store::watch_with_history` uses.
    pub(crate) async fn open(
        connection: NatsConnection,
        store: &async_nats::jetstream::kv::Store,
        key_filter: String,
        many: bool,
    ) -> Result<Self, async_nats::Error> {
        let subject = format!("{}{}", store.prefix, key_filter);
        let mut config = OrderedConfig {
            description: Some("kv watch consumer".to_string()),
            replay_policy: async_nats::jetstream::consumer::ReplayPolicy::Instant,
            deliver_policy: DeliverPolicy::LastPerSubject,
            ..Default::default()
        };
        if many {
            config.filter_subjects = vec![subject];
        } else {
            config.filter_subject = subject;
        }
        let connection_state = connection.connection_state();
        let last_state = *connection_state.borrow();
        let mut deliveries = Self {
            jetstream: connection.jetstream(),
            connection,
            bucket: store.name.clone(),
            stream_name: store.stream_name.clone(),
            key_prefix: store.prefix.clone(),
            key_filter,
            config,
            consumer_name: String::new(),
            subscriber: None,
            stream_sequence: 0,
            consumer_sequence: 0,
            heartbeat_deadline: None,
            connection_state,
            last_state,
            seen_current: false,
            warned_foreign: false,
        };
        let num_pending = deliveries
            .create(DeliverPolicy::LastPerSubject, None)
            .await?;
        deliveries.seen_current = num_pending == 0;
        Ok(deliveries)
    }

    /// Subscribes to a fresh inbox, then creates the consumer delivering to
    /// it, and returns the consumer's pending count at creation. Subscribing
    /// first means no delivery can arrive before the subscription exists.
    async fn create(
        &mut self,
        deliver_policy: DeliverPolicy,
        timeout: Option<Duration>,
    ) -> Result<u64, async_nats::Error> {
        let client = self.connection.client();
        let deliver_subject = client.new_inbox();
        let subscriber = client.subscribe(deliver_subject.clone()).await?;
        let config = OrderedConfig {
            deliver_subject,
            deliver_policy,
            ..self.config.clone()
        };
        let create = self
            .jetstream
            .create_consumer_on_stream(config, &self.stream_name);
        let consumer = match timeout {
            Some(limit) => tokio::time::timeout(limit, create).await??,
            None => create.await?,
        };
        let info = consumer.cached_info();
        self.consumer_name = info.name.clone();
        self.subscriber = Some(subscriber);
        self.consumer_sequence = 0;
        self.heartbeat_deadline = None;
        Ok(info.num_pending)
    }

    /// Recreates the consumer after the last delivered stream sequence,
    /// retrying with async-nats's ordered-consumer backoff until it succeeds.
    /// Returns false, without recreating, once the connection has closed for
    /// good.
    async fn recreate(&mut self) -> bool {
        // Before anything is delivered there is no resume point, so the
        // original request (the latest revision of every key) is repeated.
        let deliver_policy = if self.stream_sequence == 0 {
            self.config.deliver_policy
        } else {
            DeliverPolicy::ByStartSequence {
                start_sequence: self.stream_sequence + 1,
            }
        };
        let mut attempt = 0u32;
        loop {
            if !self.wait_until_connected().await {
                return false;
            }
            match self.create(deliver_policy, Some(RECREATE_TIMEOUT)).await {
                Ok(_) => return true,
                Err(error) => {
                    attempt += 1;
                    tracing::debug!(bucket = %self.bucket, %error, attempt, "register watch consumer recreation failed");
                    let delay = if attempt < 5 {
                        Duration::from_millis(500 * u64::from(attempt))
                    } else {
                        Duration::from_secs(10)
                    };
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// Waits for the client to be connected; false once it has closed.
    async fn wait_until_connected(&mut self) -> bool {
        loop {
            let state = *self.connection_state.borrow_and_update();
            self.last_state = state;
            match state {
                NatsConnectionState::Connected => return true,
                NatsConnectionState::Closed => return false,
                NatsConnectionState::Disconnected => {
                    if self.connection_state.changed().await.is_err() {
                        return false;
                    }
                }
            }
        }
    }

    /// Drops the current subscription so the next read recreates the
    /// consumer from the last delivered stream sequence.
    fn reset(&mut self) {
        self.subscriber = None;
        self.consumer_sequence = 0;
        self.heartbeat_deadline = None;
    }

    fn observe_state(&mut self, state: NatsConnectionState) {
        match (self.last_state, state) {
            // The consumer is memory-backed and may not survive whatever cut
            // the connection, so it is recreated once the client is back.
            (NatsConnectionState::Connected, NatsConnectionState::Disconnected)
            | (NatsConnectionState::Disconnected, NatsConnectionState::Connected) => self.reset(),
            _ => {}
        }
        self.last_state = state;
    }

    /// Returns the next accepted change, or `None` once the delivery
    /// subscription or the connection has ended. Cancelling this future loses
    /// no accepted message: a message is only taken off the subscription once
    /// it can be handled without waiting.
    pub(crate) async fn next(&mut self) -> BusResult<Option<KvEntry>> {
        loop {
            if self.subscriber.is_none() {
                if !self.recreate().await {
                    return Ok(None);
                }
                continue;
            }
            let deadline = *self
                .heartbeat_deadline
                .get_or_insert_with(|| Instant::now() + IDLE_HEARTBEAT * 2);
            let Some(subscriber) = self.subscriber.as_mut() else {
                continue;
            };
            enum Wake {
                /// The connection state changed; false once its sender is gone.
                State(bool),
                Message(Option<Message>),
                HeartbeatMissed,
            }
            let wake = tokio::select! {
                biased;
                changed = self.connection_state.changed() => Wake::State(changed.is_ok()),
                message = subscriber.next() => Wake::Message(message),
                () = tokio::time::sleep_until(deadline) => Wake::HeartbeatMissed,
            };
            let message = match wake {
                Wake::State(true) => {
                    let state = *self.connection_state.borrow_and_update();
                    self.observe_state(state);
                    continue;
                }
                Wake::State(false) => return Ok(None),
                Wake::HeartbeatMissed => {
                    self.reset();
                    return Err(BusError::unavailable(
                        "register watch missed its consumer's idle heartbeats; recreating the consumer",
                    ));
                }
                Wake::Message(None) => return Ok(None),
                Wake::Message(Some(message)) => message,
            };
            self.heartbeat_deadline = None;
            match message.status {
                Some(StatusCode::IDLE_HEARTBEAT) => {
                    self.handle_heartbeat(message);
                    continue;
                }
                Some(_) => continue,
                None => {}
            }
            match self.classify(message) {
                Delivery::Accepted(entry) => return Ok(Some(entry)),
                Delivery::Gap => self.reset(),
                Delivery::Foreign(subject) => self.drop_foreign(subject),
            }
        }
    }

    /// Handles an idle heartbeat or a flow-control request; both arrive with
    /// status 100.
    fn handle_heartbeat(&mut self, message: Message) {
        if let Some(sequence) = message
            .headers
            .as_ref()
            .and_then(|headers| headers.get_last(NATS_LAST_CONSUMER))
        {
            // An unreadable sequence cannot confirm the consumer is in step,
            // so it is treated like a mismatch rather than ending the watch.
            if sequence.as_str().parse::<u64>().ok() != Some(self.consumer_sequence) {
                tracing::debug!(bucket = %self.bucket, "register watch heartbeat sequence mismatch; recreating consumer");
                self.reset();
            }
        }
        if let Some(reply) = message.reply {
            let client = self.connection.client();
            tokio::spawn(async move {
                client.publish(reply, Bytes::new()).await.ok();
            });
        }
    }

    /// Decides whether a data message is a genuine change from our consumer
    /// on the watched bucket. Origin is checked before the sequence, so a
    /// foreign message never moves the resume point.
    fn classify(&mut self, message: Message) -> Delivery {
        let subject = message.subject.to_string();
        let message = async_nats::jetstream::Message {
            message,
            context: self.jetstream.clone(),
        };
        let Ok(info) = message.info() else {
            return Delivery::Foreign(subject);
        };
        if info.stream != self.stream_name || info.consumer != self.consumer_name {
            return Delivery::Foreign(subject);
        }
        let key = match subject.strip_prefix(&self.key_prefix) {
            Some(key) if is_valid_key(key) && key_matches(&self.key_filter, key) => key,
            _ => return Delivery::Foreign(subject),
        };
        if info.consumer_sequence != self.consumer_sequence + 1 {
            tracing::debug!(
                bucket = %self.bucket,
                expected = self.consumer_sequence + 1,
                received = info.consumer_sequence,
                "register watch consumer sequence gap; recreating consumer"
            );
            return Delivery::Gap;
        }
        self.consumer_sequence = info.consumer_sequence;
        self.stream_sequence = info.stream_sequence;
        if !self.seen_current && info.pending == 0 {
            self.seen_current = true;
        }
        Delivery::Accepted(KvEntry {
            key: key.to_owned(),
            value: message.message.payload.clone(),
            revision: info.stream_sequence,
            operation: operation(&message.message),
            seen_current: self.seen_current,
        })
    }

    fn drop_foreign(&mut self, subject: String) {
        self.connection.record_forged_delivery();
        if self.warned_foreign {
            tracing::debug!(bucket = %self.bucket, %subject, "dropped a register watch delivery from outside the watched bucket");
        } else {
            self.warned_foreign = true;
            tracing::warn!(bucket = %self.bucket, %subject, "dropped a register watch delivery from outside the watched bucket");
        }
    }
}

/// Reads a change's operation the way async-nats's KV watch does: the
/// `KV-Operation` header, else a server marker's reason, else a put. An
/// unrecognised value is read as a put, as the KV watch does.
fn operation(message: &Message) -> Operation {
    let Some(headers) = message.headers.as_ref() else {
        return Operation::Put;
    };
    if let Some(value) = headers.get(KV_OPERATION) {
        return match value.as_str() {
            "DEL" => Operation::Delete,
            "PURGE" => Operation::Purge,
            _ => Operation::Put,
        };
    }
    match headers.get(NATS_MARKER_REASON).map(|value| value.as_str()) {
        Some("MaxAge" | "Purge") => Operation::Purge,
        Some("Remove") => Operation::Delete,
        _ => Operation::Put,
    }
}

/// async-nats's KV key grammar.
pub(crate) fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with('.')
        && !key.ends_with('.')
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-/_=.".contains(&byte))
}

/// Whether `key` matches the NATS subject pattern `filter`, where `*` matches
/// one token and a final `>` matches one or more.
pub(crate) fn key_matches(filter: &str, key: &str) -> bool {
    let mut filter_tokens = filter.split('.');
    let mut key_tokens = key.split('.');
    loop {
        match (filter_tokens.next(), key_tokens.next()) {
            (Some(">"), Some(_)) => return true,
            (Some("*"), Some(_)) => {}
            (Some(expected), Some(token)) if expected == token => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}
