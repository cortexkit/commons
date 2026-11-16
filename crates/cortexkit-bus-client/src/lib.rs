#![doc = include_str!("../README.md")]

mod credentials;

use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use bytes::Bytes;
use cortexkit_bus_naming::{validate_token, AccountNames, TokenKind};
use cortexkit_bus_nats::{ConnectConfig, NatsConnection};
use cortexkit_bus_trait::PublishReceipt;
use subc_client_rs::SubcConsumer;
use tokio::sync::watch;

use credentials::{CredentialManager, SubcTransport};

/// Errors contain refusal codes, not credential JWTs or nonce signatures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("ck-bus refused the request: {0}")]
    Refused(String),
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
    connection: Arc<RwLock<NatsConnection>>,
    names: AccountNames,
    module: String,
    changes: watch::Receiver<u64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for BusClient {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl BusClient {
    /// Issues a credential for the module identity that the subc daemon
    /// authenticates from the module's launch nonce.
    pub async fn connect(subc: Arc<SubcConsumer>, opts: BusClientOptions) -> Result<Self> {
        let transport = SubcTransport::new(subc, &opts.module, opts.request_timeout);
        // One module process can keep several connection snapshots, but issuing
        // independently for them would revoke the other snapshots' credentials.
        static MANAGERS: std::sync::LazyLock<
            tokio::sync::Mutex<
                std::collections::HashMap<String, std::sync::Weak<CredentialManager>>,
            >,
        > = std::sync::LazyLock::new(Default::default);
        let mut managers = MANAGERS.lock().await;
        let manager = match managers
            .get(&opts.module)
            .and_then(std::sync::Weak::upgrade)
        {
            Some(manager) => manager,
            None => {
                let manager = CredentialManager::start(Arc::new(transport)).await?;
                managers.insert(opts.module.clone(), Arc::downgrade(&manager));
                manager
            }
        };
        drop(managers);
        Self::connect_manager(manager, opts).await
    }

    async fn connect_manager(
        manager: Arc<CredentialManager>,
        opts: BusClientOptions,
    ) -> Result<Self> {
        let mut revisions = manager.revisions();
        let initial = manager.credential().await;
        let names = AccountNames::derive(&initial.acct)?;
        let connection = loop {
            revisions.borrow_and_update();
            let credential = manager.credential().await;
            match connect_current(manager.clone(), &credential, opts.request_timeout).await {
                Ok(connection) if !revisions.has_changed().unwrap_or(false) => break connection,
                Ok(connection) => {
                    let _ = connection.close(Duration::from_millis(50)).await;
                }
                Err(_)
                    if manager.credential().await.credential_public
                        != credential.credential_public => {}
                Err(error) => return Err(error),
            }
        };
        let connection = Arc::new(RwLock::new(connection));
        let shared = connection.clone();
        let (changes, receiver) = watch::channel(0);
        let task = tokio::spawn(async move {
            let mut retry = None;
            loop {
                let credential = manager.credential().await;
                let delay = retry
                    .take()
                    .unwrap_or_else(|| credentials::renewal_delay(credential.exp));
                tokio::select! {
                    result = revisions.changed() => { if result.is_err() { return; } }
                    () = tokio::time::sleep(delay) => {
                        if let Err(error) = manager.renew().await {
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
                    let credential = manager.credential().await;
                    match connect_current(manager.clone(), &credential, opts.request_timeout).await
                    {
                        Ok(replacement) => {
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
                            changes.send_replace(revision);
                            let _ = old.close(Duration::from_millis(50)).await;
                            break;
                        }
                        Err(_) => {
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                        }
                    }
                }
            }
        });
        Ok(Self {
            connection,
            names,
            module: opts.module,
            changes: receiver,
            task,
        })
    }

    /// Returns a snapshot. A re-issue does not change an already held snapshot.
    pub fn connection(&self) -> NatsConnection {
        self.connection
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Changes only after a replacement connection with a new inbox is ready.
    /// Reopen subscriptions and durable pulls when this receiver changes.
    ///
    /// If credential maintenance can no longer renew or re-issue, its task ends
    /// and drops the sender. The receiver's `changed()` then returns `Err` once
    /// any unseen change has been consumed. Reconnect with [`BusClient::connect`]
    /// or report the module as degraded when the change stream closes.
    pub fn connection_changes(&self) -> watch::Receiver<u64> {
        self.changes.clone()
    }

    pub fn acct(&self) -> &str {
        self.names.account()
    }

    /// Stores an event using the fact's own id for JetStream deduplication.
    pub async fn publish_event(
        &self,
        event: &str,
        version: u32,
        qualifier: Option<&str>,
        msg_id: &str,
        body: impl Into<Bytes>,
    ) -> Result<PublishReceipt> {
        let subject = match qualifier {
            Some(reader) => {
                self.names
                    .event_subject_with_reader(&self.module, event, version, reader)?
            }
            None => self.names.event_subject(&self.module, event, version)?,
        };
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(async_nats::header::NATS_MESSAGE_ID, msg_id);
        let ack = async_nats::jetstream::new(self.connection().client())
            .publish_with_headers(subject, headers, body.into())
            .await
            .map_err(|error| Error::Nats(error.to_string()))?
            .await
            .map_err(|error| Error::Nats(error.to_string()))?;
        Ok(PublishReceipt::new(ack.sequence, ack.duplicate))
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

async fn connect_current(
    manager: Arc<CredentialManager>,
    credential: &credentials::Credential,
    timeout: Duration,
) -> Result<NatsConnection> {
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
    .request_timeout(Some(timeout));
    let mut config = ConnectConfig::new(&credential.credential_public)?;
    config.request_timeout = timeout;
    NatsConnection::connect(credential.server_url.as_str(), options, config)
        .await
        .map_err(Error::from)
}

#[cfg(test)]
mod tests;
