use std::fmt;
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use tracing::Level;

/// Renders one r2 fleet log line: `<ts> <LEVEL> <logger>: [<bound>] <message> <fields>`.
///
/// `logger` is the full dotted name (`engram`, `engram.gc.walk`), already
/// validated by [`logger_name`]. `bound` renders inside the bracket in the order
/// given; an empty slice renders NO bracket, because "nothing bound" and "an
/// empty context" are different facts and only the first is ever true.
pub(crate) fn render_line(
    at: SystemTime,
    level: &Level,
    logger: &str,
    bound: &[(String, String)],
    message: &str,
    fields: &[(String, String)],
) -> String {
    let timestamp = DateTime::<Utc>::from(at).to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut line = format!("{timestamp} {:<5} {logger}:", level.as_str());

    if !bound.is_empty() {
        line.push_str(" [");
        for (index, (key, value)) in bound.iter().enumerate() {
            if index > 0 {
                line.push(' ');
            }
            line.push_str(key);
            line.push('=');
            line.push_str(&format_value(value));
        }
        line.push(']');
    }
    if !message.is_empty() {
        line.push(' ');
        line.push_str(&escape_message(message));
    }
    for (key, value) in fields {
        line.push(' ');
        line.push_str(key);
        line.push('=');
        line.push_str(&format_value(value));
    }

    line
}

/// Joins a module id and an optional component into the logger name, refusing
/// a component that is not in the segment grammar. A `tracing` target that is
/// a Rust module path (`synapse::engine::decode`) is NOT a component: it would
/// make logger names an accident of code layout, so it maps to the bare module
/// id. A target equal to the module id is the same case spelled differently.
pub(crate) fn logger_name(module_id: &str, target: &str) -> String {
    if target.is_empty() || target == module_id || target.contains("::") {
        return module_id.to_owned();
    }
    if !target.split('.').all(is_segment) {
        return module_id.to_owned();
    }
    format!("{module_id}.{target}")
}

pub(crate) fn is_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-'))
}

// Control characters are handled per field, never across the rendered line.
// A complete terminal escape sequence inside one message or value is removed;
// every control character left is written as `\uXXXX`. Stripping escape
// sequences over the whole rendered line, as this crate did before 0.3.3, let
// a stray `ESC ]` in one value open an OSC that consumed the value's closing
// quote and every field after it. The TypeScript twin (`@cortexkit/log`)
// follows the same rule, and the golden fixture shared by both pins it.

/// Whether `character` is written as a `\uXXXX` escape: C0 controls other than
/// `\n` and `\r` (which keep their own escapes), DEL, and the C1 range.
fn is_escaped_control(character: char) -> bool {
    matches!(character as u32, 0x00..=0x1f | 0x7f..=0x9f) && !matches!(character, '\n' | '\r')
}

fn push_control_escape(output: &mut String, character: char) {
    output.push_str(&format!("\\u{:04x}", character as u32));
}

/// Removes CSI and OSC sequences that start and end inside `field`, in their
/// 7-bit (`ESC [`, `ESC ]`) and C1 (`U+009B`, `U+009D`) forms. A sequence that
/// does not end inside the field is not a sequence and is left for escaping.
fn strip_complete_sequences(field: &str) -> std::borrow::Cow<'_, str> {
    if !field
        .chars()
        .any(|character| matches!(character, '\u{1b}' | '\u{9b}' | '\u{9d}'))
    {
        return std::borrow::Cow::Borrowed(field);
    }
    let characters: Vec<char> = field.chars().collect();
    let mut output = String::with_capacity(field.len());
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        let next = characters.get(index + 1).copied();
        let csi_body = match (character, next) {
            ('\u{1b}', Some('[')) => Some(index + 2),
            ('\u{9b}', _) => Some(index + 1),
            _ => None,
        };
        let osc_body = match (character, next) {
            ('\u{1b}', Some(']')) => Some(index + 2),
            ('\u{9d}', _) => Some(index + 1),
            _ => None,
        };
        let end = if let Some(body) = csi_body {
            csi_end(&characters, body)
        } else if let Some(body) = osc_body {
            osc_end(&characters, body)
        } else {
            None
        };
        match end {
            Some(end) => index = end,
            None => {
                output.push(character);
                index += 1;
            }
        }
    }
    std::borrow::Cow::Owned(output)
}

