//! The `owned-broca` error output and the runner-neutral outage text.
//!
//! In the `owned-broca` profile a tool result's error output is
//! `{"kind": "error", "message": ...}`. When a call is refused because its
//! provider module is restarting or down, the output may also carry five
//! optional members, in this order: `module`, `code`, `detail`,
//! `nothing_sent` and `retry` (CONTRACT §4.4). The member set is closed and
//! every rule binds readers as well as producers: [`ErrorOutput::decode`]
//! refuses anything else as a profile violation, naming the member and the
//! rule ([`ErrorOutputViolation`]).
//!
//! [`render`] gives the text the model sees, which every runner reporting a
//! module outage uses, whatever its profile (CONTRACT §11.4): the message,
//! then one line per present member. Each member is one non-empty line, so
//! no member can forge another member's line in that text.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// The `kind` an error output carries.
pub const KIND_ERROR: &str = "error";

/// The optional members an error output may carry after `message`, in the
/// order they are encoded and rendered. Any other member is a violation.
pub const MEMBERS: [&str; 5] = ["module", "code", "detail", "nothing_sent", "retry"];

/// The longest `module`, in bytes.
pub const MODULE_MAX: usize = 255;

/// The longest `code`, in bytes.
pub const CODE_MAX: usize = 64;

/// The longest `detail`, in bytes of UTF-8: 1 KiB.
pub const DETAIL_MAX: usize = 1024;

/// The longest `retry`, in bytes of UTF-8.
pub const RETRY_MAX: usize = 256;

/// The rendered line standing for `nothing_sent: true`.
pub const NOTHING_SENT_LINE: &str = "nothing was sent";

/// Whether `module` is a valid `module` member: 1 to [`MODULE_MAX`] bytes
/// of UTF-8 with no control character. The member is one line in the
/// model's text, so the only property it needs is "no line break"; spaces,
/// non-ASCII text and path separators are all accepted.
pub fn is_valid_module(module: &str) -> bool {
    (1..=MODULE_MAX).contains(&module.len()) && !module.chars().any(char::is_control)
}

/// Whether `code` is a valid `code` member: it matches
/// `^[a-z][a-z0-9_]{0,63}$`, lowercase snake case starting with a letter,
/// 1 to [`CODE_MAX`] bytes.
pub fn is_valid_code(code: &str) -> bool {
    let mut bytes = code.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && code.len() <= CODE_MAX
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// The rule an error output broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorOutputRule {
    /// The output is not a JSON object.
    NotAnObject,
    /// A required member (`kind` or `message`) is absent.
    Missing,
    /// `kind` is not the string `"error"`.
    NotError,
    /// A member outside `kind`, `message` and [`MEMBERS`].
    UnknownMember,
    /// A member is `null`. Absent members are omitted, never `null`.
    Null,
    /// A text member is not a string.
    NotAString,
    /// `nothing_sent` is anything but the literal `true`.
    NotTrue,
    /// A member is the empty string. A producer with nothing to say omits it.
    Empty,
    /// A member contains `\n` or `\r`; each member renders as one line.
    LineBreak,
    /// A member is longer than its byte limit.
    TooLong,
    /// `module` or `code` is outside its grammar.
    Grammar,
}

impl ErrorOutputRule {
    /// The rule's stable name, as the test vectors spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAnObject => "not_an_object",
            Self::Missing => "missing",
            Self::NotError => "not_error",
            Self::UnknownMember => "unknown_member",
            Self::Null => "null",
            Self::NotAString => "not_a_string",
            Self::NotTrue => "not_true",
            Self::Empty => "empty",
            Self::LineBreak => "line_break",
            Self::TooLong => "too_long",
            Self::Grammar => "grammar",
        }
    }

    fn explain(self) -> &'static str {
        match self {
            Self::NotAnObject => "an error output must be a JSON object",
            Self::Missing => "the member is required",
            Self::NotError => "kind must be \"error\"",
            Self::UnknownMember => "the member set is closed",
            Self::Null => "an absent member is omitted, never null",
            Self::NotAString => "the member must be a string",
            Self::NotTrue => "nothing_sent may only be the literal true",
            Self::Empty => "a member is absent or non-empty",
            Self::LineBreak => "a member is one line, without \\n or \\r",
            Self::TooLong => "the member is over its byte limit",
            Self::Grammar => "the member is outside its grammar",
        }
    }
}

