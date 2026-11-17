use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use async_trait::async_trait;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use cortexkit_bus_naming::federation::{module_deliver_prefix, BoxId};
use cortexkit_bus_naming::{AccountNames, MODULE_BUCKETS};
use cortexkit_bus_testkit::{
    trust::{self, TrustChain},
    BusOptions, BusServer, PreloadedAccount,
};
use futures_util::StreamExt;
use nkeys::KeyPair;
use serde_json::{json, Value};

use crate::{
    credentials::{CredentialManager, CredentialTransport},
    BucketItem, BucketWatch, BusClient, BusClientOptions, Error, Result,
};

fn jwt(signer: &KeyPair, subject: &str, name: &str, nats: Value) -> String {
    let now = trust::unix_now();
    let claims = json!({"jti":format!("{subject}-{now}"), "iat":now,
        "iss":signer.public_key(), "sub":subject, "name":name, "nats":nats});
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"ed25519-nkey","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(signer.sign(input.as_bytes()).unwrap())
    )
}

pub(super) struct Issuer {
    url: String,
    account: String,
    acct: &'static str,
    inbox_prefix: Option<String>,
    trust: TrustChain,
    module: &'static str,
    keys: Mutex<Vec<KeyPair>>,
    /// Keys whose nonce signing is refused as revoked, as after a ck-bus restart.
    pub(super) revoked: Mutex<Vec<String>>,
    signing: tokio::sync::Mutex<()>,
    sign_calls: AtomicUsize,
}

impl Issuer {
    fn credential(&self, key: &KeyPair) -> Value {
        let names = AccountNames::derive(self.acct).unwrap();
        let public = key.public_key();
        let inbox_prefix = self
            .inbox_prefix
            .clone()
            .unwrap_or_else(|| format!("_INBOX.{public}"));
        // These are the module KV grant subjects emitted by ck-bus, with only
        // the issued inbox substituted. They allow unnamed consumer
        // creation but neither named consumer creation nor consumer deletion.
        let mut publish = Vec::new();
        for owner in ["aft", "cerebellum"] {
            if self.module != "occipital" && self.module != owner {
                continue;
            }
            let bucket = names.module_bucket_name(owner, "views").unwrap();
            let stream = format!("KV_{bucket}");
            publish.extend([
                format!("$JS.API.CONSUMER.CREATE.{stream}"),
                format!("$JS.API.DIRECT.GET.{stream}.>"),
                format!("$JS.API.STREAM.INFO.{stream}"),
                format!("$JS.API.STREAM.MSG.GET.{stream}"),
                format!("$JS.FC.{stream}.>"),
            ]);
            if self.module == owner {
                publish.push(format!("$KV.{bucket}.>"));
            }
        }
        // A module may read its own EVENT durable.
        let event = &names.streams().event;
        let durable = AccountNames::module_consumer_name(self.module).unwrap();
        publish.extend([
            format!("$JS.API.CONSUMER.INFO.{event}.{durable}"),
            format!("$JS.API.CONSUMER.MSG.NEXT.{event}.{durable}"),
            format!("$JS.ACK.{event}.{durable}.>"),
        ]);
        let now = trust::unix_now();
        let signer = &self.trust.signer.root(&trust::box_root_id()).pair;
        let jwt = jwt(
            signer,
            &public,
            "bucket-fixture-user",
            json!({"type":"user", "version":2, "issuer_account":self.account,
                "pub":{"allow": publish}, "sub":{"allow":[format!("{inbox_prefix}.>")]},
                "subs":-1, "data":-1, "payload":-1}),
        );
        // The credential envelope is the same shape returned by the bus. Its
        // test-only signer holds throwaway keys rather than a module seed.
        json!({"jwt":jwt, "exp":now + 900,
            "acct":names.account(), "credential_public":public,
            "server_url":self.url, "inbox_prefix":inbox_prefix,
            "spawn_generation":1, "credential_epoch":0, "user_jwt_id":"fixture"})
    }
}

#[async_trait]
impl CredentialTransport for Issuer {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let _signing = if method == "ckbus.nonce_sign" {
            self.sign_calls.fetch_add(1, Ordering::SeqCst);
            Some(self.signing.lock().await)
        } else {
            None
        };
        let mut keys = self.keys.lock().unwrap();
        match method {
            "ckbus.credential" => {
                keys.push(KeyPair::new_user());
                Ok(self.credential(keys.last().unwrap()))
            }
            "ckbus.credential_renew" => Ok(self.credential(keys.last().unwrap())),
            "ckbus.nonce_sign" => {
                let public = params["credential_public"].as_str().unwrap();
                if self.revoked.lock().unwrap().iter().any(|key| key == public) {
                    return Err(Error::Refused("ckbus_credential_revoked".into()));
                }
                let key = keys.iter().find(|key| key.public_key() == public).unwrap();
                let nonce = STANDARD
                    .decode(params["nonce_b64"].as_str().unwrap())
                    .unwrap();
                Ok(json!({"signature_b64":STANDARD.encode(key.sign(&nonce).unwrap())}))
            }
            _ => panic!("unexpected method {method}"),
        }
    }
}

