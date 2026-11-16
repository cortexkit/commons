use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

use super::FakeSubc;
use crate::{credentials::CredentialManager, BusClient, BusClientOptions};

#[derive(Default)]
struct Observed {
    auth: Vec<Value>,
    subscriptions: Vec<String>,
    publishes: Vec<(String, Vec<u8>)>,
}

struct Server {
    url: String,
    observed: Arc<Mutex<Observed>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("nats://{}", listener.local_addr().unwrap());
        let observed = Arc::new(Mutex::new(Observed::default()));
        let shared = observed.clone();
        let task = tokio::spawn(async move {
            let mut peers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let shared = shared.clone();
                        peers.spawn(async move {
                            let (reader, mut writer) = socket.into_split();
                            let mut reader = BufReader::new(reader);
                            writer.write_all(b"INFO {\"server_id\":\"fake\",\"version\":\"2.15.0\",\"proto\":1,\"headers\":true,\"max_payload\":1048576,\"auth_required\":true,\"nonce\":\"server-nonce\"}\r\n").await.unwrap();
                            let mut subscriptions = HashMap::<String, String>::new();
                            loop {
                                let mut line = String::new();
                                if reader.read_line(&mut line).await.unwrap_or(0) == 0 { break; }
                                let tokens: Vec<_> = line.split_whitespace().collect();
                                match tokens.first().copied() {
                                    Some("CONNECT") => shared.lock().unwrap().auth.push(serde_json::from_str(line.trim().strip_prefix("CONNECT ").unwrap()).unwrap()),
                                    Some("PING") => { if writer.write_all(b"PONG\r\n").await.is_err() { break; } }
                                    Some("SUB") => {
                                        let subject = tokens[1].to_owned();
                                        subscriptions.insert(subject.clone(), tokens.last().unwrap().to_string());
                                        shared.lock().unwrap().subscriptions.push(subject);
                                    }
                                    Some("PUB" | "HPUB") => {
                                        let size = tokens.last().unwrap().parse::<usize>().unwrap();
                                        let mut payload = vec![0; size + 2];
                                        reader.read_exact(&mut payload).await.unwrap();
                                        payload.truncate(size);
                                        shared.lock().unwrap().publishes.push((tokens[1].into(), payload));
                                        let has_reply = if tokens[0] == "HPUB" { tokens.len() == 5 } else { tokens.len() == 4 };
                                        if has_reply {
                                            let reply = tokens[2];
                                            let sid = subscriptions.iter().find_map(|(subject, sid)| {
                                                reply.starts_with(subject.trim_end_matches('*')).then_some(sid)
                                            }).expect("reply inbox subscription");
                                            let body = json!({"stream": "EVENT", "seq": 17, "duplicate": true}).to_string();
                                            let message = format!("MSG {reply} {sid} {}\r\n{body}\r\n", body.len());
                                            if writer.write_all(message.as_bytes()).await.is_err() { break; }
                                        }
                                    }
                                    _ => {},
                                }
                            }
                        });
                    }
                    Some(result) = peers.join_next(), if !peers.is_empty() => { result.unwrap(); }
                }
            }
        });
        Self {
            url,
            observed,
            task,
        }
    }

    async fn wait_for(&self, predicate: impl Fn(&Observed) -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if predicate(&self.observed.lock().unwrap()) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn renewal_uses_dynamic_jwt_and_reissue_replaces_inbox() {
    let server = Server::start().await;
    let subc = FakeSubc::new(&server.url);
    let manager = CredentialManager::start(subc.clone()).await.unwrap();
    let client = BusClient::connect_manager(
        manager.clone(),
        BusClientOptions::new("test_module").unwrap(),
    )
    .await
    .unwrap();
    let mut changes = client.connection_changes();
    let old = client.connection();
    assert_eq!(old.inbox_prefix(), "_INBOX.UKEY0");
    manager.renew().await.unwrap();
    old.client().force_reconnect().await.unwrap();
    server.wait_for(|seen| seen.auth.len() >= 2).await;
    assert_eq!(
        server.observed.lock().unwrap().auth[1]["jwt"],
        "renewed-jwt"
    );
    assert!(!changes.has_changed().unwrap());

    manager.reissue("UKEY0").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*changes.borrow_and_update(), 1);
    assert_eq!(client.connection().inbox_prefix(), "_INBOX.UKEY1");
    assert_eq!(
        old.inbox_prefix(),
        "_INBOX.UKEY0",
        "held snapshots must not silently change inboxes"
    );
    assert_eq!(client.acct(), "box_test");

    let receipt = client
        .publish_event("fact.created", 2, Some("reader"), "fact-42", "event-body")
        .await
        .unwrap();
    assert_eq!(receipt.stream_sequence(), 17);
    assert!(receipt.duplicate());
    client.live_hint("refresh", 1, "hint-body").await.unwrap();
    client.connection().client().flush().await.unwrap();
    server.wait_for(|seen| seen.publishes.len() == 2).await;
    let seen = server.observed.lock().unwrap();
    assert_eq!(
        seen.publishes[0].0,
        "ck.box_test.event.test_module.fact.created.v2.reader"
    );
    let event = String::from_utf8(seen.publishes[0].1.clone()).unwrap();
    assert!(event.contains("Nats-Msg-Id: fact-42\r\n"));
    assert!(event.ends_with("\r\n\r\nevent-body"));
    assert_eq!(
        seen.publishes[1],
        (
            "ck.box_test.live.test_module.refresh.v1".into(),
            b"hint-body".to_vec()
        )
    );
    assert!(seen
        .subscriptions
        .iter()
        .any(|subject| subject.starts_with("_INBOX.UKEY1.")));
}