impl fmt::Display for ErrorOutputRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An `owned-broca` profile violation: the member at fault and the rule it
/// broke. For [`ErrorOutputRule::NotAnObject`] the member is `output`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ErrorOutputViolation {
    /// The member's name as it appears in the output.
    pub member: String,
    /// The rule it broke.
    pub rule: ErrorOutputRule,
}

impl ErrorOutputViolation {
    fn new(member: impl Into<String>, rule: ErrorOutputRule) -> Self {
        Self {
            member: member.into(),
            rule,
        }
    }
}

impl fmt::Display for ErrorOutputViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "owned-broca error output member {}: {} ({})",
            self.member,
            self.rule,
            self.rule.explain()
        )
    }
}

impl std::error::Error for ErrorOutputViolation {}

/// A tool result's error output in the `owned-broca` profile. Build one
/// with [`Self::new`] and the `with_*` setters; [`Self::check`] validates
/// the members before a producer stores or sends it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ErrorOutput {
    /// The error text. The profile does not constrain it.
    pub message: String,
    /// The provider's module id.
    pub module: Option<String>,
    /// The daemon's refusal code, verbatim.
    pub code: Option<String>,
    /// One line about the refusal, opaque to readers. A producer encodes
    /// an object detail from the daemon as compact JSON.
    pub detail: Option<String>,
    /// True only when the refusal proves the call never reached the
    /// provider. Encoded as `"nothing_sent": true`, and omitted when false.
    pub nothing_sent: bool,
    /// A hint for the model, never parsed.
    pub retry: Option<String>,
}

impl ErrorOutput {
    /// A plain error output: `kind` and `message` only.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            module: None,
            code: None,
            detail: None,
            nothing_sent: false,
            retry: None,
        }
    }

    pub fn with_module(mut self, module: impl Into<String>) -> Self {
        self.module = Some(module.into());
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Mark the call as never having reached the provider. Only for a
    /// refusal at route open, or `module_reloading` before the send; never
    /// for a timeout or a lost reply (CONTRACT §11.4).
    pub fn with_nothing_sent(mut self) -> Self {
        self.nothing_sent = true;
        self
    }

    pub fn with_retry(mut self, retry: impl Into<String>) -> Self {
        self.retry = Some(retry.into());
        self
    }

    /// Whether the output carries none of the optional members.
    pub fn is_plain(&self) -> bool {
        self.module.is_none()
            && self.code.is_none()
            && self.detail.is_none()
            && !self.nothing_sent
            && self.retry.is_none()
    }

    /// Check every present member against its rules: non-empty, one line,
    /// within its byte limit, and `module` and `code` within their grammars.
    pub fn check(&self) -> Result<(), ErrorOutputViolation> {
        let members = [
            ("module", &self.module, MODULE_MAX),
            ("code", &self.code, CODE_MAX),
            ("detail", &self.detail, DETAIL_MAX),
            ("retry", &self.retry, RETRY_MAX),
        ];
        for (member, value, max) in members {
            if let Some(value) = value {
                check_text(member, value, max)?;
            }
        }
        Ok(())
    }

    /// Decode and validate an `output` value of a tool result. Refuses a
    /// value that is not an object, a missing `kind` or `message`, a `kind`
    /// other than `"error"`, any member outside the closed set, and any
    /// member breaking its rules, naming the first violation found: `kind`,
    /// then unknown members in name order, then `message`, then the
    /// optional members in [`MEMBERS`] order.
    pub fn decode(output: &Value) -> Result<Self, ErrorOutputViolation> {
        let Value::Object(members) = output else {
            return Err(ErrorOutputViolation::new(
                "output",
                ErrorOutputRule::NotAnObject,
            ));
        };
        match members.get("kind") {
            None => return Err(ErrorOutputViolation::new("kind", ErrorOutputRule::Missing)),
            Some(Value::String(kind)) if kind == KIND_ERROR => {}
            Some(_) => return Err(ErrorOutputViolation::new("kind", ErrorOutputRule::NotError)),
        }
        let mut unknown: Vec<&String> = members
            .keys()
            .filter(|name| {
                *name != "kind" && *name != "message" && !MEMBERS.contains(&name.as_str())
            })
            .collect();
        unknown.sort();
        if let Some(name) = unknown.first() {
            return Err(ErrorOutputViolation::new(
                name.as_str(),
                ErrorOutputRule::UnknownMember,
            ));
        }
        let message = match members.get("message") {
            None => {
                return Err(ErrorOutputViolation::new(
                    "message",
                    ErrorOutputRule::Missing,
                ))
            }
            Some(_) => string_member(members, "message")?.expect("present"),
        };
        // Each member is fully checked before the next one is looked at, so
        // the first violation reported follows MEMBERS order.
        let module = text_member(members, "module", MODULE_MAX)?;
        let code = text_member(members, "code", CODE_MAX)?;
        let detail = text_member(members, "detail", DETAIL_MAX)?;
        let nothing_sent = match members.get("nothing_sent") {
            None => false,
            Some(Value::Bool(true)) => true,
            Some(Value::Null) => {
                return Err(ErrorOutputViolation::new(
                    "nothing_sent",
                    ErrorOutputRule::Null,
                ))
            }
            Some(_) => {
                return Err(ErrorOutputViolation::new(
                    "nothing_sent",
                    ErrorOutputRule::NotTrue,
                ))
            }
        };
        let retry = text_member(members, "retry", RETRY_MAX)?;
        Ok(Self {
            message,
            module,
            code,
            detail,
            nothing_sent,
            retry,
        })
    }

    /// The text block the model sees for this output; see [`render`].
    pub fn render(&self) -> String {
        render(self)
    }
}