pub(super) struct Rig {
    pub(super) server: BusServer,
    account: String,
    acct: &'static str,
    pub(super) admin: async_nats::Client,
}

impl Rig {
    pub(super) async fn start() -> Option<Self> {
        Self::start_for("box_goldenfixture").await
    }

    async fn start_for(acct: &'static str) -> Option<Self> {
        let Some(binary) = std::env::var_os("CK_NATS_SERVER_BIN") else {
            assert_ne!(
                std::env::var("CK_REQUIRE_NATS_SERVER").as_deref(),
                Ok("1"),
                "NATS binary required"
            );
            eprintln!("nats-server-absent: bucket live test skipped");
            return None;
        };
        let trust = TrustChain::generate();
        let account = KeyPair::new_account().public_key();
        let server = BusServer::start(BusOptions {
            binary: Some(binary.into()),
            preload: vec![PreloadedAccount {
                public_key: account.clone(),
                signing_keys: vec![trust.box_root_public()],
            }],
            trust: Some(trust),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(
            server.version.contains("2.15.0"),
            "pinned broker: {}",
            server.version
        );
        // The testkit's preloaded accounts have no JetStream budget. Update
        // this throwaway account through the same system claim API as bootstrap.
        let system_key = KeyPair::new_user();
        let system_jwt = server.trust.user_jwt(
            &trust::system_root_id(),
            &server.system_account,
            &system_key,
            trust::unix_now() - 60,
        );
        let system = async_nats::ConnectOptions::with_jwt(system_jwt, move |nonce| {
            let signature = system_key.sign(&nonce);
            async move { signature.map_err(async_nats::AuthError::new) }
        })
        .connect(&server.url)
        .await
        .unwrap();
        let account_jwt = jwt(
            &server.trust.signer.root(&trust::signer_root_id()).pair,
            &account,
            "bucket-fixture-account",
            json!({"type":"account", "version":2,
                "limits":{"subs":-1,"data":-1,"payload":-1,"imports":-1,"exports":-1,
                    "wildcards":true,"conn":-1,"leaf":-1,"mem_storage":104857600,
                    "disk_storage":104857600,"streams":-1,"consumer":-1},
                "signing_keys":[server.trust.box_root_public()]}),
        );
        let update = system
            .request("$SYS.REQ.CLAIMS.UPDATE", account_jwt.into())
            .await
            .unwrap();
        let update: Value = serde_json::from_slice(&update.payload).unwrap();
        assert!(
            update.get("error").is_none(),
            "account claim update: {update}"
        );
        let key = KeyPair::new_user();
        let jwt = server.trust.user_jwt(
            &trust::box_root_id(),
            &account,
            &key,
            trust::unix_now() - 60,
        );
        let admin = async_nats::ConnectOptions::with_jwt(jwt, move |nonce| {
            let signature = key.sign(&nonce);
            async move { signature.map_err(async_nats::AuthError::new) }
        })
        .connect(&server.url)
        .await
        .unwrap();
        let context = async_nats::jetstream::new(admin.clone());
        let names = AccountNames::derive(acct).unwrap();
        for declaration in MODULE_BUCKETS.iter().filter(|entry| entry.name == "views") {
            // Match bus provisioning, including the shared cap of 16 watchers.
            let bucket = names
                .module_bucket_name(declaration.owner, declaration.name)
                .unwrap();
            context
                .create_key_value(async_nats::jetstream::kv::Config {
                    bucket: bucket.clone(),
                    history: i64::from(declaration.history),
                    max_value_size: i32::try_from(declaration.max_value_bytes).unwrap(),
                    max_bytes: i64::try_from(declaration.max_bytes).unwrap(),
                    max_age: Duration::from_secs(declaration.ttl.unwrap_or(0)),
                    ..Default::default()
                })
                .await
                .unwrap();
            let mut stream = context.get_stream(format!("KV_{bucket}")).await.unwrap();
            let mut config = stream.info().await.unwrap().config.clone();
            config.max_consumers = 16;
            context.update_stream(config).await.unwrap();
        }
        Some(Self {
            server,
            account,
            acct,
            admin,
        })
    }

    async fn client(&self, module: &'static str) -> (BusClient, Arc<CredentialManager>) {
        let (client, manager, _) = self.gated_client(module).await;
        (client, manager)
    }

    async fn gated_client(
        &self,
        module: &'static str,
    ) -> (BusClient, Arc<CredentialManager>, Arc<Issuer>) {
        self.client_via(module, &self.server.url).await
    }

    /// Connects a client whose credentials point at `url`, which may be a
    /// proxy in front of this rig's server.
    pub(super) async fn client_via(
        &self,
        module: &'static str,
        url: &str,
    ) -> (BusClient, Arc<CredentialManager>, Arc<Issuer>) {
        self.client_with_prefix(module, url, None).await
    }

    async fn client_with_prefix(
        &self,
        module: &'static str,
        url: &str,
        inbox_prefix: Option<String>,
    ) -> (BusClient, Arc<CredentialManager>, Arc<Issuer>) {
        let issuer = Arc::new(Issuer {
            url: url.into(),
            account: self.account.clone(),
            acct: self.acct,
            inbox_prefix,
            trust: self.server.trust.clone(),
            module,
            keys: Mutex::new(vec![]),
            revoked: Mutex::new(vec![]),
            signing: tokio::sync::Mutex::new(()),
            sign_calls: AtomicUsize::new(0),
        });
        let manager = CredentialManager::start(issuer.clone()).await.unwrap();
        let client =
            BusClient::connect_manager(manager.clone(), BusClientOptions::new(module).unwrap())
                .await
                .unwrap();
        (client, manager, issuer)
    }
}

async fn next(watch: &mut BucketWatch) -> BucketItem {
    tokio::time::timeout(Duration::from_secs(10), watch.next())
        .await
        .expect("watch timed out")
        .unwrap()
}

#[tokio::test]
async fn inbox_prefix_live_deliver_request_and_kv_watch() {
    const BOX: &str = "0123456789abcdef0123456789abcdef";
    let Some(rig) = Rig::start_for("box_0123456789abcdef0123456789abcdef").await else {
        return;
    };
    let (owner, _) = rig.client("aft").await;
    let prefix = module_deliver_prefix(&BoxId::parse(BOX).unwrap(), "occipital").unwrap();
    let (reader, manager, _) = rig
        .client_with_prefix("occipital", &rig.server.url, Some(prefix.clone()))
        .await;
    assert_eq!(reader.box_id().unwrap().as_str(), BOX);
    assert_eq!(reader.inbox_prefix(), prefix);
    let mut deliveries = rig.admin.subscribe(format!("{prefix}.>")).await.unwrap();
    rig.admin.flush().await.unwrap();

    let bucket_name = AccountNames::derive(reader.acct())
        .unwrap()
        .module_bucket_name("aft", "views")
        .unwrap();
    let reply = reader
        .connection()
        .client()
        .request(format!("$JS.API.STREAM.INFO.KV_{bucket_name}"), "".into())
        .await
        .unwrap();
    let info: Value = serde_json::from_slice(&reply.payload).unwrap();
    assert_eq!(info["config"]["name"], format!("KV_{bucket_name}"));
    let observed = tokio::time::timeout(Duration::from_secs(10), deliveries.next())
        .await
        .unwrap()
        .unwrap();
    assert!(observed.subject.starts_with(&format!("{prefix}.")));
    assert_eq!(observed.payload, reply.payload);

    let mut watch = reader
        .bucket("aft", "views")
        .unwrap()
        .watch()
        .await
        .unwrap();
    assert_eq!(next(&mut watch).await, BucketItem::CaughtUp);
    let consumers = rig
        .admin
        .request(
            format!("$JS.API.CONSUMER.LIST.KV_{bucket_name}"),
            "{}".into(),
        )
        .await
        .unwrap();
    let consumers: Value = serde_json::from_slice(&consumers.payload).unwrap();
    let consumers = consumers["consumers"].as_array().unwrap();
    assert_eq!(consumers.len(), 1);
    assert!(consumers[0]["config"]["deliver_subject"]
        .as_str()
        .unwrap()
        .starts_with(&format!("{prefix}.")));

    let revision = owner
        .bucket("aft", "views")
        .unwrap()
        .put("key", "hub-value")
        .await
        .unwrap();
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Put {
            key: "key".into(),
            revision,
            value: "hub-value".into(),
        }
    );
    // The independent subscription observes the pushed value on the exported
    // prefix as well as the watch consuming it through its own subscription.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = deliveries.next().await.unwrap();
            if message.payload == "hub-value" {
                break;
            }
        }
    })
    .await
    .unwrap();

    manager.renew().await.unwrap();
    let mut changes = reader.connection_changes();
    let public = manager.credential().await.credential_public;
    manager.reissue(&public).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reader.inbox_prefix(), prefix);
    assert_eq!(reader.box_id().unwrap().as_str(), BOX);
}

