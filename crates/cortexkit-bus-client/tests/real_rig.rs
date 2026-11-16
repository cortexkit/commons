use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use async_nats::jetstream::{self, consumer::pull};
use cortexkit_bus_naming::AccountNames;
use cortexkit_bus_testkit::{BusOptions, BusServer, DaemonOptions, Module, PrivateDaemon};
use futures_util::StreamExt;
use serde_json::Value;

const LIMIT: Duration = Duration::from_secs(60);

fn binary(name: &str) -> PathBuf {
    let path =
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set explicit {name}")));
    assert!(
        path.is_absolute() && path.is_file(),
        "{name} must be an absolute binary path"
    );
    path
}

async fn wait_file(path: &Path, daemon: &PrivateDaemon) {
    tokio::time::timeout(LIMIT, async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{} never appeared: {}", path.display(), daemon.log_text()));
}

/// Run after building the fixture binaries and `bus-client-probe` example.
/// Set `ck_subc`, `ck_bus` and `probe` to their absolute Cargo compiler-artifact
/// executable paths, then run the same command used to verify this test:
/// `CK_SUBC_BIN="$ck_subc" CK_BUS_BIN="$ck_bus" CK_NATS_SERVER_BIN="$PWD/bus-rig-tools/nats-server-v2.15.0-linux-amd64/nats-server" CK_BUS_CLIENT_PROBE_BIN="$probe" cargo test --locked -p cortexkit-bus-client --test real_rig real_bus_event_durable_survives_forced_reissue -- --ignored --exact`
/// The testkit isolates XDG homes on children, never in the test process.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit CK_SUBC_BIN, CK_BUS_BIN, CK_NATS_SERVER_BIN and CK_BUS_CLIENT_PROBE_BIN"]
async fn real_bus_event_durable_survives_forced_reissue() {
    // Refuse the testkit's ambient binary search: this test must never start an
    // installed daemon or use the operator's service or data directories.
    binary("CK_SUBC_BIN");
    binary("CK_BUS_BIN");
    let nats = binary("CK_NATS_SERVER_BIN");
    let probe = binary("CK_BUS_CLIENT_PROBE_BIN");
    let result = tempfile::tempdir().unwrap();
    let mut module = Module::new("bus_client_probe", probe);
    module.env.insert(
        "BUS_CLIENT_RESULT".into(),
        result.path().display().to_string(),
    );
    let bus = BusServer::start(BusOptions {
        binary: Some(nats),
        ..Default::default()
    })
    .await
    .unwrap();
    let daemon = PrivateDaemon::start(
        &bus,
        DaemonOptions {
            modules: vec![module],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    wait_file(&result.path().join("ready.json"), &daemon).await;
    let ready: Value =
        serde_json::from_slice(&std::fs::read(result.path().join("ready.json")).unwrap()).unwrap();
    let names = AccountNames::derive(ready["acct"].as_str().unwrap()).unwrap();
    let accounts: Vec<_> = bus
        .stored_accounts()
        .into_iter()
        .filter(|account| account != &bus.system_account)
        .collect();
    assert_eq!(
        accounts.len(),
        1,
        "only the fixture's box account is present"
    );
    let observer_key = nkeys::KeyPair::new_user();
    let jwt = bus.trust.user_jwt(
        &cortexkit_bus_testkit::trust::box_root_id(),
        &accounts[0],
        &observer_key,
        cortexkit_bus_testkit::trust::unix_now() - 60,
    );
    let observer = async_nats::ConnectOptions::with_jwt(jwt, move |nonce| {
        let signature = observer_key.sign(&nonce);
        async move { signature.map_err(async_nats::AuthError::new) }
    })
    .connect(&bus.url)
    .await
    .unwrap();
    let js = jetstream::new(observer);
    let stream = js.get_stream(&names.streams().event).await.unwrap();
    let durable = AccountNames::module_consumer_name("bus_client_reader").unwrap();
    let consumer: jetstream::consumer::Consumer<pull::Config> = stream
        .create_consumer(pull::Config {
            durable_name: Some(durable.clone()),
            filter_subject: names.event_subject("bus_client_probe", "smoke", 1).unwrap(),
            ..Default::default()
        })
        .await
        .unwrap();
    std::fs::write(result.path().join("publish-now"), b"ready").unwrap();
    wait_file(&result.path().join("published"), &daemon).await;
    let mut batch = consumer.fetch().max_messages(1).messages().await.unwrap();
    let message = tokio::time::timeout(LIMIT, batch.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(message.payload.as_ref(), b"before reissue");
    message.double_ack().await.unwrap();
    drop(batch);
    std::fs::write(result.path().join("reissue-now"), b"ready").unwrap();
    wait_file(&result.path().join("reissued"), &daemon).await;
    let reopened: jetstream::consumer::Consumer<pull::Config> =
        stream.get_consumer(&durable).await.unwrap();
    let mut batch = reopened.fetch().max_messages(1).messages().await.unwrap();
    let message = tokio::time::timeout(LIMIT, batch.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        message.payload.as_ref(),
        b"after reissue",
        "the durable resumes after its acknowledged fact"
    );
    message.double_ack().await.unwrap();
}
