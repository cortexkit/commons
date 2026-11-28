use async_nats::{Event, ServerError};
use cortexkit_bus_trait::{BusError, ConnectionFailure};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::error::{map_connect_error, permission_violation};
use crate::{
    ConnectConfig, NatsConnection, NatsConnectionEvent, NatsConnectionState,
    ASYNC_NATS_VERSION_FLOOR,
};

#[test]
fn declared_async_nats_floor_is_not_below_the_specification() {
    let floor = ASYNC_NATS_VERSION_FLOOR
        .split('.')
        .map(|part| part.parse::<u32>().expect("numeric version component"))
        .collect::<Vec<_>>();
    assert!(floor[0] > 0 || floor[1] >= 33);
}

#[test]
fn inbox_prefix_is_credential_scoped() {
    let config = ConnectConfig::new("UABC234").expect("valid public nkey");
    assert_eq!(config.inbox_prefix(), "_INBOX.UABC234");
    assert!(ConnectConfig::new("UABC.bad").is_err());
}

#[tokio::test]
async fn custom_inbox_prefix_refuses_invalid_subjects_before_connecting() {
    for prefix in [
        "",
        ".module",
        "module.",
        "module..reply",
        "module.*",
        "module.>",
        "module.re*ply",
        "module.re>ply",
        "module reply",
        "module\nreply",
        "module\0reply",
    ] {
        let result = NatsConnection::connect_with_inbox_prefix(
            "nats://127.0.0.1:1",
            async_nats::ConnectOptions::new(),
            ConnectConfig::new("UABC234").unwrap(),
            prefix,
        )
        .await;
        assert!(matches!(result, Err(BusError::Denied { .. })), "{prefix:?}");
    }
}

async fn fake_nats_handshake(socket: TcpStream) -> BufReader<TcpStream> {
    let mut socket = BufReader::new(socket);
    socket.get_mut().write_all(b"INFO {}\r\n").await.unwrap();
    let mut line = String::new();
    socket.read_line(&mut line).await.unwrap();
    assert!(line.starts_with("CONNECT "));
    line.clear();
    socket.read_line(&mut line).await.unwrap();
    assert_eq!(line, "PING\r\n");
    socket.get_mut().write_all(b"PONG\r\n").await.unwrap();
    socket
}

