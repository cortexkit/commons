use std::env;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use async_nats::jetstream::consumer::{pull, AckPolicy};
use async_nats::jetstream::{kv, stream};
use async_nats::ConnectOptions;
use bytes::Bytes;
use cortexkit_bus_trait::{
    terminally_dispose, BusError, ClaimOutcome, ContentDigest, DeadLetterRecord, Headers, Register,
    RegisterUpdate, RegisterWatch, Stream, StreamCursor, WatchEvent, WorkQueue,
};
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio::time::{sleep, Instant};

use crate::{ConnectConfig, NatsConnection};

const ADMIN_PUBLIC: &str = "UADMIN";
const PARTICIPANT_PUBLIC: &str = "UPARTICIPANT";

struct TestServer {
    child: Child,
    _directory: TempDir,
    url: String,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl TestServer {
    async fn open() -> Option<Self> {
        Self::start(None).await
    }

    async fn with_participant_permissions() -> Option<Self> {
        Self::start(Some(permission_config)).await
    }

    async fn start(config: Option<fn(u16, &TempDir) -> String>) -> Option<Self> {
        let binary = env::var_os("CK_NATS_SERVER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("nats-server"));
        let version = match Command::new(&binary).arg("--version").output() {
            Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
            Err(_) => {
                eprintln!("nats-server-absent: CK_NATS_SERVER_BIN/PATH has no nats-server");
                return None;
            }
        };
        if !server_at_least_2_10(&version) {
            eprintln!("nats-server-too-old: observed {}", version.trim());
            return None;
        }

        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let port = listener.local_addr().expect("reserved address").port();
        drop(listener);
        // The server's store lives under the process temp root; TempDir removes it
        // when the guard drops, which is on every exit path of the test.
        let directory = tempfile::Builder::new()
            .prefix("cortexkit-bus-nats-")
            .tempdir()
            .expect("temporary NATS directory");
        let mut command = Command::new(binary);
        if let Some(render) = config {
            let config_path = directory.path().join("nats.conf");
            std::fs::write(&config_path, render(port, &directory)).expect("write NATS config");
            command.arg("-c").arg(config_path);
        } else {
            command
                .arg("--jetstream")
                .arg("--store_dir")
                .arg(directory.path().join("store"))
                .arg("--port")
                .arg(port.to_string());
        }
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start nats-server");
        let url = format!("127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(5);
        while TcpStream::connect(&url).await.is_err() {
            assert!(
                Instant::now() < deadline,
                "nats-server did not listen in time"
            );
            sleep(Duration::from_millis(20)).await;
        }
        Some(Self {
            child,
            _directory: directory,
            url,
        })
    }
}

fn permission_config(port: u16, directory: &TempDir) -> String {
    let store_path = directory.path().join("store");
    let store = store_path.display();
    format!(
        r#"
port: {port}
jetstream {{ store_dir: "{store}" }}
authorization {{
  users: [
    {{ user: "admin", password: "admin" }},
    {{
      user: "participant",
      password: "participant",
      permissions: {{
        publish: {{ allow: [
          "$JS.API.CONSUMER.INFO.CK_BOX_ROOM.c_agent",
          "$JS.API.CONSUMER.INFO.CK_BOX_WAKE.c_agent",
          "$JS.API.CONSUMER.INFO.CK_BOX_PEER.c_agent",
          "$JS.API.CONSUMER.INFO.CK_BOX_EFFECT.c_agent"
        ] }},
        subscribe: {{ allow: ["_INBOX.{PARTICIPANT_PUBLIC}.>"] }}
      }}
    }}
  ]
}}
"#
    )
}

fn server_at_least_2_10(version: &str) -> bool {
    let Some(version) = version
        .split_whitespace()
        .find_map(|word| word.strip_prefix('v'))
    else {
        return false;
    };
    let mut parts = version.split('.');
    let major = parts.next().and_then(|part| part.parse::<u32>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u32>().ok());
    matches!((major, minor), (Some(major), Some(minor)) if major > 2 || major == 2 && minor >= 10)
}

async fn connect(url: &str, credential_public: &str) -> NatsConnection {
    NatsConnection::connect(
        url,
        ConnectOptions::new().request_timeout(Some(Duration::from_millis(300))),
        ConnectConfig::new(credential_public).expect("valid fixture public key"),
    )
    .await
    .expect("connect to fixture")
}

async fn connect_admin(url: &str) -> NatsConnection {
    NatsConnection::connect(
        url,
        ConnectOptions::with_user_and_password("admin".into(), "admin".into())
            .request_timeout(Some(Duration::from_millis(300))),
        ConnectConfig::new(ADMIN_PUBLIC).expect("valid admin public key"),
    )
    .await
    .expect("connect admin")
}

async fn create_stream_and_consumer(
    connection: &NatsConnection,
    stream_name: &str,
    subject: &str,
    filter_subject: &str,
    durable: &str,
    max_deliver: i64,
    ack_wait: Duration,
) {
    let stream = connection
        .jetstream()
        .create_stream(stream::Config {
            name: stream_name.into(),
            subjects: vec![subject.into()],
            storage: stream::StorageType::File,
            ..Default::default()
        })
        .await
        .expect("create stream");
    stream
        .create_consumer(pull::Config {
            durable_name: Some(durable.into()),
            ack_policy: AckPolicy::Explicit,
            ack_wait,
            max_deliver,
            filter_subject: filter_subject.into(),
            ..Default::default()
        })
        .await
        .expect("create durable consumer");
}

async fn next_claim(queue: &crate::NatsWorkQueue) -> ClaimOutcome {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match queue.claim().await.expect("claim work item") {
            ClaimOutcome::Empty if Instant::now() < deadline => {
                sleep(Duration::from_millis(20)).await
            }
            outcome => return outcome,
        }
    }
}

fn digest(value: &str) -> ContentDigest {
    ContentDigest::of_bytes(value.as_bytes())
}

#[tokio::test]
async fn configured_connection_generates_only_credential_scoped_inboxes() {
    let Some(server) = TestServer::open().await else {
        return;
    };
    let connection = connect(&server.url, "UCREDENTIAL").await;
    assert!(connection
        .client()
        .new_inbox()
        .as_str()
        .starts_with("_INBOX.UCREDENTIAL."));
}

#[tokio::test]
async fn durable_cursor_survives_restart_and_credential_rekey() {
    let Some(server) = TestServer::open().await else {
        return;
    };
    let admin = connect(&server.url, ADMIN_PUBLIC).await;
    create_stream_and_consumer(
        &admin,
        "CK_BOX_PEER",
        "ck.box.peer.agent.*.deliver",
        "ck.box.peer.agent.*.deliver",
        "c_agent",
        -1,
        Duration::from_secs(30),
    )
    .await;
    let publisher = admin.stream("CK_BOX_PEER");

    publisher
        .publish(
            "ck.box.peer.agent.s.deliver",
            "m1",
            digest("m1"),
            Headers::new(),
        )
        .await
        .expect("publish first");
    let original = connect(&server.url, "UORIGINAL").await;
    let mut cursor = original
        .stream("CK_BOX_PEER")
        .consumer("c_agent", "agent")
        .await
        .expect("bind original cursor");
    let first = cursor
        .next()
        .await
        .expect("next first")
        .expect("first item");
    assert_eq!(first.message.id, "m1");
    cursor.ack().await.expect("ack first");
    drop(cursor);
    drop(original);

    publisher
        .publish(
            "ck.box.peer.agent.s.deliver",
            "m2",
            digest("m2"),
            Headers::new(),
        )
        .await
        .expect("publish second");
    let restarted = connect(&server.url, "UORIGINAL").await;
    let mut cursor = restarted
        .stream("CK_BOX_PEER")
        .consumer("c_agent", "agent")
        .await
        .expect("bind after restart");
    let second = cursor
        .next()
        .await
        .expect("next second")
        .expect("second item");
    assert_eq!(second.message.id, "m2");
    cursor.ack().await.expect("ack second");
    drop(cursor);
    drop(restarted);

    publisher
        .publish(
            "ck.box.peer.agent.s.deliver",
            "m3",
            digest("m3"),
            Headers::new(),
        )
        .await
        .expect("publish third");
    let rekeyed = connect(&server.url, "UREKEYED").await;
    let mut cursor = rekeyed
        .stream("CK_BOX_PEER")
        .consumer("c_agent", "agent")
        .await
        .expect("bind after re-key");
    let third = cursor
        .next()
        .await
        .expect("next third")
        .expect("third item");
    assert_eq!(third.message.id, "m3");
    assert!(third.stream_seq > second.stream_seq);
    cursor.ack().await.expect("ack third");
}

#[tokio::test]
async fn register_round_trips_absence_revisions_snapshot_and_updates() {
    let Some(server) = TestServer::open().await else {
        return;
    };
    let admin = connect(&server.url, ADMIN_PUBLIC).await;
    admin
        .jetstream()
        .create_key_value(kv::Config {
            bucket: "CK_BOX_CENSUS".into(),
            history: 1,
            ..Default::default()
        })
        .await
        .expect("create census bucket");
    let register = admin
        .register("CK_BOX_CENSUS")
        .await
        .expect("bind census bucket");
    assert!(matches!(
        register.get("missing").await,
        Err(BusError::Absent { .. })
    ));
    let first_revision = register
        .put("module.a", Vec::new())
        .await
        .expect("put empty value");
    let empty = register.get("module.a").await.expect("get empty value");
    assert!(empty.value.is_empty());
    assert_eq!(empty.revision, first_revision);

    let mut watch = register.watch("module.").await.expect("watch prefix");
    assert!(!watch.has_snapshot());
    let snapshot = watch.next().await.expect("initial snapshot");
    assert!(matches!(
        snapshot,
        WatchEvent::Snapshot { entries, .. }
            if entries.len() == 1 && entries[0].key == "module.a"
    ));
    assert!(watch.has_snapshot());

    let second_revision = register
        .put("module.a", b"updated".to_vec())
        .await
        .expect("update value");
    let update = tokio::time::timeout(Duration::from_secs(2), watch.next())
        .await
        .expect("watch update timeout")
        .expect("watch update");
    assert!(matches!(
        update,
        WatchEvent::Update(RegisterUpdate::Put(entry))
            if entry.value == b"updated" && entry.revision == second_revision
    ));
    let tombstone = register.delete("module.a").await.expect("delete value");
    assert!(tombstone.revision > second_revision);
    assert!(matches!(
        register.get("module.a").await,
        Err(BusError::Absent { .. })
    ));
}

#[tokio::test]
async fn workload_binding_never_creates_and_server_denials_name_subjects() {
    let Some(server) = TestServer::with_participant_permissions().await else {
        return;
    };
    let admin = connect_admin(&server.url).await;
    let workloads = [
        ("CK_BOX_ROOM", "ck.box.room.*.post", "ck.box.room.room.post"),
        (
            "CK_BOX_WAKE",
            "ck.box.wake.*.fire",
            "ck.box.wake.agent.fire",
        ),
        (
            "CK_BOX_PEER",
            "ck.box.peer.*.*.deliver",
            "ck.box.peer.agent.*.deliver",
        ),
        (
            "CK_BOX_EFFECT",
            "ck.box.effect.*.*.intent",
            "ck.box.effect.agent.*.intent",
        ),
    ];
    for (stream, subject, filter_subject) in workloads {
        create_stream_and_consumer(
            &admin,
            stream,
            subject,
            filter_subject,
            "c_agent",
            -1,
            Duration::from_secs(30),
        )
        .await;
    }

    let options =
        ConnectOptions::with_user_and_password("participant".into(), "participant".into())
            .request_timeout(Some(Duration::from_millis(300)));
    let participant = NatsConnection::connect(
        &server.url,
        options,
        ConnectConfig::new(PARTICIPANT_PUBLIC).expect("participant config"),
    )
    .await
    .expect("participant connects");
    for (stream, _, _) in workloads {
        participant
            .stream(stream)
            .consumer("c_agent", "agent")
            .await
            .expect("INFO-only bind succeeds");

        let create_subject = format!("$JS.API.CONSUMER.DURABLE.CREATE.{stream}.c_agent_other");
        let denied = participant
            .request(&create_subject, Bytes::from_static(b"{}"))
            .await
            .expect_err("participant consumer create must be denied by the server");
        assert!(matches!(
            denied,
            BusError::Denied { subject, .. } if subject == create_subject
        ));
    }

    let default_options =
        ConnectOptions::with_user_and_password("participant".into(), "participant".into())
            .request_timeout(Some(Duration::from_millis(300)));
    let default_client = NatsConnection::connect_with_library_default(
        &server.url,
        default_options,
        ConnectConfig::new(PARTICIPANT_PUBLIC).expect("participant config"),
    )
    .await
    .expect("default-prefix client connects before subscribe is checked");
    let default_inbox = default_client.client().new_inbox().to_string();
    assert!(!default_inbox.starts_with("_INBOX.UPARTICIPANT."));
    let denied = default_client
        .subscribe(&default_inbox)
        .await
        .expect_err("server must refuse the default inbox subscription");
    assert!(matches!(
        denied,
        BusError::Denied { subject, .. } if subject == default_inbox
    ));
}

#[tokio::test]
async fn dead_letter_precedes_term_and_crash_window_keeps_original_redeliverable() {
    let Some(server) = TestServer::open().await else {
        return;
    };
    let admin = connect(&server.url, ADMIN_PUBLIC).await;
    create_stream_and_consumer(
        &admin,
        "CK_BOX_EFFECT",
        "ck.box.effect.agent.*.intent",
        "ck.box.effect.agent.*.intent",
        "c_agent",
        5,
        Duration::from_millis(150),
    )
    .await;
    create_stream_and_consumer(
        &admin,
        "CK_BOX_EFFECT_DEAD",
        "ck.box.effect.dead",
        "ck.box.effect.dead",
        "c_ckbus_dead",
        -1,
        Duration::from_secs(30),
    )
    .await;
    let effect_stream = admin.stream("CK_BOX_EFFECT");
    effect_stream
        .publish(
            "ck.box.effect.agent.s.intent",
            "effect-1",
            digest("effect-1"),
            Headers::new(),
        )
        .await
        .expect("publish work item");

    let worker = connect(&server.url, "UWORKERONE").await;
    let queue = worker
        .work_queue("CK_BOX_EFFECT", "c_agent")
        .await
        .expect("bind work queue");
    // The consumer allows 5 deliveries; the claimant's cap is derived one below,
    // leaving the 5th as the spare that finishes a crashed dead-letter.
    assert_eq!(queue.max_deliveries(), 4);
    for expected in 1..4 {
        let ClaimOutcome::Item(item) = next_claim(&queue).await else {
            panic!("delivery {expected} should be a work item");
        };
        assert_eq!(item.delivery_count, expected);
        queue
            .nak(item.token, Duration::ZERO)
            .await
            .expect("request redelivery");
    }
    let ClaimOutcome::MaxDeliveriesExceeded(exhausted) = next_claim(&queue).await else {
        panic!("fourth delivery should report application cap exhaustion");
    };
    let record = DeadLetterRecord::from_exhaustion(&exhausted);
    let dead_stream = admin.stream("CK_BOX_EFFECT_DEAD");
    dead_stream
        .publish(
            "ck.box.effect.dead",
            &record.message_id,
            digest("dead-letter-record"),
            record.headers(),
        )
        .await
        .expect("dead-letter publish is confirmed before term");

    // Dropping both handles simulates a worker crash after the dead-letter publish but before term().
    drop(queue);
    drop(worker);
    sleep(Duration::from_millis(250)).await;

    let restarted = connect(&server.url, "UWORKERTWO").await;
    let queue = restarted
        .work_queue("CK_BOX_EFFECT", "c_agent")
        .await
        .expect("rebind queue after crash");
    let ClaimOutcome::MaxDeliveriesExceeded(redelivered) = next_claim(&queue).await else {
        panic!("unterminated item must be redelivered after the crash point");
    };
    assert_eq!(redelivered.item.message.id, "effect-1");

    let mut dead = admin
        .stream("CK_BOX_EFFECT_DEAD")
        .consumer("c_ckbus_dead", "ckbus_dead")
        .await
        .expect("bind dead-letter durable");
    let dead_record = dead
        .next()
        .await
        .expect("read dead letter")
        .expect("dead letter is present");
    assert_eq!(dead_record.message.id, "effect-1");
    assert_eq!(
        dead_record
            .message
            .headers
            .get("reason")
            .map(String::as_str),
        Some("max_deliveries_exceeded")
    );

    terminally_dispose(
        &dead_stream,
        &queue,
        "box",
        &redelivered,
        digest("dead-letter-record"),
    )
    .await
    .expect("confirmed dead-letter publish precedes term");
    dead.ack().await.expect("ack dead-letter record");
}

#[tokio::test]
async fn a_work_queue_without_a_spare_redelivery_is_refused() {
    let Some(server) = TestServer::open().await else {
        return;
    };
    let admin = connect(&server.url, ADMIN_PUBLIC).await;
    for (stream, durable, max_deliver) in [
        ("CK_BOX_UNBOUNDED", "c_unbounded", -1),
        ("CK_BOX_ONESHOT", "c_oneshot", 1),
    ] {
        let subject = format!("ck.box.{}.intent", durable);
        create_stream_and_consumer(
            &admin,
            stream,
            &subject,
            &subject,
            durable,
            max_deliver,
            Duration::from_secs(30),
        )
        .await;
        let refused = admin
            .work_queue(stream, durable)
            .await
            .err()
            .expect("a consumer with no spare redelivery must be refused");
        assert!(
            matches!(&refused, BusError::Denied { reason, .. } if reason.contains("at least 2")),
            "{refused:?}"
        );
    }
}

/// A claimant that terminates an item and exits at once must not lose the
/// term: `Ok` from `term()` means the server has it.
#[tokio::test]
async fn a_term_survives_the_claimant_exiting_straight_after_it() {
    let Some(server) = TestServer::open().await else {
        return;
    };
    let admin = connect(&server.url, ADMIN_PUBLIC).await;
    create_stream_and_consumer(
        &admin,
        "CK_BOX_EFFECT",
        "ck.box.effect.agent.*.intent",
        "ck.box.effect.agent.*.intent",
        "c_agent",
        5,
        Duration::from_millis(150),
    )
    .await;
    let effect_stream = admin.stream("CK_BOX_EFFECT");
    for index in 0..20 {
        let id = format!("effect-{index}");
        effect_stream
            .publish(
                "ck.box.effect.agent.s.intent",
                &id,
                digest(&id),
                Headers::new(),
            )
            .await
            .expect("publish work item");
    }

    // Each claimant takes one item, terminates it and exits without any flush.
    for index in 0..20 {
        let worker = connect(&server.url, &format!("UTERMWORKER{index}")).await;
        let queue = worker
            .work_queue("CK_BOX_EFFECT", "c_agent")
            .await
            .expect("bind work queue");
        let ClaimOutcome::Item(item) = next_claim(&queue).await else {
            panic!("item {index} should be claimable");
        };
        queue.term(item.token).await.expect("term");
        drop(queue);
        drop(worker);
    }

    // Past the ack wait, nothing may come back.
    sleep(Duration::from_millis(400)).await;
    let observer = connect(&server.url, "UTERMOBSERVER").await;
    let queue = observer
        .work_queue("CK_BOX_EFFECT", "c_agent")
        .await
        .expect("bind observer");
    match queue.claim().await.expect("claim") {
        ClaimOutcome::Empty => {}
        other => panic!("a terminated item came back: {other:?}"),
    }
}