#[tokio::test]
async fn bucket_live_snapshot_changes_reissue_and_terminal_renewal() {
    let Some(rig) = Rig::start().await else {
        return;
    };
    let (owner, _) = rig.client("aft").await;
    let (reader, manager, issuer) = rig.gated_client("occipital").await;
    let bucket = owner.bucket("aft", "views").unwrap();
    let read = reader.bucket("aft", "views").unwrap();
    assert!(read.get("missing").await.unwrap().is_none());
    let first = bucket.put("one", "initial").await.unwrap();
    let entry = read.get("one").await.unwrap().unwrap();
    assert_eq!(entry.revision, first);
    assert_eq!(entry.value, "initial");
    let second = bucket.put("two", "second").await.unwrap();
    let mut watch = read.watch().await.unwrap();
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Put {
            key: "one".into(),
            revision: first,
            value: "initial".into()
        }
    );
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Put {
            key: "two".into(),
            revision: second,
            value: "second".into()
        }
    );
    assert_eq!(next(&mut watch).await, BucketItem::CaughtUp);
    let live = bucket.put("one", "live").await.unwrap();
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Put {
            key: "one".into(),
            revision: live,
            value: "live".into()
        }
    );
    let deleted = bucket.delete("two").await.unwrap();
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Delete {
            key: "two".into(),
            revision: deleted
        }
    );
    assert!(read.get("two").await.unwrap().is_none());
    assert!(matches!(
        read.put("forbidden", "value").await,
        Err(Error::BucketNotOwned { .. })
    ));
    let cap = vec![7; 262_144];
    let at_limit = bucket.put("cap", cap.clone()).await.unwrap();
    assert_eq!(read.get("cap").await.unwrap().unwrap().value.as_ref(), cap);
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Put {
            key: "cap".into(),
            revision: at_limit,
            value: cap.into()
        }
    );

    // Close the actual reader socket before writing. History 1 deliberately
    // evicts the intermediate value; re-issue must resume retained state.
    let old = reader.connection();
    old.close(Duration::from_secs(1)).await.unwrap();
    bucket.put("one", "intermediate").await.unwrap();
    let final_revision = bucket.put("one", "final").await.unwrap();
    let delete_revision = bucket.delete("cap").await.unwrap();
    let mut changes = reader.connection_changes();
    manager
        .reissue(old.inbox_prefix().strip_prefix("_INBOX.").unwrap())
        .await
        .unwrap();
    // Closing the current connection also makes the client replace it under
    // the same key, so wait for the connection carrying the re-issued key.
    tokio::time::timeout(Duration::from_secs(10), async {
        while reader.connection().inbox_prefix() == old.inbox_prefix() {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    changes.borrow_and_update();
    assert_ne!(old.inbox_prefix(), reader.connection().inbox_prefix());
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Put {
            key: "one".into(),
            revision: final_revision,
            value: "final".into()
        }
    );
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Delete {
            key: "cap".into(),
            revision: delete_revision
        }
    );
    let sentinel = bucket.put("sentinel", "after reconnect").await.unwrap();
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Put {
            key: "sentinel".into(),
            revision: sentinel,
            value: "after reconnect".into()
        }
    );
    // Hold nonce signing so the existing connection cannot reconnect with its
    // current credential until both overwrites and the delete are stored.
    let paused = issuer.signing.lock().await;
    let sign_calls = issuer.sign_calls.load(Ordering::SeqCst);
    reader
        .connection()
        .client()
        .force_reconnect()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while issuer.sign_calls.load(Ordering::SeqCst) == sign_calls {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("reconnect must reach nonce signing");
    let pending = tokio::spawn(async move {
        let item = next(&mut watch).await;
        (watch, item)
    });
    bucket.put("one", "offline intermediate").await.unwrap();
    let raw_revision = bucket.put("one", "offline final").await.unwrap();
    let raw_delete = bucket.delete("sentinel").await.unwrap();
    drop(paused);
    let (mut watch, item) = pending.await.unwrap();
    assert_eq!(
        item,
        BucketItem::Put {
            key: "one".into(),
            revision: raw_revision,
            value: "offline final".into()
        }
    );
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Delete {
            key: "sentinel".into(),
            revision: raw_delete
        }
    );
    assert!(
        !changes.has_changed().unwrap(),
        "same-key reconnect must not replace the connection"
    );
    // With no further writes, a pending read must not replay old revisions.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), watch.next())
            .await
            .is_err()
    );
    reader.task.abort();
    let error = tokio::time::timeout(Duration::from_secs(5), watch.next())
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(error, Error::Transport(ref text) if text == "credential maintenance stopped")
    );
}

