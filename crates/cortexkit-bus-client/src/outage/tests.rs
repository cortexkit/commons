use std::sync::Arc;

use async_nats::ServerError;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::EnvFilter;

use super::*;
use crate::ASYNC_NATS_LOG_DIRECTIVE;

const SERVER: &str = "nats://127.0.0.1:4222";

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Capture {
    fn start() -> (Self, DefaultGuard) {
        let capture = Self::default();
        let writer = capture.clone();
        let filter = ASYNC_NATS_LOG_DIRECTIVE
            .split(',')
            .fold(EnvFilter::new("info"), |filter, directive| {
                filter.add_directive(directive.parse().unwrap())
            });
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_env_filter(filter)
            .with_writer(move || writer.clone())
            .finish();
        (capture, tracing::subscriber::set_default(subscriber))
    }

    fn lines(&self) -> Vec<String> {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn emit(observation: &ConnectionObservation, log: &OutageLog, event: Event) {
    observation.observe(NatsConnectionEvent::Event(&event), log, SERVER, false);
}

#[tokio::test(start_paused = true)]
async fn repeated_disconnects_report_one_start_and_one_recovery_per_outage() {
    let (capture, _subscriber) = Capture::start();
    let log = OutageLog::new();
    let connection = ConnectionObservation::default();
    emit(&connection, &log, Event::Connected);
    connection.ready(&log, SERVER);
    assert!(capture.lines().is_empty());

    for _ in 0..5 {
        emit(&connection, &log, Event::Disconnected);
        emit(
            &connection,
            &log,
            Event::ClientError(ClientError::Other("IO error".into())),
        );
    }
    tokio::time::advance(Duration::from_secs(5)).await;
    emit(&connection, &log, Event::Connected);

    let lines = capture.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with(" WARN cortexkit_bus_client::outage:")
            && lines[0].contains(&format!("bus connection to {SERVER} lost; reconnecting")),
        "{lines:?}"
    );
    assert!(
        lines[1].contains("INFO cortexkit_bus_client::outage:")
            && lines[1].contains(&format!("bus connection to {SERVER} restored"))
            && lines[1].contains("outage_ms=5000")
            && lines[1].contains("reported_failed_retries=5"),
        "{lines:?}"
    );

    emit(&connection, &log, Event::Disconnected);
    tokio::time::advance(Duration::from_secs(2)).await;
    emit(&connection, &log, Event::Connected);
    let lines = capture.lines();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[2].contains("lost; reconnecting"));
    assert!(lines[3].contains("outage_ms=2000") && lines[3].contains("reported_failed_retries=0"));
}

#[tokio::test(start_paused = true)]
async fn long_outage_reports_on_schedule_even_after_reconnects_are_exhausted() {
    let (capture, _subscriber) = Capture::start();
    let log = Arc::new(OutageLog::new());
    let reporter = tokio::spawn({
        let log = log.clone();
        async move { log.report_long_outages().await }
    });
    let old = ConnectionObservation::default();
    old.ready(&log, SERVER);
    emit(&old, &log, Event::Disconnected);
    emit(&old, &log, Event::ClientError(ClientError::MaxReconnects));
    old.finish(Some((&log, SERVER)));
    tokio::task::yield_now().await;

    tokio::time::advance(Duration::from_secs(59)).await;
    let lines = capture.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let lines = capture.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[1].contains("ERROR cortexkit_bus_client::outage:")
            && lines[1].contains("outage_ms=60000"),
        "{lines:?}"
    );

    tokio::time::advance(Duration::from_secs(599)).await;
    assert_eq!(capture.lines().len(), 2);
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let lines = capture.lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[2].contains("ERROR cortexkit_bus_client::outage:")
            && lines[2].contains("outage_ms=660000"),
        "{lines:?}"
    );

    let replacement = ConnectionObservation::default();
    emit(&replacement, &log, Event::Connected);
    assert_eq!(
        capture.lines().len(),
        3,
        "dialing a candidate is not recovery"
    );
    replacement.ready(&log, SERVER);
    tokio::time::advance(Duration::from_secs(600)).await;
    tokio::task::yield_now().await;
    let lines = capture.lines();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[3].contains("restored") && lines[3].contains("outage_ms=660000"));
    reporter.abort();
    assert!(reporter.await.unwrap_err().is_cancelled());
}

