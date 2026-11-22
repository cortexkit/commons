use std::time::Duration;

use async_nats::jetstream::{self, consumer::pull, message::Info, AckKind};
use bytes::Bytes;
use cortexkit_bus_naming::{AccountNames, EventSubject};
use cortexkit_bus_trait::Headers;
use futures_util::StreamExt;

use crate::{binding::ConnectionBinding, BusClient, Error, Result};

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
    /// The reader no longer returns this reason; it discards deliveries with
    /// unparseable metadata.
    BadMetadata,
}

/// A malformed record. Log the reason and call [`Self::term`] to prevent retries.
/// Nothing, including malformed records, is acknowledged automatically.
#[non_exhaustive]
pub struct MalformedDelivery {
    pub raw_subject: String,
    pub reason: MalformedReason,
    /// The sequence in the bound EVENT stream. Readers only yield records
    /// with parseable JetStream metadata, so this is always present.
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
    binding: ConnectionBinding,
    names: AccountNames,
    durable: String,
    bound: Option<BoundConsumer>,
    warned_foreign: bool,
}

struct BoundConsumer {
    consumer: jetstream::consumer::Consumer<pull::Config>,
    filters: Vec<String>,
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
            binding: ConnectionBinding::new(client),
            names: client.names.clone(),
            durable: durable.into(),
            bound: None,
            warned_foreign: false,
        };
        reader.bind().await?;
        Ok(reader)
    }

    async fn bind(&mut self) -> Result<()> {
        let durable = &self.durable;
        let stream = &self.names.streams().event;
        self.bound = Some(
            self.binding
                .bind(|connection| async move {
                    let consumer = jetstream::new(connection.client())
                        .get_consumer_from_stream(durable, stream)
                        .await
                        .map_err(|error| Error::Nats(error.to_string()))?;
                    let config = &consumer.cached_info().config;
                    let mut filters = config.filter_subjects.clone();
                    if !config.filter_subject.is_empty() {
                        filters.push(config.filter_subject.clone());
                    }
                    if filters.is_empty() {
                        return Err(Error::Nats(format!(
                            "EVENT durable {durable} on stream {stream} has no subject filter"
                        )));
                    }
                    Ok(BoundConsumer { consumer, filters })
                })
                .await?,
        );
        Ok(())
    }

    /// Pulls one event, rebinding after credential re-issue if necessary.
    /// This never acknowledges a message. Commit before acknowledging a valid
    /// delivery; log and terminate a malformed record instead of retrying it.
    /// Deliveries outside the bound durable are counted and discarded without
    /// replying, then the reader fetches again.
    /// If the connection closes for good, this waits for the client's
    /// replacement connection and rebinds instead of returning an error.
    /// A closed connection-change stream reports a transport error so the caller
    /// can reconnect the client or report degraded credential maintenance.
    pub async fn next(&mut self) -> Result<EventItem> {
        self.next_from(fetch_one).await
    }

    pub(crate) async fn next_from(
        &mut self,
        mut fetch: impl AsyncFnMut(
            &jetstream::consumer::Consumer<pull::Config>,
        ) -> Result<Option<jetstream::Message>>,
    ) -> Result<EventItem> {
        loop {
            if self.binding.has_changed() || self.bound.is_none() {
                self.bind().await?;
            }
            let bound = self.bound.as_ref().expect("bound consumer");
            let result = tokio::select! {
                biased;
                changed = self.binding.changed() => {
                    changed?;
                    self.bound = None;
                    continue;
                }
                result = fetch(&bound.consumer) => result,
            };
            match result {
                Ok(Some(message)) => {
                    let stream = &self.names.streams().event;
                    let durable = &self.durable;
                    let subject = message.subject.as_str();
                    if belongs_to_durable(message.info(), subject, stream, durable, &bound.filters)
                    {
                        return Ok(decode(&self.names, message));
                    }
                    // Never use the reply subject of a delivery outside the bound durable.
                    self.binding.record_forged_delivery();
                    if self.warned_foreign {
                        tracing::debug!(%stream, %durable, %subject, "dropped a pull delivery from outside the bound durable");
                    } else {
                        self.warned_foreign = true;
                        tracing::warn!(%stream, %durable, %subject, "dropped a pull delivery from outside the bound durable");
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.binding.recover(error).await?;
                    self.bound = None;
                }
            }
        }
    }
}

pub(crate) async fn fetch_one(
    consumer: &jetstream::consumer::Consumer<pull::Config>,
) -> Result<Option<jetstream::Message>> {
    let mut batch = consumer
        .fetch()
        .max_messages(1)
        .expires(Duration::from_secs(30))
        .messages()
        .await
        .map_err(|error| Error::Nats(error.to_string()))?;
    batch
        .next()
        .await
        .transpose()
        .map_err(|error| Error::Nats(error.to_string()))
}

fn belongs_to_durable(
    metadata: std::result::Result<Info<'_>, async_nats::Error>,
    subject: &str,
    stream: &str,
    durable: &str,
    filters: &[String],
) -> bool {
    metadata.is_ok_and(|info| {
        info.stream == stream
            && info.consumer == durable
            && filters
                .iter()
                .any(|filter| subject_matches(filter, subject))
    })
}

fn subject_matches(filter: &str, subject: &str) -> bool {
    let mut tokens = subject.split('.');
    for filter_token in filter.split('.') {
        let Some(token) = tokens.next().filter(|token| !token.is_empty()) else {
            return false;
        };
        match filter_token {
            ">" => return tokens.all(|token| !token.is_empty()),
            "*" => {}
            expected if expected == token => {}
            _ => return false,
        }
    }
    tokens.next().is_none()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn info<'a>(stream: &'a str, consumer: &'a str) -> Info<'a> {
        Info {
            domain: None,
            acc_hash: None,
            stream,
            consumer,
            stream_sequence: 41,
            consumer_sequence: 1,
            delivered: 1,
            pending: 0,
            published: async_nats::datetime::from_nanos(0).expect("epoch"),
            token: None,
        }
    }

    #[test]
    fn event_origin_requires_matching_stream_and_consumer() {
        let filters = ["events.one".to_owned()];
        for (stream, consumer, expected) in [
            ("EVENT", "reader", true),
            ("OTHER", "reader", false),
            ("EVENT", "other", false),
        ] {
            assert_eq!(
                belongs_to_durable(
                    Ok(info(stream, consumer)),
                    "events.one",
                    "EVENT",
                    "reader",
                    &filters
                ),
                expected,
                "{stream}/{consumer}"
            );
        }
    }

    #[test]
    fn event_origin_rejects_unparseable_reply_metadata() {
        let invalid_metadata =
            std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid reply");
        assert!(!belongs_to_durable(
            Err(invalid_metadata.into()),
            "events.one",
            "EVENT",
            "reader",
            &["events.one".into()],
        ));
    }

    #[test]
    fn event_origin_requires_one_matching_filter_with_nats_token_wildcards() {
        let filters = [
            "events.one".into(),
            "events.*.ready".into(),
            "archive.>".into(),
        ];
        for (subject, expected) in [
            ("events.one", true),
            ("events.two", false),
            ("events.two.ready", true),
            ("events.ready", false),
            ("events.two.three.ready", false),
            ("events..ready", false),
            ("archive.one", true),
            ("archive.one.two", true),
            ("archive", false),
            ("archive.one.", false),
        ] {
            assert_eq!(
                belongs_to_durable(
                    Ok(info("EVENT", "reader")),
                    subject,
                    "EVENT",
                    "reader",
                    &filters
                ),
                expected,
                "{subject}"
            );
        }
    }
}
