use std::{sync::Arc, time::Duration};

use async_nats::jetstream;
use cortexkit_bus_naming::federation::BoxId;
use subc_client_rs::SubcConsumer;
use tokio::sync::watch;

use crate::{
    binding::ConnectionBinding,
    credentials::{self, CredentialManager, HUB_DOMAIN},
    BucketWatch, BusClientOptions, Error, ManagedConnection, Result,
};

/// A module's separate federation-account connection to the hub.
///
/// Opening a [`crate::BusClient`] does not request a hub credential. Open a
/// `HubClient` only when the module needs the hub. Dropping it stops its
/// credential maintenance without affecting the box connection.
pub struct HubClient {
    managed: ManagedConnection,
    manager: Arc<CredentialManager>,
    box_id: Option<BoxId>,
}

impl HubClient {
    /// Requests a hub credential over the module's existing daemon route.
    ///
    /// Refusals are returned without retrying the initial connection. Retry
    /// [`Error::HubUnavailable`] when the module next needs the hub; an
    /// established client recovers in the background. [`Error::HubNotGranted`]
    /// is terminal.
    pub async fn connect(subc: Arc<SubcConsumer>, opts: BusClientOptions) -> Result<Self> {
        let manager = credentials::module_manager(subc, &opts, true).await?;
        Self::connect_manager(manager, opts.request_timeout).await
    }

    pub(crate) async fn connect_manager(
        manager: Arc<CredentialManager>,
        timeout: Duration,
    ) -> Result<Self> {
        let box_id = manager.credential().await.box_id();
        let managed = ManagedConnection::connect(manager.clone(), timeout).await?;
        Ok(Self {
            managed,
            manager,
            box_id,
        })
    }

    /// Returns the computer's box id as supplied by the hub credential.
    ///
    /// Modules must never derive this id themselves. A valid hub credential
    /// always supplies a canonical box id, and renewal and reissue cannot
    /// change it.
    pub fn box_id(&self) -> Option<BoxId> {
        self.box_id.clone()
    }

    /// Returns the current raw NATS client for core publishes. A held snapshot
    /// does not follow credential replacements; get a new one after a change.
    /// Returns [`Error::HubUnavailable`] until recovery installs a replacement.
    pub fn client(&self) -> Result<async_nats::Client> {
        let stopped = self.managed.changes.has_changed().is_err();
        self.manager
            .revision()
            .check_hub_connection(*self.managed.changes.borrow())?;
        if stopped {
            return Err(Error::Reply("hub credential maintenance stopped"));
        }
        Ok(self.managed.snapshot().client())
    }

    /// Returns a JetStream context on the current connection, in the `hub`
    /// domain. Like [`Self::client`], it must be reopened after replacement.
    /// Returns [`Error::HubUnavailable`] while the client recovers.
    pub fn jetstream(&self) -> Result<jetstream::Context> {
        Ok(jetstream::with_domain(self.client()?, HUB_DOMAIN))
    }

    /// Watches an existing named KV bucket in the `hub` JetStream domain.
    /// Delivers its current entries, then a [`crate::BucketItem::CaughtUp`]
    /// boundary and live changes. A replacement bucket yields
    /// [`Error::BucketReset`] before its new snapshot. The hub credential must
    /// allow stream info and consumer creation for this bucket. Keep the hub
    /// client alive for credential upkeep and automatic rebinding.
    /// Opening a watch returns [`Error::HubUnavailable`] during recovery;
    /// an already-open watch waits for a replacement connection if it loses
    /// its connection.
    pub async fn watch_bucket(&self, bucket: &str) -> Result<BucketWatch> {
        self.client()?;
        BucketWatch::open(
            ConnectionBinding::from_managed(&self.managed),
            bucket.to_owned(),
            Some(HUB_DOMAIN),
        )
        .await
    }

    /// Notifies after a replacement connection is ready. The receiver closes
    /// when maintenance stops, including after [`Error::HubNotGranted`].
    /// During [`Error::HubUnavailable`] it stays open and notifies once the
    /// recovered connection is ready. Use [`Self::client`] for a fresh snapshot
    /// or to check for a typed refusal on closure.
    pub fn connection_changes(&self) -> watch::Receiver<u64> {
        self.managed.changes.clone()
    }
}
