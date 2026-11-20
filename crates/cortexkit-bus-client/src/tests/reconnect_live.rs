//! Connection lifetime across bus outages, against a real nats-server.
//!
//! A TCP proxy stands between the client and the server. Taking it down cuts
//! every live session and closes each new one at once, which the client sees
//! exactly as a restarting server; bringing it back up restores the same
//! address. The proxy counts every TCP connection the client opens, so tests
//! count connection attempts where the server would see them.

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_nats::jetstream::{self, consumer::pull};
use cortexkit_bus_naming::AccountNames;
use cortexkit_bus_nats::NatsConnectionState;
use tokio::{
    net::{TcpListener, TcpStream},
    task::AbortHandle,
    time::Instant,
};

use super::buckets_live::Rig;
use crate::{EventDelivery, EventItem, EventReader};

struct Proxy {
    url: String,
    accepts: Arc<AtomicUsize>,
    /// `None` while down; otherwise the live sessions to cut on the next outage.
    sessions: Arc<Mutex<Option<Vec<AbortHandle>>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
        self.down();
    }
}

impl Proxy {
    async fn start(upstream_port: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("nats://{}", listener.local_addr().unwrap());
        let accepts = Arc::new(AtomicUsize::new(0));
        let sessions = Arc::new(Mutex::new(Some(Vec::new())));
        let task = tokio::spawn({
            let accepts = accepts.clone();
            let sessions = sessions.clone();
            async move {
                loop {
                    let (mut client, _) = listener.accept().await.unwrap();
                    accepts.fetch_add(1, Ordering::SeqCst);
                    // Checking and registering under one lock means an outage
                    // can't miss a session accepted while it starts.
                    let mut live = sessions.lock().unwrap();
                    let Some(live) = live.as_mut() else {
                        continue;
                    };
                    let session = tokio::spawn(async move {
                        if let Ok(mut server) =
                            TcpStream::connect(("127.0.0.1", upstream_port)).await
                        {
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                        }
                    });
                    live.push(session.abort_handle());
                }
            }
        });
        Self {
            url,
            accepts,
            sessions,
            task,
        }
    }

    fn down(&self) {
        for session in self.sessions.lock().unwrap().take().into_iter().flatten() {
            session.abort();
        }
    }

    fn up(&self) {
        self.sessions.lock().unwrap().get_or_insert_with(Vec::new);
    }

    fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }
}

/// Provisions the account's EVENT stream and the module's own durable.
async fn provision_events(rig: &Rig, module: &str) -> (AccountNames, jetstream::Context) {
    let names = AccountNames::derive("box_goldenfixture").unwrap();
    let context = jetstream::new(rig.admin.clone());
    context
        .create_stream(jetstream::stream::Config {
            name: names.streams().event.clone(),
            subjects: vec![format!("ck.{}.event.>", names.account())],
            ..Default::default()
        })
        .await
        .unwrap();
    context
        .get_stream(&names.streams().event)
        .await
        .unwrap()
        .create_consumer(pull::Config {
            durable_name: Some(AccountNames::module_consumer_name(module).unwrap()),
            ..Default::default()
        })
        .await
        .unwrap();
    (names, context)
}

async fn publish(names: &AccountNames, context: &jetstream::Context, msg_id: &str) {
    let mut headers = async_nats::HeaderMap::new();
    headers.insert(async_nats::header::NATS_MESSAGE_ID, msg_id);
    context
        .publish_with_headers(
            names.event_subject("aft", "fact.created", 1).unwrap(),
            headers,
            msg_id.to_owned().into(),
        )
        .await
        .unwrap()
        .await
        .unwrap();
}

async fn delivery(reader: &mut EventReader) -> EventDelivery {
    match tokio::time::timeout(Duration::from_secs(30), reader.next())
        .await
        .expect("reader timed out")
        .unwrap()
    {
        EventItem::Delivery(delivery) => delivery,
        EventItem::Malformed(malformed) => panic!("malformed: {:?}", malformed.reason),
    }
}