/// A CSI body is parameter and intermediate bytes (`0x20`-`0x3f`) ended by a
/// final byte (`0x40`-`0x7e`). Anything else before the final byte means it
/// was never a sequence.
fn csi_end(characters: &[char], body: usize) -> Option<usize> {
    for (offset, character) in characters[body..].iter().enumerate() {
        match *character as u32 {
            0x40..=0x7e => return Some(body + offset + 1),
            0x20..=0x3f => {}
            _ => return None,
        }
    }
    None
}

/// An OSC ends at `BEL`, `ESC \` or C1 `ST` (`U+009C`).
fn osc_end(characters: &[char], body: usize) -> Option<usize> {
    let mut index = body;
    while index < characters.len() {
        match characters[index] {
            '\u{7}' | '\u{9c}' => return Some(index + 1),
            '\u{1b}' if characters.get(index + 1) == Some(&'\\') => return Some(index + 2),
            _ => index += 1,
        }
    }
    None
}

// Backslash is escaped first so a literal `\n` or `\u0041` in the input
// survives as `\\n` or `\\u0041` and cannot be read back as an escape.
fn escape_message(message: &str) -> String {
    let cleaned = strip_complete_sequences(message);
    let mut output = String::with_capacity(cleaned.len());
    for character in cleaned.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\r' => output.push_str("\\r"),
            '\n' => output.push_str("\\n"),
            control if is_escaped_control(control) => push_control_escape(&mut output, control),
            other => output.push(other),
        }
    }
    output
}

fn format_value(value: &str) -> String {
    // Stripped before the quoting decision, so a colored plain word stays unquoted.
    let cleaned = strip_complete_sequences(value);
    let needs_quotes = cleaned.is_empty()
        || cleaned.chars().any(|character| {
            matches!(character, ' ' | '"' | '\n' | '\r' | ']') || is_escaped_control(character)
        });
    if !needs_quotes {
        // An unquoted value is verbatim, backslashes included: only the quoted
        // form has an escape grammar, and a reader decodes only quoted values.
        return cleaned.into_owned();
    }
    let mut output = String::with_capacity(cleaned.len() + 2);
    output.push('"');
    for character in cleaned.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\r' => output.push_str("\\r"),
            '\n' => output.push_str("\\n"),
            control if is_escaped_control(control) => push_control_escape(&mut output, control),
            other => output.push(other),
        }
    }
    output.push('"');
    output
}

/// The guard after a module redactor: writes any raw control character the
/// redactor put into an already-rendered line as an escape, and never strips.
/// Backslashes are left alone, since the line's own escapes are already there.
pub(crate) fn escape_raw_controls(line: &str) -> std::borrow::Cow<'_, str> {
    if !line
        .chars()
        .any(|character| matches!(character, '\n' | '\r') || is_escaped_control(character))
    {
        return std::borrow::Cow::Borrowed(line);
    }
    let mut output = String::with_capacity(line.len());
    for character in line.chars() {
        match character {
            '\r' => output.push_str("\\r"),
            '\n' => output.push_str("\\n"),
            control if is_escaped_control(control) => push_control_escape(&mut output, control),
            other => output.push(other),
        }
    }
    std::borrow::Cow::Owned(output)
}

/// A level parsed from a fleet log line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParsedLevel {
    /// A trace event.
    Trace,
    /// A debug event.
    Debug,
    /// An informational event.
    Info,
    /// A warning event.
    Warn,
    /// An error event.
    Error,
}

/// The fields needed to merge and filter a fleet log line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedLine<'a> {
    /// Event time decoded from the UTC timestamp.
    pub timestamp: SystemTime,
    /// Event severity.
    pub level: ParsedLevel,
    /// The full dotted logger name, rooted at the module id.
    pub logger: &'a str,
    /// The module id: the logger's first segment.
    pub module_id: &'a str,
    /// The raw text inside the bound bracket, absent when there was none.
    pub bound: Option<&'a str>,
    /// The message and ordered event fields after the bracket.
    pub body: &'a str,
}

impl ParsedLine<'_> {
    /// The `session=` bound value when one is present, whole, issuer included.
    pub fn session(&self) -> Option<&str> {
        self.bound.and_then(|bound| bound_value(bound, "session"))
    }
}

fn bound_value<'a>(bound: &'a str, key: &str) -> Option<&'a str> {
    bound
        .split(' ')
        .filter_map(|pair| pair.split_once('='))
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, value)| value)
}

/// A stable parse failure returned by [`parse_line`](crate::parse_line).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseError {
    reason: &'static str,
}

