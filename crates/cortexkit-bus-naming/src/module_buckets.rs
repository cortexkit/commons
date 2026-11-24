use std::collections::BTreeSet;

use crate::{validate_token, AccountNames, NamingError, TokenKind};

// Module buckets have only a handful of owner and reader watches. Cap each
// bucket's consumers with room for more readers and watch overlap on reconnect.
const MODULE_BUCKET_MAX_CONSUMERS: i32 = 32;

/// A reviewed KV bucket. Only `owner` may write; listed modules may read.
/// Limits describe stored values and retention, not transport overhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModuleBucket {
    pub owner: &'static str,
    pub name: &'static str,
    pub readers: &'static [&'static str],
    pub max_value_bytes: u32,
    pub history: u8,
    pub ttl: Option<u64>,
    pub max_bytes: u64,
}

impl ModuleBucket {
    /// Maximum JetStream consumers on this bucket's backing stream.
    pub const fn max_consumers(&self) -> i32 {
        MODULE_BUCKET_MAX_CONSUMERS
    }

    /// `ttl` is in seconds; `None` retains values without an age limit.
    pub const fn new(
        owner: &'static str,
        name: &'static str,
        readers: &'static [&'static str],
        max_value_bytes: u32,
        history: u8,
        ttl: Option<u64>,
        max_bytes: u64,
    ) -> Self {
        Self {
            owner,
            name,
            readers,
            max_value_bytes,
            history,
            ttl,
            max_bytes,
        }
    }
}

/// Closed bucket declarations; runtime configuration cannot add writers or readers.
/// Byte caps budget 1,024 status keys, 128 multipliers, 65,536 read marks,
/// 65,536 changes, 16,384 session-liveness keys, and 128 schema pins per views
/// bucket at their maximum value size. Each views key is the lowercase hex
/// encoding of the whole schema pin string's UTF-8 bytes; its value is exactly
/// what `tool.views` serves for that pin. Budgeting 128 pins leaves room for multiple tools and
/// schema revisions while bounding each views bucket to 32 MiB.
pub const MODULE_BUCKETS: &[ModuleBucket] = &[
    ModuleBucket::new(
        "aft",
        "status",
        &["prefrontal"],
        2_048,
        1,
        Some(10),
        2_048 * 1_024,
    ),
    ModuleBucket::new(
        "astrocyte",
        "multiplier",
        &["striatum"],
        65_536,
        1,
        None,
        65_536 * 128,
    ),
    ModuleBucket::new(
        "astrocyte",
        "read-marks",
        &["thalamus-harness"],
        256,
        1,
        None,
        256 * 65_536,
    ),
    ModuleBucket::new(
        "entorhinal",
        "changes",
        &["prefrontal", "plexus", "magic-context"],
        512,
        1,
        None,
        512 * 65_536,
    ),
    ModuleBucket::new(
        "prefrontal",
        "session-liveness",
        &["entorhinal"],
        1_024,
        1,
        None,
        1_024 * 16_384,
    ),
    ModuleBucket::new(
        "aft",
        "views",
        &["occipital"],
        256 * 1_024,
        1,
        None,
        256 * 1_024 * 128,
    ),
    ModuleBucket::new(
        "cerebellum",
        "views",
        &["occipital"],
        256 * 1_024,
        1,
        None,
        256 * 1_024 * 128,
    ),
];

impl AccountNames {
    /// `CK_{ACCT}_{OWNER}_{NAME}`, uppercased with `-` and `.` replaced by `_`.
    /// Owner is a module id; the local name is one or more valid identity tokens
    /// separated by dots. Normalization applies only to the derived bucket name.
    pub fn module_bucket_name(&self, owner: &str, name: &str) -> Result<String, NamingError> {
        validate_token(TokenKind::ModuleId, owner)?;
        for level in name.split('.') {
            validate_token(TokenKind::BucketName, level)?;
        }
        let bucket = format!("CK_{}_{owner}_{name}", self.account())
            .to_ascii_uppercase()
            .replace(['-', '.'], "_");
        validate_module_bucket_name(&bucket)?;
        Ok(bucket)
    }
}

/// Validates the derived bucket spelling, not ownership or table membership.
/// Underscore normalization is irreversible; use [`validate_module_buckets`]
/// to reject colliding declarations for an account.
pub fn validate_module_bucket_name(name: &str) -> Result<(), NamingError> {
    let tail = name.strip_prefix("CK_").unwrap_or_default();
    if tail.is_empty()
        || !tail
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        || !tail
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        || tail.split('_').filter(|part| !part.is_empty()).count() < 3
    {
        return Err(NamingError::new(
            TokenKind::BucketName,
            name,
            "expected CK_{ACCT}_{OWNER}_{NAME} in uppercase underscore-normalized form",
        ));
    }
    Ok(())
}

/// Validates bucket identities, unique normalized names, reader ids and limits.
pub fn validate_module_buckets(
    account: &AccountNames,
    buckets: &[ModuleBucket],
) -> Result<(), NamingError> {
    let mut names = BTreeSet::new();
    for bucket in buckets {
        let name = account.module_bucket_name(bucket.owner, bucket.name)?;
        if !names.insert(name) {
            return Err(NamingError::new(
                TokenKind::BucketName,
                bucket.name,
                "duplicate normalized bucket name",
            ));
        }
        let mut readers = BTreeSet::new();
        for reader in bucket.readers {
            validate_token(TokenKind::ModuleId, reader)?;
            if *reader == bucket.owner || !readers.insert(reader) {
                return Err(NamingError::new(
                    TokenKind::ModuleId,
                    reader,
                    "bucket readers must be unique and distinct from the owner",
                ));
            }
        }
        if bucket.max_value_bytes == 0
            || !(1..=64).contains(&bucket.history)
            || bucket.ttl == Some(0)
            || bucket.max_bytes < u64::from(bucket.max_value_bytes) * u64::from(bucket.history)
        {
            return Err(NamingError::new(TokenKind::BucketName, bucket.name, "bucket needs positive value and byte caps, history 1..=64, and a positive TTL if set"));
        }
    }
    Ok(())
}
