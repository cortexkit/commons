use std::{borrow::Cow, sync::LazyLock};

use regex::Regex;

const REDACTED: &str = "[redacted]";

// Keep these in the order used by cortexkit-log, after its Authorization scan.
static FLEET: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"(?i)\b(Bearer)[ \t]+[A-Za-z0-9._~+/=-]+", "$1 [redacted]"),
        (r"\beyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b", REDACTED),
        (r"\bckh_[A-Za-z0-9_-]+\b", REDACTED),
        (r"\bsk-[A-Za-z0-9_-]+\b", REDACTED),
        (r"\bgh[pousr]_[A-Za-z0-9_-]+\b|\bgithub_pat_[A-Za-z0-9_]+\b", REDACTED),
        (r#"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/?#@"]+@"#, "$1[redacted]@"),
        (r#"(?i)([?&](?:access_token|token|api_key|apikey|password|secret|client_secret)=)(?:\\.|[^&#\s"\\)\]])+"#, "$1[redacted]"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| (Regex::new(pattern).expect("valid fleet regex"), replacement))
    .collect()
});

static PEM_BEGIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----").expect("valid PEM begin regex")
});
static PEM_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-----END [A-Z ]*PRIVATE KEY-----").expect("valid PEM end regex"));
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+").expect("valid email regex")
});
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/_=-]+").expect("valid token regex"));

/// Replace secret patterns with `[redacted]`. This is not a general free-text
/// sanitizer; ordinary private text may remain.
pub fn redact_secrets(input: &str) -> Cow<'_, str> {
    let mut current = redact_authorization(input);
    for (pattern, replacement) in FLEET.iter() {
        if let Cow::Owned(text) = pattern.replace_all(&current, *replacement) {
            current = Cow::Owned(text);
        }
    }
    if let Some(text) = redact_pem(&current) {
        current = Cow::Owned(text);
    }
    if let Cow::Owned(text) = EMAIL.replace_all(&current, REDACTED) {
        current = Cow::Owned(text);
    }
    if let Some(text) = redact_tokens(&current) {
        current = Cow::Owned(text);
    }
    current
}

pub(crate) fn secret_free(text: &str) -> bool {
    redact_secrets(text) == text
}

fn redact_pem(input: &str) -> Option<String> {
    let mut from = 0;
    let spans = std::iter::from_fn(|| {
        let begin = PEM_BEGIN.find_at(input, from)?;
        let end = PEM_END
            .find_at(input, begin.end())
            .map_or(input.len(), |m| m.end());
        from = end;
        Some(begin.start()..end)
    });
    redact_spans(input, spans)
}

fn redact_tokens(input: &str) -> Option<String> {
    let spans = TOKEN
        .find_iter(input)
        .filter(|m| token_is_secret(m.as_str()))
        .map(|m| m.range());
    redact_spans(input, spans)
}

fn redact_spans(
    input: &str,
    spans: impl Iterator<Item = std::ops::Range<usize>>,
) -> Option<String> {
    let mut output = String::new();
    let mut from = 0;
    for span in spans {
        output.push_str(&input[from..span.start]);
        output.push_str(REDACTED);
        from = span.end;
    }
    if from == 0 {
        return None;
    }
    output.push_str(&input[from..]);
    Some(output)
}

fn token_is_secret(token: &str) -> bool {
    let mut hex_run = 0;
    let (mut digit, mut upper, mut lower) = (false, false, false);
    for b in token.bytes() {
        if b.is_ascii_hexdigit() {
            hex_run += 1;
            if hex_run == 32 {
                return true;
            }
        } else {
            hex_run = 0;
        }
        digit |= b.is_ascii_digit();
        upper |= b.is_ascii_uppercase();
        lower |= b.is_ascii_lowercase();
    }
    token.len() >= 40 && digit && upper && lower
}

// Mirror cortexkit-log's Authorization scanner, including its quote handling.
fn redact_authorization(input: &str) -> Cow<'_, str> {
    let lowercase = input.to_ascii_lowercase();
    let mut search_from = 0;
    let mut output = String::new();
    let mut copied_through = 0;

    while let Some(relative) = lowercase[search_from..].find("authorization:") {
        let marker = search_from + relative;
        let after_marker = marker + "authorization:".len();
        let mut value_start = after_marker;
        while input
            .as_bytes()
            .get(value_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            value_start += 1;
        }
        if value_start >= input.len() {
            break;
        }

        let value_end = authorization_value_end(input, value_start);
        let redaction_start = if input.as_bytes()[value_start] == b'"' {
            value_start + 1
        } else if input.as_bytes()[value_start] == b'\\'
            && input.as_bytes().get(value_start + 1) == Some(&b'"')
        {
            value_start + 2
        } else {
            value_start
        };
        output.push_str(&input[copied_through..redaction_start]);
        output.push_str(REDACTED);
        copied_through = value_end;
        search_from = value_end;
    }

    if copied_through == 0 {
        Cow::Borrowed(input)
    } else {
        output.push_str(&input[copied_through..]);
        Cow::Owned(output)
    }
}

fn authorization_value_end(input: &str, start: usize) -> usize {
    let bytes = input.as_bytes();
    if bytes[start] == b'"' {
        return quoted_end(bytes, start + 1, false);
    }
    if bytes[start] == b'\\' && bytes.get(start + 1) == Some(&b'"') {
        return quoted_end(bytes, start + 2, true);
    }

    let first_end = token_end(bytes, start);
    let scheme = &input[start..first_end];
    if !["basic", "bearer", "digest"].contains(&scheme.to_ascii_lowercase().as_str()) {
        return preserve_escaped_quote(bytes, start, first_end);
    }

    let mut credential_start = first_end;
    while bytes
        .get(credential_start)
        .is_some_and(u8::is_ascii_whitespace)
    {
        credential_start += 1;
    }
    let credential_end = token_end(bytes, credential_start);
    preserve_escaped_quote(bytes, credential_start, credential_end)
}

fn quoted_end(bytes: &[u8], mut index: usize, escaped_delimiter: bool) -> usize {
    while index < bytes.len() {
        if escaped_delimiter && bytes[index] == b'\\' && bytes.get(index + 1) == Some(&b'"') {
            return index;
        }
        if !escaped_delimiter && bytes[index] == b'"' {
            return index;
        }
        index += 1;
    }
    bytes.len()
}

fn token_end(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

fn preserve_escaped_quote(bytes: &[u8], start: usize, end: usize) -> usize {
    if end >= start + 2 && bytes.get(end - 2..end) == Some(br#"\""#) {
        end - 2
    } else if end > start && bytes[end - 1] == b'"' {
        end - 1
    } else {
        end
    }
}
