#![doc = include_str!("../README.md")]

mod binding;
mod buckets;
mod credentials;
mod events;
mod hub;

pub use buckets::{Bucket, BucketEntry, BucketItem, BucketWatch};
pub use events::{EventDelivery, EventItem, EventReader, MalformedDelivery, MalformedReason};
pub use hub::HubClient;

use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use bytes::Bytes;
use cortexkit_bus_naming::{validate_token, AccountNames, TokenKind};
use cortexkit_bus_nats::{ConnectConfig, NatsConnection};
use cortexkit_bus_trait::{ContentDigest, Headers, PublishReceipt, Stream};

/// The header carrying a body's SHA-256, spelled as the NATS codec writes it
/// for claim checks.
const CONTENT_DIGEST_HEADER: &str = "Ck-Content-Digest";
use subc_client_rs::SubcConsumer;
use tokio::sync::watch;

use credentials::CredentialManager;

/// Errors contain refusal codes, not credential JWTs or nonce signatures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("ck-bus refused the request: {0}")]
    Refused(String),
    /// Retry when the module next needs the hub.
    #[error("ck-bus refused the hub credential: hub_unavailable")]
    HubUnavailable,
    /// This module is not allowed to use the hub. Do not retry.
    #[error("ck-bus refused the hub credential: hub_not_granted")]
    HubNotGranted,
    #[error("ck-bus request failed: {0}")]
    Transport(String),
    #[error("invalid ck-bus reply: {0}")]
    Reply(&'static str),
    #[error(transparent)]
    Naming(#[from] cortexkit_bus_naming::NamingError),
    #[error(transparent)]
    Bus(#[from] cortexkit_bus_trait::BusError),
    #[error("NATS operation failed: {0}")]
    Nats(String),
    #[error("event body is {size} bytes; the maximum is 4096 bytes")]
    EventBodyTooLarge { size: usize },
    #[error("durable {durable} is not owned by module {module}")]
    DurableNotOwned { durable: String, module: String },
    #[error("module bucket {owner}.{bucket} is not declared")]
    BucketNotDeclared { owner: String, bucket: String },
    #[error("module {module} may not read bucket {bucket}")]
    BucketNotReadable { module: String, bucket: String },
    #[error("module {module} does not own bucket {bucket}")]
    BucketNotOwned { module: String, bucket: String },
    #[error("bucket value is {size} bytes; the maximum is {cap} bytes")]
    BucketValueTooLarge { size: usize, cap: u32 },
    #[error("bucket {bucket} has no watch consumer slots available")]
    WatchCapacity { bucket: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Options for one module's bus connection.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct BusClientOptions {
    module: String,
    pub request_timeout: Duration,
}

impl BusClientOptions {
    pub fn new(module: impl Into<String>) -> Result<Self> {
        let module = module.into();
        validate_token(TokenKind::ModuleId, &module)?;
        Ok(Self {
            module,
            request_timeout: Duration::from_secs(5),
        })
    }
}

/// A module's current connection and the task that maintains its credentials.
pub struct BusClient {
    managed: ManagedConnection,
    names: AccountNames,
    module: String,
}

struct ManagedConnection {
    connection: Arc<RwLock<NatsConnection>>,
    changes: watch::Receiver<u64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ManagedConnection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl BusClient {
    /// Issues a credential for the module identity that the subc daemon
    /// authenticates from the module's launch nonce.
    pub async fn connect(subc: Arc<SubcConsumer>, opts: BusClientOptions) -> Result<Self> {
        let manager = credentials::module_manager(subc, &opts, false).await?;
        Self::connect_manager(manager, opts).await
    }

    async fn connect_manager(
        manager: Arc<CredentialManager>,
        opts: BusClientOptions,
    ) -> Result<Self> {
        let initial = manager.credential().await;
        let names = AccountNames::derive(&initial.acct)?;
        let managed = ManagedConnection::connect(manager, opts.request_timeout).await?;
        Ok(Self {
            managed,
            names,
            module: opts.module,
        })
    }
}

impl ManagedConnection {
    async fn connect(manager: Arc<CredentialManager>, timeout: Duration) -> Result<Self> {
        let mut revisions = manager.revisions();
        let (connection, mut ended) = loop {
            revisions.borrow_and_update();
            manager.check_hub_refusal()?;
            let credential = manager.credential().await;
            match connect_current(manager.clone(), &credential, timeout).await {
                Ok(current) if !revisions.has_changed().unwrap_or(false) => break current,
                Ok((connection, _)) => {
                    let _ = connection.close(Duration::from_millis(50)).await;
                }
                Err(_)
                    if manager.credential().await.credential_public
                        != credential.credential_public => {}
                Err(error) => {
                    manager.check_hub_refusal()?;
                    return Err(error);
                }
            }
        };
        let connection = Arc::new(RwLock::new(connection));
        let shared = connection.clone();
        let (changes, receiver) = watch::channel(0);
        let task = tokio::spawn(async move {
            let mut retry = None;
            loop {
                if manager.check_hub_refusal().is_err() {
                    let current = shared
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    let _ = current.close(Duration::from_millis(50)).await;
                    return;
                }
                let credential = manager.credential().await;
                let delay = retry
                    .take()
                    .unwrap_or_else(|| credentials::renewal_delay(credential.exp));
                tokio::select! {
                    result = revisions.changed() => { if result.is_err() { return; } }
                    () = connection_ended(&mut ended) => {
                        // Replace it through the same path as a re-issue, so
                        // event readers and bucket watches rebind to the new one.
                        tracing::warn!("bus connection closed for good; replacing it");
                    }
                    () = tokio::time::sleep(delay) => {
                        if let Err(error) = manager.renew().await {
                            if matches!(error, Error::HubUnavailable | Error::HubNotGranted) {
                                continue;
                            }
                            if credentials::transient(&error) || matches!(error, Error::Transport(_)) {
                                retry = Some(Duration::from_secs(5));
                                continue;
                            }
                            // A terminal refusal (notably generation_not_live)
                            // must not cause issuance under a dead generation.
                            let refusal_code = match &error {
                                Error::Refused(code) => code.as_str(),
                                _ => "client_renewal_error",
                            };
                            tracing::warn!(
                                refusal_code,
                                "bus credential maintenance stopped; reconnect or report degraded"
                            );
                            return;
                        }
                        if !revisions.has_changed().unwrap_or(false) { continue; }
                    }
                }
                let mut backoff = Duration::from_millis(250);
                loop {
                    let revision = *revisions.borrow_and_update();
                    if manager.check_hub_refusal().is_err() {
                        break;
                    }
                    let credential = manager.credential().await;
                    match connect_current(manager.clone(), &credential, timeout).await {
                        Ok((replacement, replacement_ended)) => {
                            if revisions.has_changed().unwrap_or(false) {
                                let _ = replacement.close(Duration::from_millis(50)).await;
                                continue;
                            }
                            let old = {
                                let mut slot = shared
                                    .write()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                std::mem::replace(&mut *slot, replacement)
                            };
                            ended = replacement_ended;
                            changes.send_replace(revision);
                            // A replaced connection that is reconnecting can't
                            // drain, and its auth callback now refuses every
                            // attempt; MAX_RECONNECTS ends it instead.
                            let _ = old.close(Duration::from_millis(50)).await;
                            break;
                        }
                        Err(_) => {
                            if manager.check_hub_refusal().is_err() {
                                break;
                            }
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                        }
                    }
                }
            }
        });
        Ok(Self {
            connection,
            changes: receiver,
            task,
        })
    }

    fn snapshot(&self) -> NatsConnection {
        self.connection
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl BusClient {
    /// Returns a snapshot. A re-issue does not change an already held snapshot.
    pub fn connection(&self) -> NatsConnection {
        self.managed.snapshot()
    }

    /// Changes only after a replacement connection with its issued inbox prefix is ready.
    /// Reopen raw subscriptions and durable pulls when this receiver changes;
    /// [`EventReader`] handles its own rebinding automatically.
    ///
    /// If credential maintenance can no longer renew or re-issue, its task ends
    /// and drops the sender. The receiver's `changed()` then returns `Err` once
    /// any unseen change has been consumed. Reconnect with [`BusClient::connect`]
    /// or report the module as degraded when the change stream closes.
    pub fn connection_changes(&self) -> watch::Receiver<u64> {
        self.managed.changes.clone()
    }

    pub fn acct(&self) -> &str {
        self.names.account()
    }

    /// Returns the computer's box id as supplied by the bus module's account.
    ///
    /// Modules must never derive this id themselves. Returns `None` unless the
    /// account is `box_` followed by a canonical 32-character box id.
    pub fn box_id(&self) -> Option<cortexkit_bus_naming::federation::BoxId> {
        credentials::box_id(self.acct())
    }

    /// Returns the inbox prefix used by the current connection.
    ///
    /// A per-credential prefix changes after credential re-issue; a module's
    /// `ckdel` prefix stays stable. Use [`Self::connection_changes`] to observe
    /// replacements, or [`NatsConnection::inbox_prefix`] for a held snapshot.
    pub fn inbox_prefix(&self) -> String {
        self.connection().inbox_prefix().to_owned()
    }

    /// Attaches to an existing EVENT durable owned by this module.
    ///
    /// ck-bus provisions durables; this method never creates or changes one.
    /// The name must follow bus-naming's `m_{module}[_suffix]` owner rule.
    /// The reader rebinds automatically after credential re-issue. Nothing is
    /// acknowledged automatically: commit your own transaction before `ack()`.
    pub async fn event_reader(&self, durable: &str) -> Result<EventReader> {
        EventReader::attach(self, durable).await
    }

    /// Opens a declared module bucket for its owner or a declared reader.
    /// Does not provision buckets or change their limits.
    pub fn bucket(&self, owner_module: &str, bucket: &str) -> Result<Bucket> {
        Bucket::attach(self, owner_module, bucket)
    }

    /// Stores an event using the fact's own id for JetStream deduplication.
    ///
    /// Event bodies carry ids, counters and enums, not large content. Bodies
    /// larger than 4 KiB are refused; retrieve large content through its owner's
    /// own call instead.
    /// A non-empty body is sent with its SHA-256 as `Ck-Content-Digest`, so a
    /// reader can check the bytes it received. An empty body is sent without one,
    /// because an empty payload with a digest is the claim-check shape.
    /// Inline bodies reach every reader whose durable matches the subject, with
    /// no per-reader visibility check; reserve them for pointers and facts every
    /// declared reader may see. Flow triggers must use [`Self::publish_claim_check`],
    /// the `cortexkit_bus_trait::Stream::publish` claim-check path (empty payload plus
    /// content digest), with the publisher serving the body under the reading
    /// flow's own scope to check which agent may see it. The flow engine refuses
    /// inline trigger bodies.
    pub async fn publish_event(
        &self,
        event: &str,
        version: u32,
        qualifier: Option<&str>,
        msg_id: &str,
        body: impl Into<Bytes>,
    ) -> Result<PublishReceipt> {
        let body = body.into();
        if body.len() > 4096 {
            return Err(Error::EventBodyTooLarge { size: body.len() });
        }
        let subject = self.event_subject(event, version, qualifier)?;
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(async_nats::header::NATS_MESSAGE_ID, msg_id);
        if !body.is_empty() {
            headers.insert(
                CONTENT_DIGEST_HEADER,
                ContentDigest::of_bytes(&body).to_string().as_str(),
            );
        }
        let ack = async_nats::jetstream::new(self.connection().client())
            .publish_with_headers(subject, headers, body)
            .await
            .map_err(|error| Error::Nats(error.to_string()))?
            .await
            .map_err(|error| Error::Nats(error.to_string()))?;
        Ok(PublishReceipt::new(ack.sequence, ack.duplicate))
    }

    /// Publishes an empty-payload claim check carrying the body's content digest.
    /// Store the body before publishing and serve it through the publisher's own
    /// `events_get` query under the reading flow's scope. This is the event shape
    /// for triggering flows; inline bodies cannot be used as flow triggers.
    /// The NATS codec writes Nats-Msg-Id and Ck-Content-Digest and refuses caller
    /// headers spelling either reserved name, including case variants.
    #[allow(clippy::too_many_arguments)]
    pub async fn publish_claim_check(
        &self,
        event: &str,
        version: u32,
        qualifier: Option<&str>,
        msg_id: &str,
        digest: ContentDigest,
        headers: Headers,
    ) -> Result<PublishReceipt> {
        let subject = self.event_subject(event, version, qualifier)?;
        self.connection()
            .stream(&self.names.streams().event)
            .publish(&subject, msg_id, digest, headers)
            .await
            .map_err(Error::from)
    }

    fn event_subject(&self, event: &str, version: u32, qualifier: Option<&str>) -> Result<String> {
        Ok(match qualifier {
            Some(reader) => {
                self.names
                    .event_subject_with_reader(&self.module, event, version, reader)?
            }
            None => self.names.event_subject(&self.module, event, version)?,
        })
    }

    /// Publishes an ephemeral, versioned hint, not a stored fact.
    pub async fn live_hint(&self, kind: &str, version: u32, body: impl Into<Bytes>) -> Result<()> {
        let subject = self.names.live_subject(&self.module, kind, version)?;
        self.connection()
            .client()
            .publish(subject, body.into())
            .await
            .map_err(|error| Error::Nats(error.to_string()))
    }
}

/// How many reconnect attempts async-nats makes after a connection drops before
/// it gives up and ends the connection for good.
///
/// async-nats waits nothing before the first attempt, then 2 ms doubling up to
/// a 4 s cap, so twenty attempts span about 36 s: a ck-bus restart of about
/// 30 s rides through on the same connection. The bound matters most for a
/// replaced connection. Its credential is no longer current, so its auth
/// callback refuses every attempt; without a bound it would keep connecting
/// and failing every 4 s for the life of the process.
const MAX_RECONNECTS: usize = 20;

/// A subject under the connection's own inbox that nothing ever publishes to.
/// A hyphen never appears in async-nats's generated inbox tokens, so it
/// cannot collide with a request inbox.
pub(crate) const LIVENESS_SUBJECT: &str = "bus-client-liveness";

/// Resolves once async-nats has ended the connection for good: after it runs
/// out of reconnect attempts, or after a drain. The connection's handler task
/// then exits and drops every subscription, which ends this stream. The
/// subscription survives ordinary reconnects because async-nats resubscribes it.
/// async-nats 0.50 emits no `Event::Closed` when it gives up reconnecting, only
/// a `ClientError::MaxReconnects` event that can be dropped from a full queue,
/// and `NatsConnection` keeps its event stream private; the ended subscription
/// is the signal that can't be missed.
async fn connection_ended(ended: &mut async_nats::Subscriber) {
    use futures_util::StreamExt;
    while ended.next().await.is_some() {}
}

async fn connect_current(
    manager: Arc<CredentialManager>,
    credential: &credentials::Credential,
    timeout: Duration,
) -> Result<(NatsConnection, async_nats::Subscriber)> {
    let public = credential.credential_public.clone();
    let options = async_nats::ConnectOptions::with_auth_callback(move |nonce| {
        let manager = manager.clone();
        let public = public.clone();
        // The spawned subc request future only needs to be Send; async-nats
        // also requires the callback future to be Sync.
        let task = tokio::spawn(async move {
            let credential = manager.credential().await;
            if credential.credential_public != public {
                return Err(Error::Reply("connection credential was replaced"));
            }
            let signature = manager.sign(&public, nonce).await?;
            let mut auth = async_nats::Auth::new();
            auth.jwt = Some(credential.jwt);
            auth.signature = Some(signature);
            Ok(auth)
        });
        async move {
            task.await
                .map_err(async_nats::AuthError::new)?
                .map_err(async_nats::AuthError::new)
        }
    })
    .connection_timeout(timeout)
    .request_timeout(Some(timeout))
    .max_reconnects(MAX_RECONNECTS);
    let mut config = ConnectConfig::new(&credential.credential_public)?;
    config.request_timeout = timeout;
    let connection = if credential.inbox_prefix == config.inbox_prefix() {
        NatsConnection::connect(credential.server_url.as_str(), options, config).await
    } else {
        NatsConnection::connect_with_inbox_prefix(
            credential.server_url.as_str(),
            options,
            config,
            &credential.inbox_prefix,
        )
        .await
    }
    .map_err(Error::from)?;
    let subject = format!("{}.{LIVENESS_SUBJECT}", connection.inbox_prefix());
    match connection.client().subscribe(subject).await {
        Ok(ended) => Ok((connection, ended)),
        Err(error) => {
            let _ = connection.close(Duration::from_millis(50)).await;
            Err(Error::Nats(error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests;
