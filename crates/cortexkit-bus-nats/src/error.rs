use async_nats::{Event, ServerError};
use cortexkit_bus_trait::{AsyncPermissionViolation, BusError};
use tokio::sync::broadcast;
use tokio::time::{Duration, Instant};

pub(crate) fn permission_violation(event: &Event) -> Option<AsyncPermissionViolation> {
    match event {
        Event::ServerError(ServerError::AuthorizationViolation) => Some(AsyncPermissionViolation {
            subject: "connection".into(),
            reason: "server authorization violation".into(),
        }),
        Event::ServerError(ServerError::Other(reason)) if is_permission_reason(reason) => {
            Some(AsyncPermissionViolation {
                subject: quoted_subject(reason).unwrap_or_else(|| "unknown".into()),
                reason: reason.clone(),
            })
        }
        _ => None,
    }
}

fn subscription_permission_violation(
    event: &Event,
    subject: &str,
) -> Option<AsyncPermissionViolation> {
    let violation = permission_violation(event)?;
    if matches!(
        event,
        Event::ServerError(ServerError::AuthorizationViolation)
    ) || matches!(
        event,
        Event::ServerError(ServerError::Other(reason))
            if reason.starts_with("Permissions Violation for Subscription to \"")
                && quoted_subject(reason).as_deref() == Some(subject)
    ) {
        Some(violation)
    } else {
        tracing::debug!(
            ignored_subject = %violation.subject,
            subscription_subject = subject,
            reason = %violation.reason,
            "ignoring permission violation unrelated to this subscription"
        );
        None
    }
}

pub(crate) async fn wait_for_subscription_permission_violation(
    events: &mut broadcast::Receiver<Event>,
    subject: &str,
    wait: Duration,
) -> Option<AsyncPermissionViolation> {
    let deadline = Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(event)) => {
                if let Some(violation) = subscription_permission_violation(&event, subject) {
                    return Some(violation);
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return None,
        }
    }
}

pub(crate) fn map_connect_error(error: impl std::fmt::Display) -> BusError {
    let reason = error.to_string();
    if reason
        .to_ascii_lowercase()
        .contains("authorization violation")
    {
        BusError::denied("connection", reason)
    } else {
        BusError::unavailable(reason)
    }
}

pub(crate) fn map_operation_error(
    subject: &str,
    error: impl std::fmt::Display,
    events: &mut broadcast::Receiver<Event>,
) -> BusError {
    if let Some(violation) = drain_permission_violation(events) {
        return violation.as_denied();
    }

    let reason = error.to_string();
    if is_permission_reason(&reason)
        || reason
            .to_ascii_lowercase()
            .contains("authorization violation")
    {
        BusError::denied(subject, reason)
    } else if names_max_bytes_limit(&reason) {
        BusError::unavailable_limit("max_bytes", reason)
    } else {
        BusError::unavailable(reason)
    }
}

pub(crate) fn drain_permission_violation(
    events: &mut broadcast::Receiver<Event>,
) -> Option<AsyncPermissionViolation> {
    loop {
        match events.try_recv() {
            Ok(event) => {
                if let Some(violation) = permission_violation(&event) {
                    return Some(violation);
                }
            }
            Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                return None;
            }
        }
    }
}

fn is_permission_reason(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("permissions violation") || lower.contains("permission violation")
}

fn names_max_bytes_limit(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("maximum bytes")
        || lower.contains("max bytes")
        || lower.contains("maximum storage")
        || lower.contains("insufficient storage")
}

fn quoted_subject(reason: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        let mut pieces = reason.split(quote);
        let _before = pieces.next()?;
        if let Some(candidate) = pieces.next() {
            if !candidate.is_empty() {
                return Some(candidate.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_only_own_subscription_or_connection_permission_violations() {
        // nats-server 2.15.0 server/client.go constructs the subscription error
        // with `Permissions Violation for Subscription to %q`.
        let cases = [
            (
                "Permissions Violation for Subscription to \"events.mine\"",
                "events.mine",
                true,
            ),
            (
                "Permissions Violation for Subscription to \"events.other\"",
                "events.mine",
                false,
            ),
            (
                "Permissions Violation for Subscription to \"events.Mine\"",
                "events.mine",
                false,
            ),
            (
                "Permissions Violation for Publish to \"events.mine\"",
                "events.mine",
                false,
            ),
            (
                "Permissions Violation for Subscription to events.mine",
                "events.mine",
                false,
            ),
            (
                "Permissions Violation for Subscription to unknown",
                "unknown",
                false,
            ),
            (
                "Permissions Violation for Subscription to \"\"",
                "unknown",
                false,
            ),
            (
                "Permissions Violation for Publish to \"connection\"",
                "events.mine",
                false,
            ),
        ];
        for (reason, subject, expected) in cases {
            let event = Event::ServerError(ServerError::Other(reason.into()));
            let violation = subscription_permission_violation(&event, subject);
            assert_eq!(violation.is_some(), expected, "{reason}");
            if let Some(violation) = violation {
                assert_eq!(violation.subject, subject);
                assert_eq!(violation.reason, reason);
            }
        }
        let violation = subscription_permission_violation(
            &Event::ServerError(ServerError::AuthorizationViolation),
            "events.mine",
        )
        .expect("connection authorization failure applies to every subscription");
        assert_eq!(violation.subject, "connection");
    }

    #[tokio::test]
    async fn subscription_permission_wait_checks_past_unrelated_events() {
        let (sender, mut receiver) = broadcast::channel(2);
        for reason in [
            "Permissions Violation for Publish to \"events.mine\"",
            "Permissions Violation for Subscription to \"events.mine\"",
        ] {
            sender
                .send(Event::ServerError(ServerError::Other(reason.into())))
                .expect("event receiver exists");
        }
        let violation = wait_for_subscription_permission_violation(
            &mut receiver,
            "events.mine",
            Duration::from_millis(100),
        )
        .await
        .expect("own subscription denial follows unrelated publish denial");
        assert_eq!(violation.subject, "events.mine");
        assert_eq!(
            violation.reason,
            "Permissions Violation for Subscription to \"events.mine\""
        );
    }

    #[test]
    fn extracts_subject_from_server_permission_text() {
        let event = Event::ServerError(ServerError::Other(
            "Permissions Violation for Subscription to \"_INBOX.random\"".into(),
        ));
        assert!(matches!(
            permission_violation(&event),
            Some(AsyncPermissionViolation { subject, .. }) if subject == "_INBOX.random"
        ));
    }

    #[test]
    fn severed_write_stays_unavailable() {
        let (sender, mut receiver) = broadcast::channel(1);
        drop(sender);
        assert!(matches!(
            map_operation_error(
                "ck.box.peer.a.s.deliver",
                "connection closed",
                &mut receiver
            ),
            BusError::Unavailable { .. }
        ));
    }
}