/// True once async-nats has ended the connection: its command channel is gone.
async fn ended(connection: &cortexkit_bus_nats::NatsConnection) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_millis(500), connection.client().flush()).await,
        Ok(Err(_))
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_connection_closes_promptly_and_the_waiting_reader_resumes() {
    let Some(rig) = Rig::start().await else {
        return;
    };
    let (names, context) = provision_events(&rig, "reader").await;
    let proxy = Proxy::start(rig.server.port).await;
    let (client, _manager, issuer) = rig.client_via("reader", &proxy.url).await;
    let mut reader = client.event_reader("m_reader").await.unwrap();
    publish(&names, &context, "before-revocation").await;
    let event = delivery(&mut reader).await;
    assert_eq!(event.msg_id, "before-revocation");
    event.ack().await.unwrap();
    let mut changes = client.connection_changes();
    changes.borrow_and_update();
    let old = client.connection();
    let old_key = old.inbox_prefix().strip_prefix("_INBOX.").unwrap();

    let next = delivery(&mut reader);
    tokio::pin!(next);
    assert!(futures_util::poll!(&mut next).is_pending());
    tokio::select! {
        () = async {
            rig.revoke(&issuer, old_key).await;
            tokio::time::timeout(Duration::from_secs(5), changes.changed())
                .await
                .expect("revocation prompts a swap without waiting for renewal")
                .unwrap();
        } => {}
        _ = &mut next => panic!("the reader returned before the replacement and publish"),
    }
    assert_ne!(client.connection().inbox_prefix(), old.inbox_prefix());
    assert!(
        ended(&old).await,
        "the revoked connection must already have ended at the swap"
    );
    let settled = proxy.accepts();
    publish(&names, &context, "after-revocation").await;
    let event = tokio::time::timeout(Duration::from_secs(5), next)
        .await
        .expect("the pending reader resumes on the replacement");
    assert_eq!(event.msg_id, "after-revocation");
    event.ack().await.unwrap();
    assert_eq!(
        proxy.accepts(),
        settled,
        "the revoked connection does not try again after the swap"
    );
    client.connection().client().flush().await.unwrap();
    assert!(!changes.has_changed().unwrap());
}

