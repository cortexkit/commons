use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use super::{is_liveness, FakeSubc, Server};
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
    fields: Vec<(&'static str, Value)>,
    issue_refusal: Option<&'static str>,
    reissue_refusal: Option<&'static str>,
    renew_refusal: Option<&'static str>,
    sign_refusal: Mutex<Option<&'static str>>,
    renew_fields: Vec<(&'static str, Value)>,
    renew_now: bool,
}

#[async_trait]
impl CredentialTransport for HubIssuer {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut calls = self.calls.lock().unwrap();
        let issued = calls
            .iter()
            .filter(|(name, _)| name == "ckbus.hub_credential")
            .count();
        calls.push((method.to_owned(), params.clone()));
        let exp = super::super::expiry();
        match method {
            "ckbus.hub_credential" => {
                assert_eq!(params, json!({}));
                let refusal = if issued == 0 {
                    self.issue_refusal
                } else {
                    self.reissue_refusal
                };
                if let Some(code) = refusal {
                    return Err(Error::Refused(code.to_owned()));
                }
                let mut reply = json!({
                    "jwt": format!("hub-jwt-{issued}"), "credential_public": format!("UHUB{issued}"),
                    "exp": if self.renew_now { exp - 780 } else { exp }, "acct": format!("fed_{BOX}"),
                    "domain": "hub", "box_id": BOX, "inbox_prefix": PREFIX, "server_url": self.url,
                });
                for (name, value) in &self.fields {
                    reply[*name] = value.clone();
                }
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
                    json!({"credential_public": format!("UHUB{}", issued - 1), "nonce_b64": STANDARD.encode(b"server-nonce")})
                );
                if let Some(code) = *self.sign_refusal.lock().unwrap() {
                    return Err(Error::Refused(code.to_owned()));
                }
                Ok(json!({"signature_b64": STANDARD.encode([42; 64])}))
            }
            _ => panic!("unexpected method {method}"),
        }
    }
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
            .any(|subject| subject.starts_with(&format!("{PREFIX}.")) && !is_liveness(subject)));
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
    assert!(!hub.connection_changes().has_changed().unwrap());
}

#[tokio::test]
async fn hub_refusals_are_typed_and_not_retried_on_issue_or_renewal() {
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

        let issuer = Arc::new(HubIssuer {
            url: server.url.clone(),
            renew_now: true,
            renew_refusal: Some(code),
            ..Default::default()
        });
        let manager = CredentialManager::start_hub(issuer.clone(), MODULE)
            .await
            .unwrap();
        let hub = HubClient::connect_manager(manager.clone(), TIMEOUT)
            .await
            .unwrap();
        let mut changes = hub.connection_changes();
        assert!(tokio::time::timeout(TIMEOUT, changes.changed())
            .await
            .unwrap()
            .is_err());
        assert!(
            matches!(hub.client(), Err(error) if is_expected(&error)),
            "{code}"
        );
        assert!(
            matches!(manager.renew().await, Err(error) if is_expected(&error)),
            "{code}"
        );
        let calls = issuer.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "ckbus.hub_credential",
                "ckbus.nonce_sign",
                "ckbus.hub_credential_renew"
            ]
        );
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
            "box_id",
            json!("fedcba9876543210fedcba9876543210"),
            "renewal changed the hub credential box id",
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
async fn hub_unavailable_during_nonce_reissue_stops_maintenance() {
    let server = Server::start().await;
    let issuer = Arc::new(HubIssuer {
        url: server.url.clone(),
        reissue_refusal: Some("hub_unavailable"),
        ..Default::default()
    });
    let manager = CredentialManager::start_hub(issuer.clone(), MODULE)
        .await
        .unwrap();
    let hub = HubClient::connect_manager(manager, TIMEOUT).await.unwrap();
    let mut changes = hub.connection_changes();
    let raw = hub.client().unwrap();

    *issuer.sign_refusal.lock().unwrap() = Some("ckbus_credential_revoked");
    raw.force_reconnect().await.unwrap();
    assert!(tokio::time::timeout(TIMEOUT, changes.changed())
        .await
        .unwrap()
        .is_err());
    assert!(matches!(hub.client(), Err(Error::HubUnavailable)));
    let calls = issuer.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "ckbus.hub_credential",
            "ckbus.nonce_sign",
            "ckbus.nonce_sign",
            "ckbus.hub_credential"
        ]
    );
}
