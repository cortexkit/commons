use std::time::Duration;

use async_nats::{Client, ConnectOptions, Event, Subscriber, ToServerAddrs};
use async_trait::async_trait;
use cortexkit_bus_trait::{
    AsyncPermissionViolation, BusError, BusResult, PermissionViolationSource,
};
use tokio::sync::broadcast;

use crate::error::{
    drain_permission_violation, map_connect_error, map_operation_error, permission_violation,
    wait_for_permission_violation,
};
use crate::{NatsRegister, NatsStream, NatsWorkQueue, INBOX_ROOT};

const EVENT_BUFFER: usize = 128;

#[derive(Debug, Clone)]
pub struct ConnectConfig {
    pub credential_public: String,
    pub request_timeout: Duration,
}

impl ConnectConfig {
    pub fn new(credential_public: impl Into<String>) -> BusResult<Self> {
        let credential_public = credential_public.into();
        validate_credential_public(&credential_public)?;
        Ok(Self {
            credential_public,
            request_timeout: Duration::from_secs(2),
        })
    }

    pub fn inbox_prefix(&self) -> String {
        format!("{INBOX_ROOT}.{}", self.credential_public)
    }
}

#[derive(Clone)]
pub struct NatsConnection {
    client: Client,
    events: broadcast::Sender<Event>,
    request_timeout: Duration,
    inbox_prefix: String,
}

impl NatsConnection {
    pub async fn connect<A: ToServerAddrs>(
        addrs: A,
        options: ConnectOptions,
        config: ConnectConfig,
    ) -> BusResult<Self> {
        Self::connect_inner(addrs, options, config, true).await
    }

    async fn connect_inner<A: ToServerAddrs>(
        addrs: A,
        options: ConnectOptions,
        config: ConnectConfig,
        set_inbox_prefix: bool,
    ) -> BusResult<Self> {
        validate_credential_public(&config.credential_public)?;
        let inbox_prefix = config.inbox_prefix();
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let event_sender = events.clone();
        let options = options.event_callback(move |event| {
            let event_sender = event_sender.clone();
            async move {
                let _ = event_sender.send(event);
            }
        });
        let options = if set_inbox_prefix {
            options.custom_inbox_prefix(&inbox_prefix)
        } else {
            options
        };
        let client = options.connect(addrs).await.map_err(map_connect_error)?;
        Ok(Self {
            client,
            events,
            request_timeout: config.request_timeout,
            inbox_prefix,
        })
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    pub fn inbox_prefix(&self) -> &str {
        &self.inbox_prefix
    }

    pub fn permission_violations(&self) -> NatsPermissionViolations {
        NatsPermissionViolations {
            receiver: self.events.subscribe(),
        }
    }

    pub fn stream(&self, stream: impl Into<String>) -> NatsStream {
        NatsStream::new(self.clone(), stream.into(), self.request_timeout)
    }

    pub async fn register(&self, bucket: impl Into<String>) -> BusResult<NatsRegister> {
        NatsRegister::bind(self.clone(), bucket.into()).await
    }

    pub async fn work_queue(
        &self,
        stream: impl Into<String>,
        durable: impl Into<String>,
    ) -> BusResult<NatsWorkQueue> {
        NatsWorkQueue::bind(
            self.clone(),
            stream.into(),
            durable.into(),
            self.request_timeout,
        )
        .await
    }

    pub async fn subscribe(&self, subject: &str) -> BusResult<Subscriber> {
        let mut events = self.events.subscribe();
        let subscriber = self
            .client
            .subscribe(subject.to_owned())
            .await
            .map_err(|error| map_operation_error(subject, error, &mut events))?;
        self.client
            .flush()
            .await
            .map_err(|error| map_operation_error(subject, error, &mut events))?;
        if let Some(violation) = drain_permission_violation(&mut events) {
            return Err(violation.as_denied());
        }
        if let Some(violation) =
            wait_for_permission_violation(&mut events, Duration::from_millis(100)).await
        {
            return Err(violation.as_denied());
        }
        Ok(subscriber)
    }

    pub(crate) fn jetstream(&self) -> async_nats::jetstream::Context {
        async_nats::jetstream::new(self.client.clone())
    }

    #[cfg(test)]
    pub(crate) async fn request(
        &self,
        subject: &str,
        payload: bytes::Bytes,
    ) -> BusResult<async_nats::Message> {
        let mut events = self.events.subscribe();
        self.client
            .request(subject.to_owned(), payload)
            .await
            .map_err(|error| map_operation_error(subject, error, &mut events))
    }

    pub(crate) fn event_receiver(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    #[cfg(test)]
    pub(crate) async fn connect_with_library_default<A: ToServerAddrs>(
        addrs: A,
        options: ConnectOptions,
        config: ConnectConfig,
    ) -> BusResult<Self> {
        Self::connect_inner(addrs, options, config, false).await
    }
}

pub struct NatsPermissionViolations {
    receiver: broadcast::Receiver<Event>,
}

#[async_trait]
impl PermissionViolationSource for NatsPermissionViolations {
    async fn next_violation(&mut self) -> BusResult<Option<AsyncPermissionViolation>> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    if let Some(violation) = permission_violation(&event) {
                        return Ok(Some(violation));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return Ok(None),
            }
        }
    }
}

fn validate_credential_public(value: &str) -> BusResult<()> {
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| byte == b'.' || byte == b'*' || byte == b'>' || byte.is_ascii_whitespace())
    {
        return Err(BusError::denied(
            format!("{INBOX_ROOT}.{value}"),
            "credential public key is not a valid inbox subject token",
        ));
    }
    Ok(())
}
