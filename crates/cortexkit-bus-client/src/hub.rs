use std::{sync::Arc, time::Duration};

use async_nats::jetstream;
use subc_client_rs::SubcConsumer;
use tokio::sync::watch;

use crate::{
    credentials::{self, CredentialManager, HUB_DOMAIN},
    BusClientOptions, Error, ManagedConnection, Result,
};

/// A module's separate federation-account connection to the hub.
///
/// Opening a [`crate::BusClient`] does not request a hub credential. Open a
/// `HubClient` only when the module needs the hub. Dropping it stops its
/// credential maintenance without affecting the box connection.
pub struct HubClient {
    managed: ManagedConnection,
    manager: Arc<CredentialManager>,
}

impl HubClient {
    /// Requests a hub credential over the module's existing daemon route.
    ///
    /// [`Error::HubUnavailable`] can be retried when the module next needs the
    /// hub. [`Error::HubNotGranted`] is terminal. Neither refusal is retried here.
    pub async fn connect(subc: Arc<SubcConsumer>, opts: BusClientOptions) -> Result<Self> {
        let manager = credentials::module_manager(subc, &opts, true).await?;
        Self::connect_manager(manager, opts.request_timeout).await
    }

    pub(crate) async fn connect_manager(
        manager: Arc<CredentialManager>,
        timeout: Duration,
    ) -> Result<Self> {
        let managed = ManagedConnection::connect(manager.clone(), timeout).await?;
        Ok(Self { managed, manager })
    }

    /// Returns the current raw NATS client for core publishes. A held snapshot
    /// does not follow credential replacements; get a new one after a change.
    pub fn client(&self) -> Result<async_nats::Client> {
        let stopped = self.managed.changes.has_changed().is_err();
        self.manager.check_hub_refusal()?;
        if stopped {
            return Err(Error::Reply("hub credential maintenance stopped"));
        }
        Ok(self.managed.snapshot().client())
    }

    /// Returns a JetStream context on the current connection, in the `hub`
    /// domain. Like [`Self::client`], it must be reopened after replacement.
    pub fn jetstream(&self) -> Result<jetstream::Context> {
        Ok(jetstream::with_domain(self.client()?, HUB_DOMAIN))
    }

    /// Notifies after a replacement connection is ready. The receiver closes
    /// when maintenance stops, including after a hub refusal. Get a fresh
    /// snapshot with [`Self::client`] to check for a typed refusal on closure;
    /// after [`Error::HubUnavailable`], call [`Self::connect`] when ready to retry.
    pub fn connection_changes(&self) -> watch::Receiver<u64> {
        self.managed.changes.clone()
    }
}
