//! The rich UI stamp a provider declares on a system-text answer.
//!
//! The phone renders an agent reply as rich UI only when the reply carries a
//! stamp naming the rich UI dialect the model was taught and the digest of
//! the catalog it was taught against; an unstamped reply renders as plain
//! Markdown. The stamp starts here: the module that owns the rich UI catalog
//! serves a system-text answer whose text teaches the catalog's tags, and
//! declares [`RichUiStamp`] on it as `system_text.rich_ui`. A runner copies
//! the stamp from the answer it froze for a run onto what that run writes.
//!
//! `dialect` is the rich UI language dialect the catalog declares about
//! itself (for example `"1.0"`). It is not the `rich-ui/1` reference scan
//! mode of the rich UI syntax crate: the two are versioned separately, so a
//! revision of the scanner never looks like a new language.

use serde::{Deserialize, Serialize};

/// The longest `dialect` a stamp may carry, in characters (each is one byte).
pub const MAX_DIALECT_LEN: usize = 32;

/// The exact length of `catalog_digest`: SHA-256 as lowercase hex.
pub const CATALOG_DIGEST_LEN: usize = 64;

/// The stamp's member names, as a refusal names them.
pub mod fields {
    /// The stamp's dialect member.
    pub const DIALECT: &str = "dialect";
    /// The stamp's catalog digest member.
    pub const CATALOG_DIGEST: &str = "catalog_digest";
}

/// `{dialect, catalog_digest}`: which rich UI dialect a system text teaches,
/// and against which catalog.
///
/// Both values are validated on construction and on decode, so a held stamp
/// is always well formed. A provider copies both from the catalog owner's
/// own published export and never re-derives them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawRichUiStamp")]
pub struct RichUiStamp {
    dialect: String,
    catalog_digest: String,
}

/// Why a stamp value is malformed. [`RichUiStampError::field`] names the
/// member at fault.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RichUiStampError {
    /// `dialect` is empty, longer than [`MAX_DIALECT_LEN`], or holds a
    /// character outside printable ASCII (0x21-0x7E), whitespace included.
    Dialect,
    /// `catalog_digest` is not exactly [`CATALOG_DIGEST_LEN`] lowercase hex
    /// characters with no prefix.
    CatalogDigest,
}

impl RichUiStampError {
    /// The stamp member at fault: `dialect` or `catalog_digest`.
    pub fn field(&self) -> &'static str {
        match self {
            Self::Dialect => fields::DIALECT,
            Self::CatalogDigest => fields::CATALOG_DIGEST,
        }
    }
}

impl std::fmt::Display for RichUiStampError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dialect => write!(
                f,
                "rich_ui.{}: must be 1-{MAX_DIALECT_LEN} printable ASCII characters with no whitespace",
                fields::DIALECT
            ),
            Self::CatalogDigest => write!(
                f,
                "rich_ui.{}: must be exactly {CATALOG_DIGEST_LEN} lowercase hex characters with no prefix",
                fields::CATALOG_DIGEST
            ),
        }
    }
}

impl std::error::Error for RichUiStampError {}

