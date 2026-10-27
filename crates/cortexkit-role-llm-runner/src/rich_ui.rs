//! The rich UI stamp: which rich UI dialect, and which catalog, an assistant
//! reply was taught to write (CONTRACT §9.17, the `rich_ui_stamp` group).
//!
//! A reader renders an assistant reply as rich UI only when the reply carries
//! a stamp; an unstamped reply renders as plain Markdown. The stamp starts at
//! the module that owns the rich UI catalog: it serves a `tool-provider/v1`
//! system-text answer that teaches the catalog's tags, and declares the stamp
//! on that answer as `system_text.rich_ui`. A plan composer includes such a
//! stamped teaching item only in plans for a runner declaring
//! `rich_ui_stamp`. That runner copies the stamp, verbatim, from the
//! system-text answers it fetched and froze for a run, and serves it beside
//! each assistant message the run writes:
//!
//! - `display::DisplayRow::rich_ui` on every assistant row of the run;
//! - `live::RunStarted::rich_ui`;
//! - `read::HeadMeta::last_run_rich_ui` while `last_run_state` names the run;
//! - `read::ReadMessage::rich_ui` on the run's assistant entries, only when
//!   the `session.read` request opts in with `include_rich_ui: true`.
//!
//! [`RichUiStamp`] has exactly the wire form of the tool-provider crate's
//! type of the same name. This crate does not depend on the tool-provider
//! crate, so it defines its own copy; a test decodes the same vectors with
//! both and checks they agree.
//!
//! `dialect` is the rich UI language dialect the catalog declares about
//! itself (for example `"1.0"`). It is not the `rich-ui/1` reference scan
//! mode of the rich UI syntax crate: the two are versioned separately, so a
//! revision of the scanner never looks like a new language.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

/// `{dialect, catalog_digest}`: which rich UI dialect a run's system text
/// taught, and against which catalog.
///
/// Both values are validated on construction and on decode, so a held stamp
/// is always well formed. A runner never builds one from parts: it copies the
/// stamp it fetched.
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
    /// `dialect` is missing, not a string, empty, longer than
    /// [`MAX_DIALECT_LEN`], or holds a character outside printable ASCII
    /// (0x21-0x7E), whitespace included.
    Dialect,
    /// `catalog_digest` is missing, not a string, or not exactly
    /// [`CATALOG_DIGEST_LEN`] lowercase hex characters with no prefix.
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

    /// The rich UI language dialect the run's system text taught.
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
    dialect: Option<Value>,
    #[serde(default)]
    catalog_digest: Option<Value>,
}

impl TryFrom<RawRichUiStamp> for RichUiStamp {
    type Error = RichUiStampError;

    fn try_from(raw: RawRichUiStamp) -> Result<Self, Self::Error> {
        let Some(Value::String(dialect)) = raw.dialect else {
            return Err(RichUiStampError::Dialect);
        };
        let Some(Value::String(catalog_digest)) = raw.catalog_digest else {
            return Err(RichUiStampError::CatalogDigest);
        };
        Self::new(dialect, catalog_digest)
    }
}

/// The `kind` of a `plan_stale` difference that names a stamped system-text
/// item.
pub const DIFFERENCE_KIND: &str = "rich_ui";

/// Two fetched system-text answers in one plan declare different stamps.
///
/// A run carries one stamp, so a runner declaring `rich_ui_stamp` refuses
/// such a plan at admission, before anything is written, as
/// [`crate::errors::PLAN_STALE`] with [`RichUiConflict::detail`]:
/// `{differences: [{kind: "rich_ui", provider, rich_ui}, ...]}`, one entry
/// per stamped answer, in plan order. Answers declaring identical stamps
/// never conflict, and an answer declaring none takes no part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichUiConflict {
    /// Each stamped answer: the module id of the provider that served it,
    /// and the stamp it declared.
    pub stamped: Vec<(String, RichUiStamp)>,
}

impl RichUiConflict {
    /// The refusal code: `plan_stale`.
    pub fn code(&self) -> &'static str {
        crate::errors::PLAN_STALE
    }

    /// The refusal's `detail`.
    pub fn detail(&self) -> Value {
        let differences: Vec<Value> = self
            .stamped
            .iter()
            .map(|(provider, stamp)| {
                json!({ "kind": DIFFERENCE_KIND, "provider": provider, "rich_ui": stamp })
            })
            .collect();
        json!({ "differences": differences })
    }
}

