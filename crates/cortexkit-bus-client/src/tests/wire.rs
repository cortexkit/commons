use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

use super::FakeSubc;
use crate::{
    credentials::CredentialManager, BusClient, BusClientOptions, Error, EventDelivery, EventItem,
    MalformedReason,
};

#[derive(Default)]
struct Observed {
    auth: Vec<Value>,
    subscriptions: Vec<String>,
    publishes: Vec<(String, Vec<u8>)>,
    events: VecDeque<FakeEvent>,
    attempts: i64,
    consumer_sequence: u64,
    hold_old_pulls: bool,
}

struct FakeEvent {
    sequence: u64,
    subject: String,
    headers: String,
    body: String,
}

impl FakeEvent {
    fn new(sequence: u64) -> Self {
        Self {
            sequence,
            subject: "ck.box_test.event.publisher.fact.created.v2.test_module".into(),
            headers: format!("Nats-Msg-Id: fact-{sequence}\r\n"),
            body: format!("body-{sequence}"),
        }
    }
}

fn response(
    seen: &mut Observed,
    subject: &str,
    payload: &[u8],
    reply: &str,
    sid: &str,
) -> Option<String> {
    if subject.starts_with("$JS.API.CONSUMER.MSG.NEXT.") {
        let pull: Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(pull["batch"], 1, "reader must pull one message at a time");
        if seen.hold_old_pulls && reply.starts_with("_INBOX.UKEY0.") {
            return None;
        }
        let event = seen.events.front()?;
        seen.attempts += 1;
        seen.consumer_sequence += 1;
        let ack = format!(
            "$JS.ACK.CK_BOX_TEST_EVENT.m_test_module.{}.{}.{}.1.0",
            seen.attempts, event.sequence, seen.consumer_sequence
        );
        let headers = format!("NATS/1.0\r\n{}\r\n", event.headers);
        return Some(format!(
            "HMSG {} {sid} {ack} {} {}\r\n{headers}{}\r\n",
            event.subject,
            headers.len(),
            headers.len() + event.body.len(),
            event.body
        ));
    }
    let body = if subject.starts_with("$JS.API.CONSUMER.INFO.") {
        if subject.ends_with(".m_test_module_absent") {
            json!({"error": {"code": 404, "err_code": 10014, "description": "consumer not found"}})
                .to_string()
        } else {
            let name = subject.rsplit('.').next().unwrap();
            json!({
                "stream_name": "CK_BOX_TEST_EVENT", "name": name,
                "created": "2026-01-01T00:00:00Z",
                "config": {"durable_name": name, "deliver_policy": "all", "ack_policy": "explicit", "ack_wait": 30000000000_u64, "max_deliver": -1, "replay_policy": "instant"},
                "delivered": {"consumer_seq": 0, "stream_seq": 0},
                "ack_floor": {"consumer_seq": 0, "stream_seq": 0},
                "num_ack_pending": 0, "num_redelivered": 0, "num_waiting": 0, "num_pending": seen.events.len()
            }).to_string()
        }
    } else if subject.starts_with("$JS.ACK.") {
        if payload == b"+ACK" || payload == b"+TERM" {
            seen.events.pop_front();
            seen.attempts = 0;
        }
        String::new()
    } else {
        json!({"stream": "EVENT", "seq": 17, "duplicate": true}).to_string()
    };
    Some(format!("MSG {reply} {sid} {}\r\n{body}\r\n", body.len()))
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
                                        shared.lock().unwrap().publishes.push((tokens[1].into(), payload.clone()));
                                        let has_reply = if tokens[0] == "HPUB" { tokens.len() == 5 } else { tokens.len() == 4 };
                                        if has_reply {
                                            let reply = tokens[2];
                                            let sid = subscriptions.iter().find_map(|(subject, sid)| {
                                                reply.starts_with(subject.trim_end_matches('*')).then_some(sid)
                                            }).expect("reply inbox subscription");
                                            let message = response(&mut shared.lock().unwrap(), tokens[1], &payload, reply, sid);
                                            if let Some(message) = message {
                                                if writer.write_all(message.as_bytes()).await.is_err() { break; }
                                            }
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

async fn event_client(server: &Server) -> (BusClient, Arc<CredentialManager>) {
    let manager = CredentialManager::start(FakeSubc::new(&server.url))
        .await
        .unwrap();
    let client = BusClient::connect_manager(
        manager.clone(),
        BusClientOptions::new("test_module").unwrap(),
    )
    .await
    .unwrap();
    (client, manager)
}

#[tokio::test]
async fn bucket_guards_refuse_undeclared_unreadable_and_nonowner_access() {
    let server = Server::start().await;
    let (outsider, _) = event_client(&server).await;
    assert!(matches!(
        outsider.bucket("aft", "absent"),
        Err(Error::BucketNotDeclared { .. })
    ));
    assert!(matches!(
        outsider.bucket("aft", "views"),
        Err(Error::BucketNotReadable { .. })
    ));
    let manager = CredentialManager::start(FakeSubc::new(&server.url))
        .await
        .unwrap();
    let reader = BusClient::connect_manager(manager, BusClientOptions::new("occipital").unwrap())
        .await
        .unwrap();
    for owner in ["aft", "cerebellum"] {
        let bucket = reader.bucket(owner, "views").unwrap();
        assert!(matches!(
            bucket.put("key", "value").await,
            Err(Error::BucketNotOwned { .. })
        ));
        assert!(matches!(
            bucket.delete("key").await,
            Err(Error::BucketNotOwned { .. })
        ));
    }
    assert!(
        server.observed.lock().unwrap().publishes.is_empty(),
        "guards must not send requests"
    );
}

#[tokio::test]
async fn bucket_value_cap_is_checked_before_sending() {
    let server = Server::start().await;
    let manager = CredentialManager::start(FakeSubc::new(&server.url))
        .await
        .unwrap();
    let owner = BusClient::connect_manager(manager, BusClientOptions::new("aft").unwrap())
        .await
        .unwrap();
    let bucket = owner.bucket("aft", "views").unwrap();
    assert!(matches!(
        bucket.put("key", vec![0; 262_145]).await,
        Err(Error::BucketValueTooLarge {
            size: 262_145,
            cap: 262_144
        })
    ));
    assert!(server.observed.lock().unwrap().publishes.is_empty());
}

#[tokio::test]
async fn bucket_delete_rejects_invalid_keys_before_sending() {
    let server = Server::start().await;
    let manager = CredentialManager::start(FakeSubc::new(&server.url))
        .await
        .unwrap();
    let owner = BusClient::connect_manager(manager, BusClientOptions::new("aft").unwrap())
        .await
        .unwrap();
    let bucket = owner.bucket("aft", "views").unwrap();
    for key in ["", ".key", "key.", "*", ">", "key with space", "key\r\n"] {
        assert!(
            matches!(bucket.delete(key).await, Err(Error::Nats(ref error)) if error == "invalid KV key"),
            "{key:?}"
        );
    }
    assert!(server.observed.lock().unwrap().publishes.is_empty());
}

#[tokio::test]
async fn bucket_rebind_capacity_wait_is_bounded_and_logs_once() {
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
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_writer(move || Capture(writer.clone()))
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let server = Server::start().await;
    let (client, _) = event_client(&server).await;
    let mut binding = crate::binding::ConnectionBinding::new(&client);
    tokio::time::pause();
    let start = tokio::time::Instant::now();
    let mut attempts = Vec::new();
    let error = crate::buckets::bind_watch(&mut binding, "full-bucket", true, |_| {
        attempts.push(start.elapsed().as_secs());
        async {
            Err::<(), _>(Error::WatchCapacity {
                bucket: "full-bucket".into(),
            })
        }
    })
    .await
    .unwrap_err();
    assert!(matches!(error, Error::WatchCapacity { .. }));
    // Tokio's timer wheel rounds deadline wakes up to the next millisecond tick.
    assert!(
        start.elapsed() >= Duration::from_secs(60)
            && start.elapsed() < Duration::from_secs(60) + Duration::from_millis(2)
    );
    assert_eq!(
        attempts,
        [0, 1, 3, 7, 12, 17, 22, 27, 32, 37, 42, 47, 52, 57, 60]
    );
    let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert_eq!(
        logs.matches("waiting for bucket watch consumer capacity during rebind")
            .count(),
        1
    );
}

#[tokio::test]
async fn bucket_rebind_capacity_wait_stops_on_terminal_maintenance() {
    let server = Server::start().await;
    let (client, _) = event_client(&server).await;
    let mut binding = crate::binding::ConnectionBinding::new(&client);
    client.task.abort();
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        crate::buckets::bind_watch(&mut binding, "full-bucket", true, |_| async {
            Err::<(), _>(Error::WatchCapacity {
                bucket: "full-bucket".into(),
            })
        }),
    )
    .await
    .expect("terminal maintenance must interrupt capacity backoff")
    .unwrap_err();
    assert!(
        matches!(error, Error::Transport(ref text) if text == "credential maintenance stopped")
    );
}

#[tokio::test]
async fn claim_check_sends_codec_headers_and_empty_payload() {
    use cortexkit_bus_trait::{ContentDigest, Headers};
    let server = Server::start().await;
    let (client, _) = event_client(&server).await;
    let digest_text = "0123456789abcdef".repeat(4);
    let digest: ContentDigest = digest_text.parse().unwrap();
    for qualifier in [None, Some("reader")] {
        let mut headers = Headers::new();
        headers.insert("scope".into(), "agent-public".into());
        let receipt = client
            .publish_claim_check("fact.created", 2, qualifier, "claim-42", digest, headers)
            .await
            .unwrap();
        assert_eq!(receipt.stream_sequence(), 17);
        assert!(receipt.duplicate());
    }
    client.connection().client().flush().await.unwrap();
    let seen = server.observed.lock().unwrap();
    assert_eq!(seen.publishes.len(), 2);
    assert_eq!(
        seen.publishes[0].0,
        "ck.box_test.event.test_module.fact.created.v2"
    );
    assert_eq!(
        seen.publishes[1].0,
        "ck.box_test.event.test_module.fact.created.v2.reader"
    );
    for (_, payload) in &seen.publishes {
        let wire = std::str::from_utf8(payload).unwrap();
        let (headers, body) = wire.split_once("\r\n\r\n").unwrap();
        assert!(body.is_empty(), "claim checks never send an inline body");
        assert!(headers.contains("\r\nNats-Msg-Id: claim-42"));
        assert!(headers.contains(&format!("\r\nCk-Content-Digest: {digest_text}")));
        assert!(headers.contains("\r\nscope: agent-public"));
    }
}

#[tokio::test]
async fn claim_check_refuses_reserved_caller_headers_before_sending() {
    use cortexkit_bus_trait::{BusError, ContentDigest, Headers};
    let server = Server::start().await;
    let (client, _) = event_client(&server).await;
    for name in [
        "Ck-Content-Digest",
        "Nats-Msg-Id",
        "ck-content-digest",
        "NATS-MSG-ID",
    ] {
        let headers = Headers::from([(name.into(), "caller override".into())]);
        let error = client
            .publish_claim_check(
                "fact",
                1,
                None,
                "id",
                ContentDigest::of_bytes(b"body"),
                headers,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::Bus(BusError::Denied { ref subject, .. }) if subject == name)
        );
    }
    assert!(server.observed.lock().unwrap().publishes.is_empty());
}