#[tokio::test]
async fn invalid_helper_names_never_reach_the_server() {
    let server = Server::start().await;
    let manager = CredentialManager::start(FakeSubc::new(&server.url))
        .await
        .unwrap();
    let client = BusClient::connect_manager(manager, BusClientOptions::new("test_module").unwrap())
        .await
        .unwrap();
    assert!(client
        .publish_event("bad.*", 1, None, "id", "body")
        .await
        .is_err());
    assert!(client
        .publish_event("valid", 0, None, "id", "body")
        .await
        .is_err());
    assert!(client
        .publish_event("valid", 1, Some("bad.reader"), "id", "body")
        .await
        .is_err());
    assert!(client.live_hint("bad.*", 1, "body").await.is_err());
    assert!(server.observed.lock().unwrap().publishes.is_empty());
}

#[tokio::test]
async fn terminal_renew_refusal_closes_changes_and_warns_once() {
    use std::sync::atomic::Ordering;

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

    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let writer_output = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_writer(move || Capture(writer_output.clone()))
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let server = Server::start().await;
    for code in ["ckbus_generation_not_live", "ckbus_grant_refused"] {
        let subc = FakeSubc::new(&server.url);
        // A still-valid credential already at its renewal deadline makes the
        // real maintenance task renew immediately without a long test sleep.
        subc.issue_exp
            .store(super::expiry() - 600, Ordering::SeqCst);
        *subc.renew_refusal.lock().unwrap() = Some(code.into());
        let manager = CredentialManager::start(subc.clone()).await.unwrap();
        let client =
            BusClient::connect_manager(manager, BusClientOptions::new("test_module").unwrap())
                .await
                .unwrap();
        let mut changes = client.connection_changes();
        let closed = tokio::time::timeout(Duration::from_secs(5), changes.changed())
            .await
            .expect("terminal renewal must close the change stream");
        assert!(
            closed.is_err(),
            "terminal renewal must not emit a replacement"
        );
        assert_eq!(
            subc.issues.load(Ordering::SeqCst),
            1,
            "no re-issue after terminal refusal"
        );
        assert_eq!(
            subc.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(method, _)| method == "ckbus.credential_renew")
                .count(),
            1
        );
        let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert_eq!(
            logs.matches(code).count(),
            1,
            "one warning with the refusal code"
        );
        assert!(
            !logs.contains("jwt-0"),
            "warnings must not contain credentials"
        );
    }
    let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert_eq!(
        logs.matches("bus credential maintenance stopped").count(),
        2
    );
}