/// Whether `dialect` may be a stamp's dialect: 1 to 32 characters, each
/// printable ASCII other than space (0x21-0x7E).
pub fn valid_dialect(dialect: &str) -> bool {
    (1..=MAX_DIALECT_LEN).contains(&dialect.len())
        && dialect.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

/// Whether `digest` may be a stamp's catalog digest: exactly 64 lowercase
/// hex characters, no `sha256:` or other prefix.
pub fn valid_catalog_digest(digest: &str) -> bool {
    digest.len() == CATALOG_DIGEST_LEN
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl RichUiStamp {
    /// A stamp for `dialect` and `catalog_digest`, refused naming the first
    /// malformed member (dialect is checked first).
    pub fn new(
        dialect: impl Into<String>,
        catalog_digest: impl Into<String>,
    ) -> Result<Self, RichUiStampError> {
        let dialect = dialect.into();
        let catalog_digest = catalog_digest.into();
        if !valid_dialect(&dialect) {
            return Err(RichUiStampError::Dialect);
        }
        if !valid_catalog_digest(&catalog_digest) {
            return Err(RichUiStampError::CatalogDigest);
        }
        Ok(Self {
            dialect,
            catalog_digest,
        })
    }

    /// The rich UI language dialect the text teaches.
    pub fn dialect(&self) -> &str {
        &self.dialect
    }

    /// The SHA-256 the catalog's owner publishes over its canonical catalog.
    pub fn catalog_digest(&self) -> &str {
        &self.catalog_digest
    }
}

/// The unchecked wire form; decoding goes through [`RichUiStamp::new`].
/// Members are read as any JSON value so that a missing or non-string member
/// is refused by the same error, naming it, as a malformed string. Members
/// this release does not know are ignored.
#[derive(Deserialize)]
struct RawRichUiStamp {
    #[serde(default)]
    dialect: Option<serde_json::Value>,
    #[serde(default)]
    catalog_digest: Option<serde_json::Value>,
}

impl TryFrom<RawRichUiStamp> for RichUiStamp {
    type Error = RichUiStampError;

    fn try_from(raw: RawRichUiStamp) -> Result<Self, Self::Error> {
        let Some(serde_json::Value::String(dialect)) = raw.dialect else {
            return Err(RichUiStampError::Dialect);
        };
        let Some(serde_json::Value::String(catalog_digest)) = raw.catalog_digest else {
            return Err(RichUiStampError::CatalogDigest);
        };
        Self::new(dialect, catalog_digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_valid_stamp_round_trips_with_exactly_its_two_members() {
        let stamp = RichUiStamp::new("1.0", DIGEST).unwrap();
        let wire = serde_json::to_value(&stamp).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"dialect": "1.0", "catalog_digest": DIGEST})
        );
        assert_eq!(serde_json::from_value::<RichUiStamp>(wire).unwrap(), stamp);
    }

    #[test]
    fn dialect_bounds_are_one_to_thirty_two_printable_characters() {
        assert!(valid_dialect("x"));
        assert!(valid_dialect(&"~".repeat(MAX_DIALECT_LEN)));
        assert!(!valid_dialect(""));
        assert!(!valid_dialect(&"a".repeat(MAX_DIALECT_LEN + 1)));
        assert!(!valid_dialect("1 0"));
        assert!(!valid_dialect("1\t0"));
        assert!(!valid_dialect("1.0\u{7f}"));
        assert!(!valid_dialect("é"));
    }

    #[test]
    fn catalog_digest_is_exactly_sixty_four_lowercase_hex() {
        assert!(valid_catalog_digest(DIGEST));
        assert!(!valid_catalog_digest(&DIGEST.to_uppercase()));
        assert!(!valid_catalog_digest(&DIGEST[1..]));
        assert!(!valid_catalog_digest(&format!("{DIGEST}0")));
        assert!(!valid_catalog_digest(&format!("sha256:{}", &DIGEST[7..])));
    }

    #[test]
    fn a_decode_refusal_names_the_member_at_fault() {
        let bad_dialect = serde_json::json!({"dialect": "", "catalog_digest": DIGEST});
        let error = serde_json::from_value::<RichUiStamp>(bad_dialect).unwrap_err();
        assert!(error.to_string().contains("rich_ui.dialect"), "{error}");
        let bad_digest = serde_json::json!({"dialect": "1.0", "catalog_digest": "abc"});
        let error = serde_json::from_value::<RichUiStamp>(bad_digest).unwrap_err();
        assert!(
            error.to_string().contains("rich_ui.catalog_digest"),
            "{error}"
        );
        assert_eq!(RichUiStamp::new("", "x").unwrap_err().field(), "dialect");
        assert_eq!(
            RichUiStamp::new("1.0", "x").unwrap_err().field(),
            "catalog_digest"
        );
    }
}
