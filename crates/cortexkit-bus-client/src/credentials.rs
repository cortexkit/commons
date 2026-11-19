use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, OnceLock, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use cortexkit_bus_naming::federation::{module_deliver_prefix, BoxId};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex, RwLock};

use crate::{BusClientOptions, Error, Result};

pub(crate) const REPLACED_CREDENTIAL: &str = "connection credential was replaced";

pub(crate) const HUB_DOMAIN: &str = "hub";

pub(crate) async fn module_manager(
    subc: Arc<subc_client_rs::SubcConsumer>,
    opts: &BusClientOptions,
    hub: bool,
) -> Result<Arc<CredentialManager>> {
    let transport = Arc::new(SubcTransport::new(subc, &opts.module, opts.request_timeout));
    cached_manager(transport, &opts.module, hub).await
}

pub(crate) async fn cached_manager(
    transport: Arc<dyn CredentialTransport>,
    module: &str,
    hub: bool,
) -> Result<Arc<CredentialManager>> {
    // Issuing a second credential for the same module and account revokes the
    // first. Keep independent caches so hub calls never wait on box issuance.
    type Managers = Mutex<HashMap<String, Weak<CredentialManager>>>;
    static BOX_MANAGERS: LazyLock<Managers> = LazyLock::new(Default::default);
    static HUB_MANAGERS: LazyLock<Managers> = LazyLock::new(Default::default);
    let mut managers = if hub { &HUB_MANAGERS } else { &BOX_MANAGERS }.lock().await;
    if let Some(manager) = managers.get(module).and_then(Weak::upgrade) {
        if manager.check_hub_refusal().is_ok() {
            return Ok(manager);
        }
    }
    let manager = if hub {
        CredentialManager::start_hub(transport, module).await?
    } else {
        CredentialManager::start(transport).await?
    };
    managers.insert(module.to_owned(), Arc::downgrade(&manager));
    Ok(manager)
}

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
    #[serde(skip)]
    box_id: Option<BoxId>,
}

impl Credential {
    fn parse(value: Value, hub_module: Option<&str>) -> Result<Self> {
        let hub_box = if hub_module.is_some() {
            if value["domain"].as_str() != Some(HUB_DOMAIN) {
                return Err(Error::Reply("expected hub credential domain to be hub"));
            }
            Some(
                value["box_id"]
                    .as_str()
                    .and_then(|id| BoxId::parse(id).ok())
                    .ok_or(Error::Reply("missing or invalid hub credential box id"))?,
            )
        } else {
            None
        };
        let mut credential: Self =
            serde_json::from_value(value).map_err(|_| Error::Reply("missing credential fields"))?;
        let config = cortexkit_bus_nats::ConnectConfig::new(&credential.credential_public)?;
        if let Some((module, id)) = hub_module.zip(hub_box.as_ref()) {
            if credential.inbox_prefix != module_deliver_prefix(id, module)? {
                return Err(Error::Reply(
                    "hub credential inbox prefix does not match its box and module",
                ));
            }
            if credential.acct != format!("fed_{id}") {
                return Err(Error::Reply(
                    "hub credential account does not match its box",
                ));
            }
        } else {
            let deliver_prefix =
                credential
                    .inbox_prefix
                    .rsplit_once('.')
                    .and_then(|(_, module)| {
                        module_deliver_prefix(&box_id(&credential.acct)?, module).ok()
                    });
            if config.inbox_prefix() != credential.inbox_prefix
                && deliver_prefix.as_deref() != Some(credential.inbox_prefix.as_str())
            {
                return Err(Error::Reply(
                    "credential inbox prefix does not match its key",
                ));
            }
            cortexkit_bus_naming::AccountNames::derive(&credential.acct)?;
        }
        let port = credential
            .server_url
            .strip_prefix("nats://127.0.0.1:")
            .and_then(|port| port.parse::<u16>().ok());
        if !port.is_some_and(|port| port != 0) {
            return Err(Error::Reply("expected a plaintext loopback NATS URL"));
        }
        credential.box_id = hub_box;
        Ok(credential)
    }
}

#[derive(Clone, Copy)]
enum HubRefusal {
    Unavailable,
    NotGranted,
}

impl HubRefusal {
    fn from_error(error: &Error) -> Option<Self> {
        match error {
            Error::Refused(code) if code == "hub_unavailable" => Some(Self::Unavailable),
            Error::Refused(code) if code == "hub_not_granted" => Some(Self::NotGranted),
            _ => None,
        }
    }

    fn error(self) -> Error {
        match self {
            Self::Unavailable => Error::HubUnavailable,
            Self::NotGranted => Error::HubNotGranted,
        }
    }
}

pub(crate) struct CredentialManager {
    transport: Arc<dyn CredentialTransport>,
    hub_module: Option<String>,
    hub_refusal: OnceLock<HubRefusal>,
    current: RwLock<Credential>,
    // Serializing issue and renewal prevents replies for an old key from
    // replacing the current credential.
    flight: Mutex<()>,
    revisions: watch::Sender<u64>,
}