async fn delivery(reader: &mut crate::EventReader) -> EventDelivery {
    match tokio::time::timeout(Duration::from_secs(5), reader.next())
        .await
        .expect("event pull timed out")
        .unwrap()
    {
        EventItem::Delivery(delivery) => delivery,
        EventItem::Malformed(malformed) => {
            panic!("unexpected malformed event: {:?}", malformed.reason)
        }
    }
}

#[tokio::test]
async fn publish_event_body_limit_is_4096_bytes() {
    let server = Server::start().await;
    let (client, _) = event_client(&server).await;
    let at_limit = client
        .publish_event("fact", 1, None, "at-limit", vec![0; 4096])
        .await
        .unwrap();
    assert_eq!(at_limit.stream_sequence(), 17);
    let empty = client
        .publish_event("fact", 1, None, "empty", "")
        .await
        .unwrap();
    assert_eq!(empty.stream_sequence(), 17);
    let error = client
        .publish_event("fact", 1, None, "over-limit", vec![0; 4097])
        .await
        .unwrap_err();
    assert!(matches!(error, Error::EventBodyTooLarge { size: 4097 }));
    client.connection().client().flush().await.unwrap();
    assert_eq!(
        server.observed.lock().unwrap().publishes.len(),
        2,
        "oversize body never reaches NATS"
    );
}

