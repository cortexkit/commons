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
use tokio::{
    net::{TcpListener, TcpStream},
    task::AbortHandle,
    time::Instant,
};

use super::buckets_live::Rig;
use crate::{EventDelivery, EventItem, EventReader};

/// async-nats gives up after the client's reconnect bound, about 36 s after
/// the connection drops. Waiting this long leaves room for scheduling delays.
const PAST_THE_BOUND: Duration = Duration::from_secs(45);

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
async fn replaced_connection_stops_reconnecting_within_the_bound() {
    let Some(rig) = Rig::start().await else {
        return;
    };
    let proxy = Proxy::start(rig.server.port).await;
    let (client, _manager, issuer) = rig.client_via("occipital", &proxy.url).await;
    let mut changes = client.connection_changes();
    changes.borrow_and_update();
    let old = client.connection();
    let old_key = old
        .inbox_prefix()
        .strip_prefix("_INBOX.")
        .unwrap()
        .to_owned();

    // A ck-bus restart: every connection drops and the old key is revoked, so
    // the old connection's reconnect makes the client re-issue and replace it.
    issuer.revoked.lock().unwrap().push(old_key);
    let outage = Instant::now();
    proxy.down();
    tokio::time::sleep(Duration::from_secs(1)).await;
    proxy.up();
    tokio::time::timeout(Duration::from_secs(20), changes.changed())
        .await
        .expect("the client replaces the revoked connection")
        .unwrap();
    assert_ne!(client.connection().inbox_prefix(), old.inbox_prefix());
    let at_replacement = proxy.accepts();

    tokio::time::sleep_until(outage + PAST_THE_BOUND).await;
    let settled = proxy.accepts();
    assert!(
        settled > at_replacement,
        "the replaced connection kept trying after the swap, until its bound"
    );
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        proxy.accepts(),
        settled,
        "the replaced connection still opens connections past its reconnect bound"
    );
    assert!(ended(&old).await, "the replaced connection has ended");
    // The replacement is unaffected and was not replaced again.
    client.connection().client().flush().await.unwrap();
    assert!(!changes.has_changed().unwrap());
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
async fn long_outage_replaces_the_connection_and_the_reader_resumes() {
    let Some(rig) = Rig::start().await else {
        return;
    };
    let (names, context) = provision_events(&rig, "occipital").await;
    let proxy = Proxy::start(rig.server.port).await;
    let (client, _manager, _issuer) = rig.client_via("occipital", &proxy.url).await;
    let mut reader = client.event_reader("m_occipital").await.unwrap();
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
    tokio::time::timeout(Duration::from_secs(60), changes.changed())
        .await
        .expect("the client replaces the ended connection")
        .unwrap();
    client.connection().client().flush().await.unwrap();

    publish(&names, &context, "after-outage").await;
    let (_reader, event) = tokio::time::timeout(Duration::from_secs(40), pending)
        .await
        .expect("the reader resumes on the replacement")
        .unwrap();
    assert_eq!(event.msg_id, "after-outage");
    event.ack().await.unwrap();
}
