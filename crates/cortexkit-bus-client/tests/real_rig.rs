use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use async_nats::jetstream::{self, consumer::pull};
use cortexkit_bus_naming::{AccountNames, EVENT_CONSUMERS};
use cortexkit_bus_testkit::{BusOptions, BusServer, DaemonOptions, Module, PrivateDaemon};
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
/// `CK_SUBC_BIN="$ck_subc" CK_BUS_BIN="$ck_bus" CK_NATS_SERVER_BIN="$PWD/bus-rig-tools/nats/nats-server-v2.15.0-linux-amd64/nats-server" CK_BUS_CLIENT_PROBE_BIN="$probe" cargo test --locked -p cortexkit-bus-client --test real_rig real_bus_event_durable_survives_forced_reissue -- --ignored --exact`
/// The testkit isolates XDG homes on children, never in the test process.
/// Only modules in the closed EVENT_CONSUMERS table may read EVENT, so the
/// probes borrow declared identities: thalamus-harness publishes step.finished.v1,
/// and astrocyte reads m_astrocyte. The released ck-bus uses cortexkit-bus-naming
/// 0.3.4, whose EVENT_CONSUMERS table already authorizes that durable and event.
/// Using this pre-0.4.0 durable keeps the fixture compatible with the released
/// bus. The test's administrative NATS connection provisions the declared
/// filters before the reader attaches. Re-issue replaces the reader's credential
/// without changing production grants or filters.
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
    let modules = ["thalamus-harness", "astrocyte"].map(|id| {
        let mut module = Module::new(id, probe.clone());
        module.env.insert(
            "BUS_CLIENT_RESULT".into(),
            result.path().display().to_string(),
        );
        module
    });
    let bus = BusServer::start(BusOptions {
        binary: Some(nats),
        ..Default::default()
    })
    .await
    .unwrap();
    let daemon = PrivateDaemon::start(
        &bus,
        DaemonOptions {
            modules: modules.into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    wait_file(&result.path().join("ready-thalamus-harness.json"), &daemon).await;
    wait_file(&result.path().join("ready-astrocyte.json"), &daemon).await;
    let ready: Value =
        serde_json::from_slice(&std::fs::read(result.path().join("ready-astrocyte.json")).unwrap())
            .unwrap();
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
    let durable = "m_astrocyte";
    let declaration = EVENT_CONSUMERS
        .iter()
        .find(|entry| entry.durable == durable)
        .unwrap();
    assert!(declaration
        .filter_subjects
        .contains(&"event.thalamus-harness.step.finished.v1"));
    let mut consumer: jetstream::consumer::Consumer<pull::Config> = stream
        .create_consumer(pull::Config {
            durable_name: Some(durable.into()),
            filter_subjects: declaration
                .filter_subjects
                .iter()
                .map(|filter| format!("ck.{}.{filter}", names.account()))
                .collect(),
            ..Default::default()
        })
        .await
        .unwrap();
    std::fs::write(result.path().join("read-now"), b"ready").unwrap();
    std::fs::write(result.path().join("publish-now"), b"ready").unwrap();
    wait_file(&result.path().join("read-before"), &daemon).await;
    assert_eq!(consumer.info().await.unwrap().ack_floor.stream_sequence, 1);
    std::fs::write(result.path().join("reissue-now"), b"ready").unwrap();
    wait_file(&result.path().join("reissued"), &daemon).await;
    std::fs::write(result.path().join("publish-after-reissue"), b"ready").unwrap();
    wait_file(&result.path().join("read-after"), &daemon).await;
    assert_eq!(
        consumer.info().await.unwrap().ack_floor.stream_sequence,
        2,
        "the module reader read and acknowledged both bodies across reissue"
    );
}
