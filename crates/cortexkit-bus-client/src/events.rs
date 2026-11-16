use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use async_nats::jetstream::{self, consumer::pull, AckKind};
use bytes::Bytes;
use cortexkit_bus_naming::{AccountNames, EventSubject};
use cortexkit_bus_nats::NatsConnection;
use cortexkit_bus_trait::Headers;
use futures_util::StreamExt;
use tokio::sync::watch;

use crate::{BusClient, Error, Result};

/// One EVENT pull result. Malformed records can only be terminated, not acked.
#[non_exhaustive]
pub enum EventItem {
    Delivery(EventDelivery),
    Malformed(MalformedDelivery),
}

/// A stored event. Commit your own transaction before calling [`Self::ack`].
/// Dropping a delivery does not acknowledge it; the server may redeliver it.
#[non_exhaustive]
pub struct EventDelivery {
    pub subject: EventSubject,
    pub msg_id: String,
    pub body: Bytes,
    pub stream_sequence: u64,
    pub delivered_count: i64,
    headers: Headers,
    message: jetstream::Message,
}

impl EventDelivery {
    /// Headers copied once from NATS. A repeated header keeps its first value.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Looks up a header by its stored name, keeping the first repeated value.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// Acknowledges the event after the caller's own commit, waiting for the
    /// server to confirm receipt. No reader operation acknowledges automatically.
    pub async fn ack(&self) -> Result<()> {
        self.message
            .double_ack()
            .await
            .map_err(|error| Error::Nats(error.to_string()))
    }

    /// Requests redelivery after a delay (zero requests immediate redelivery).
    pub async fn nak(&self, delay: Duration) -> Result<()> {
        self.message
            .double_ack_with(AckKind::Nak(Some(delay)))
            .await
            .map_err(|error| Error::Nats(error.to_string()))
    }

    /// Permanently stops redelivery of this event.
    pub async fn term(&self) -> Result<()> {
        terminate(&self.message).await
    }
}

/// Why a stored record cannot be exposed as a valid event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MalformedReason {
    MissingMsgId,
    BadSubject,
    BadMetadata,
}

/// A malformed record. Log the reason and call [`Self::term`] to prevent retries.
/// Nothing, including malformed records, is acknowledged automatically.
#[non_exhaustive]
pub struct MalformedDelivery {
    pub raw_subject: String,
    pub reason: MalformedReason,
    /// Absent only when the server supplied malformed JetStream metadata.
    pub stream_sequence: Option<u64>,
    message: jetstream::Message,
}

impl MalformedDelivery {
    /// Permanently stops redelivery of a malformed record.
    pub async fn term(&self) -> Result<()> {
        terminate(&self.message).await
    }
}

async fn terminate(message: &jetstream::Message) -> Result<()> {
    message
        .double_ack_with(AckKind::Term)
        .await
        .map_err(|error| Error::Nats(error.to_string()))
}

/// A one-at-a-time reader of an existing module-owned EVENT durable.
/// The server owns its cursor. Re-issue rebinds the reader to the same durable;
/// unacknowledged messages may redeliver. Keep the [`BusClient`] alive so its
/// credential maintenance continues to run.
pub struct EventReader {
    connection: Arc<RwLock<NatsConnection>>,
    changes: watch::Receiver<u64>,
    names: AccountNames,
    durable: String,
    consumer: Option<jetstream::consumer::Consumer<pull::Config>>,
}

impl EventReader {
    pub(crate) async fn attach(client: &BusClient, durable: &str) -> Result<Self> {
        let base = AccountNames::module_consumer_name(&client.module)?;
        // EventConsumer::durable_name accepts only static declarations; apply
        // the same owner rule directly to a runtime durable name.
        if durable != base
            && !durable
                .strip_prefix(&format!("{base}_"))
                .is_some_and(|suffix| {
                    !suffix.is_empty()
                        && suffix.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                        })
                })
        {
            return Err(Error::DurableNotOwned {
                durable: durable.into(),
                module: client.module.clone(),
            });
        }
        let mut reader = Self {
            connection: client.connection.clone(),
            changes: client.connection_changes(),
            names: client.names.clone(),
            durable: durable.into(),
            consumer: None,
        };
        reader.bind().await?;
        Ok(reader)
    }

    async fn bind(&mut self) -> Result<()> {
        loop {
            self.changes.borrow_and_update();
            let connection = self
                .connection
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let result = jetstream::new(connection.client())
                .get_consumer_from_stream(&self.durable, &self.names.streams().event)
                .await;
            if self.changes.has_changed().unwrap_or(false) {
                continue;
            }
            self.consumer = Some(result.map_err(|error| Error::Nats(error.to_string()))?);
            return Ok(());
        }
    }

    /// Pulls one event, rebinding after credential re-issue if necessary.
    /// This never acknowledges a message. Commit before acknowledging a valid
    /// delivery; log and terminate a malformed record instead of retrying it.
    /// A closed connection-change stream reports a transport error so the caller
    /// can reconnect the client or report degraded credential maintenance.
    pub async fn next(&mut self) -> Result<EventItem> {
        loop {
            if self.changes.has_changed().unwrap_or(false) || self.consumer.is_none() {
                self.bind().await?;
            }
            let consumer = self.consumer.as_ref().expect("bound consumer");
            tokio::select! {
                biased;
                changed = self.changes.changed() => {
                    changed.map_err(|_| Error::Transport("credential maintenance stopped".into()))?;
                    self.consumer = None;
                }
                result = async {
                    let mut batch = consumer.fetch().max_messages(1)
                        .expires(Duration::from_secs(30)).messages().await
                        .map_err(|error| Error::Nats(error.to_string()))?;
                    batch.next().await.transpose().map_err(|error| Error::Nats(error.to_string()))
                } => {
                    if let Some(message) = result? {
                        return Ok(decode(&self.names, message));
                    }
                }
            }
        }
    }
}

fn decode(names: &AccountNames, message: jetstream::Message) -> EventItem {
    let metadata = message.info();
    let stream_sequence = metadata.as_ref().ok().map(|info| info.stream_sequence);
    let subject = names.parse_event_subject(message.subject.as_str());
    let headers: Headers = message
        .headers
        .as_ref()
        .into_iter()
        .flat_map(|headers| headers.iter())
        .filter_map(|(name, values)| {
            values
                .first()
                .map(|value| (name.to_string(), value.to_string()))
        })
        .collect();
    let msg_id = message.headers.as_ref().and_then(|headers| {
        headers
            .get(async_nats::header::NATS_MESSAGE_ID)
            .map(|value| value.as_str().to_owned())
    });
    let reason = if metadata.is_err() {
        Some(MalformedReason::BadMetadata)
    } else if subject.is_err() {
        Some(MalformedReason::BadSubject)
    } else if msg_id.is_none() {
        Some(MalformedReason::MissingMsgId)
    } else {
        None
    };
    if let Some(reason) = reason {
        return EventItem::Malformed(MalformedDelivery {
            raw_subject: message.subject.to_string(),
            reason,
            stream_sequence,
            message,
        });
    }
    let delivered_count = metadata.expect("validated metadata").delivered;
    EventItem::Delivery(EventDelivery {
        subject: subject.expect("validated subject"),
        msg_id: msg_id.expect("validated msg id"),
        body: message.payload.clone(),
        stream_sequence: stream_sequence.expect("validated sequence"),
        delivered_count,
        headers,
        message,
    })
}