impl CredentialManager {
    pub async fn start(transport: Arc<dyn CredentialTransport>) -> Result<Arc<Self>> {
        Self::start_for(transport, None).await
    }

    pub async fn start_hub(
        transport: Arc<dyn CredentialTransport>,
        module: &str,
    ) -> Result<Arc<Self>> {
        Self::start_for(transport, Some(module.to_owned())).await
    }

    async fn start_for(
        transport: Arc<dyn CredentialTransport>,
        hub_module: Option<String>,
    ) -> Result<Arc<Self>> {
        let credential = Self::issue(&*transport, hub_module.as_deref())
            .await
            .map_err(|error| {
                if hub_module.is_some() {
                    if let Some(refusal) = HubRefusal::from_error(&error) {
                        return refusal.error();
                    }
                }
                error
            })?;
        let (revisions, _) = watch::channel(0);
        Ok(Arc::new(Self {
            transport,
            hub_module,
            hub_refusal: OnceLock::new(),
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

    pub fn check_hub_refusal(&self) -> Result<()> {
        match self.hub_refusal.get() {
            Some(refusal) => Err(refusal.error()),
            None => Ok(()),
        }
    }

    fn record_refusal(&self, error: Error) -> Error {
        if self.hub_module.is_some() {
            if let Some(refusal) = HubRefusal::from_error(&error) {
                // Wake maintenance even if the refusal came from a nonce
                // callback during NATS reconnect, rather than a renewal.
                if self.hub_refusal.set(refusal).is_ok() {
                    self.revisions.send_modify(|_| {});
                }
                return refusal.error();
            }
        }
        error
    }

    async fn issue(
        transport: &dyn CredentialTransport,
        hub_module: Option<&str>,
    ) -> Result<Credential> {
        if hub_module.is_some() {
            let value = transport.call("ckbus.hub_credential", json!({})).await?;
            return Credential::parse(value, hub_module);
        }
        let mut backoff = Duration::from_millis(250);
        loop {
            match transport.call("ckbus.credential", json!({})).await {
                Ok(value) => return Credential::parse(value, None),
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
        self.check_hub_refusal()?;
        if self.current.read().await.credential_public != observed_public {
            return Ok(());
        }
        let credential = Self::issue(&*self.transport, self.hub_module.as_deref())
            .await
            .map_err(|error| self.record_refusal(error))?;
        let mut current = self.current.write().await;
        if credential.acct != current.acct {
            return Err(Error::Reply("re-issue changed the account"));
        }
        if credential.box_id != current.box_id {
            return Err(Error::Reply("re-issue changed the hub credential box id"));
        }
        *current = credential;
        self.revisions.send_modify(|revision| *revision += 1);
        Ok(())
    }

    pub async fn sign(&self, public: &str, nonce: Vec<u8>) -> Result<Vec<u8>> {
        self.check_hub_refusal()?;
        if self.current.read().await.credential_public != public {
            return Err(Error::Reply(REPLACED_CREDENTIAL));
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
                // This connection still uses the old credential. Only a replacement
                // connection may authenticate with the newly issued key.
                Err(Error::Reply(REPLACED_CREDENTIAL))
            }
            Err(error) => Err(error),
        }
    }

    pub async fn renew(&self) -> Result<()> {
        let observed = self.credential().await;
        let _flight = self.flight.lock().await;
        self.check_hub_refusal()?;
        if self.current.read().await.credential_public != observed.credential_public {
            return Ok(());
        }
        let method = if self.hub_module.is_some() {
            "ckbus.hub_credential_renew"
        } else {
            "ckbus.credential_renew"
        };
        let value = self
            .transport
            .call(
                method,
                json!({
                    "credential_public": observed.credential_public,
                }),
            )
            .await;
        match value {
            Ok(value) => {
                if let Some(box_id) = &observed.box_id {
                    for (field, expected, error) in [
                        (
                            "box_id",
                            box_id.as_str(),
                            "renewal changed the hub credential box id",
                        ),
                        (
                            "domain",
                            HUB_DOMAIN,
                            "renewal changed the hub credential domain",
                        ),
                        (
                            "inbox_prefix",
                            observed.inbox_prefix.as_str(),
                            "renewal changed the inbox prefix",
                        ),
                    ] {
                        if value
                            .get(field)
                            .is_some_and(|got| got.as_str() != Some(expected))
                        {
                            return Err(Error::Reply(error));
                        }
                    }
                }
                #[derive(Deserialize)]
                struct Renewal {
                    jwt: String,
                    exp: u64,
                    credential_public: String,
                    inbox_prefix: Option<String>,
                }
                let renewal: Renewal = serde_json::from_value(value)
                    .map_err(|_| Error::Reply("missing renewal fields"))?;
                if renewal.credential_public != observed.credential_public {
                    return Err(Error::Reply("renewal changed the credential key"));
                }
                if renewal
                    .inbox_prefix
                    .is_some_and(|prefix| prefix != observed.inbox_prefix)
                {
                    return Err(Error::Reply("renewal changed the inbox prefix"));
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
            Err(error) => Err(self.record_refusal(error)),
        }
    }
}

pub(crate) fn box_id(acct: &str) -> Option<BoxId> {
    BoxId::parse(acct.strip_prefix("box_")?).ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: &str = "0123456789abcdef0123456789abcdef";
    const DELIVER: &str = "ckdel.0123456789abcdef0123456789abcdef.example-module_2";

    fn reply(prefix: &str, acct: &str) -> Value {
        json!({"jwt":"jwt", "exp":1234, "credential_public":"UKEY0",
            "acct":acct, "server_url":"nats://127.0.0.1:4222", "inbox_prefix":prefix})
    }

    #[test]
    fn inbox_prefix_accepts_credential_and_box_module_forms() {
        for prefix in ["_INBOX.UKEY0", DELIVER] {
            let credential = Credential::parse(reply(prefix, &format!("box_{BOX}")), None).unwrap();
            assert_eq!(credential.inbox_prefix, prefix);
            assert_eq!(box_id(&credential.acct).unwrap().as_str(), BOX);
        }
        for acct in ["box_test", "another_account"] {
            let credential = Credential::parse(reply("_INBOX.UKEY0", acct), None).unwrap();
            assert_eq!(box_id(&credential.acct), None);
        }
    }

    #[test]
    fn inbox_prefix_refuses_unrelated_or_malformed_deliver_subjects() {
        for prefix in [
            "_INBOX.UKEY1",
            "ckdel.fedcba9876543210fedcba9876543210.example-module_2",
            "ckdel.0123456789abcdef0123456789abcdef.*",
            "ckdel.0123456789abcdef0123456789abcdef.>",
            "ckdel.0123456789abcdef0123456789abcdef.example.module",
            "ckdel.0123456789abcdef0123456789abcdef.",
            "ckdel.0123456789abcdef0123456789abcdef",
            "ckdel.0123456789abcdef0123456789abcdef.Example",
            "ckdel.0123456789abcdef0123456789abcdef.module name",
        ] {
            assert!(
                matches!(
                    Credential::parse(reply(prefix, &format!("box_{BOX}")), None),
                    Err(Error::Reply(
                        "credential inbox prefix does not match its key"
                    ))
                ),
                "{prefix}"
            );
        }
        for acct in [
            "box_test",
            "another_account",
            "box_0123456789ABCDEF0123456789ABCDEF",
        ] {
            assert!(
                matches!(
                    Credential::parse(reply(DELIVER, acct), None),
                    Err(Error::Reply(
                        "credential inbox prefix does not match its key"
                    ))
                ),
                "{acct}"
            );
        }
    }

    struct PrefixTransport {
        initial: &'static str,
        renewed: Option<&'static str>,
    }

    #[async_trait]
    impl CredentialTransport for PrefixTransport {
        async fn call(&self, method: &str, _: Value) -> Result<Value> {
            match method {
                "ckbus.credential" => Ok(reply(self.initial, &format!("box_{BOX}"))),
                "ckbus.credential_renew" => {
                    let mut value =
                        json!({"jwt":"renewed", "exp":5678, "credential_public":"UKEY0"});
                    if let Some(prefix) = self.renewed {
                        value["inbox_prefix"] = prefix.into();
                    }
                    Ok(value)
                }
                _ => panic!("unexpected method {method}"),
            }
        }
    }

    #[tokio::test]
    async fn inbox_prefix_renewal_accepts_omitted_or_unchanged_prefix() {
        for initial in ["_INBOX.UKEY0", DELIVER] {
            for renewed in [None, Some(initial)] {
                let manager =
                    CredentialManager::start(Arc::new(PrefixTransport { initial, renewed }))
                        .await
                        .unwrap();
                let revisions = manager.revisions();
                manager.renew().await.unwrap();
                let credential = manager.credential().await;
                assert_eq!(credential.jwt, "renewed");
                assert_eq!(credential.inbox_prefix, initial);
                assert!(!revisions.has_changed().unwrap());
            }
        }
    }

    #[tokio::test]
    async fn inbox_prefix_renewal_refuses_changed_form_or_module() {
        for (initial, renewed) in [
            ("_INBOX.UKEY0", DELIVER),
            (DELIVER, "_INBOX.UKEY0"),
            (
                DELIVER,
                "ckdel.0123456789abcdef0123456789abcdef.other-module",
            ),
        ] {
            let manager = CredentialManager::start(Arc::new(PrefixTransport {
                initial,
                renewed: Some(renewed),
            }))
            .await
            .unwrap();
            assert!(matches!(
                manager.renew().await,
                Err(Error::Reply("renewal changed the inbox prefix"))
            ));
            let credential = manager.credential().await;
            assert_eq!(credential.jwt, "jwt");
            assert_eq!(credential.inbox_prefix, initial);
        }
    }
}