#[tokio::test]
async fn event_reader_attaches_only_to_existing_owned_durables() {
    let server = Server::start().await;
    let (client, _) = event_client(&server).await;
    for name in [
        "m_other",
        "m_test_modulex",
        "m_test_module_",
        "m_test_module_BAD",
        "m_test_module_bad.suffix",
    ] {
        assert!(
            matches!(
                client.event_reader(name).await,
                Err(Error::DurableNotOwned { .. })
            ),
            "{name}"
        );
    }
    assert!(server.observed.lock().unwrap().publishes.is_empty());
    client.event_reader("m_test_module").await.unwrap();
    client.event_reader("m_test_module_reader_2").await.unwrap();
    assert!(matches!(
        client.event_reader("m_test_module_absent").await,
        Err(Error::Nats(_))
    ));
    let seen = server.observed.lock().unwrap();
    assert_eq!(seen.publishes.len(), 3);
    assert!(
        seen.publishes
            .iter()
            .all(|(subject, _)| subject.starts_with("$JS.API.CONSUMER.INFO.CK_BOX_TEST_EVENT.")),
        "attaching must not create or update consumers"
    );
}

#[tokio::test]
async fn event_reader_reads_headers_and_settles_ack_nak_term() {
    let server = Server::start().await;
    let mut event = FakeEvent::new(41);
    event.headers.push_str("epoch: 7\r\nprovider: first\r\nprovider: second\r\naccount: public\r\nseq: 12\r\nCk-Content-Digest: digest-41\r\n");
    server
        .observed
        .lock()
        .unwrap()
        .events
        .extend([event, FakeEvent::new(42)]);
    let (client, _) = event_client(&server).await;
    let mut reader = client.event_reader("m_test_module").await.unwrap();
    let first = delivery(&mut reader).await;
    assert_eq!(first.subject.module, "publisher");
    assert_eq!(first.subject.event, "fact.created");
    assert_eq!(first.subject.version, 2);
    assert_eq!(first.subject.reader.as_deref(), Some("test_module"));
    assert_eq!(first.msg_id, "fact-41");
    assert_eq!(first.body.as_ref(), b"body-41");
    assert_eq!(first.stream_sequence, 41);
    assert_eq!(first.delivered_count, 1);
    assert_eq!(first.header("provider"), Some("first"));
    assert_eq!(first.header("epoch"), Some("7"));
    assert_eq!(first.header("account"), Some("public"));
    assert_eq!(first.header("seq"), Some("12"));
    assert_eq!(first.header("Ck-Content-Digest"), Some("digest-41"));
    assert_eq!(first.header("absent"), None);
    assert_eq!(first.headers().len(), 6);
    assert_eq!(
        first.headers().get("provider").map(String::as_str),
        Some("first")
    );
    drop(first);
    assert!(
        !server
            .observed
            .lock()
            .unwrap()
            .publishes
            .iter()
            .any(|(subject, _)| subject.starts_with("$JS.ACK.")),
        "read and drop must not acknowledge"
    );
    let redelivered = delivery(&mut reader).await;
    assert_eq!(redelivered.stream_sequence, 41);
    assert_eq!(redelivered.delivered_count, 2);
    redelivered.nak(Duration::from_millis(250)).await.unwrap();
    let retried = delivery(&mut reader).await;
    assert_eq!(retried.delivered_count, 3);
    retried.term().await.unwrap();
    let second = delivery(&mut reader).await;
    assert_eq!(second.stream_sequence, 42);
    second.ack().await.unwrap();
    let seen = server.observed.lock().unwrap();
    let settlements: Vec<_> = seen
        .publishes
        .iter()
        .filter(|(subject, _)| subject.starts_with("$JS.ACK."))
        .map(|(_, body)| body.as_slice())
        .collect();
    assert_eq!(
        settlements,
        [b"-NAK {\"delay\":250000000}".as_slice(), b"+TERM", b"+ACK"]
    );
    assert!(seen.events.is_empty());
}

