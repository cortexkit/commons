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

pub(crate) async fn wait_for_permission_violation(
    events: &mut broadcast::Receiver<Event>,
    wait: Duration,
) -> Option<AsyncPermissionViolation> {
    let deadline = Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(event)) => {
                if let Some(violation) = permission_violation(&event) {
                    return Some(violation);
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return None,
        }
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
