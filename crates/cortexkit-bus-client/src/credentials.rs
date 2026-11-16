use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex, RwLock};

use crate::{Error, Result};

pub(crate) struct SubcTransport {
    consumer: Arc<subc_client_rs::SubcConsumer>,
    module: String,
    timeout: Duration,
}

impl SubcTransport {
    pub fn new(
        consumer: Arc<subc_client_rs::SubcConsumer>,
        module: &str,
        timeout: Duration,
    ) -> Self {
        Self {
            consumer,
            module: module.into(),
            timeout,
        }
    }
}

#[async_trait]
impl CredentialTransport for SubcTransport {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = self
            .consumer
            .call(
                subc_protocol::RouteTarget::ManagementSurface {
                    module_id: "ckbus".into(),
                },
                subc_protocol::BindIdentity::new("bus-client", &self.module),
                serde_json::to_vec(&json!({"method": method, "params": params}))
                    .map_err(|_| Error::Reply("request could not be encoded"))?,
                subc_client_rs::CallOptions {
                    timeout: self.timeout,
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| match error {
                subc_client_rs::CallError::Module(body) => Error::Refused(body.code),
                other => Error::Transport(other.to_string()),
            })?;
        let value: Value =
            serde_json::from_slice(&body).map_err(|_| Error::Reply("response is not JSON"))?;
        value
            .get("result")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or(Error::Reply("missing result object"))
    }
}

#[async_trait]
pub(crate) trait CredentialTransport: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> Result<Value>;
}

#[derive(Clone, Deserialize)]
pub(crate) struct Credential {
    pub jwt: String,
    pub exp: u64,
    pub credential_public: String,
    pub acct: String,
    pub server_url: String,
    pub inbox_prefix: String,
}

impl Credential {
    fn parse(value: Value) -> Result<Self> {
        let credential: Self =
            serde_json::from_value(value).map_err(|_| Error::Reply("missing credential fields"))?;
        let config = cortexkit_bus_nats::ConnectConfig::new(&credential.credential_public)?;
        if config.inbox_prefix() != credential.inbox_prefix {
            return Err(Error::Reply(
                "credential inbox prefix does not match its key",
            ));
        }
        cortexkit_bus_naming::AccountNames::derive(&credential.acct)?;
        let port = credential
            .server_url
            .strip_prefix("nats://127.0.0.1:")
            .and_then(|port| port.parse::<u16>().ok());
        if !port.is_some_and(|port| port != 0) {
            return Err(Error::Reply("expected a plaintext loopback NATS URL"));
        }
        Ok(credential)
    }
}

pub(crate) struct CredentialManager {
    transport: Arc<dyn CredentialTransport>,
    current: RwLock<Credential>,
    // Serializing issue and renewal prevents replies for an old key from
    // replacing the current credential.
    flight: Mutex<()>,
    revisions: watch::Sender<u64>,
}

impl CredentialManager {
    pub async fn start(transport: Arc<dyn CredentialTransport>) -> Result<Arc<Self>> {
        let credential = Self::issue(&*transport).await?;
        let (revisions, _) = watch::channel(0);
        Ok(Arc::new(Self {
            transport,
            current: RwLock::new(credential),
            flight: Mutex::new(()),
            revisions,
        }))
    }

    pub async fn credential(&self) -> Credential {
        self.current.read().await.clone()
    }

    pub fn revisions(&self) -> watch::Receiver<u64> {
        self.revisions.subscribe()
    }

    async fn issue(transport: &dyn CredentialTransport) -> Result<Credential> {
        let mut backoff = Duration::from_millis(250);
        loop {
            match transport.call("ckbus.credential", json!({})).await {
                Ok(value) => return Credential::parse(value),
                Err(error) if transient(&error) => {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Every refusal names the observed key. A waiter must not replace the key
    /// that another waiter has just issued, because issuance revokes that key.
    pub async fn reissue(&self, observed_public: &str) -> Result<()> {
        let _flight = self.flight.lock().await;
        if self.current.read().await.credential_public != observed_public {
            return Ok(());
        }
        let credential = Self::issue(&*self.transport).await?;
        let mut current = self.current.write().await;
        if credential.acct != current.acct {
            return Err(Error::Reply("re-issue changed the account"));
        }
        *current = credential;
        self.revisions.send_modify(|revision| *revision += 1);
        Ok(())
    }

    pub async fn sign(&self, public: &str, nonce: Vec<u8>) -> Result<Vec<u8>> {
        if self.current.read().await.credential_public != public {
            return Err(Error::Reply("connection credential was replaced"));
        }
        match self
            .transport
            .call(
                "ckbus.nonce_sign",
                json!({
                    "credential_public": public,
                    "nonce_b64": STANDARD.encode(nonce),
                }),
            )
            .await
        {
            Ok(value) => STANDARD
                .decode(
                    value["signature_b64"]
                        .as_str()
                        .ok_or(Error::Reply("missing nonce signature"))?,
                )
                .map_err(|_| Error::Reply("invalid base64 nonce signature")),
            Err(error) if needs_reissue(&error) => {
                self.reissue(public).await?;
                // This connection still has the old inbox. Only a replacement
                // connection may authenticate with the newly issued key.
                Err(Error::Reply("connection credential was replaced"))
            }
            Err(error) => Err(error),
        }
    }

    pub async fn renew(&self) -> Result<()> {
        let observed = self.credential().await;
        let _flight = self.flight.lock().await;
        if self.current.read().await.credential_public != observed.credential_public {
            return Ok(());
        }
        let value = self
            .transport
            .call(
                "ckbus.credential_renew",
                json!({
                    "credential_public": observed.credential_public,
                }),
            )
            .await;
        match value {
            Ok(value) => {
                #[derive(Deserialize)]
                struct Renewal {
                    jwt: String,
                    exp: u64,
                    credential_public: String,
                }
                let renewal: Renewal = serde_json::from_value(value)
                    .map_err(|_| Error::Reply("missing renewal fields"))?;
                if renewal.credential_public != observed.credential_public {
                    return Err(Error::Reply("renewal changed the credential key"));
                }
                let mut current = self.current.write().await;
                current.jwt = renewal.jwt;
                current.exp = renewal.exp;
                Ok(())
            }
            Err(error) if needs_reissue(&error) => {
                drop(_flight);
                self.reissue(&observed.credential_public).await
            }
            Err(error) => Err(error),
        }
    }
}

fn needs_reissue(error: &Error) -> bool {
    matches!(error, Error::Refused(code) if matches!(code.as_str(),
        "ckbus_credential_superseded" | "ckbus_credential_revoked"))
}

pub(crate) fn transient(error: &Error) -> bool {
    matches!(error, Error::Refused(code) if matches!(code.as_str(),
        "ckbus_census_unavailable" | "ckbus_not_ready" |
        "ckbus_spawn_snapshot_unavailable" | "ckbus_signing_failed"))
}

pub(crate) fn renewal_delay(exp: u64) -> Duration {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    // Renew about ten minutes into a fifteen-minute lease. Process id supplies
    // a stable, small jitter.
    let jitter = u64::from(std::process::id() % 7);
    Duration::from_secs(exp.saturating_sub(300 + jitter)).saturating_sub(now)
}