/// The text the model sees for a tool error, and what a runner storing tool
/// errors as plain strings stores: `message`, then one line per present
/// member in this order: `module: <module>`, `code: <code>`,
/// `detail: <detail>`, `nothing was sent`, `retry: <retry>`, joined by `\n`
/// with no trailing newline. A plain error renders as its message alone.
/// Render only an output that passes [`ErrorOutput::check`].
pub fn render(output: &ErrorOutput) -> String {
    let mut lines = vec![output.message.clone()];
    if let Some(module) = &output.module {
        lines.push(format!("module: {module}"));
    }
    if let Some(code) = &output.code {
        lines.push(format!("code: {code}"));
    }
    if let Some(detail) = &output.detail {
        lines.push(format!("detail: {detail}"));
    }
    if output.nothing_sent {
        lines.push(NOTHING_SENT_LINE.to_owned());
    }
    if let Some(retry) = &output.retry {
        lines.push(format!("retry: {retry}"));
    }
    lines.join("\n")
}

fn string_member(
    members: &Map<String, Value>,
    member: &str,
) -> Result<Option<String>, ErrorOutputViolation> {
    match members.get(member) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Null) => Err(ErrorOutputViolation::new(member, ErrorOutputRule::Null)),
        Some(_) => Err(ErrorOutputViolation::new(
            member,
            ErrorOutputRule::NotAString,
        )),
    }
}

fn text_member(
    members: &Map<String, Value>,
    member: &str,
    max: usize,
) -> Result<Option<String>, ErrorOutputViolation> {
    let value = string_member(members, member)?;
    if let Some(value) = &value {
        check_text(member, value, max)?;
    }
    Ok(value)
}

fn check_text(member: &str, value: &str, max: usize) -> Result<(), ErrorOutputViolation> {
    let rule = if value.is_empty() {
        ErrorOutputRule::Empty
    } else if value.contains(['\n', '\r']) {
        ErrorOutputRule::LineBreak
    } else if value.len() > max {
        ErrorOutputRule::TooLong
    } else if (member == "module" && !is_valid_module(value))
        || (member == "code" && !is_valid_code(value))
    {
        ErrorOutputRule::Grammar
    } else {
        return Ok(());
    };
    Err(ErrorOutputViolation::new(member, rule))
}