#[tokio::test]
async fn observed_connection_delivers_first_disconnect_without_a_late_subscription() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        drop(listener);
        // Close immediately after the handshake; the caller has not attached
        // any watcher or event receiver to the returned connection.
        drop(fake_nats_handshake(socket).await);
    });
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel();
    let connection = NatsConnection::connect_observed(
        format!("nats://{address}"),
        async_nats::ConnectOptions::new(),
        ConnectConfig::new("UABC234").unwrap(),
        None,
        move |event| {
            if let NatsConnectionEvent::Event(event) = event {
                let _ = send.send(event.clone());
            }
        },
    )
    .await
    .unwrap();
    server.await.unwrap();
    let first_events = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        (events.recv().await, events.recv().await)
    })
    .await
    .expect("observer receives both handshake and disconnect");
    assert_eq!(
        first_events,
        (Some(Event::Connected), Some(Event::Disconnected))
    );
    assert_eq!(
        *connection.connection_state().borrow(),
        NatsConnectionState::Disconnected
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_exhaustion_closes_state_when_terminal_event_is_lost() {
    use std::sync::{mpsc, Mutex};
    use std::time::Duration;

    const QUEUE_CAPACITY: usize = 128; // async-nats' event queue, not our broadcast.
    let timeout = Duration::from_secs(5);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (flood, start_flood) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = fake_nats_handshake(socket).await;
        start_flood.await.unwrap();
        let mut frames = b"-ERR 'test event'\r\n".repeat(QUEUE_CAPACITY);
        frames.extend_from_slice(b"PING\r\n");
        socket.get_mut().write_all(&frames).await.unwrap();
        // The PONG proves the handler parsed every error before the outage.
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(socket.read_line(&mut line).await.unwrap(), 0);
            match line.as_str() {
                "PONG\r\n" => break,
                "PING\r\n" => socket.get_mut().write_all(b"PONG\r\n").await.unwrap(),
                _ => panic!("unexpected client frame: {line:?}"),
            }
        }
        drop(socket);
        // Keep the listening port reserved through the one allowed retry.
        let (retry, _) = listener.accept().await.unwrap();
        drop(retry);
    });
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel();
    let (release, blocked) = mpsc::channel();
    let blocked = Mutex::new(blocked);
    let connection = NatsConnection::connect_observed(
        format!("nats://{address}"),
        async_nats::ConnectOptions::new().max_reconnects(1),
        ConnectConfig::new("UABC234").unwrap(),
        None,
        move |event| {
            if let NatsConnectionEvent::Event(event) = event {
                send.send(event.clone()).unwrap();
                if *event == Event::Connected {
                    // Hold the event dispatch task while the handler fills
                    // its queue and exits without delivering MaxReconnects.
                    let _ = blocked.lock().unwrap().recv();
                }
            }
        },
    )
    .await
    .unwrap();
    let mut states = connection.connection_state();
    assert_eq!(
        tokio::time::timeout(timeout, events.recv()).await.unwrap(),
        Some(Event::Connected)
    );
    flood.send(()).unwrap();
    tokio::time::timeout(timeout, server)
        .await
        .expect("the server accepts the one reconnect attempt")
        .unwrap();
    assert!(tokio::time::timeout(timeout, connection.client().flush())
        .await
        .expect("the capped handler stops processing commands")
        .is_err());
    release.send(()).unwrap();

    tokio::time::timeout(
        timeout,
        states.wait_for(|state| *state == NatsConnectionState::Closed),
    )
    .await
    .expect("handler exit ends the wait even without a terminal event")
    .expect("the state watch retains the final state");
    let delivered: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    assert_eq!(delivered.len(), QUEUE_CAPACITY);
    assert!(delivered
        .iter()
        .all(|event| matches!(event, Event::ServerError(_))));
}

#[tokio::test]
async fn observed_connection_reports_the_initial_error_kind_before_returning() {
    let (send, mut failures) = tokio::sync::mpsc::unbounded_channel();
    let result = NatsConnection::connect_observed(
        "nats://[",
        async_nats::ConnectOptions::new(),
        ConnectConfig::new("UABC234").unwrap(),
        None,
        move |event| {
            if let NatsConnectionEvent::ConnectFailed(error) = event {
                send.send(error.kind()).unwrap();
            }
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(
        failures.try_recv(),
        Ok(async_nats::ConnectErrorKind::ServerParse)
    );
}

#[test]
fn connection_boundary_taxonomy_matches_kick_drop_and_severed_write() {
    let refused = map_connect_error("Authorization Violation");
    assert!(matches!(refused, BusError::Denied { .. }));

    let dropped = map_connect_error("connection reset by peer");
    assert!(matches!(dropped, BusError::Unavailable { .. }));

    let severed =
        cortexkit_bus_trait::map_connection_failure(ConnectionFailure::WriteOnSeveredSocket);
    assert!(matches!(severed, BusError::Unavailable { .. }));

    let kicked_then_refused =
        permission_violation(&Event::ServerError(ServerError::AuthorizationViolation))
            .expect("authorization refusal is observable")
            .as_denied();
    assert!(matches!(kicked_then_refused, BusError::Denied { .. }));
}

#[test]
fn register_watch_accepts_only_valid_keys_inside_its_filter() {
    use crate::kv_watch::{is_valid_key, key_matches};

    assert!(key_matches(">", "module.a"));
    assert!(key_matches("module.>", "module.a.b"));
    assert!(!key_matches("module.>", "module"));
    assert!(!key_matches("module.>", "other.a"));
    assert!(key_matches("module.*", "module.a"));
    assert!(!key_matches("module.*", "module.a.b"));
    assert!(key_matches("module.a", "module.a"));
    assert!(!key_matches("module.a", "module.ab"));

    assert!(is_valid_key("module.a-b_c/d=e"));
    assert!(!is_valid_key(""));
    assert!(!is_valid_key(".module"));
    assert!(!is_valid_key("module."));
    assert!(!is_valid_key("module a"));
    assert!(!is_valid_key("module.*"));
}
