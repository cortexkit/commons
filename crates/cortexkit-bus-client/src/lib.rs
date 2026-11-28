#![doc = include_str!("../README.md")]

mod binding;
mod buckets;
mod credentials;
mod events;
mod hub;
mod live;
mod outage;

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
use outage::{ConnectionObservation, OutageLog};

/// `EnvFilter` directives that replace async-nats connector diagnostics with
/// this client's outage and error logs. Other async-nats warnings and errors
/// remain enabled. See the crate's Logging section to restore connector details.
pub const ASYNC_NATS_LOG_DIRECTIVE: &str = "async_nats=warn,async_nats::connector=off";

/// Errors contain refusal codes, not credential JWTs or nonce signatures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("ck-bus refused the request: {0}")]
    Refused(String),
    /// ck-bus cannot reach the hub. Retry initial connect when needed; an
    /// established hub client recovers in the background.
    #[error("ck-bus refused the hub credential: hub_unavailable")]
    HubUnavailable,
    /// This module is not allowed to use the hub. Do not retry.
    #[error("ck-bus refused the hub credential: hub_not_granted")]
    HubNotGranted,
    /// The daemon refused to open a route to ck-bus; the request was not sent.
    #[error("ck-bus route refused: {code} ({verdict})", verdict = match expected_back {
        Some(true) => "ck-bus is expected back after restart",
        Some(false) => "ck-bus requires operator action",
        None => "unknown whether ck-bus is expected back",
    })]
    RouteRefused {
        /// The daemon's refusal code.
        code: String,
        /// `Some(true)` means ck-bus is restarting and a later retry may work.
        /// `Some(false)` means ck-bus will not return without operator action;
        /// callers can watch the daemon's module catalog for ck-bus to become
        /// available instead of polling the route.
        /// `None` means the daemon gave no usable verdict.
        expected_back: Option<bool>,
    },
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
    /// The watch has switched to a replacement bucket. Clear the old entries
    /// and revisions, then read the new snapshot on the same watch.
    #[error("bucket {bucket} was reset")]
    BucketReset { bucket: String },
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
    forged_deliveries_dropped: Arc<AtomicU64>,
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
        let outages = Arc::new(OutageLog::new());
        let mut revisions = manager.revisions();
        let (connection, lifetime, recovered_at) = loop {
            let revision = *revisions.borrow_and_update();
            revision.check_hub_availability()?;
            let credential = manager.credential().await;
            match connect_current(manager.clone(), &credential, timeout, &outages).await {
                Ok((connection, lifetime)) if !revisions.has_changed().unwrap_or(false) => {
                    lifetime.ready(&outages);
                    break (connection, lifetime, revision.recovered_at);
                }
                Ok((connection, mut lifetime)) => {
                    lifetime.retire(&connection).await;
                }
                Err(_)
                    if manager.credential().await.credential_public
                        != credential.credential_public => {}
                Err(error) => {
                    manager.revision().check_hub_availability()?;
                    return Err(error);
                }
            }
        };
        let connection = Arc::new(RwLock::new(connection));
        let shared = connection.clone();
        let (changes, receiver) = watch::channel(recovered_at);
        let task = tokio::spawn(async move {
            // Renewal is a request to ck-bus that can wait or retry while
            // async-nats has already given up on the connection. Run both
            // side by side, so a lost connection is replaced at once and a
            // replacement never cancels a renewal that is still in flight.
            tokio::select! {
                () = renew_credentials(&manager) => {}
                () = replace_connections(&manager, &shared, &changes, revisions, lifetime, timeout, &outages) => {}
                () = outages.report_long_outages() => {}
            }
            if manager.revision().check_hub_grant().is_err() {
                let _ = snapshot(&shared).close(Duration::from_millis(50)).await;
            }
        });
        Ok(Self {
            connection,
            changes: receiver,
            task,
            forged_deliveries_dropped: Arc::new(AtomicU64::new(0)),
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
        let revision = *revisions.borrow_and_update();
        if revision.check_hub_grant().is_err() {
            return;
        }
        if revision.hub_unavailable() {
            // Connection maintenance is reissuing the credential. Do not keep
            // trying to renew the old lease while ck-bus cannot reach the hub.
            retry = None;
            if revisions.changed().await.is_err() {
                return;
            }
            continue;
        }
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
            if matches!(error, Error::HubUnavailable) {
                continue;
            }
            if matches!(error, Error::HubNotGranted) {
                return;
            }
            if credentials::transient(&error)
                || matches!(error, Error::Transport(_) | Error::RouteRefused { .. })
            {
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
    mut revisions: watch::Receiver<credentials::CredentialRevision>,
    mut lifetime: ConnectionLifetime,
    timeout: Duration,
    outages: &Arc<OutageLog>,
) {
    loop {
        let credential_changed = tokio::select! {
            result = revisions.changed() => {
                if result.is_err() || manager.revision().check_hub_grant().is_err() { return; }
                true
            }
            _ = &mut lifetime.ended => {
                let diagnostics = &lifetime.diagnostics;
                let ended = (!diagnostics.stale.load(Ordering::Relaxed))
                    .then_some((outages.as_ref(), diagnostics.server_url.as_str()));
                diagnostics.observation.finish(ended);
                false
            }
        };
        // Only a superseded connection gets the "errors are expected" line;
        // an exhausted reconnect budget does not end an ongoing outage.
        if credential_changed || lifetime.diagnostics.stale.load(Ordering::Relaxed) {
            lifetime.log_retirement();
        }
        if credential_changed {
            lifetime.retire(&snapshot(connection)).await;
        }

        let mut backoff = Duration::from_millis(250);
        loop {
            let revision = *revisions.borrow_and_update();
            if revision.check_hub_grant().is_err() {
                return;
            }
            let credential = manager.credential().await;
            if revisions.has_changed().unwrap_or(false) {
                continue;
            }
            if revision.hub_unavailable() {
                if manager.reissue(&credential.credential_public).await.is_ok() {
                    continue;
                }
            } else if let Ok((replacement, mut replacement_lifetime)) =
                connect_current(manager.clone(), &credential, timeout, outages).await
            {
                if revisions.has_changed().unwrap_or(false) {
                    replacement_lifetime.retire(&replacement).await;
                    continue;
                }
                *connection
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = replacement;
                replacement_lifetime.ready(outages);
                lifetime = replacement_lifetime;
                changes.send_replace(revision.number);
                break;
            }

            let recovering_hub = matches!(
                manager.revision().check_hub_connection(*changes.borrow()),
                Err(Error::HubUnavailable)
            );
            tokio::select! {
                // A new key normally wakes replacement immediately. During a
                // hub outage, do not reset backoff on failed authentication
                // with a newly issued key, or issue and connect can spin.
                result = revisions.wait_for(|latest| {
                    latest.check_hub_grant().is_err()
                        || (!recovering_hub && latest.number != revision.number)
                }) => {
                    if !matches!(result, Ok(latest) if latest.check_hub_grant().is_ok()) { return; }
                    backoff = Duration::from_millis(250);
                }
                () = tokio::time::sleep(backoff) => {
                    backoff = (backoff * 2).min(Duration::from_secs(30));
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

    /// Counts deliveries that live hints, bucket watches and event readers
    /// discarded because their origin or subject could not be verified.
    pub fn forged_deliveries_dropped(&self) -> u64 {
        self.managed
            .forged_deliveries_dropped
            .load(Ordering::Relaxed)
    }

    /// Attaches to an existing EVENT durable owned by this module.
    ///
    /// ck-bus provisions durables; this method never creates or changes one.
    /// The name must follow bus-naming's `m_{module}[_suffix]` owner rule, and
    /// the durable must have at least one subject filter.
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
    observation: ConnectionObservation,
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
    fn ready(&self, outages: &OutageLog) {
        self.diagnostics
            .observation
            .ready(outages, &self.diagnostics.server_url);
    }

    fn log_retirement(&self) {
        tracing::info!(
            connection_id = self.diagnostics.id,
            server = %self.diagnostics.server_url,
            "retiring bus connection for replacement; brief async-nats reconnect and auth errors for it are expected"
        );
    }

    async fn retire(&mut self, connection: &NatsConnection) {
        self.diagnostics.stale.store(true, Ordering::Relaxed);
        self.diagnostics.observation.finish(None);
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
    outages: &Arc<OutageLog>,
) -> Result<(NatsConnection, ConnectionLifetime)> {
    let public = credential.credential_public.clone();
    let diagnostics = Arc::new(ConnectionDiagnostics {
        stale: AtomicBool::new(false),
        id: NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed),
        server_url: credential.server_url.clone(),
        observation: ConnectionObservation::default(),
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
                let signature = manager.sign(&public, nonce).await.inspect_err(|error| {
                    if matches!(error, Error::Reply(REPLACED_CREDENTIAL)) {
                        stale.store(true, Ordering::Relaxed);
                    }
                })?;
                // Renewal keeps the signing key but can replace the JWT while
                // the nonce-signing request is in flight.
                let credential = manager.credential().await;
                if credential.credential_public != public {
                    stale.store(true, Ordering::Relaxed);
                    return Err(Error::Reply(REPLACED_CREDENTIAL));
                }
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
    let inbox_prefix =
        (credential.inbox_prefix != config.inbox_prefix()).then(|| credential.inbox_prefix.clone());
    let observation = diagnostics.clone();
    let outages = Arc::downgrade(outages);
    let connection = NatsConnection::connect_observed(
        credential.server_url.as_str(),
        options,
        config,
        inbox_prefix,
        move |event| {
            if let Some(outages) = outages.upgrade() {
                observation.observation.observe(
                    event,
                    &outages,
                    &observation.server_url,
                    observation.stale.load(Ordering::Relaxed),
                );
            }
        },
    )
    .await
    .map_err(Error::from)?;
    Ok((connection, ConnectionLifetime { diagnostics, ended }))
}

#[cfg(test)]
mod tests;
