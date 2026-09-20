use std::error::Error;
use std::fmt;

/// The identity field being interpolated into a NATS name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Account,
    AgentId,
    SessionId,
    RoomId,
    ModuleId,
    RosterHostId,
    CredentialPublic,
}

impl TokenKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Account => "acct",
            Self::AgentId => "agent_id",
            Self::SessionId => "session_id",
            Self::RoomId => "room_id",
            Self::ModuleId => "module_id",
            Self::RosterHostId => "roster_host_id",
            Self::CredentialPublic => "credential_public",
        }
    }
}

/// A token that cannot be interpolated without changing or broadening identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamingError {
    kind: TokenKind,
    token: String,
    reason: &'static str,
}

impl NamingError {
    pub fn kind(&self) -> TokenKind {
        self.kind
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }

    fn new(kind: TokenKind, token: &str, reason: &'static str) -> Self {
        Self {
            kind,
            token: token.to_owned(),
            reason,
        }
    }
}

impl fmt::Display for NamingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid {} token {:?}: {}",
            self.kind.label(),
            self.token,
            self.reason
        )
    }
}

impl Error for NamingError {}

/// Validates the account-only lexicon `[a-z0-9][a-z0-9_]{0,62}`.
pub fn validate_account_token(token: &str) -> Result<(), NamingError> {
    validate_len_and_first(TokenKind::Account, token)?;
    if token
        .bytes()
        .skip(1)
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        Ok(())
    } else {
        Err(NamingError::new(
            TokenKind::Account,
            token,
            "expected [a-z0-9][a-z0-9_]{0,62}; hyphens and normalization are forbidden",
        ))
    }
}

/// Validates the shared identity lexicon `[a-z0-9][a-z0-9_-]{0,62}`.
///
/// `CredentialPublic` is an inbox-prefix component rather than a registry token;
/// it accepts ASCII letters because real NATS public nkeys are uppercase.
pub fn validate_token(kind: TokenKind, token: &str) -> Result<(), NamingError> {
    if kind == TokenKind::Account {
        return validate_account_token(token);
    }
    if kind == TokenKind::CredentialPublic {
        return validate_credential_public(token);
    }

    validate_len_and_first(kind, token)?;
    if token.bytes().skip(1).all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
    }) {
        Ok(())
    } else {
        Err(NamingError::new(
            kind,
            token,
            "expected [a-z0-9][a-z0-9_-]{0,62}; dots, wildcards, whitespace, and normalization are forbidden",
        ))
    }
}

fn validate_len_and_first(kind: TokenKind, token: &str) -> Result<(), NamingError> {
    if token.is_empty() || token.len() > 63 {
        return Err(NamingError::new(
            kind,
            token,
            "token must contain between 1 and 63 ASCII bytes",
        ));
    }
    let first = token.as_bytes()[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(NamingError::new(
            kind,
            token,
            "token must start with a lowercase ASCII letter or digit",
        ));
    }
    Ok(())
}

fn validate_credential_public(token: &str) -> Result<(), NamingError> {
    if token.is_empty() || token.len() > 63 {
        return Err(NamingError::new(
            TokenKind::CredentialPublic,
            token,
            "inbox component must contain between 1 and 63 ASCII bytes",
        ));
    }
    if token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Ok(())
    } else {
        Err(NamingError::new(
            TokenKind::CredentialPublic,
            token,
            "inbox component may contain only ASCII letters, digits, underscore, or hyphen",
        ))
    }
}