#[tokio::test]
async fn bucket_live_empty_snapshot_and_watch_capacity() {
    let Some(rig) = Rig::start().await else {
        return;
    };
    let (reader, _) = rig.client("occipital").await;
    let bucket = reader.bucket("cerebellum", "views").unwrap();
    let mut watches = Vec::new();
    for _ in 0..16 {
        let mut watch = bucket.watch().await.unwrap();
        assert_eq!(next(&mut watch).await, BucketItem::CaughtUp);
        watches.push(watch);
    }
    let (owner, _) = rig.client("cerebellum").await;
    let revision = owner
        .bucket("cerebellum", "views")
        .unwrap()
        .put("owner", "value")
        .await
        .unwrap();
    assert_eq!(
        next(&mut watches[0]).await,
        BucketItem::Put {
            key: "owner".into(),
            revision,
            value: "value".into()
        }
    );
    let stream_name = "KV_CK_BOX_GOLDENFIXTURE_CEREBELLUM_VIEWS";
    let response = rig.admin.request(format!("$JS.API.CONSUMER.CREATE.{stream_name}"),
        serde_json::to_vec(&json!({"stream_name":stream_name, "config":{"deliver_subject":"_INBOX.probe", "ack_policy":"none"}})).unwrap().into()).await.unwrap();
    let body: Value = serde_json::from_slice(&response.payload).unwrap();
    eprintln!("NATS 2.15.0 full watch response: {}", body["error"]);
    assert_eq!(body["error"]["code"], 400);
    assert_eq!(body["error"]["err_code"], 10026);
    assert_eq!(
        body["error"]["description"],
        "maximum consumers limit reached"
    );
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), bucket.watch())
            .await
            .expect("a new watch must fail without capacity backoff"),
        Err(Error::WatchCapacity { .. })
    ));
    drop(watches);
    // No consumer-delete permission exists. Prove that unsubscribing and the
    // server's inactivity timer free all slots without an administrative delete.
    let context = async_nats::jetstream::new(rig.admin.clone());
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let stream = context.get_stream(stream_name).await.unwrap();
            if stream.cached_info().state.consumer_count == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("inactive watches must release slots");
    bucket.watch().await.unwrap();
}

