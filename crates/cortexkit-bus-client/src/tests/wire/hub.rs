use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use super::{FakeSubc, Server};
use crate::{
    credentials::{cached_manager, CredentialManager, CredentialTransport},
    BusClient, BusClientOptions, Error, HubClient, Result,
};

const BOX: &str = "0123456789abcdef0123456789abcdef";
const MODULE: &str = "test_module";
const PREFIX: &str = "ckdel.0123456789abcdef0123456789abcdef.test_module";
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct HubIssuer {
    url: String,
    calls: Mutex<Vec<(String, Value)>>,
    issued: Mutex<usize>,
    fields: Vec<(&'static str, Value)>,
    issue_refusal: Option<&'static str>,
    reissue_refusal: Mutex<Option<&'static str>>,
    renew_refusal: Option<&'static str>,
    sign_refusal: Mutex<Option<&'static str>>,
    renew_fields: Vec<(&'static str, Value)>,
    renew_now: bool,
    server_nonce: Option<&'static str>,
}

#[async_trait]
impl CredentialTransport for HubIssuer {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut calls = self.calls.lock().unwrap();
        let mut issued_count = self.issued.lock().unwrap();
        let issued = *issued_count;
        calls.push((method.to_owned(), params.clone()));
        let exp = super::super::expiry();
        match method {
            "ckbus.hub_credential" => {
                assert_eq!(params, json!({}));
                let refusal = if issued == 0 {
                    self.issue_refusal
                } else {
                    *self.reissue_refusal.lock().unwrap()
                };
                if let Some(code) = refusal {
                    return Err(Error::Refused(code.to_owned()));
                }
                let mut reply = json!({
                    "jwt": format!("hub-jwt-{issued}"), "credential_public": format!("UHUB{issued}"),
                    "exp": if issued == 0 && self.renew_now { exp - 780 } else { exp }, "acct": format!("fed_{BOX}"),
                    "domain": "hub", "box_id": BOX, "inbox_prefix": PREFIX, "server_url": self.url,
                });
                for (name, value) in &self.fields {
                    reply[*name] = value.clone();
                }
                *issued_count += 1;
                Ok(reply)
            }
            "ckbus.hub_credential_renew" => {
                assert_eq!(
                    params,
                    json!({"credential_public": format!("UHUB{}", issued - 1)})
                );
                if let Some(code) = self.renew_refusal {
                    return Err(Error::Refused(code.to_owned()));
                }
                let mut reply = json!({"jwt": "hub-renewed-jwt", "exp": exp, "credential_public": params["credential_public"]});
                for (name, value) in &self.renew_fields {
                    reply[*name] = value.clone();
                }
                Ok(reply)
            }
            "ckbus.nonce_sign" => {
                assert_eq!(
                    params,
                    json!({"credential_public": format!("UHUB{}", issued - 1),
                        "nonce_b64": STANDARD.encode(self.server_nonce.unwrap_or("server-nonce"))})
                );
                if let Some(code) = self.sign_refusal.lock().unwrap().take() {
                    return Err(Error::Refused(code.to_owned()));
                }
                Ok(json!({"signature_b64": STANDARD.encode([42; 64])}))
            }
            _ => panic!("unexpected method {method}"),
        }
    }
}

async fn wait_for_refused_reissues(
    issuer: &HubIssuer,
    changes: &tokio::sync::watch::Receiver<u64>,
    retries: usize,
) {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let reissues = issuer
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(name, _)| name == "ckbus.hub_credential")
                .skip(1)
                .count();
            if reissues >= retries {
                break;
            }
            assert!(
                changes.has_changed().is_ok(),
                "maintenance stopped during a transient hub outage"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("maintenance retries the refused hub credential");
}

#[tokio::test]
async fn hub_bucket_watch_live_history_origin_and_reset() {
    use async_nats::jetstream::{self, consumer::push, kv};
    use futures_util::StreamExt;
    use std::process::{Child, Command, Stdio};

    let Some(binary) = std::env::var_os("CK_NATS_SERVER_BIN") else {
        assert_ne!(
            std::env::var("CK_REQUIRE_NATS_SERVER").as_deref(),
            Ok("1"),
            "NATS binary required"
        );
        eprintln!("nats-server-absent: hub bucket live test skipped");
        return;
    };
    struct Broker(Child);
    impl Drop for Broker {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("hub.conf");
    // The live module rig does not configure a domain. This local server
    // exercises the hub-domain API; the issuer supplies no real credentials.
    std::fs::write(&config, format!(
        "listen: \"127.0.0.1:-1\"\nports_file_dir: {}\njetstream {{ domain: hub, store_dir: {} }}\n",
        json!(directory.path()), json!(directory.path().join("store")),
    )).unwrap();
    let mut broker = Broker(
        Command::new(binary)
            .arg("-c")
            .arg(config)
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let url = tokio::time::timeout(TIMEOUT, async {
        let suffix = format!("_{}.ports", broker.0.id());
        loop {
            for entry in std::fs::read_dir(directory.path()).unwrap() {
                let entry = entry.unwrap();
                if entry.file_name().to_string_lossy().ends_with(&suffix) {
                    if let Ok(ports) =
                        serde_json::from_slice::<Value>(&std::fs::read(entry.path()).unwrap())
                    {
                        return ports["nats"][0].as_str().unwrap().to_owned();
                    }
                }
            }
            assert!(
                broker.0.try_wait().unwrap().is_none(),
                "hub broker exited before listening"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("hub broker did not listen");
    let issuer = Arc::new(HubIssuer {
        url: url.clone(),
        server_nonce: Some(""),
        ..Default::default()
    });
    let manager = CredentialManager::start_hub(issuer.clone(), MODULE)
        .await
        .unwrap();
    let hub = HubClient::connect_manager(manager.clone(), TIMEOUT)
        .await
        .unwrap();
    let admin = async_nats::connect(url).await.unwrap();
    let context = jetstream::with_domain(admin.clone(), "hub");
    let config = kv::Config {
        bucket: "hub-fixture".into(),
        history: 1,
        ..Default::default()
    };
    let bucket = context.create_key_value(config.clone()).await.unwrap();
    bucket.put("old", "overwritten".into()).await.unwrap();
    assert_eq!(bucket.put("old", "latest old".into()).await.unwrap(), 2);
    let mut watch = hub.watch_bucket(&config.bucket).await.unwrap();
    assert_eq!(
        tokio::time::timeout(TIMEOUT, watch.next())
            .await
            .unwrap()
            .unwrap(),
        crate::BucketItem::Put {
            key: "old".into(),
            revision: 2,
            value: "latest old".into(),
        }
    );
    assert_eq!(watch.next().await.unwrap(), crate::BucketItem::CaughtUp);

    let stream = context.get_stream(&bucket.stream_name).await.unwrap();
    let consumer = stream.consumers().next().await.unwrap().unwrap();
    let inbox = consumer.config.deliver_subject.unwrap();
    let mut observed = admin.subscribe(inbox.clone()).await.unwrap();
    let foreign = context
        .create_key_value(kv::Config {
            bucket: "foreign-fixture".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    context
        .create_consumer_on_stream(
            push::Config {
                deliver_subject: inbox,
                ack_policy: jetstream::consumer::AckPolicy::None,
                ..Default::default()
            },
            &foreign.stream_name,
        )
        .await
        .unwrap();
    foreign
        .put("unrelated", "foreign value".into())
        .await
        .unwrap();
    tokio::time::timeout(TIMEOUT, async {
        while observed.next().await.unwrap().subject.as_str() != "$KV.foreign-fixture.unrelated" {}
    })
    .await
    .expect("expected a foreign delivery on the watch inbox");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), watch.next())
            .await
            .is_err(),
        "the hub watch must discard foreign subjects"
    );

    context.delete_key_value(&config.bucket).await.unwrap();
    let name = config.bucket.clone();
    let new_bucket = context.create_key_value(config).await.unwrap();
    new_bucket.put("new", "overwritten".into()).await.unwrap();
    // The replacement reaches the same revision, so only the creation time
    // distinguishes it from the original stream.
    assert_eq!(new_bucket.put("new", "latest new".into()).await.unwrap(), 2);
    let reset = tokio::time::timeout(Duration::from_secs(20), watch.next())
        .await
        .expect("hub watch should signal the bucket reset");
    assert!(
        matches!(&reset, Err(Error::BucketReset { bucket }) if bucket == &name),
        "{reset:?}"
    );
    assert_eq!(
        tokio::time::timeout(TIMEOUT, watch.next())
            .await
            .unwrap()
            .unwrap(),
        crate::BucketItem::Put {
            key: "new".into(),
            revision: 2,
            value: "latest new".into(),
        }
    );
    assert_eq!(watch.next().await.unwrap(), crate::BucketItem::CaughtUp);

    let old = hub.client().unwrap();
    let mut changes = hub.connection_changes();
    *issuer.reissue_refusal.lock().unwrap() = Some("hub_unavailable");
    assert!(matches!(
        manager.reissue("UHUB0").await,
        Err(Error::HubUnavailable)
    ));
    assert!(matches!(
        hub.watch_bucket(&name).await,
        Err(Error::HubUnavailable)
    ));
    tokio::time::timeout(TIMEOUT, async {
        while old.flush().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the original connection closes during hub recovery");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), watch.next())
            .await
            .is_err(),
        "the open watch must wait for the replacement"
    );

    assert_eq!(
        new_bucket
            .put("during_outage", "third".into())
            .await
            .unwrap(),
        3
    );
    *issuer.reissue_refusal.lock().unwrap() = None;
    tokio::time::timeout(TIMEOUT, changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(hub.watch_bucket(&name).await.is_ok());
    assert_eq!(
        tokio::time::timeout(TIMEOUT, watch.next())
            .await
            .unwrap()
            .unwrap(),
        crate::BucketItem::Put {
            key: "during_outage".into(),
            revision: 3,
            value: "third".into(),
        }
    );
}

#[tokio::test]
async fn hub_obtains_connects_and_renews_with_hub_domain_and_inbox() {
    let server = Server::start().await;
    let issuer = Arc::new(HubIssuer {
        url: server.url.clone(),
        renew_now: true,
        ..Default::default()
    });
    let manager = CredentialManager::start_hub(issuer.clone(), MODULE)
        .await
        .unwrap();
    let hub = HubClient::connect_manager(manager.clone(), TIMEOUT)
        .await
        .unwrap();
    assert_eq!(hub.box_id().as_ref().map(|id| id.as_str()), Some(BOX));
    let raw = hub.client().unwrap();
    let context = hub.jetstream().unwrap();
    assert!(raw.new_inbox().starts_with(&format!("{PREFIX}.")));

    // The fake server sends a PubAck-shaped reply to this management request.
    // The subject, not the response body, is what establishes the domain.
    assert!(context.get_stream("HUB_FIXTURE").await.is_err());
    let ack = context
        .publish("hub.fixture", "fact".into())
        .await
        .unwrap()
        .await
        .unwrap();
    assert_eq!(ack.sequence, 17);
    {
        let observed = server.observed.lock().unwrap();
        assert!(observed
            .publishes
            .iter()
            .any(|(subject, _)| subject == "$JS.hub.API.STREAM.INFO.HUB_FIXTURE"));
        assert!(observed
            .subscriptions
            .iter()
            .any(|subject| subject.starts_with(&format!("{PREFIX}."))));
        assert_eq!(observed.auth[0]["jwt"], "hub-jwt-0");
    }

    tokio::time::timeout(TIMEOUT, async {
        while manager.credential().await.jwt != "hub-renewed-jwt" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("maintenance renews the short lease");
    raw.force_reconnect().await.unwrap();
    server.wait_for(|seen| seen.auth.len() >= 2).await;
    assert_eq!(
        server.observed.lock().unwrap().auth[1]["jwt"],
        "hub-renewed-jwt"
    );
    assert_eq!(hub.box_id().as_ref().map(|id| id.as_str()), Some(BOX));
    assert!(!hub.connection_changes().has_changed().unwrap());
}

#[tokio::test]
async fn hub_first_issue_refusals_are_typed_and_not_retried() {
    let server = Server::start().await;
    for code in ["hub_unavailable", "hub_not_granted"] {
        let is_expected = |error: &Error| {
            matches!(
                (code, error),
                ("hub_unavailable", Error::HubUnavailable)
                    | ("hub_not_granted", Error::HubNotGranted)
            )
        };
        let issuer = Arc::new(HubIssuer {
            url: server.url.clone(),
            issue_refusal: Some(code),
            ..Default::default()
        });
        let result = tokio::time::timeout(
            TIMEOUT,
            CredentialManager::start_hub(issuer.clone(), MODULE),
        )
        .await
        .unwrap();
        assert!(
            matches!(result, Err(error) if is_expected(&error)),
            "{code}"
        );
        assert_eq!(issuer.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn hub_not_granted_is_sticky_after_renewal_or_nonce_sign() {
    let server = Server::start().await;
    for renewal in [true, false] {
        let issuer = Arc::new(HubIssuer {
            url: server.url.clone(),
            renew_now: renewal,
            renew_refusal: renewal.then_some("hub_not_granted"),
            ..Default::default()
        });
        let manager = CredentialManager::start_hub(issuer.clone(), MODULE)
            .await
            .unwrap();
        let hub = HubClient::connect_manager(manager.clone(), TIMEOUT)
            .await
            .unwrap();
        let mut changes = hub.connection_changes();
        if !renewal {
            *issuer.sign_refusal.lock().unwrap() = Some("hub_not_granted");
            hub.client().unwrap().force_reconnect().await.unwrap();
        }
        assert!(tokio::time::timeout(TIMEOUT, changes.changed())
            .await
            .unwrap()
            .is_err());
        assert!(matches!(hub.client(), Err(Error::HubNotGranted)));
        assert!(matches!(manager.renew().await, Err(Error::HubNotGranted)));
        assert!(matches!(
            manager.reissue("UHUB0").await,
            Err(Error::HubNotGranted)
        ));
        assert!(matches!(
            manager.sign("UHUB0", vec![]).await,
            Err(Error::HubNotGranted)
        ));
        let calls = issuer.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "ckbus.hub_credential",
                "ckbus.nonce_sign",
                if renewal {
                    "ckbus.hub_credential_renew"
                } else {
                    "ckbus.nonce_sign"
                }
            ]
        );
    }
}

#[tokio::test]
async fn hub_recovers_from_unavailable_renewal_or_nonce_sign() {
    let server = Server::start().await;
    for renewal in [true, false] {
        let issuer = Arc::new(HubIssuer {
            url: server.url.clone(),
            renew_now: renewal,
            renew_refusal: renewal.then_some("hub_unavailable"),
            reissue_refusal: Mutex::new(Some("hub_unavailable")),
            ..Default::default()
        });
        let manager = CredentialManager::start_hub(issuer.clone(), MODULE)
            .await
            .unwrap();
        let hub = HubClient::connect_manager(manager, TIMEOUT).await.unwrap();
        let mut changes = hub.connection_changes();
        if !renewal {
            *issuer.sign_refusal.lock().unwrap() = Some("hub_unavailable");
            hub.client().unwrap().force_reconnect().await.unwrap();
        }
        wait_for_refused_reissues(&issuer, &changes, 2).await;
        assert!(matches!(hub.client(), Err(Error::HubUnavailable)));
        assert!(!changes.has_changed().unwrap());
        *issuer.reissue_refusal.lock().unwrap() = None;

        tokio::time::timeout(TIMEOUT, changes.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*changes.borrow(), 1);
        hub.client().unwrap().flush().await.unwrap();
    }
}

#[tokio::test]
async fn hub_issue_refuses_mismatched_domain_account_box_or_module_prefix() {
    for (field, value, expected) in [
        (
            "domain",
            json!("leaf"),
            "expected hub credential domain to be hub",
        ),
        (
            "domain",
            Value::Null,
            "expected hub credential domain to be hub",
        ),
        (
            "box_id",
            json!("0123456789ABCDEF0123456789ABCDEF"),
            "missing or invalid hub credential box id",
        ),
        (
            "box_id",
            Value::Null,
            "missing or invalid hub credential box id",
        ),
        (
            "box_id",
            json!("fedcba9876543210fedcba9876543210"),
            "hub credential inbox prefix does not match its box and module",
        ),
        (
            "inbox_prefix",
            json!("ckdel.0123456789abcdef0123456789abcdef.other_module"),
            "hub credential inbox prefix does not match its box and module",
        ),
        (
            "inbox_prefix",
            json!("_INBOX.UHUB0"),
            "hub credential inbox prefix does not match its box and module",
        ),
        (
            "acct",
            json!("fed_fedcba9876543210fedcba9876543210"),
            "hub credential account does not match its box",
        ),
        (
            "acct",
            json!(format!("box_{BOX}")),
            "hub credential account does not match its box",
        ),
    ] {
        let issuer = Arc::new(HubIssuer {
            url: "nats://127.0.0.1:4222".into(),
            fields: vec![(field, value.clone())],
            ..Default::default()
        });
        let result = CredentialManager::start_hub(issuer, MODULE).await;
        assert!(
            matches!(result, Err(Error::Reply(reason)) if reason == expected),
            "{field}: {value}"
        );
    }
}

#[tokio::test]
async fn hub_renewal_refuses_changed_metadata_but_accepts_matching_echoes() {
    for (field, value, expected) in [
        (
            "domain",
            json!("leaf"),
            "renewal changed the hub credential domain",
        ),
        (
            "inbox_prefix",
            json!("ckdel.0123456789abcdef0123456789abcdef.other_module"),
            "renewal changed the inbox prefix",
        ),
        (
            "inbox_prefix",
            Value::Null,
            "renewal changed the inbox prefix",
        ),
    ] {
        let issuer = Arc::new(HubIssuer {
            url: "nats://127.0.0.1:4222".into(),
            renew_fields: vec![(field, value.clone())],
            ..Default::default()
        });
        let manager = CredentialManager::start_hub(issuer, MODULE).await.unwrap();
        assert!(
            matches!(manager.renew().await, Err(Error::Reply(reason)) if reason == expected),
            "{field}: {value}"
        );
        assert_eq!(manager.credential().await.jwt, "hub-jwt-0");
    }
    let issuer = Arc::new(HubIssuer {
        url: "nats://127.0.0.1:4222".into(),
        renew_fields: vec![
            ("domain", json!("hub")),
            ("box_id", json!(BOX)),
            ("inbox_prefix", json!(PREFIX)),
        ],
        ..Default::default()
    });
    let manager = CredentialManager::start_hub(issuer, MODULE).await.unwrap();
    manager.renew().await.unwrap();
    assert_eq!(manager.credential().await.jwt, "hub-renewed-jwt");
}

#[tokio::test]
async fn hub_box_id_is_unchanged_when_renewal_refuses_a_different_box() {
    let server = Server::start().await;
    let issuer = Arc::new(HubIssuer {
        url: server.url.clone(),
        renew_fields: vec![("box_id", json!("fedcba9876543210fedcba9876543210"))],
        ..Default::default()
    });
    let manager = CredentialManager::start_hub(issuer, MODULE).await.unwrap();
    let hub = HubClient::connect_manager(manager.clone(), TIMEOUT)
        .await
        .unwrap();

    assert!(matches!(
        manager.renew().await,
        Err(Error::Reply("renewal changed the hub credential box id"))
    ));
    assert_eq!(manager.credential().await.jwt, "hub-jwt-0");
    assert_eq!(hub.box_id().as_ref().map(|id| id.as_str()), Some(BOX));
}

#[tokio::test]
async fn hub_and_box_replacements_are_independent() {
    let server = Server::start().await;
    let box_manager = cached_manager(FakeSubc::new(&server.url), MODULE, false)
        .await
        .unwrap();
    let bus =
        BusClient::connect_manager(box_manager.clone(), BusClientOptions::new(MODULE).unwrap())
            .await
            .unwrap();
    let hub_manager = cached_manager(
        Arc::new(HubIssuer {
            url: server.url.clone(),
            ..Default::default()
        }),
        MODULE,
        true,
    )
    .await
    .unwrap();
    let hub = HubClient::connect_manager(hub_manager.clone(), TIMEOUT)
        .await
        .unwrap();
    let mut box_changes = bus.connection_changes();
    let mut hub_changes = hub.connection_changes();
    let old_hub = hub.client().unwrap();

    box_manager.reissue("UKEY0").await.unwrap();
    tokio::time::timeout(TIMEOUT, box_changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bus.inbox_prefix(), "_INBOX.UKEY1");
    assert!(!hub_changes.has_changed().unwrap());
    old_hub.flush().await.unwrap();
    let new_box = bus.connection().client();

    hub_manager.reissue("UHUB0").await.unwrap();
    tokio::time::timeout(TIMEOUT, hub_changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hub.box_id().as_ref().map(|id| id.as_str()), Some(BOX));
    assert_eq!(*hub_changes.borrow(), 1);
    assert!(!box_changes.has_changed().unwrap());
    new_box.flush().await.unwrap();
    let raw = hub.client().unwrap();
    raw.publish("hub.fixture.core", "fact".into())
        .await
        .unwrap();
    raw.flush().await.unwrap();
    server
        .wait_for(|seen| seen.publishes.iter().any(|(s, _)| s == "hub.fixture.core"))
        .await;
    {
        let observed = server.observed.lock().unwrap();
        let jwts: Vec<_> = observed
            .auth
            .iter()
            .map(|auth| auth["jwt"].as_str().unwrap())
            .collect();
        assert_eq!(jwts, ["jwt-0", "hub-jwt-0", "jwt-1", "hub-jwt-1"]);
    }

    tokio::time::timeout(TIMEOUT, async {
        while old_hub.flush().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the replaced hub connection must close");
}

#[tokio::test]
async fn hub_recovers_when_nonce_reissue_is_unavailable_after_connection_ends() {
    let server = Server::start().await;
    let issuer = Arc::new(HubIssuer {
        url: server.url.clone(),
        reissue_refusal: Mutex::new(Some("hub_unavailable")),
        ..Default::default()
    });
    let manager = CredentialManager::start_hub(issuer.clone(), MODULE)
        .await
        .unwrap();
    let hub = HubClient::connect_manager(manager, TIMEOUT).await.unwrap();
    let mut changes = hub.connection_changes();
    let raw = hub.client().unwrap();

    *issuer.sign_refusal.lock().unwrap() = Some("ckbus_credential_revoked");
    raw.drain().await.unwrap();
    tokio::time::timeout(TIMEOUT, async {
        while !matches!(hub.client(), Err(Error::HubUnavailable)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("hub reports the nonce reissue refusal");
    assert!(matches!(hub.client(), Err(Error::HubUnavailable)));
    assert!(matches!(hub.jetstream(), Err(Error::HubUnavailable)));
    assert!(matches!(
        hub.watch_bucket("fixture").await,
        Err(Error::HubUnavailable)
    ));
    wait_for_refused_reissues(&issuer, &changes, 2).await;
    assert!(!changes.has_changed().unwrap());
    *issuer.reissue_refusal.lock().unwrap() = None;

    tokio::time::timeout(TIMEOUT, changes.changed())
        .await
        .expect("maintenance publishes the recovered connection")
        .expect("maintenance remains alive");
    assert_eq!(*changes.borrow(), 1);
    hub.client().unwrap().flush().await.unwrap();
    assert!(hub.jetstream().is_ok());
    assert_eq!(server.observed.lock().unwrap().auth[1]["jwt"], "hub-jwt-1");
}
