use std::{
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use crate::{
    credentials::{CredentialManager, CredentialTransport},
    Error, Result,
};

mod buckets_live;
mod reconnect_live;

pub(crate) struct FakeSubc {
    pub issues: AtomicUsize,
    pub issue_exp: AtomicU64,
    pub renewals: AtomicUsize,
    pub refusals: Mutex<Vec<String>>,
    pub renew_refusal: Mutex<Option<String>>,
    pub sign_refusal: Mutex<Option<String>>,
    pub calls: Mutex<Vec<(String, Value)>>,
    pub url: String,
}

impl FakeSubc {
    pub fn new(url: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            issues: AtomicUsize::new(0),
            issue_exp: AtomicU64::new(0),
            renewals: AtomicUsize::new(0),
            refusals: Mutex::new(vec![]),
            renew_refusal: Mutex::new(None),
            sign_refusal: Mutex::new(None),
            calls: Mutex::new(vec![]),
            url: url.into(),
        })
    }
}

fn expiry() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 900
}

#[async_trait]
impl CredentialTransport for FakeSubc {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        match method {
            "ckbus.credential" => {
                let count = self.issues.fetch_add(1, Ordering::SeqCst);
                let refusal = self.refusals.lock().unwrap().pop();
                if let Some(code) = refusal {
                    return Err(Error::Refused(code));
                }
                // An asynchronous issuer lets concurrent callers overlap; a
                // serialization test would miss the race with a ready future.
                tokio::time::sleep(Duration::from_millis(10)).await;
                let exp = match self.issue_exp.load(Ordering::SeqCst) {
                    0 => expiry(),
                    exp => exp,
                };
                Ok(json!({
                    "jwt": format!("jwt-{count}"), "exp": exp, "acct": "box_test",
                    "credential_public": format!("UKEY{count}"),
                    "server_url": self.url, "inbox_prefix": format!("_INBOX.UKEY{count}"),
                    "spawn_generation": 7, "credential_epoch": count,
                    "user_jwt_id": format!("id-{count}"),
                }))
            }
            "ckbus.credential_renew" => {
                let refusal = self.renew_refusal.lock().unwrap().clone();
                if let Some(code) = refusal {
                    return Err(Error::Refused(code));
                }
                self.renewals.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"jwt": "renewed-jwt", "exp": expiry() + 600,
                    "credential_public": params["credential_public"],
                    "spawn_generation": 7, "credential_epoch": 0, "user_jwt_id": "renewal-id"}))
            }
            "ckbus.nonce_sign" => {
                let refusal = self.sign_refusal.lock().unwrap().clone();
                if let Some(code) = refusal {
                    return Err(Error::Refused(code));
                }
                assert_eq!(
                    STANDARD
                        .decode(params["nonce_b64"].as_str().unwrap())
                        .unwrap(),
                    b"server-nonce"
                );
                Ok(json!({"signature_b64": STANDARD.encode([42; 64])}))
            }
            _ => panic!("unexpected method {method}"),
        }
    }
}

#[tokio::test]
async fn credential_and_nonce_use_key_without_seed() {
    let subc = FakeSubc::new("nats://127.0.0.1:14222");
    let manager = CredentialManager::start(subc.clone()).await.unwrap();
    let credential = manager.credential().await;
    assert_eq!(credential.inbox_prefix, "_INBOX.UKEY0");
    assert_eq!(
        manager
            .sign("UKEY0", b"server-nonce".to_vec())
            .await
            .unwrap(),
        vec![42; 64]
    );
    let calls = subc.calls.lock().unwrap();
    assert_eq!(calls[0], ("ckbus.credential".into(), json!({})));
    assert_eq!(
        calls[1].1,
        json!({"credential_public": "UKEY0", "nonce_b64": STANDARD.encode(b"server-nonce")})
    );
}

#[tokio::test]
async fn credential_url_cannot_hide_a_remote_host_in_userinfo() {
    let subc = FakeSubc::new("nats://127.0.0.1:password@remote.example:4222");
    assert!(matches!(
        CredentialManager::start(subc).await,
        Err(Error::Reply("expected a plaintext loopback NATS URL"))
    ));
}