#[tokio::test]
async fn event_reader_malformed_records_can_be_terminated() {
    let server = Server::start().await;
    let mut missing_id = FakeEvent::new(1);
    missing_id.headers.clear();
    let mut bad_subject = FakeEvent::new(2);
    bad_subject.subject = "ck.box_other.event.publisher.fact.v1".into();
    server
        .observed
        .lock()
        .unwrap()
        .events
        .extend([missing_id, bad_subject]);
    let (client, _) = event_client(&server).await;
    let mut reader = client.event_reader("m_test_module").await.unwrap();
    for (sequence, reason) in [
        (1, MalformedReason::MissingMsgId),
        (2, MalformedReason::BadSubject),
    ] {
        let item = tokio::time::timeout(Duration::from_secs(5), reader.next())
            .await
            .unwrap()
            .unwrap();
        let EventItem::Malformed(malformed) = item else {
            panic!("invalid record accepted");
        };
        assert_eq!(malformed.reason, reason);
        assert_eq!(malformed.stream_sequence, Some(sequence));
        assert!(malformed.raw_subject.starts_with("ck."));
        assert_eq!(
            server.observed.lock().unwrap().events.len(),
            (3 - sequence) as usize,
            "no automatic settlement"
        );
        malformed.term().await.unwrap();
    }
    assert!(server.observed.lock().unwrap().events.is_empty());
}

