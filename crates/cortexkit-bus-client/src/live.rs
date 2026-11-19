use async_nats::Subscriber;
use bytes::Bytes;
use futures_util::StreamExt;

use crate::{binding::ConnectionBinding, BusClient, Error, Result};

/// A stream of payloads from one exact live-hint subject. Delivery subjects are
/// checked before yielding so pushed JetStream records cannot become live hints.
pub struct LiveHints {
    binding: ConnectionBinding,
    subject: String,
    subscription: Option<Subscriber>,
    forged_warning_logged: bool,
}

impl LiveHints {
    pub(crate) async fn attach(
        client: &BusClient,
        module: &str,
        name: &str,
        version: u32,
    ) -> Result<Self> {
        let subject = client.names.live_subject(module, name, version)?;
        let mut hints = Self {
            binding: ConnectionBinding::new(client),
            subject,
            subscription: None,
            forged_warning_logged: false,
        };
        hints.bind().await?;
        Ok(hints)
    }

    async fn bind(&mut self) -> Result<()> {
        self.subscription = None;
        let subject = self.subject.clone();
        let subscription = self
            .binding
            .bind(|connection| {
                let subject = subject.clone();
                async move {
                    connection
                        .client()
                        .subscribe(subject)
                        .await
                        .map_err(|error| Error::Nats(error.to_string()))
                }
            })
            .await?;
        self.subscription = Some(subscription);
        Ok(())
    }

    /// Returns the next verified hint payload, rebinding after credential
    /// re-issue and waiting for a replacement when the connection ends for good.
    pub async fn next(&mut self) -> Result<Bytes> {
        loop {
            if self.binding.has_changed() || self.subscription.is_none() {
                self.bind().await?;
            }
            tokio::select! {
                biased;
                changed = self.binding.changed() => {
                    changed?;
                    self.subscription = None;
                }
                message = self.subscription.as_mut().expect("bound live hints").next() => {
                    let Some(message) = message else {
                        self.binding.recover(Error::Nats("live hint subscription closed".into())).await?;
                        self.subscription = None;
                        continue;
                    };
                    if message.subject.as_str() != self.subject {
                        self.binding.drop_forged(
                            &mut self.forged_warning_logged,
                            &self.subject,
                            message.subject.as_str(),
                        );
                        continue;
                    }
                    return Ok(message.payload);
                }
            }
        }
    }
}
