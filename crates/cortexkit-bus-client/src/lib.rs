#![doc = include_str!("../README.md")]

mod binding;
mod buckets;
mod credentials;
mod events;
mod hub;
mod live;

pub use buckets::{Bucket, BucketEntry, BucketItem, BucketWatch};
pub use events::{EventDelivery, EventItem, EventReader, MalformedDelivery, MalformedReason};
pub use hub::HubClient;
pub use live::LiveHints;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock,
    },
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
use tokio::sync::{oneshot, watch};

use credentials::{CredentialManager, REPLACED_CREDENTIAL};

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
    forged_deliveries_dropped: Arc<AtomicU64>,
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
            forged_deliveries_dropped: Arc::new(AtomicU64::new(0)),
        })
    }
}

impl ManagedConnection {
    async fn connect(manager: Arc<CredentialManager>, timeout: Duration) -> Result<Self> {
        let mut revisions = manager.revisions();
        let (connection, lifetime) = loop {
            revisions.borrow_and_update();
            manager.check_hub_refusal()?;
            let credential = manager.credential().await;
            match connect_current(manager.clone(), &credential, timeout).await {
                Ok(current) if !revisions.has_changed().unwrap_or(false) => break current,
                Ok((connection, mut lifetime)) => {
                    lifetime.retire(&connection).await;
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
            // Renewal is a request to ck-bus that can wait or retry while
            // async-nats has already given up on the connection. Run both
            // side by side, so a lost connection is replaced at once and a
            // replacement never cancels a renewal that is still in flight.
            tokio::select! {
                () = renew_credentials(&manager) => {}
                () = replace_connections(&manager, &shared, &changes, revisions, lifetime, timeout) => {}
            }
            if manager.check_hub_refusal().is_err() {
                let _ = snapshot(&shared).close(Duration::from_millis(50)).await;
            }
        });
        Ok(Self {
            connection,
            changes: receiver,
            task,
        })
    }

    fn snapshot(&self) -> NatsConnection {
        snapshot(&self.connection)
    }
}

fn snapshot(connection: &RwLock<NatsConnection>) -> NatsConnection {
    connection
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

async fn renew_credentials(manager: &CredentialManager) {
    let mut revisions = manager.revisions();
    let mut retry = None;
    loop {
        if manager.check_hub_refusal().is_err() {
            return;
        }
        revisions.borrow_and_update();
        let delay = match retry.take() {
            Some(delay) => delay,
            None => credentials::renewal_delay(manager.credential().await.exp),
        };
        tokio::select! {
            result = revisions.changed() => {
                if result.is_err() { return; }
                continue;
            }
            () = tokio::time::sleep(delay) => {}
        }
        if let Err(error) = manager.renew().await {
            if matches!(error, Error::HubUnavailable | Error::HubNotGranted) {
                return;
            }
            if credentials::transient(&error) || matches!(error, Error::Transport(_)) {
                retry = Some(Duration::from_secs(5));
                continue;
            }
            // A terminal refusal (notably generation_not_live) must not cause
            // issuance under a dead generation.
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
    }
}

async fn replace_connections(
    manager: &Arc<CredentialManager>,
    connection: &RwLock<NatsConnection>,
    changes: &watch::Sender<u64>,
    mut revisions: watch::Receiver<u64>,
    mut lifetime: ConnectionLifetime,
    timeout: Duration,
) {
    loop {
        let credential_changed = tokio::select! {
            result = revisions.changed() => {
                if result.is_err() || manager.check_hub_refusal().is_err() { return; }
                true
            }
            _ = &mut lifetime.ended => {
                if !lifetime.diagnostics.stale.load(Ordering::Relaxed) {
                    tracing::warn!("bus connection closed for good; replacing it");
                }
                false
            }
        };
        // Only a superseded connection gets the "errors are expected" line. A
        // connection that ended on its own (an outage that exhausted its
        // reconnects) already logged a warning, and its errors are real.
        if credential_changed || lifetime.diagnostics.stale.load(Ordering::Relaxed) {
            lifetime.log_retirement();
        }
        if credential_changed {
            lifetime.retire(&snapshot(connection)).await;
        }

        let mut backoff = Duration::from_millis(250);
        loop {
            let revision = *revisions.borrow_and_update();
            if manager.check_hub_refusal().is_err() {
                return;
            }
            let credential = manager.credential().await;
            match connect_current(manager.clone(), &credential, timeout).await {
                Ok((replacement, mut replacement_lifetime)) => {
                    if revisions.has_changed().unwrap_or(false) {
                        replacement_lifetime.retire(&replacement).await;
                        continue;
                    }
                    *connection
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = replacement;
                    lifetime = replacement_lifetime;
                    changes.send_replace(revision);
                    break;
                }
                Err(_) => {
                    tokio::select! {
                        result = revisions.changed() => {
                            if result.is_err() { return; }
                            backoff = Duration::from_millis(250);
                        }
                        () = tokio::time::sleep(backoff) => {
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                        }
                    }
                }
            }
        }
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

    /// Counts deliveries dropped because their origin or subject did not match
    /// the receiving subscription, preventing forged pushes from becoming hints
    /// or bucket changes.
    pub fn forged_deliveries_dropped(&self) -> u64 {
        self.forged_deliveries_dropped.load(Ordering::Relaxed)
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

    /// Subscribes to one exact live-hint subject and verifies every delivery's
    /// subject before yielding it, so a JetStream push cannot inject bucket data.
    pub async fn live_hints(&self, module: &str, name: &str, version: u32) -> Result<LiveHints> {
        LiveHints::attach(self, module, name, version).await
    }
}

/// How many reconnect attempts async-nats makes after a connection drops before
/// it gives up and ends the connection for good.
///
/// async-nats waits nothing before the first attempt, then 2 ms doubling up to
/// a 4 s cap, so twenty attempts span about 36 s for a current credential:
/// a ck-bus restart of about 30 s rides through on the same connection.
const MAX_RECONNECTS: usize = 20;

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

struct ConnectionDiagnostics {
    stale: AtomicBool,
    id: u64,
    server_url: String,
}

impl ConnectionDiagnostics {
    fn log_stale_auth_refusal(&self) {
        tracing::debug!(
            connection_id = self.id,
            server = %self.server_url,
            "retired bus connection refused authentication"
        );
    }
}

struct ConnectionLifetime {
    diagnostics: Arc<ConnectionDiagnostics>,
    ended: oneshot::Receiver<()>,
}

impl ConnectionLifetime {
    fn log_retirement(&self) {
        tracing::info!(
            connection_id = self.diagnostics.id,
            server = %self.diagnostics.server_url,
            "retiring bus connection for replacement; brief async-nats reconnect and auth errors for it are expected"
        );
    }

    async fn retire(&mut self, connection: &NatsConnection) {
        self.diagnostics.stale.store(true, Ordering::Relaxed);
        // A connected handler can drain; a reconnecting one can't read that
        // command. It skips further backoff now that its key can never sign.
        let closed = tokio::time::timeout(Duration::from_secs(2), async {
            let _ = connection.close(Duration::from_millis(50)).await;
            let _ = (&mut self.ended).await;
        })
        .await;
        if closed.is_err() {
            tracing::debug!(
                connection_id = self.diagnostics.id,
                server = %self.diagnostics.server_url,
                "retired bus connection did not close in time; replacing it anyway"
            );
        }
    }
}

async fn connect_current(
    manager: Arc<CredentialManager>,
    credential: &credentials::Credential,
    timeout: Duration,
) -> Result<(NatsConnection, ConnectionLifetime)> {
    let public = credential.credential_public.clone();
    let diagnostics = Arc::new(ConnectionDiagnostics {
        stale: AtomicBool::new(false),
        id: NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed),
        server_url: credential.server_url.clone(),
    });
    let auth_diagnostics = diagnostics.clone();
    let delay_diagnostics = diagnostics.clone();
    let (handler, ended) = oneshot::channel();
    let options = async_nats::ConnectOptions::with_auth_callback(move |nonce| {
        let manager = manager.clone();
        let public = public.clone();
        let diagnostics = auth_diagnostics.clone();
        async move {
            if diagnostics.stale.load(Ordering::Relaxed) {
                diagnostics.log_stale_auth_refusal();
                return Err(async_nats::AuthError::new(Error::Reply(
                    REPLACED_CREDENTIAL,
                )));
            }
            // The spawned nonce-signing request future only needs to be Send;
            // async-nats also requires the callback future to be Sync.
            let signing_diagnostics = diagnostics.clone();
            let task = tokio::spawn(async move {
                let stale = &signing_diagnostics.stale;
                let credential = manager.credential().await;
                if credential.credential_public != public {
                    stale.store(true, Ordering::Relaxed);
                    return Err(Error::Reply(REPLACED_CREDENTIAL));
                }
                let signature = manager.sign(&public, nonce).await.inspect_err(|error| {
                    if matches!(error, Error::Reply(REPLACED_CREDENTIAL)) {
                        stale.store(true, Ordering::Relaxed);
                    }
                })?;
                if stale.load(Ordering::Relaxed) {
                    return Err(Error::Reply(REPLACED_CREDENTIAL));
                }
                let mut auth = async_nats::Auth::new();
                auth.jwt = Some(credential.jwt);
                auth.signature = Some(signature);
                Ok(auth)
            });
            let result = task
                .await
                .map_err(async_nats::AuthError::new)
                .and_then(|result| result.map_err(async_nats::AuthError::new));
            if result.is_err() && diagnostics.stale.load(Ordering::Relaxed) {
                diagnostics.log_stale_auth_refusal();
            }
            result
        }
    })
    .reconnect_delay_callback(move |attempts| {
        // The handler owns this callback, so dropping it wakes the supervisor
        // even when async-nats exits without emitting Closed (MaxReconnects).
        let _handler = &handler;
        if delay_diagnostics.stale.load(Ordering::Relaxed) || attempts <= 1 {
            Duration::ZERO
        } else {
            // Match async-nats 0.50's default delay for a current credential.
            Duration::from_millis((1 << (attempts - 1).min(12)).min(4000))
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
    Ok((connection, ConnectionLifetime { diagnostics, ended }))
}

#[cfg(test)]
mod tests;