#[tokio::test]
async fn event_reader_reissue_rebinds_pending_pull_and_redelivers_unacked_event() {
    let server = Server::start().await;
    server
        .observed
        .lock()
        .unwrap()
        .events
        .extend([FakeEvent::new(1), FakeEvent::new(2)]);
    let (client, manager) = event_client(&server).await;
    let mut reader = client.event_reader("m_test_module").await.unwrap();
    let first = delivery(&mut reader).await;
    assert_eq!(first.delivered_count, 1);
    drop(first);
    server.observed.lock().unwrap().hold_old_pulls = true;
    let pending = tokio::spawn(async move {
        let message = delivery(&mut reader).await;
        (reader, message)
    });
    server
        .wait_for(|seen| {
            seen.publishes
                .iter()
                .filter(|(subject, _)| subject.starts_with("$JS.API.CONSUMER.MSG.NEXT."))
                .count()
                == 2
        })
        .await;
    manager.reissue("UKEY0").await.unwrap();
    let (mut reader, redelivered) = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(redelivered.stream_sequence, 1);
    assert_eq!(redelivered.msg_id, "fact-1");
    assert_eq!(redelivered.body.as_ref(), b"body-1");
    assert_eq!(redelivered.delivered_count, 2);
    redelivered.ack().await.unwrap();
    let next = delivery(&mut reader).await;
    assert_eq!(
        next.stream_sequence, 2,
        "server cursor survives credential reissue"
    );
    next.ack().await.unwrap();
    let seen = server.observed.lock().unwrap();
    assert_eq!(
        seen.publishes
            .iter()
            .filter(
                |(subject, _)| subject == "$JS.API.CONSUMER.INFO.CK_BOX_TEST_EVENT.m_test_module"
            )
            .count(),
        2,
        "same durable must be rebound"
    );
    assert!(!seen
        .publishes
        .iter()
        .any(|(subject, _)| subject.contains("CONSUMER.CREATE")
            || subject.contains("CONSUMER.DELETE")));
    assert!(seen
        .subscriptions
        .iter()
        .any(|subject| subject.starts_with("_INBOX.UKEY1.")));
}