#[tokio::test]
async fn retired_connection_logs_expected_refusals_at_debug_and_live_auth_failure_stays_error() {
    let Some(rig) = Rig::start().await else {
        return;
    };

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
    let writer_output = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || Capture(writer_output.clone()))
        .finish();
    // Tokio's current-thread runtime keeps spawned reconnect and maintenance
    // tasks on the thread with this test's subscriber.
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let proxy = Proxy::start(rig.server.port).await;
    let (client, _, issuer) = rig.client_via("reader", &proxy.url).await;
    let mut changes = client.connection_changes();
    changes.borrow_and_update();
    let old = client.connection();
    let old_key = old.inbox_prefix().strip_prefix("_INBOX.").unwrap();
    rig.revoke(&issuer, old_key).await;
    tokio::time::timeout(Duration::from_secs(5), changes.changed())
        .await
        .expect("revoking the active credential prompts replacement")
        .unwrap();
    let live = client.connection();
    assert_ne!(live.inbox_prefix(), old.inbox_prefix());

    // Refuse one nonce on the new, still-current connection. It must log the
    // error and then reconnect without replacing the credential.
    let mut state = live.connection_state();
    proxy.down();
    tokio::time::timeout(
        Duration::from_secs(5),
        state.wait_for(|state| *state == NatsConnectionState::Disconnected),
    )
    .await
    .expect("the live session disconnects")
    .unwrap();
    issuer.refuse_next_sign.store(true, Ordering::SeqCst);
    proxy.up();
    tokio::time::timeout(
        Duration::from_secs(5),
        state.wait_for(|state| *state == NatsConnectionState::Connected),
    )
    .await
    .expect("the live session reconnects after one signing refusal")
    .unwrap();
    assert!(!changes.has_changed().unwrap());

    let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    let retirements: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("retiring bus connection for replacement"))
        .collect();
    assert_eq!(retirements.len(), 1, "expected one retirement record");
    let retirement = retirements[0];
    assert!(retirement.trim_start().starts_with("INFO "), "{retirement}");
    assert!(retirement.contains("errors for it are expected"));
    assert!(retirement.contains(&format!("server={}", proxy.url)));
    let retired_id = retirement
        .split_whitespace()
        .find(|field| field.starts_with("connection_id="))
        .expect("the retirement record identifies the old connection");

    let client_refusals: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("retired bus connection refused authentication"))
        .collect();
    assert!(
        !client_refusals.is_empty(),
        "expected a retired auth refusal"
    );
    for refusal in client_refusals {
        assert!(refusal.starts_with("DEBUG "), "{refusal}");
        assert!(refusal.split_whitespace().any(|field| field == retired_id));
    }
    assert!(logs.lines().any(|line| {
        line.starts_with("ERROR ")
            && line.contains("auth callback failed")
            && line.contains("test_sign_refused")
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn short_outage_keeps_the_same_connection() {
    let Some(rig) = Rig::start().await else {
        return;
    };
    let (names, context) = provision_events(&rig, "occipital").await;
    let proxy = Proxy::start(rig.server.port).await;
    let (client, _manager, _issuer) = rig.client_via("occipital", &proxy.url).await;
    let mut reader = client.event_reader("m_occipital").await.unwrap();
    let mut changes = client.connection_changes();
    changes.borrow_and_update();
    let before = client.connection();

    proxy.down();
    tokio::time::sleep(Duration::from_secs(20)).await;
    proxy.up();
    // A flush completes only once the same connection has reconnected.
    tokio::time::timeout(Duration::from_secs(15), before.client().flush())
        .await
        .expect("the connection reconnects after a 20 s outage")
        .unwrap();
    assert!(
        !changes.has_changed().unwrap(),
        "an outage shorter than the bound must not replace the connection"
    );
    publish(&names, &context, "after-short-outage").await;
    let event = delivery(&mut reader).await;
    assert_eq!(event.msg_id, "after-short-outage");
    event.ack().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ended_connection_is_replaced_during_a_pending_renewal() {
    let Some(rig) = Rig::start().await else {
        return;
    };
    let (names, context) = provision_events(&rig, "reader").await;
    let proxy = Proxy::start(rig.server.port).await;
    let (client, renewal) = rig.client_with_pending_renewal("reader", &proxy.url).await;
    tokio::time::timeout(Duration::from_secs(5), renewal.started.notified())
        .await
        .expect("renewal is pending before the outage");
    let mut reader = client.event_reader("m_reader").await.unwrap();
    publish(&names, &context, "before-outage").await;
    let event = delivery(&mut reader).await;
    assert_eq!(event.msg_id, "before-outage");
    event.ack().await.unwrap();
    let mut changes = client.connection_changes();
    changes.borrow_and_update();
    let before = client.connection();

    // The reader is mid-pull for the whole outage; it must neither fail while
    // the connection is gone nor stay bound to the dead one.
    let pending = tokio::spawn(async move {
        let event = match reader.next().await.unwrap() {
            EventItem::Delivery(delivery) => delivery,
            EventItem::Malformed(malformed) => panic!("malformed: {:?}", malformed.reason),
        };
        (reader, event)
    });
    let outage = Instant::now();
    proxy.down();
    tokio::time::timeout(Duration::from_secs(60), async {
        while !ended(&before).await {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("the connection ends after its reconnect bound");
    let lasted = outage.elapsed();
    assert!(
        lasted >= Duration::from_secs(30),
        "the connection rides through about 30 s before ending, ended after {lasted:?}"
    );
    // Stay down a little longer so the replacement's first attempts fail.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!pending.is_finished(), "the reader waits for a replacement");
    proxy.up();
    tokio::time::timeout(Duration::from_secs(5), changes.changed())
        .await
        .expect("the client replaces the ended connection without waiting for renewal")
        .unwrap();
    renewal.release.notify_one();
    client.connection().client().flush().await.unwrap();

    publish(&names, &context, "after-outage").await;
    let (_reader, event) = tokio::time::timeout(Duration::from_secs(40), pending)
        .await
        .expect("the reader resumes on the replacement")
        .unwrap();
    assert_eq!(event.msg_id, "after-outage");
    event.ack().await.unwrap();
}