#[tokio::test]
async fn bucket_live_full_reissue_waits_for_own_slot() {
    let Some(rig) = Rig::start().await else {
        return;
    };
    let (owner, _) = rig.client("aft").await;
    let (reader, manager) = rig.client("occipital").await;
    let bucket = owner.bucket("aft", "views").unwrap();
    let read = reader.bucket("aft", "views").unwrap();
    let mut watches = Vec::new();
    for _ in 0..16 {
        let mut watch = read.watch().await.unwrap();
        assert_eq!(next(&mut watch).await, BucketItem::CaughtUp);
        watches.push(watch);
    }
    let mut watch = watches.remove(0);
    let before = bucket.put("key", "before reissue").await.unwrap();
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Put {
            key: "key".into(),
            revision: before,
            value: "before reissue".into()
        }
    );
    let old = reader.connection();
    let mut changes = reader.connection_changes();
    manager
        .reissue(old.inbox_prefix().strip_prefix("_INBOX.").unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), changes.changed())
        .await
        .unwrap()
        .unwrap();
    let stream = async_nats::jetstream::new(rig.admin.clone())
        .get_stream("KV_CK_BOX_GOLDENFIXTURE_AFT_VIEWS")
        .await
        .unwrap();
    assert_eq!(
        stream.cached_info().state.consumer_count,
        16,
        "old consumers still occupy every slot"
    );
    bucket.put("key", "intermediate").await.unwrap();
    let after = bucket.put("key", "after reissue").await.unwrap();
    // Opening a replacement must wait for the old consumer's inactivity timer,
    // not expose a transient capacity error to an established reader.
    let item = tokio::time::timeout(Duration::from_secs(65), watch.next())
        .await
        .expect("capacity retry is bounded")
        .unwrap();
    assert_eq!(
        item,
        BucketItem::Put {
            key: "key".into(),
            revision: after,
            value: "after reissue".into()
        }
    );
    let sentinel = bucket.delete("key").await.unwrap();
    assert_eq!(
        next(&mut watch).await,
        BucketItem::Delete {
            key: "key".into(),
            revision: sentinel
        }
    );
    drop(watches);
}