/// The stamp a plan's run carries: the one stamp every stamped fetched
/// system-text answer agrees on, `None` when no answer declares one, or the
/// conflict to refuse the plan with when two declare different stamps.
///
/// `answers` are the plan's fetched system-text answers, in plan order, each
/// with the module id of the provider that served it and the stamp it
/// declared, if any.
pub fn plan_stamp<'a, I>(answers: I) -> Result<Option<RichUiStamp>, RichUiConflict>
where
    I: IntoIterator<Item = (&'a str, Option<&'a RichUiStamp>)>,
{
    let stamped: Vec<(String, RichUiStamp)> = answers
        .into_iter()
        .filter_map(|(provider, stamp)| Some((provider.to_owned(), stamp?.clone())))
        .collect();
    let Some((_, first)) = stamped.first() else {
        return Ok(None);
    };
    if stamped.iter().all(|(_, stamp)| stamp == first) {
        return Ok(Some(first.clone()));
    }
    Err(RichUiConflict { stamped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn valid_stamps_decode_and_round_trip() {
        let file = vectors::load("rich-ui-stamp.json");
        for case in vectors::cases(&file, "valid_stamps") {
            let name = case["name"].as_str().unwrap();
            let stamp: RichUiStamp = vectors::round_trip(name, &case["rich_ui"]);
            assert_eq!(
                RichUiStamp::new(stamp.dialect(), stamp.catalog_digest()).as_ref(),
                Ok(&stamp),
                "{name}"
            );
        }
    }

    #[test]
    fn malformed_stamps_are_refused_naming_the_field() {
        let file = vectors::load("rich-ui-stamp.json");
        for case in vectors::cases(&file, "refused_stamps") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            let error = serde_json::from_value::<RichUiStamp>(case["rich_ui"].clone())
                .expect_err(name)
                .to_string();
            assert!(
                error.contains(&format!("rich_ui.{field}")),
                "{name}: {error}"
            );
        }
    }

    /// This crate's stamp and the tool-provider crate's stamp have the same
    /// wire form: each decodes every stamp the other's vectors carry to the
    /// same members, re-encodes it to the same JSON, and refuses the same
    /// malformed stamps naming the same member.
    #[test]
    fn the_tool_provider_stamp_and_this_stamp_agree_on_both_vector_files() {
        use cortexkit_role_tool_provider::rich_ui::RichUiStamp as ProviderStamp;
        let ours = vectors::load("rich-ui-stamp.json");
        let theirs: Value = serde_json::from_str(
            cortexkit_role_tool_provider::vectors::file("rich-ui-stamp.json").unwrap(),
        )
        .unwrap();
        let valid = vectors::cases(&ours, "valid_stamps")
            .iter()
            .chain(vectors::cases(&theirs, "valid"));
        for case in valid {
            let name = case["name"].as_str().unwrap();
            let runner: RichUiStamp = serde_json::from_value(case["rich_ui"].clone()).unwrap();
            let provider: ProviderStamp = serde_json::from_value(case["rich_ui"].clone()).unwrap();
            assert_eq!(runner.dialect(), provider.dialect(), "{name}");
            assert_eq!(runner.catalog_digest(), provider.catalog_digest(), "{name}");
            assert_eq!(
                serde_json::to_value(&runner).unwrap(),
                serde_json::to_value(&provider).unwrap(),
                "{name}"
            );
        }
        let refused = vectors::cases(&ours, "refused_stamps")
            .iter()
            .chain(vectors::cases(&theirs, "refused"));
        for case in refused {
            let name = case["name"].as_str().unwrap();
            let runner = serde_json::from_value::<RichUiStamp>(case["rich_ui"].clone())
                .expect_err(name)
                .to_string();
            let provider = serde_json::from_value::<ProviderStamp>(case["rich_ui"].clone())
                .expect_err(name)
                .to_string();
            assert_eq!(runner, provider, "{name}");
        }
        // The tool-provider vector's stamped system-text answer carries a
        // stamp this crate reads as is.
        let answer = &vectors::cases(&theirs, "answers")[1];
        let copied: RichUiStamp =
            serde_json::from_value(answer["system_text"]["rich_ui"].clone()).unwrap();
        assert_eq!(serde_json::to_value(copied).unwrap(), answer["rich_ui"]);
    }

    #[test]
    fn a_plan_carries_the_stamp_its_answers_agree_on_and_refuses_a_conflict() {
        let file = vectors::load("rich-ui-stamp.json");
        for case in vectors::cases(&file, "plans") {
            let name = case["name"].as_str().unwrap();
            let answers: Vec<(String, Option<RichUiStamp>)> = case["answers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|answer| {
                    (
                        answer["provider"].as_str().unwrap().to_owned(),
                        answer
                            .get("rich_ui")
                            .map(|stamp| serde_json::from_value(stamp.clone()).unwrap()),
                    )
                })
                .collect();
            let result = plan_stamp(
                answers
                    .iter()
                    .map(|(provider, stamp)| (provider.as_str(), stamp.as_ref())),
            );
            match (&case["rich_ui"], &case["refusal"]) {
                (stamp, Value::Null) => {
                    let expected: Option<RichUiStamp> =
                        serde_json::from_value(stamp.clone()).unwrap();
                    assert_eq!(result, Ok(expected), "{name}");
                }
                (_, refusal) => {
                    let conflict = result.expect_err(name);
                    assert_eq!(refusal["code"], conflict.code(), "{name}");
                    assert_eq!(refusal["detail"], conflict.detail(), "{name}");
                    assert!(crate::errors::CODES.contains(&conflict.code()), "{name}");
                    assert!(!crate::errors::is_retryable(conflict.code()), "{name}");
                }
            }
        }
    }

    fn stamp_of(case: &Value) -> Option<RichUiStamp> {
        case.get("rich_ui")
            .filter(|stamp| !stamp.is_null())
            .map(|stamp| serde_json::from_value(stamp.clone()).unwrap())
    }

    #[test]
    fn the_group_extends_read_and_head_and_is_refused_without_them() {
        use crate::{
            capabilities,
            describe::{check_describe, DescribeProblem},
            ops,
        };
        let file = vectors::load("rich-ui-stamp.json");
        let describe = &file["describe"];
        for name in ["accept", "read_and_head_only"] {
            let answer = check_describe(&describe[name]).unwrap();
            assert!(answer.declares(capabilities::RICH_UI_STAMP), "{name}");
        }
        let without = check_describe(&describe["without_group"]).unwrap();
        assert!(!without.declares(capabilities::RICH_UI_STAMP));
        let refused = &describe["group_incomplete"];
        assert_eq!(refused["capability"], capabilities::RICH_UI_STAMP);
        assert_eq!(refused["op"], ops::SESSION_HEAD);
        assert_eq!(
            check_describe(&refused["answer"]),
            Err(vec![DescribeProblem::GroupIncomplete {
                capability: capabilities::RICH_UI_STAMP,
                op: ops::SESSION_HEAD,
            }])
        );
        assert_eq!(
            capabilities::group_ops(capabilities::RICH_UI_STAMP),
            Some(&[ops::SESSION_READ, ops::SESSION_HEAD][..])
        );
    }

    /// Stamped rows carry the stamp beside the row's other members; an
    /// unstamped row has no `rich_ui` member at all, so its bytes are those
    /// of a runner without the group.
    #[test]
    fn display_rows_carry_the_stamp_or_leave_the_member_out() {
        use crate::display::DisplayRow;
        let file = vectors::load("rich-ui-stamp.json");
        for case in vectors::cases(&file, "display_rows") {
            let name = case["name"].as_str().unwrap();
            let row: DisplayRow = vectors::round_trip(name, &case["row"]);
            assert_eq!(row.rich_ui, stamp_of(case), "{name}");
            assert_eq!(
                case["row"].get("rich_ui").is_some(),
                row.rich_ui.is_some(),
                "{name}"
            );
            let mut built = DisplayRow::new(&row.role, row.parts.clone());
            if let Some(model) = &row.model {
                built = built.with_model(model);
            }
            if let Some(stamp) = stamp_of(case) {
                built = built.with_rich_ui(stamp);
            }
            assert_eq!(built, row, "{name}");
        }
    }

    #[test]
    fn run_started_carries_the_stamp_or_leaves_the_member_out() {
        use crate::live::{LiveEvent, RunStarted};
        let file = vectors::load("rich-ui-stamp.json");
        for case in vectors::cases(&file, "run_started") {
            let name = case["name"].as_str().unwrap();
            let event =
                LiveEvent::decode_strict(&case["event"]).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                serde_json::to_value(&event).unwrap(),
                case["event"],
                "{name}"
            );
            let LiveEvent::RunStarted(started) = event else {
                panic!("{name}: not run_started");
            };
            assert_eq!(started.rich_ui, stamp_of(case), "{name}");
            let mut built = RunStarted::new(&started.run_id);
            if let Some(send_id) = &started.send_id {
                built = built.with_send_id(send_id);
            }
            if let Some(stamp) = stamp_of(case) {
                built = built.with_rich_ui(stamp);
            }
            assert_eq!(built, started, "{name}");
        }
    }

    #[test]
    fn the_head_names_the_last_runs_stamp_only_beside_that_run() {
        use crate::read::HeadMeta;
        let file = vectors::load("rich-ui-stamp.json");
        for case in vectors::cases(&file, "heads") {
            let name = case["name"].as_str().unwrap();
            let head: HeadMeta = vectors::round_trip(name, &case["answer"]);
            assert_eq!(head.last_run_rich_ui, stamp_of(case), "{name}");
            assert!(head.lineage_consistent(), "{name}");
        }
        for case in vectors::cases(&file, "inconsistent_heads") {
            let name = case["name"].as_str().unwrap();
            let head: HeadMeta = vectors::round_trip(name, &case["answer"]);
            assert!(!head.lineage_consistent(), "{name}");
        }
        // The builder adds only the member.
        let stamped = &vectors::cases(&file, "heads")[0];
        let head: HeadMeta = serde_json::from_value(stamped["answer"].clone()).unwrap();
        let mut bare = head.clone();
        bare.last_run_rich_ui = None;
        assert_eq!(bare.with_last_run_rich_ui(stamp_of(stamped).unwrap()), head);
    }

    /// The opt-in decodes, round-trips, and is refused naming
    /// `include_rich_ui` by a runner without the group; a `false` opt-in
    /// encodes as absence, so a request without it keeps its bytes.
    #[test]
    fn the_read_opt_in_is_served_only_with_the_group() {
        use crate::{capabilities, read::ReadRequest};
        let file = vectors::load("rich-ui-stamp.json");
        for case in vectors::cases(&file, "read_requests") {
            let name = case["name"].as_str().unwrap();
            let request: ReadRequest = serde_json::from_value(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let encoded = case.get("encoded").unwrap_or(&case["request"]);
            assert_eq!(&serde_json::to_value(&request).unwrap(), encoded, "{name}");
            let opted = case["include_rich_ui"].as_bool().unwrap();
            assert_eq!(request.include_rich_ui, opted, "{name}");
            assert_eq!(
                request.check_rich_ui(&[capabilities::RICH_UI_STAMP]),
                Ok(())
            );
            let without: [&str; 0] = [];
            assert_eq!(
                request.check_rich_ui(&without),
                if opted {
                    Err(crate::read::fields::INCLUDE_RICH_UI)
                } else {
                    Ok(())
                },
                "{name}"
            );
        }
        assert_eq!(
            serde_json::to_value(ReadRequest::range(0).with_include_rich_ui(true)).unwrap(),
            vectors::cases(&file, "read_requests")[0]["request"]
        );
    }

    #[test]
    fn read_entries_carry_the_stamp_beside_mid_never_inside_the_message() {
        use crate::read::ReadMessage;
        let file = vectors::load("rich-ui-stamp.json");
        for case in vectors::cases(&file, "read_entries") {
            let name = case["name"].as_str().unwrap();
            let entry: ReadMessage = vectors::round_trip(name, &case["entry"]);
            assert_eq!(entry.rich_ui, stamp_of(case), "{name}");
            assert!(entry.message.get("rich_ui").is_none(), "{name}");
        }
        // Without the opt-in the same entry differs only by the member.
        let entries = vectors::cases(&file, "read_entries");
        let stamped: ReadMessage = serde_json::from_value(entries[0]["entry"].clone()).unwrap();
        let plain: ReadMessage = serde_json::from_value(entries[1]["entry"].clone()).unwrap();
        assert_eq!(
            plain.clone().with_rich_ui(stamp_of(&entries[0]).unwrap()),
            stamped
        );
    }
}
