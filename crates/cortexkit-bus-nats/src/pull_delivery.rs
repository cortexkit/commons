use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::consumer::{pull, Consumer};
use async_nats::jetstream::message::Info;
use cortexkit_bus_trait::{BusError, BusResult};
use futures_util::StreamExt;

use crate::error::map_operation_error;
use crate::kv_watch::key_matches;
use crate::NatsConnection;

/// The server identity and subject filters every delivery on one pull durable
/// must match before its bus headers can be trusted.
#[derive(Clone)]
pub(crate) struct PullOrigin {
    stream: String,
    consumer: String,
    filters: Vec<String>,
    warned_foreign: Arc<AtomicBool>,
}

pub(crate) struct GenuineDelivery {
    pub(crate) message: async_nats::jetstream::Message,
    pub(crate) stream_sequence: u64,
    pub(crate) delivered: i64,
}

impl PullOrigin {
    /// Captures the durable's filters. An unfiltered durable accepts the
    /// stream's subjects, which requires an additional privileged lookup.
    pub(crate) async fn bind(
        connection: &NatsConnection,
        consumer: &Consumer<pull::Config>,
        stream: &str,
        durable: &str,
    ) -> BusResult<Self> {
        let config = &consumer.cached_info().config;
        let mut filters = Vec::new();
        if !config.filter_subject.is_empty() {
            filters.push(config.filter_subject.clone());
        }
        filters.extend(config.filter_subjects.iter().cloned());
        if filters.is_empty() {
            let api_subject = format!("$JS.API.STREAM.INFO.{stream}");
            let mut events = connection.event_receiver();
            let stream_handle =
                connection
                    .jetstream()
                    .get_stream(stream)
                    .await
                    .map_err(|error| {
                        let mapped = map_operation_error(&api_subject, error, &mut events);
                        stream_info_bind_error(mapped, stream, durable, &api_subject)
                    })?;
            filters = stream_handle.cached_info().config.subjects.clone();
            // NATS gives a stream with no explicit subjects its own name as
            // the sole subject, so preserve that default without broadening
            // the allowlist.
            if filters.is_empty() {
                filters.push(stream.to_owned());
            }
        }
        Ok(Self {
            stream: stream.to_owned(),
            consumer: durable.to_owned(),
            filters,
            warned_foreign: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Checks metadata before decoding. Parser failures and every origin or
    /// subject mismatch are indistinguishable to the caller: all are foreign.
    fn classify(&self, message: &async_nats::jetstream::Message) -> Option<(u64, i64)> {
        self.classify_info(message.subject.as_str(), message.info())
    }

    fn classify_info<'a>(
        &self,
        subject: &str,
        info: Result<Info<'a>, async_nats::Error>,
    ) -> Option<(u64, i64)> {
        let info = info.ok()?;
        if info.stream != self.stream || info.consumer != self.consumer {
            return None;
        }
        if !self
            .filters
            .iter()
            .any(|filter| subject_matches(filter, subject))
        {
            return None;
        }
        Some((info.stream_sequence, info.delivered))
    }

    /// Counts and logs a foreign delivery without ever using its reply subject.
    fn drop_foreign(&self, connection: &NatsConnection, subject: &str) {
        connection.record_forged_delivery();
        if self.warned_foreign.swap(true, Ordering::Relaxed) {
            tracing::debug!(
                stream = %self.stream,
                durable = %self.consumer,
                %subject,
                "dropped a pull delivery from outside the bound durable"
            );
        } else {
            tracing::warn!(
                stream = %self.stream,
                durable = %self.consumer,
                %subject,
                "dropped a pull delivery from outside the bound durable"
            );
        }
    }
}

/// Fetches one genuine delivery. Every foreign delivery ends its one-message
/// batch and starts another server fetch, so a full forged inbox cannot make
/// this loop consume buffered messages without a server round trip.
pub(crate) async fn fetch_genuine(
    connection: &NatsConnection,
    consumer: &Consumer<pull::Config>,
    origin: &PullOrigin,
    pull_expires: Duration,
    operation: &'static str,
) -> BusResult<Option<GenuineDelivery>> {
    let mut events = connection.event_receiver();
    loop {
        let mut messages = consumer
            .fetch()
            .max_messages(1)
            .expires(pull_expires)
            .messages()
            .await
            .map_err(|error| map_operation_error(operation, error, &mut events))?;
        let Some(message) = messages.next().await else {
            return Ok(None);
        };
        let message =
            message.map_err(|error| map_operation_error(operation, error, &mut events))?;
        let Some((stream_sequence, delivered)) = origin.classify(&message) else {
            origin.drop_foreign(connection, message.subject.as_str());
            continue;
        };
        return Ok(Some(GenuineDelivery {
            message,
            stream_sequence,
            delivered,
        }));
    }
}

/// NATS subject wildcards have the same token grammar as the KV watch helper.
fn subject_matches(filter: &str, subject: &str) -> bool {
    key_matches(filter, subject)
}

fn stream_info_bind_error(
    error: BusError,
    stream: &str,
    durable: &str,
    api_subject: &str,
) -> BusError {
    let context = format!(
        "binding unfiltered durable {durable} on stream {stream} requires {api_subject} permission to read the stream subjects"
    );
    match error {
        BusError::Denied { subject, reason } => {
            BusError::denied(subject, format!("{context}: {reason}"))
        }
        BusError::Unavailable {
            reason,
            exhausted_limit,
        } => BusError::Unavailable {
            reason: format!("{context}: {reason}"),
            exhausted_limit,
        },
        other => BusError::unavailable(format!("{context}: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{subject_matches, PullOrigin};

    fn origin(filters: &[&str]) -> PullOrigin {
        PullOrigin {
            stream: "EVENTS".into(),
            consumer: "durable".into(),
            filters: filters.iter().map(|filter| (*filter).to_owned()).collect(),
            warned_foreign: Default::default(),
        }
    }

    fn info<'a>(stream: &'a str, consumer: &'a str) -> async_nats::jetstream::message::Info<'a> {
        async_nats::jetstream::message::Info {
            domain: None,
            acc_hash: None,
            stream,
            consumer,
            stream_sequence: 41,
            consumer_sequence: 7,
            delivered: 2,
            pending: 0,
            published: async_nats::datetime::from_nanos(0).expect("epoch"),
            token: None,
        }
    }

    #[test]
    fn genuine_metadata_and_matching_subject_are_accepted() {
        let accepted = origin(&["events.one"])
            .classify_info("events.one", Ok(info("EVENTS", "durable")))
            .expect("genuine delivery");
        assert_eq!(accepted, (41, 2));
    }

    #[test]
    fn a_wrong_stream_is_foreign() {
        assert!(origin(&["events.one"])
            .classify_info("events.one", Ok(info("OTHER", "durable")))
            .is_none());
    }

    #[test]
    fn a_wrong_consumer_is_foreign() {
        assert!(origin(&["events.one"])
            .classify_info("events.one", Ok(info("EVENTS", "other")))
            .is_none());
    }

    #[test]
    fn an_unparseable_reply_is_foreign() {
        let parse_error = std::io::Error::new(std::io::ErrorKind::InvalidData, "bad token number");
        assert!(origin(&["events.one"])
            .classify_info("events.one", Err(parse_error.into()))
            .is_none());
    }

    #[test]
    fn a_subject_outside_the_filter_is_foreign() {
        let origin = origin(&["events.one"]);
        assert!(origin
            .classify_info("events.two", Ok(info("EVENTS", "durable")))
            .is_none());
    }

    #[test]
    fn every_one_of_multiple_filter_subjects_is_accepted() {
        let origin = origin(&["events.one", "events.two", "events.three.>"]);
        for subject in ["events.one", "events.two", "events.three.four"] {
            assert!(
                origin
                    .classify_info(subject, Ok(info("EVENTS", "durable")))
                    .is_some(),
                "{subject}"
            );
        }
        assert!(origin
            .classify_info("events.four", Ok(info("EVENTS", "durable")))
            .is_none());
    }

    #[test]
    fn an_empty_durable_filter_uses_the_stream_subjects() {
        // `PullOrigin::bind` replaces an empty durable filter with this list
        // from STREAM.INFO before any delivery is classified.
        let origin = origin(&["events.one", "events.archive.>"]);
        for subject in ["events.one", "events.archive.one"] {
            assert!(origin
                .classify_info(subject, Ok(info("EVENTS", "durable")))
                .is_some());
        }
        assert!(origin
            .classify_info("events.two", Ok(info("EVENTS", "durable")))
            .is_none());
    }

    #[test]
    fn subject_matching_uses_nats_token_wildcards() {
        for (filter, subject, expected) in [
            ("events.one", "events.one", true),
            ("events.one", "events.two", false),
            ("events.*", "events.one", true),
            ("events.*", "events.one.two", false),
            ("events.*.ready", "events.one.ready", true),
            ("events.*.ready", "events.one.two.ready", false),
            ("events.>", "events.one", true),
            ("events.>", "events.one.two", true),
            ("events.>", "events", false),
            ("events.>", "other.one", false),
        ] {
            assert_eq!(
                subject_matches(filter, subject),
                expected,
                "{filter} / {subject}"
            );
        }
    }
}