#[tokio::test]
async fn same_key_renewal_updates_jwt_without_revision() {
    let subc = FakeSubc::new("nats://127.0.0.1:14222");
    let manager = CredentialManager::start(subc.clone()).await.unwrap();
    let changes = manager.revisions();
    manager.renew().await.unwrap();
    let credential = manager.credential().await;
    assert_eq!(credential.jwt, "renewed-jwt");
    assert_eq!(credential.credential_public, "UKEY0");
    assert!(!changes.has_changed().unwrap());
    assert_eq!(subc.issues.load(Ordering::SeqCst), 1);
    assert_eq!(
        subc.calls.lock().unwrap()[1].1,
        json!({"credential_public": "UKEY0"})
    );
}

#[tokio::test]
async fn concurrent_reissue_is_single_flight() {
    let subc = FakeSubc::new("nats://127.0.0.1:14222");
    let manager = CredentialManager::start(subc.clone()).await.unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(33));
    let mut tasks = vec![];
    for _ in 0..32 {
        let manager = manager.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            manager.reissue("UKEY0").await.unwrap();
        }));
    }
    barrier.wait().await;
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        subc.issues.load(Ordering::SeqCst),
        2,
        "one initial issue and one shared re-issue"
    );
    assert_eq!(manager.credential().await.credential_public, "UKEY1");
    assert_eq!(*manager.revisions().borrow(), 1);
}

#[tokio::test]
async fn revoked_and_superseded_sign_or_renew_reissue_once() {
    for code in ["ckbus_credential_superseded", "ckbus_credential_revoked"] {
        for signing in [false, true] {
            let subc = FakeSubc::new("nats://127.0.0.1:14222");
            let manager = CredentialManager::start(subc.clone()).await.unwrap();
            if signing {
                *subc.sign_refusal.lock().unwrap() = Some(code.into());
                assert!(manager
                    .sign("UKEY0", b"server-nonce".to_vec())
                    .await
                    .is_err());
                assert!(manager
                    .sign("UKEY0", b"server-nonce".to_vec())
                    .await
                    .is_err());
            } else {
                *subc.renew_refusal.lock().unwrap() = Some(code.into());
                manager.renew().await.unwrap();
            }
            assert_eq!(
                subc.issues.load(Ordering::SeqCst),
                2,
                "{code}, signing={signing}"
            );
            assert_eq!(manager.credential().await.credential_public, "UKEY1");
        }
    }
}

#[tokio::test]
async fn dead_generation_never_reissues() {
    let subc = FakeSubc::new("nats://127.0.0.1:14222");
    let manager = CredentialManager::start(subc.clone()).await.unwrap();
    *subc.renew_refusal.lock().unwrap() = Some("ckbus_generation_not_live".into());
    assert!(
        matches!(manager.renew().await, Err(Error::Refused(code)) if code == "ckbus_generation_not_live")
    );
    *subc.sign_refusal.lock().unwrap() = Some("ckbus_generation_not_live".into());
    assert!(manager
        .sign("UKEY0", b"server-nonce".to_vec())
        .await
        .is_err());
    assert_eq!(subc.issues.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn transient_issuance_refusals_retry_with_backoff() {
    let subc = FakeSubc::new("nats://127.0.0.1:14222");
    subc.refusals.lock().unwrap().extend(
        [
            "ckbus_signing_failed",
            "ckbus_spawn_snapshot_unavailable",
            "ckbus_not_ready",
            "ckbus_census_unavailable",
        ]
        .map(str::to_owned),
    );
    let start = tokio::time::Instant::now();
    let _manager = CredentialManager::start(subc.clone()).await.unwrap();
    assert_eq!(subc.issues.load(Ordering::SeqCst), 5);
    assert_eq!(start.elapsed(), Duration::from_millis(3760));
}

#[test]
fn renewal_is_about_five_minutes_before_expiry() {
    let delay = crate::credentials::renewal_delay(expiry());
    assert!(delay >= Duration::from_secs(592) && delay <= Duration::from_secs(600));
}

mod wire;