/// The wire order: `kind`, `message`, then [`MEMBERS`] order, each optional
/// member omitted when absent. A plain error encodes as exactly
/// `{"kind":"error","message":...}`.
#[derive(Serialize)]
struct Wire<'a> {
    kind: &'static str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nothing_sent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry: Option<&'a str>,
}

impl Serialize for ErrorOutput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Wire {
            kind: KIND_ERROR,
            message: &self.message,
            module: self.module.as_deref(),
            code: self.code.as_deref(),
            detail: self.detail.as_deref(),
            nothing_sent: self.nothing_sent.then_some(true),
            retry: self.retry.as_deref(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ErrorOutput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::decode(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// Every file in `test-vectors/owned-broca-tool-outage/` except
    /// `SHA256SUMS`, in the order `SHA256SUMS` lists them.
    const FILES: &[(&str, &str)] = &[
        (
            "session_history.display.plain-error.row.json",
            include_str!(
                "../../test-vectors/owned-broca-tool-outage/session_history.display.plain-error.row.json"
            ),
        ),
        (
            "session_history.display.row.json",
            include_str!("../../test-vectors/owned-broca-tool-outage/session_history.display.row.json"),
        ),
        (
            "session_history.row.json",
            include_str!("../../test-vectors/owned-broca-tool-outage/session_history.row.json"),
        ),
        (
            "session_read.reply.json",
            include_str!("../../test-vectors/owned-broca-tool-outage/session_read.reply.json"),
        ),
        (
            "tool_finished.event.json",
            include_str!("../../test-vectors/owned-broca-tool-outage/tool_finished.event.json"),
        ),
        (
            "tool_finished.frame.json",
            include_str!("../../test-vectors/owned-broca-tool-outage/tool_finished.frame.json"),
        ),
        (
            "tool_result.record.json",
            include_str!("../../test-vectors/owned-broca-tool-outage/tool_result.record.json"),
        ),
        (
            "valid-error-outputs.json",
            include_str!("../../test-vectors/owned-broca-tool-outage/valid-error-outputs.json"),
        ),
        (
            "invalid-error-outputs.json",
            include_str!("../../test-vectors/owned-broca-tool-outage/invalid-error-outputs.json"),
        ),
        (
            "README.md",
            include_str!("../../test-vectors/owned-broca-tool-outage/README.md"),
        ),
    ];

    const SHA256SUMS: &str = include_str!("../../test-vectors/owned-broca-tool-outage/SHA256SUMS");

    fn text(name: &str) -> &'static str {
        FILES
            .iter()
            .find(|(file, _)| *file == name)
            .unwrap_or_else(|| panic!("{name} is not listed"))
            .1
    }

    fn load(name: &str) -> Value {
        serde_json::from_str(text(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// The captured error outputs: file and the pointer to the output.
    const CAPTURES: &[(&str, &str)] = &[
        ("session_history.row.json", "/message/content/0/output"),
        (
            "session_read.reply.json",
            "/result/messages/2/message/content/0/output",
        ),
        ("tool_finished.frame.json", "/unit/result/output"),
        ("tool_result.record.json", "/record/result/output"),
    ];

    fn display_content(name: &str) -> String {
        load(name)["row"]["parts"][0]["content"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: no display content"))
            .to_owned()
    }

    #[test]
    fn files_on_disk_are_listed_and_match_sha256sums() {
        let dir = format!(
            "{}/test-vectors/owned-broca-tool-outage",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name != "SHA256SUMS")
            .collect();
        on_disk.sort();
        let mut listed: Vec<String> = FILES.iter().map(|(name, _)| name.to_string()).collect();
        listed.sort();
        assert_eq!(on_disk, listed);
        assert_eq!(
            std::fs::read_to_string(format!("{dir}/SHA256SUMS")).unwrap(),
            SHA256SUMS
        );
        let lines: Vec<(&str, &str)> = SHA256SUMS
            .lines()
            .map(|line| line.split_once("  ").expect("sha256sum line"))
            .collect();
        assert_eq!(lines.len(), FILES.len());
        for ((digest, listed_name), (name, contents)) in lines.iter().zip(FILES) {
            assert_eq!(listed_name, name, "SHA256SUMS lists files in order");
            assert_eq!(
                &format!("{:x}", Sha256::digest(contents.as_bytes())),
                digest,
                "{name}: SHA-256 mismatch"
            );
        }
    }

    #[test]
    fn every_capture_decodes_and_renders_the_display_row_exactly() {
        let expected = ErrorOutput::new("tool plane unavailable")
            .with_module("aft")
            .with_code("target_unavailable")
            .with_detail("supervised_not_registered")
            .with_nothing_sent()
            .with_retry("the module is restarting or unavailable; retry the call in a minute");
        let content = display_content("session_history.display.row.json");
        for (name, pointer) in CAPTURES {
            let file = load(name);
            let output = file
                .pointer(pointer)
                .unwrap_or_else(|| panic!("{name}: nothing at {pointer}"));
            let decoded = ErrorOutput::decode(output).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(decoded, expected, "{name}");
            assert!(!decoded.is_plain(), "{name}");
            assert_eq!(decoded.render(), content, "{name}");
            let typed: ErrorOutput = serde_json::from_value(output.clone()).unwrap();
            assert_eq!(typed, decoded, "{name}");
        }
        assert_eq!(content.lines().count(), 6);
        assert!(!content.ends_with('\n'));
    }

    #[test]
    fn captured_compact_outputs_re_encode_to_their_exact_bytes() {
        for name in ["tool_finished.frame.json", "tool_result.record.json"] {
            let pointer = CAPTURES.iter().find(|(file, _)| *file == name).unwrap().1;
            let decoded = ErrorOutput::decode(load(name).pointer(pointer).unwrap()).unwrap();
            let encoded = serde_json::to_string(&decoded).unwrap();
            let marker = format!("\"output\":{encoded}");
            assert!(text(name).contains(&marker), "{name}: {encoded}");
        }
    }

    #[test]
    fn the_event_capture_carries_no_error_output() {
        let event = load("tool_finished.event.json");
        assert_eq!(event["type"], "tool_finished");
        assert!(event.get("output").is_none());
    }

    #[test]
    fn a_plain_error_decodes_as_plain_and_stays_byte_identical() {
        let content = display_content("session_history.display.plain-error.row.json");
        let file = load("valid-error-outputs.json");
        let plain = file["valid"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "plain_error")
            .unwrap();
        let decoded = ErrorOutput::decode(&plain["output"]).unwrap();
        assert!(decoded.is_plain());
        assert_eq!(decoded, ErrorOutput::new(content.clone()));
        assert_eq!(decoded.render(), content);
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            r#"{"kind":"error","message":"plain failure"}"#
        );
        assert_eq!(serde_json::to_value(&decoded).unwrap(), plain["output"]);
    }

    #[test]
    fn valid_samples_decode_and_render_in_the_fixed_order() {
        let file = load("valid-error-outputs.json");
        assert_eq!(file["profile"], crate::serializer_profile::OWNED_BROCA);
        let cases = file["valid"].as_array().unwrap();
        assert_eq!(cases.len(), 7);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let decoded =
                ErrorOutput::decode(&case["output"]).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(decoded.check(), Ok(()), "{name}");
            assert_eq!(decoded.is_plain(), case["plain"] == true, "{name}");
            assert_eq!(Value::from(decoded.render()), case["rendered"], "{name}");
            assert_eq!(
                serde_json::to_value(&decoded).unwrap(),
                case["output"],
                "{name}: re-encoding changes no member"
            );
        }
    }

    #[test]
    fn every_invalid_sample_is_refused_by_member_and_rule() {
        let file = load("invalid-error-outputs.json");
        assert_eq!(file["profile"], crate::serializer_profile::OWNED_BROCA);
        let cases = file["invalid"].as_array().unwrap();
        let names: Vec<&str> = cases.iter().map(|c| c["name"].as_str().unwrap()).collect();
        for required in [
            "nothing_sent_false",
            "unknown_member",
            "null_member",
            "detail_newline_forging_a_line",
            "detail_over_1_kib",
            "retry_over_256_bytes",
            "detail_empty",
            "retry_empty",
            "module_with_newline",
            "module_with_c1_control",
            "module_over_255_bytes",
            "code_uppercase",
        ] {
            assert!(
                names.contains(&required),
                "missing invalid sample {required}"
            );
        }
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let expect = &case["expect"];
            assert_eq!(expect["outcome"], "reader_refuses", "{name}");
            let violation = ErrorOutput::decode(&case["output"])
                .expect_err(&format!("{name}: a reader must refuse it"));
            assert_eq!(
                violation.member,
                expect["member"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(
                violation.rule.as_str(),
                expect["rule"].as_str().unwrap(),
                "{name}"
            );
            let message = violation.to_string();
            assert!(message.contains(&violation.member), "{name}: {message}");
            assert!(
                message.contains(violation.rule.as_str()),
                "{name}: {message}"
            );
            let typed = serde_json::from_value::<ErrorOutput>(case["output"].clone());
            assert!(typed.is_err(), "{name}: serde decode must refuse too");
        }
    }

    #[test]
    fn producer_check_refuses_what_readers_refuse() {
        let ok = ErrorOutput::new("tool plane unavailable")
            .with_module("fed:peer-1:aft")
            .with_code("module_reloading");
        assert_eq!(ok.check(), Ok(()));
        for (output, member, rule) in [
            (ok.clone().with_detail(""), "detail", ErrorOutputRule::Empty),
            (ok.clone().with_retry(""), "retry", ErrorOutputRule::Empty),
            (
                ok.clone().with_detail("a\nnothing was sent"),
                "detail",
                ErrorOutputRule::LineBreak,
            ),
            (
                ok.clone().with_retry("x".repeat(RETRY_MAX + 1)),
                "retry",
                ErrorOutputRule::TooLong,
            ),
            (
                ok.clone().with_detail("x".repeat(DETAIL_MAX + 1)),
                "detail",
                ErrorOutputRule::TooLong,
            ),
            (
                ok.clone().with_code("Upper"),
                "code",
                ErrorOutputRule::Grammar,
            ),
            (
                ok.clone().with_module("a\tmodule"),
                "module",
                ErrorOutputRule::Grammar,
            ),
        ] {
            let violation = output.check().unwrap_err();
            assert_eq!((violation.member.as_str(), violation.rule), (member, rule));
        }
    }

    #[test]
    fn grammars_match_their_definitions() {
        for module in [
            "aft",
            "fed:peer-1:aft",
            "!",
            "~",
            "aft module",
            "aft-\u{e9}",
            "a/b",
            &"m".repeat(MODULE_MAX),
        ] {
            assert!(is_valid_module(module), "{module}");
        }
        for module in [
            "",
            "aft\tmodule",
            "aft\u{7f}",
            "aft\u{85}",
            "a\nb",
            &"m".repeat(MODULE_MAX + 1),
        ] {
            assert!(!is_valid_module(module), "{module:?}");
        }
        for code in [
            "a",
            "target_unavailable",
            "a0_",
            &format!("a{}", "_".repeat(63)),
        ] {
            assert!(is_valid_code(code), "{code}");
        }
        for code in [
            "",
            "_a",
            "0a",
            "aB",
            "a-b",
            "a b",
            &format!("a{}", "_".repeat(64)),
        ] {
            assert!(!is_valid_code(code), "{code:?}");
        }
    }

    #[test]
    fn render_puts_each_present_member_on_its_own_line() {
        let output = ErrorOutput::new("tool plane unavailable")
            .with_retry("later")
            .with_nothing_sent()
            .with_code("module_reloading");
        assert_eq!(
            output.render(),
            "tool plane unavailable\ncode: module_reloading\nnothing was sent\nretry: later"
        );
        assert_eq!(render(&ErrorOutput::new("plain")), "plain");
    }
}
