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

/// What [`NatsConnection::close`] observed before returning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloseOutcome {
    /// The connection's handler has exited: closure was observed, or the
    /// command channel was already gone. This does not by itself confirm a
    /// graceful drain; a configured reconnect limit can also end a connection.
    /// The underlying NATS connection, shared by every clone and every handle
    /// built from it, accepts no new NATS operations. Local accessors and
    /// already-buffered register watch results may still succeed.
    Closed,
    /// Closure was not confirmed in time, usually because the connection is
    /// disconnected and reconnecting. A background task keeps trying to queue
    /// the drain, and the drain runs if the connection reconnects and resumes
    /// processing commands.
    Pending,
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

    /// Requests a connection-wide drain and waits for the connection to close.
    ///
    /// Draining unsubscribes every subscription, flushes queued messages and
    /// closes the connection without reconnecting. The underlying connection is
    /// shared by every clone of this one and every handle built from it.
    ///
    /// It waits for closure for `deadline`, then makes one final probe that can
    /// take up to another 50 ms, and returns [`CloseOutcome::Pending`] if
    /// closure is still unconfirmed.
    ///
    /// While async-nats is reconnecting it processes no client commands, so
    /// this call can't make a queued drain run sooner. With unlimited
    /// reconnects (the default), a client that can never reconnect keeps
    /// retrying for the life of its runtime; a configured reconnect limit ends
    /// it without draining. So a caller replacing a connection must keep the old
    /// one able to authenticate, through its configured auth options or auth
    /// callback, for the drain to complete after an outage. `close` doesn't
    /// refresh credentials.
    pub async fn close(&self, deadline: Duration) -> BusResult<CloseOutcome> {
        // Subscribe before requesting the drain to avoid a subscribe-after-
        // drain race. A `Closed` event emitted earlier, or dropped, is caught
        // by the enqueue checks instead.
        let mut events = self.events.subscribe();
        // Keep the drain's enqueue attempt alive after this call returns, even
        // while it waits for room in a full command queue.
        let client = self.client.clone();
        let mut queued = Some(tokio::spawn(async move { client.drain().await.is_ok() }));
        let closed = async {
            loop {
                tokio::select! {
                    result = async { queued.as_mut().expect("guarded by is_some").await }, if queued.is_some() => {
                        queued = None;
                        // Only a refused enqueue means the command channel is
                        // gone. A failed task proves nothing about the
                        // connection, so keep waiting for the event.
                        if let Ok(false) = result {
                            return true;
                        }
                    }
                    event = events.recv() => match event {
                        Ok(Event::Closed) => return true,
                        // A lagging receiver may have dropped `Closed`; the
                        // final probe covers that case.
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return false,
                    },
                }
            }
        };
        if let Ok(true) = tokio::time::timeout(deadline, closed).await {
            return Ok(CloseOutcome::Closed);
        }
        // Once the connection's handler has exited, a drain is refused at once
        // because its command channel is gone. A live connection accepts it,
        // which is harmless, or keeps it waiting behind a full queue.
        match tokio::time::timeout(Duration::from_millis(50), self.client.drain()).await {
            Ok(Err(_)) => Ok(CloseOutcome::Closed),
            _ => Ok(CloseOutcome::Pending),
        }
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
