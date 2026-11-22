use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

use cortexkit_bus_nats::NatsConnection;
use tokio::sync::watch;

use crate::{BusClient, Error, ManagedConnection, Result};

/// Shared connection-generation handling for long-lived readers.
#[derive(Clone)]
pub(crate) struct ConnectionBinding {
    connection: Arc<RwLock<NatsConnection>>,
    changes: watch::Receiver<u64>,
    /// The connection the last successful [`Self::bind`] opened on.
    bound: Option<NatsConnection>,
    forged_deliveries_dropped: Arc<AtomicU64>,
    /// Warning-branch executions for this binding, independent of tracing filters.
    #[cfg(test)]
    pub(crate) bucket_watch_capacity_warnings: usize,
}

impl ConnectionBinding {
    pub(crate) fn new(client: &BusClient) -> Self {
        Self::from_managed(&client.managed)
    }

    pub(crate) fn from_managed(managed: &ManagedConnection) -> Self {
        Self {
            connection: managed.connection.clone(),
            changes: managed.changes.clone(),
            bound: None,
            forged_deliveries_dropped: managed.forged_deliveries_dropped.clone(),
            #[cfg(test)]
            bucket_watch_capacity_warnings: 0,
        }
    }

    pub(crate) fn snapshot(&self) -> NatsConnection {
        self.connection
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn has_changed(&self) -> bool {
        self.changes.has_changed().unwrap_or(false)
    }

    pub(crate) async fn changed(&mut self) -> Result<()> {
        self.changes
            .changed()
            .await
            .map_err(|_| Error::Transport("credential maintenance stopped".into()))
    }

    pub(crate) async fn bind<T, F, Fut>(&mut self, mut open: F) -> Result<T>
    where
        F: FnMut(NatsConnection) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        loop {
            self.changes.borrow_and_update();
            let connection = self.snapshot();
            let result = open(connection.clone()).await;
            // Never install a reader on a connection replaced while opening it.
            if self.has_changed() {
                continue;
            }
            match result {
                Ok(value) => {
                    self.bound = Some(connection);
                    return Ok(value);
                }
                Err(error) => wait_for_replacement(&mut self.changes, &connection, error).await?,
            }
        }
    }

    /// Handles a failed operation on the bound connection. When async-nats has
    /// ended that connection for good, waits for its replacement and returns
    /// `Ok`; the caller then drops its handle and rebinds. Any other failure is
    /// returned unchanged.
    pub(crate) async fn recover(&mut self, error: Error) -> Result<()> {
        match &self.bound {
            Some(connection) => wait_for_replacement(&mut self.changes, connection, error).await,
            None => Err(error),
        }
    }

    pub(crate) fn record_forged_delivery(&self) {
        self.forged_deliveries_dropped
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn drop_forged(&self, warned: &mut bool, expected: &str, subject: &str) {
        self.record_forged_delivery();
        if !*warned {
            *warned = true;
            tracing::warn!(
                subject,
                expected,
                "dropping forged delivery with unexpected origin or subject"
            );
        }
    }
}

/// Between the current connection ending and the maintenance task installing
/// its replacement, every operation on it fails at once. Waiting here keeps a
/// reader from returning those failures to its caller. If the change stream
/// closes instead, because maintenance stopped or the client was dropped, the
/// original error is returned rather than waiting forever.
async fn wait_for_replacement(
    changes: &mut watch::Receiver<u64>,
    connection: &NatsConnection,
    error: Error,
) -> Result<()> {
    if !has_ended(connection).await {
        return Err(error);
    }
    changes.changed().await.map_err(|_| error)
}

/// True once async-nats has ended the connection for good, after running out
/// of reconnect attempts or after a drain: its handler has exited, so a flush
/// request is refused at once. A connected client completes the flush and a
/// reconnecting one holds it, so neither reports an error.
async fn has_ended(connection: &NatsConnection) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_millis(50), connection.client().flush()).await,
        Ok(Err(_))
    )
}
