use std::fmt;
use std::time::Duration;

use async_trait::async_trait;

pub type BusResult<T> = Result<T, BusError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusError {
    Unavailable {
        reason: String,
        exhausted_limit: Option<String>,
    },
    Denied {
        subject: String,
        reason: String,
    },
    Absent {
        resource: String,
    },
    Clamped {
        clamp: String,
        retry_after: Option<Duration>,
    },
}

impl BusError {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
            exhausted_limit: None,
        }
    }

    pub fn unavailable_limit(limit: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
            exhausted_limit: Some(limit.into()),
        }
    }

    pub fn denied(subject: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Denied {
            subject: subject.into(),
            reason: reason.into(),
        }
    }

    pub fn absent(resource: impl Into<String>) -> Self {
        Self::Absent {
            resource: resource.into(),
        }
    }

    pub fn clamped(clamp: impl Into<String>, retry_after: Option<Duration>) -> Self {
        Self::Clamped {
            clamp: clamp.into(),
            retry_after,
        }
    }
}

impl fmt::Display for BusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable {
                reason,
                exhausted_limit,
            } => match exhausted_limit {
                Some(limit) => write!(formatter, "unavailable ({limit} exhausted): {reason}"),
                None => write!(formatter, "unavailable: {reason}"),
            },
            Self::Denied { subject, reason } => {
                write!(formatter, "denied for subject {subject}: {reason}")
            }
            Self::Absent { resource } => write!(formatter, "absent: {resource}"),
            Self::Clamped { clamp, retry_after } => match retry_after {
                Some(delay) => write!(formatter, "clamped ({clamp}); retry after {delay:?}"),
                None => write!(formatter, "clamped: {clamp}"),
            },
        }
    }
}

impl std::error::Error for BusError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionFailure {
    ServerClosedThenReauthenticationRefused { subject: String, reason: String },
    LostWithoutRefusal { reason: String },
    WriteOnSeveredSocket,
}

pub fn map_connection_failure(failure: ConnectionFailure) -> BusError {
    match failure {
        ConnectionFailure::ServerClosedThenReauthenticationRefused { subject, reason } => {
            BusError::denied(subject, reason)
        }
        ConnectionFailure::LostWithoutRefusal { reason } => BusError::unavailable(reason),
        ConnectionFailure::WriteOnSeveredSocket => {
            BusError::unavailable("write attempted on a severed connection")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncPermissionViolation {
    pub subject: String,
    pub reason: String,
}

impl AsyncPermissionViolation {
    pub fn as_denied(&self) -> BusError {
        BusError::denied(self.subject.clone(), self.reason.clone())
    }
}

/// Reports a permission violation observed during the request instead of its less-specific timeout.
pub fn attribute_sentinel_failure(
    requester_error: BusError,
    permission_violation: Option<&AsyncPermissionViolation>,
) -> BusError {
    permission_violation
        .map(AsyncPermissionViolation::as_denied)
        .unwrap_or(requester_error)
}

#[async_trait]
pub trait PermissionViolationSource: Send {
    async fn next_violation(&mut self) -> BusResult<Option<AsyncPermissionViolation>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSignFailure {
    RecordMissing { record: String },
    AuthorizationRejected { record: String, reason: String },
}

pub fn map_credential_sign_failure(failure: CredentialSignFailure) -> BusError {
    match failure {
        CredentialSignFailure::RecordMissing { record } => {
            BusError::absent(format!("credential signing record {record}"))
        }
        CredentialSignFailure::AuthorizationRejected { record, reason } => {
            BusError::denied(record, reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_boundary_uses_following_refusal_not_socket_timing() {
        assert!(matches!(
            map_connection_failure(ConnectionFailure::ServerClosedThenReauthenticationRefused {
                subject: "ck.a.room".into(),
                reason: "grant revoked".into(),
            }),
            BusError::Denied { .. }
        ));
        assert!(matches!(
            map_connection_failure(ConnectionFailure::LostWithoutRefusal {
                reason: "connection reset".into(),
            }),
            BusError::Unavailable { .. }
        ));
        assert!(matches!(
            map_connection_failure(ConnectionFailure::WriteOnSeveredSocket),
            BusError::Unavailable { .. }
        ));
    }

    #[test]
    fn asynchronous_permission_violation_names_offending_subject() {
        let error = AsyncPermissionViolation {
            subject: "ck.account.denied".into(),
            reason: "publish permission violation".into(),
        }
        .as_denied();

        assert!(matches!(
            error,
            BusError::Denied { subject, .. } if subject == "ck.account.denied"
        ));
    }

    #[test]
    fn signing_against_deleted_record_is_absent() {
        let error = map_credential_sign_failure(CredentialSignFailure::RecordMissing {
            record: "spawn/7".into(),
        });

        assert!(matches!(error, BusError::Absent { .. }));
    }

    #[test]
    fn sentinel_permission_violation_outranks_request_timeout() {
        let violation = AsyncPermissionViolation {
            subject: "_INBOX.UABC.reply".into(),
            reason: "publish permission violation".into(),
        };
        let attributed = attribute_sentinel_failure(
            BusError::unavailable("request timed out"),
            Some(&violation),
        );

        assert!(matches!(
            attributed,
            BusError::Denied { subject, .. } if subject == "_INBOX.UABC.reply"
        ));
        assert!(matches!(
            attribute_sentinel_failure(BusError::unavailable("request timed out"), None),
            BusError::Unavailable { .. }
        ));
    }
}
