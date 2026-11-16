//! A supervised module used only by the opt-in private-rig integration test.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use cortexkit_bus_client::{BusClient, BusClientOptions, EventItem};
use serde_json::json;
use subc_client_rs::{HandlerOutcome, ModuleHandler, RequestCtx, SubcConsumer};
use subc_protocol::{
    manifest::{
        Concurrency, ManagementOperation, ManagementOperationKind, ModuleManifest, ProviderRole,
    },
    BindIdentity, RouteTarget,
};

struct Idle;

#[async_trait]
impl ModuleHandler for Idle {
    async fn handle(&self, _ctx: RequestCtx, _body: Vec<u8>) -> HandlerOutcome {
        HandlerOutcome::Response(b"{}".to_vec())
    }
}

async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("integration test signal");
}

fn main() {
    subc_client_rs::launch_nonce().unwrap();
    let mut args = std::env::args_os().skip(1);
    let connection = loop {
        match args.next() {
            Some(arg) if arg == "--subc" => break PathBuf::from(args.next().expect("--subc path")),
            Some(_) => {}
            None => panic!("daemon must pass --subc"),
        }
    };
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            let module = std::env::var("SUBC_MODULE_ID").unwrap();
            let result = PathBuf::from(std::env::var_os("BUS_CLIENT_RESULT").unwrap());
            let manifest = ModuleManifest::builder(&module, "0.0.0-test")
                .provides(vec![ProviderRole::ManagementSurface {
                    operations: vec![ManagementOperation {
                        name: "probe.ping".into(),
                        kind: ManagementOperationKind::Query,
                        description: None,
                    }],
                    config_schema: json!({}),
                    observability: vec![],
                    identity_scope: vec![],
                    concurrency: Concurrency::ModuleManaged,
                }])
                .build();
            let (_handle, serving) = subc_client_rs::serve_with_handle(&connection, manifest, Idle)
                .await
                .unwrap();
            let serving = tokio::spawn(serving);
            let consumer = Arc::new(
                SubcConsumer::connect(&connection, Default::default())
                    .await
                    .unwrap(),
            );
            let bus = BusClient::connect(consumer.clone(), BusClientOptions::new(&module).unwrap())
                .await
                .unwrap();
            std::fs::write(
                result.join(format!("ready-{module}.json")),
                serde_json::to_vec(
                    &json!({"acct": bus.acct(), "inbox": bus.connection().inbox_prefix()}),
                )
                .unwrap(),
            )
            .unwrap();
            if module == "thalamus-harness" {
                for (signal, msg_id, body) in [
                    ("publish-now", "fact-before", "before reissue"),
                    ("publish-after-reissue", "fact-after", "after reissue"),
                ] {
                    wait_file(&result.join(signal)).await;
                    let receipt = bus
                        .publish_event("step.finished", 1, None, msg_id, body)
                        .await
                        .unwrap();
                    assert!(!receipt.duplicate());
                }
                serving.await.unwrap().unwrap();
                return;
            }
            assert_eq!(module, "astrocyte");
            wait_file(&result.join("read-now")).await;
            let mut reader = bus.event_reader("m_astrocyte").await.unwrap();
            let EventItem::Delivery(delivery) = reader.next().await.unwrap() else {
                panic!("published event must be valid");
            };
            assert_eq!(delivery.body.as_ref(), b"before reissue");
            assert_eq!(delivery.msg_id, "fact-before");
            delivery.ack().await.unwrap();
            std::fs::write(result.join("read-before"), b"ready").unwrap();
            wait_file(&result.join("reissue-now")).await;
            let mut changes = bus.connection_changes();
            let old_inbox = bus.connection().inbox_prefix().to_owned();
            // An independent issue supersedes the client's key, exercising the
            // production nonce-sign refusal path rather than an internal test hook.
            consumer
                .call(
                    RouteTarget::ManagementSurface {
                        module_id: "ckbus".into(),
                    },
                    BindIdentity::new("bus-client", "probe"),
                    serde_json::to_vec(&json!({"method": "ckbus.credential", "params": {}}))
                        .unwrap(),
                    subc_client_rs::CallOptions {
                        timeout: Duration::from_secs(5),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            bus.connection().client().force_reconnect().await.unwrap();
            tokio::time::timeout(Duration::from_secs(30), changes.changed())
                .await
                .unwrap()
                .unwrap();
            assert_ne!(old_inbox, bus.connection().inbox_prefix());
            std::fs::write(result.join("reissued"), b"ready").unwrap();
            let EventItem::Delivery(delivery) = reader.next().await.unwrap() else {
                panic!("published event must be valid after reissue");
            };
            assert_eq!(delivery.body.as_ref(), b"after reissue");
            assert_eq!(delivery.msg_id, "fact-after");
            delivery.ack().await.unwrap();
            std::fs::write(result.join("read-after"), b"ready").unwrap();
            serving.await.unwrap().unwrap();
        });
}