impl ParseError {
    pub(crate) const fn new(reason: &'static str) -> Self {
        Self { reason }
    }

    /// Returns the stable reason used by conformance fixtures and CLI diagnostics.
    pub const fn reason(self) -> &'static str {
        self.reason
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason)
    }
}

impl std::error::Error for ParseError {}

pub(crate) fn parse(line: &str) -> Result<ParsedLine<'_>, ParseError> {
    if line.contains('\u{1b}') || line.contains('\u{9b}') {
        return Err(ParseError::new("ansi_forbidden"));
    }
    if line.contains(['\n', '\r']) {
        return Err(ParseError::new("line_break"));
    }

    let (timestamp_text, after_timestamp) = line
        .split_once(' ')
        .ok_or_else(|| ParseError::new("timestamp_missing"))?;
    if !timestamp_text.ends_with('Z') {
        return Err(ParseError::new("timestamp_not_utc_z"));
    }
    if timestamp_text.len() != 24 {
        return Err(ParseError::new("timestamp_precision"));
    }
    let timestamp = DateTime::parse_from_rfc3339(timestamp_text)
        .map_err(|_| ParseError::new("timestamp_invalid"))?;

    let (level, after_level) = parse_level(after_timestamp)?;

    // The logger runs to the first space and MUST end in a colon. An r1 line
    // (`fusiform poll changed`) has no colon and fails here by design: it is
    // the colon that makes the logger unambiguous against a one-word message.
    let (logger_token, mut body) = match after_level.split_once(' ') {
        Some(split) => split,
        None => (after_level, ""),
    };
    let logger = logger_token
        .strip_suffix(':')
        .ok_or_else(|| ParseError::new("logger_not_terminated"))?;
    if logger.is_empty() {
        return Err(ParseError::new("logger_missing"));
    }
    if !logger.split('.').all(is_segment) {
        return Err(ParseError::new("logger_segment_grammar"));
    }
    let module_id = logger.split('.').next().unwrap_or(logger);

    let mut bound = None;
    if let Some(rest) = body.strip_prefix('[') {
        let close =
            find_bracket_close(rest).ok_or_else(|| ParseError::new("bound_unterminated"))?;
        let inside = &rest[..close];
        if inside.is_empty() {
            return Err(ParseError::new("empty_bound_bracket"));
        }
        if let Some(session) = bound_value(inside, "session") {
            let valid = session
                .rsplit_once(':')
                .is_some_and(|(issuer, id)| !issuer.is_empty() && !id.is_empty());
            // `session=global` and other issuer-less placeholders fail here on
            // their missing `issuer:` half; the renderer never has to know any
            // particular sentinel.
            if !valid {
                return Err(ParseError::new("session_missing_issuer"));
            }
        }
        bound = Some(inside);
        body = rest[close + 1..]
            .strip_prefix(' ')
            .unwrap_or(&rest[close + 1..]);
    }

    // A bracket AFTER the message is not context: context precedes the message
    // so its column is stable. Reject rather than silently reading it as a
    // field, because a reader that accepted both would train writers to put
    // it wherever, and the alignment property would be gone in a month.
    if bound.is_none() && body.contains(" [") && body.ends_with(']') {
        return Err(ParseError::new("bound_after_message"));
    }

    Ok(ParsedLine {
        timestamp: SystemTime::from(timestamp),
        level,
        logger,
        module_id,
        bound,
        body,
    })
}

// The bracket closes at the first `]` that is not inside a quoted value. A
// bound value containing `]` was quoted by the renderer for exactly this
// reason, so a naive `find(']')` would split a path like `[root="a]b"]`.
fn find_bracket_close(input: &str) -> Option<usize> {
    let mut in_quotes = false;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ']' if !in_quotes => return Some(index),
            _ => {}
        }
    }
    None
}

fn parse_level(input: &str) -> Result<(ParsedLevel, &str), ParseError> {
    for (prefix, level) in [
        ("TRACE ", ParsedLevel::Trace),
        ("DEBUG ", ParsedLevel::Debug),
        ("INFO  ", ParsedLevel::Info),
        ("WARN  ", ParsedLevel::Warn),
        ("ERROR ", ParsedLevel::Error),
    ] {
        if let Some(rest) = input.strip_prefix(prefix) {
            return Ok((level, rest));
        }
    }

    if ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]
        .iter()
        .any(|level| input.starts_with(level))
    {
        Err(ParseError::new("level_column_width"))
    } else {
        Err(ParseError::new("level_invalid"))
    }
}
