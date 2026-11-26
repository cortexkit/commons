use std::{sync::Mutex, time::Duration};

use async_nats::{ClientError, ConnectErrorKind, Event, ServerError};
use cortexkit_bus_nats::NatsConnectionEvent;
use tokio::{sync::watch, time::Instant};

const FIRST_REPORT: Duration = Duration::from_secs(60);
const REPORT_INTERVAL: Duration = Duration::from_secs(600);
/// The server's `-ERR` text when a connection's user JWT expires; async-nats
/// 0.50 has no variant for it, so it arrives as `ServerError::Other`.
const CREDENTIAL_EXPIRED: &str = "User Authentication Expired";

struct Outage {
    server: String,
    started: Instant,
    next_report: Instant,
    failed_retries: u64,
}

/// Shared by a managed connection and its replacements, since async-nats can
/// use up its reconnect budget before the first long-outage report is due.
pub(crate) struct OutageLog(watch::Sender<Option<Outage>>);

impl OutageLog {
    pub(crate) fn new() -> Self {
        Self(watch::Sender::new(None))
    }

    fn disconnected(&self, server: &str) {
        self.0.send_if_modified(|state| {
            if state.is_some() {
                return false;
            }
            let now = Instant::now();
            tracing::warn!("bus connection to {server} lost; reconnecting");
            *state = Some(Outage {
                server: server.to_owned(),
                started: now,
                next_report: now + FIRST_REPORT,
                failed_retries: 0,
            });
            true
        });
    }

    fn connected(&self, server: &str) {
        self.0.send_if_modified(|state| {
            let Some(outage) = state.take() else {
                return false;
            };
            tracing::info!(
                outage_ms = outage.started.elapsed().as_millis() as u64,
                reported_failed_retries = outage.failed_retries,
                "bus connection to {server} restored"
            );
            true
        });
    }

    fn retry_failed(&self) -> bool {
        // A failed retry does not change the next report's deadline.
        let mut counted = false;
        self.0.send_if_modified(|state| {
            if let Some(outage) = state {
                outage.failed_retries += 1;
                counted = true;
            }
            false
        });
        counted
    }

    pub(crate) async fn report_long_outages(&self) {
        let mut outages = self.0.subscribe();
        loop {
            let deadline = outages.borrow_and_update().as_ref().map(|s| s.next_report);
            if let Some(deadline) = deadline {
                tokio::select! {
                    biased;
                    _ = outages.changed() => continue,
                    () = tokio::time::sleep_until(deadline) => {}
                }
                self.0.send_if_modified(|state| {
                    let now = Instant::now();
                    let Some(outage) = state.as_mut().filter(|s| s.next_report <= now) else {
                        return false;
                    };
                    tracing::error!(
                        outage_ms = (now - outage.started).as_millis() as u64,
                        reported_failed_retries = outage.failed_retries,
                        "bus connection to {} is still down; reconnecting",
                        outage.server
                    );
                    outage.next_report = now + REPORT_INTERVAL;
                    true
                });
            } else {
                let _ = outages.changed().await;
            }
        }
    }
}

#[derive(Default, PartialEq)]
enum Status {
    #[default]
    Initial,
    Connected,
    Disconnected,
    Finished,
}

#[derive(Default)]
struct ConnectionState {
    status: Status,
    ready: bool,
}

#[derive(Default)]
pub(crate) struct ConnectionObservation(Mutex<ConnectionState>);

impl ConnectionObservation {
    pub(crate) fn observe(
        &self,
        event: NatsConnectionEvent<'_>,
        log: &OutageLog,
        server: &str,
        stale: bool,
    ) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if stale {
            return;
        }
        match event {
            NatsConnectionEvent::ConnectFailed(error) => {
                let kind = error.kind();
                let in_outage = log.retry_failed();
                if !transient_kind(kind) || !in_outage {
                    tracing::error!(server, %kind, "bus connection attempt failed");
                }
            }
            NatsConnectionEvent::Event(event) => match event {
                Event::Disconnected if state.status != Status::Finished => {
                    state.status = Status::Disconnected;
                    log.disconnected(server);
                }
                Event::Connected if state.status != Status::Finished => {
                    if state.ready && state.status == Status::Disconnected {
                        log.connected(server);
                    }
                    state.status = Status::Connected;
                }
                Event::ServerError(ServerError::Other(reason))
                    if reason.eq_ignore_ascii_case(CREDENTIAL_EXPIRED) =>
                {
                    // ck-bus signs each user JWT for a short life and renews it
                    // before then, but NATS cannot re-authenticate a live
                    // connection: the server closes it when the JWT it connected
                    // with expires, and the client redials with the renewed one.
                    // That's expected and recovers on its own, so it logs at
                    // debug, where it doesn't bury real errors.
                    tracing::debug!(
                        server,
                        "bus connection credential expired; reconnecting with the renewed one"
                    );
                }
                Event::ServerError(error) => tracing::error!(server, %error, "bus server error"),
                Event::ClientError(error) => {
                    let folded = match error {
                        ClientError::Other(kind) => {
                            let in_outage = log.retry_failed();
                            transient_retry(kind) && (in_outage || state.status == Status::Finished)
                        }
                        // The manager keeps reconnecting even after this handler gives up.
                        ClientError::MaxReconnects => {
                            state.status == Status::Finished || log.0.borrow().is_some()
                        }
                        _ => false,
                    };
                    if !folded {
                        tracing::error!(server, %error, "bus client error");
                    }
                }
                Event::SlowConsumer(subscription) => {
                    tracing::warn!(server, subscription, "slow bus consumer")
                }
                Event::LameDuckMode => tracing::warn!(server, "bus server entered lame-duck mode"),
                Event::Connected | Event::Disconnected | Event::Closed | Event::Draining => {}
            },
            _ => {}
        }
    }

    pub(crate) fn ready(&self, log: &OutageLog, server: &str) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Initial Connected is sent for every dial, including replacements the
        // manager rejects. End the outage only when a replacement is ready.
        state.ready = true;
        if matches!(state.status, Status::Initial | Status::Connected) {
            log.connected(server);
        }
    }

    pub(crate) fn finish(&self, ended: Option<(&OutageLog, &str)>) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((log, server)) = ended {
            // A handler can exit before its queued Disconnected is delivered.
            log.disconnected(server);
        }
        state.status = Status::Finished;
    }
}

fn transient_kind(kind: ConnectErrorKind) -> bool {
    matches!(kind, ConnectErrorKind::Io | ConnectErrorKind::TimedOut)
}

fn transient_retry(kind: &str) -> bool {
    // async-nats 0.50 puts only ConnectErrorKind's Display text in Other for
    // reconnect failures; unknown strings must stay visible.
    [ConnectErrorKind::Io, ConnectErrorKind::TimedOut]
        .iter()
        .any(|transient| *kind == transient.to_string())
}

#[cfg(test)]
mod tests;