#[test]
fn filter_hides_connector_logs_and_keeps_other_nats_diagnostics() {
    let (capture, _subscriber) = Capture::start();
    tracing::error!(target: "async_nats::connector", "connector handshake detail");
    tracing::info!(target: "async_nats", "event echo");
    tracing::warn!(target: "async_nats::jetstream::context", "acker warning");
    let log = OutageLog::new();
    let connection = ConnectionObservation::default();
    connection.ready(&log, SERVER);
    emit(&connection, &log, Event::Disconnected);
    emit(
        &connection,
        &log,
        Event::ClientError(ClientError::Other("failed signing nonce".into())),
    );
    emit(
        &connection,
        &log,
        Event::ClientError(ClientError::Other("timed out".into())),
    );
    emit(
        &connection,
        &log,
        Event::ClientError(ClientError::ServerNotInPool),
    );
    emit(
        &connection,
        &log,
        Event::ServerError(ServerError::AuthorizationViolation),
    );
    emit(&connection, &log, Event::SlowConsumer(3));
    emit(&connection, &log, Event::LameDuckMode);

    let lines = capture.lines();
    assert_eq!(lines.len(), 7, "{lines:?}");
    for (line, (level, message)) in lines.iter().zip([
        ("WARN", "acker warning"),
        ("WARN", "lost; reconnecting"),
        ("ERROR", "failed signing nonce"),
        ("ERROR", "server not in pool"),
        ("ERROR", "authorization violation"),
        ("WARN", "slow bus consumer"),
        ("WARN", "lame-duck mode"),
    ]) {
        assert!(
            line.trim_start().starts_with(level) && line.contains(message),
            "{line}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn replacement_dial_reports_non_transient_kinds_and_keeps_the_outage_open() {
    let (capture, _subscriber) = Capture::start();
    let log = OutageLog::new();
    let old = ConnectionObservation::default();
    old.ready(&log, SERVER);
    old.finish(Some((&log, SERVER)));

    let dial = ConnectionObservation::default();
    for kind in [ConnectErrorKind::Io, ConnectErrorKind::Tls] {
        let error = async_nats::ConnectError::with_source(kind, "connector detail");
        dial.observe(
            NatsConnectionEvent::ConnectFailed(&error),
            &log,
            SERVER,
            false,
        );
    }
    let replacement = ConnectionObservation::default();
    emit(&replacement, &log, Event::Connected);
    emit(&replacement, &log, Event::Disconnected);
    replacement.ready(&log, SERVER);
    let lines = capture.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("lost; reconnecting"));
    assert!(lines[1].starts_with("ERROR ") && lines[1].contains("TLS error"));
    assert!(!lines[1].contains("connector detail"));

    emit(&replacement, &log, Event::Connected);
    let lines = capture.lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[2].contains("restored") && lines[2].contains("reported_failed_retries=2"));
}

#[test]
fn expired_credential_reconnect_stays_out_of_errors_and_other_server_errors_do_not() {
    let (capture, _subscriber) = Capture::start();
    let log = OutageLog::new();
    let connection = ConnectionObservation::default();
    connection.ready(&log, SERVER);
    // async-nats strips the quotes from `-ERR 'User Authentication Expired'`.
    emit(
        &connection,
        &log,
        Event::ServerError(ServerError::Other("User Authentication Expired".into())),
    );
    emit(
        &connection,
        &log,
        Event::ServerError(ServerError::Other("user authentication expired".into())),
    );
    emit(
        &connection,
        &log,
        Event::ServerError(ServerError::Other("Maximum Connections Exceeded".into())),
    );

    let lines = capture.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].trim_start().starts_with("ERROR")
            && lines[0].contains("Maximum Connections Exceeded"),
        "{lines:?}"
    );
}
