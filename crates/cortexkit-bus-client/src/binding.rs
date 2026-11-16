use std::{
    future::Future,
    sync::{Arc, RwLock},
};

use cortexkit_bus_nats::NatsConnection;
use tokio::sync::watch;

use crate::{BusClient, Error, Result};

/// Shared connection-generation handling for long-lived readers.
#[derive(Clone)]
pub(crate) struct ConnectionBinding {
    connection: Arc<RwLock<NatsConnection>>,
    changes: watch::Receiver<u64>,
}

impl ConnectionBinding {
    pub(crate) fn new(client: &BusClient) -> Self {
        Self {
            connection: client.connection.clone(),
            changes: client.connection_changes(),
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
            let result = open(self.snapshot()).await;
            // Never install a reader on a connection replaced while opening it.
            if self.has_changed() {
                continue;
            }
            return result;
        }
    }
}
