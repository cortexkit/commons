//! Cleaning the tag notation a model writes itself, and the text a runner
//! renders for a counted part.
//!
//! A port of the tag cleaning in standalone Magic Context, the plugin that
//! manages context inside OpenCode and Pi (`tag-content-primitives.ts`,
//! `text-complete.ts`), so every runner and
//! every compaction provider produce the same bytes. The port follows the
//! JavaScript regular expressions, with the two deliberate differences named
//! last:
//!
//! - `\d` is an ASCII digit and `\w` an ASCII letter, digit or `_`, as in a
//!   JavaScript regular expression without the `u` flag.
//! - `\s` and `String.prototype.trim` use JavaScript's whitespace set
//!   ([`is_js_whitespace`]), which differs from Rust's
//!   [`char::is_whitespace`] (JavaScript counts U+FEFF and not U+0085).
//! - First difference: where a JavaScript pattern consumes "one character"
//!   as an improvised closer, it consumes one UTF-16 code unit, so a closer
//!   outside the Basic Multilingual Plane (an emoji) loses only its first
//!   half there. A Rust string cannot hold half a character, so this port
//!   consumes the whole character.
//! - Second difference: a `§` followed by digits, then `.` and a digit, is
//!   never a dangling tag, at any digit count. Standalone's pattern
//!   `§\d+(?!\.\d)` backtracks one digit when the whole run is followed by a
//!   decimal, so it matches `§1` in `§12.3` and leaves `2.3`. This port instead
//!   treats the whole digit run as a section reference, leaving `12.3` after
//!   removing the stray sigil.

use super::SIGIL;

/// Whether `c` is whitespace to a JavaScript regular expression's `\s` and to
/// `String.prototype.trim`: the ECMAScript WhiteSpace and LineTerminator
/// characters.
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// A JavaScript `\w`: an ASCII letter, an ASCII digit or `_`.
fn is_js_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The byte index just after the run of ASCII digits starting at `from`.
fn digits_end(text: &str, from: usize) -> usize {
    from + text[from..]
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count()
}

/// The byte index just after the run of JavaScript whitespace starting at
/// `from` (`\s*`).
fn whitespace_end(text: &str, from: usize) -> usize {
    from + text[from..]
        .chars()
        .take_while(|c| is_js_whitespace(*c))
        .map(char::len_utf8)
        .sum::<usize>()
}

/// `§\d+` at `at`: the byte index after the digits, or `None` when `at` does
/// not hold the sigil followed by at least one digit.
fn sigil_digits(text: &str, at: usize) -> Option<usize> {
    let after = at + text[at..].strip_prefix(SIGIL).map(|_| SIGIL.len_utf8())?;
    let end = digits_end(text, after);
    (end > after).then_some(end)
}

/// `§\d+§` at `at`: a complete pair. The digit run is greedy, and a shorter
/// run would leave a digit, never the closing sigil, so there is nothing to
/// backtrack into.
fn complete_pair(text: &str, at: usize) -> Option<usize> {
    let end = sigil_digits(text, at)?;
    text[end..]
        .starts_with(SIGIL)
        .then(|| end + SIGIL.len_utf8())
}

/// `§\d+">§(?:\d+§)?` at `at`: the malformed hybrid as a leading prefix
/// sees it (the opening sigil after `">` is required).
fn malformed_prefix_item(text: &str, at: usize) -> Option<usize> {
    let end = sigil_digits(text, at)?;
    let rest = text[end..].strip_prefix("\">")?.strip_prefix(SIGIL)?;
    let after = text.len() - rest.len();
    Some(complete_tail(text, after).unwrap_or(after))
}

/// `§\d+">(?:§(?:\d+§)?)?` at `at`: the malformed hybrid anywhere in the
/// text (the sigil after `">` is optional).
fn malformed_anywhere(text: &str, at: usize) -> Option<usize> {
    let end = sigil_digits(text, at)?;
    let rest = text[end..].strip_prefix("\">")?;
    let after = text.len() - rest.len();
    let Some(rest) = rest.strip_prefix(SIGIL) else {
        return Some(after);
    };
    let after = text.len() - rest.len();
    Some(complete_tail(text, after).unwrap_or(after))
}

/// `\d+§` at `at`, the optional tail of a malformed hybrid.
fn complete_tail(text: &str, at: usize) -> Option<usize> {
    let end = digits_end(text, at);
    (end > at && text[end..].starts_with(SIGIL)).then(|| end + SIGIL.len_utf8())
}

/// `§\d+` closed with at most one improvised character (`[^\s§\w.]?`) at
/// `at`: a dangling tag, the model's own `§N` closed with something other
/// than `§`, or with nothing.
///
/// The digit run is taken whole. When it is followed by `.` and a digit
/// (`§5.1`, `§12.3`), it is a section reference and never a dangling tag,
/// at any digit count; the stray-sigil pass then removes only its `§`.
fn dangling(text: &str, at: usize) -> Option<usize> {
    let end = sigil_digits(text, at)?;
    let mut rest = text[end..].chars();
    if rest.next() == Some('.') && rest.next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    match text[end..].chars().next() {
        Some(c) if !(is_js_whitespace(c) || c == SIGIL || is_js_word(c) || c == '.') => {
            Some(end + c.len_utf8())
        }
        _ => Some(end),
    }
}

/// Remove a leading run of `item`, each followed by optional whitespace:
/// the JavaScript `^(?:item\s*)+`. Each repetition is greedy and the
/// pattern has nothing after the group, so it never backtracks into an
/// earlier repetition and a plain loop matches the same text.
fn strip_leading(text: &str, item: fn(&str, usize) -> Option<usize>) -> &str {
    let mut at = 0;
    while let Some(end) = item(text, at) {
        at = whitespace_end(text, end);
    }
    &text[at..]
}

/// Remove every match of `item` (a pattern that starts with the sigil),
/// scanning left to right as a JavaScript global replace does: after a
/// match the scan resumes where it ended, otherwise one character later.
fn remove_everywhere(text: &str, item: fn(&str, usize) -> Option<usize>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if let Some(end) = text[at..]
            .starts_with(SIGIL)
            .then(|| item(text, at))
            .flatten()
        {
            at = end;
            continue;
        }
        let c = text[at..].chars().next().expect("at is a char boundary");
        out.push(c);
        at += c.len_utf8();
    }
    out
}

/// Trim JavaScript whitespace from both ends, as `String.prototype.trim`.
fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

/// The cleaning of a model's own text: what every exposure of it shows (a
/// read, a resolve, a run's final text, a render, the summarizer's feed) for
/// text a tag-rendering run wrote. Standalone's save-time clean, in order:
///
/// 1. remove a leading run of `§N§`, each with the whitespace after it;
/// 2. remove every complete `§N§` anywhere;
/// 3. remove every malformed `§N">…` hybrid;
/// 4. remove every `§N` closed with an improvised character, but never one
///    followed by a decimal, at any digit count (`§5.1`, `§12.3`);
/// 5. remove every remaining `§`, then trim whitespace from both ends.
///
/// The result holds no `§` at all, so cleaning it again only trims, which
/// it already is: the function is idempotent. A section reference loses its
/// sigil (`§5.1` becomes `5.1`); bare digits are never removed.
pub fn clean_model_text(text: &str) -> String {
    let text = strip_leading(text, complete_pair);
    let text = remove_everywhere(text, complete_pair);
    let text = remove_everywhere(&text, malformed_anywhere);
    let text = remove_everywhere(&text, dangling);
    let text: String = text.chars().filter(|c| *c != SIGIL).collect();
    js_trim(&text).to_owned()
}

/// How many rounds [`strip_leading_tag_notation`] makes at most, as
/// standalone does.
pub const LEADING_STRIP_ROUNDS: usize = 8;

/// Strip only tag notation from the start of `text`, never bare digits
/// (`99 files`, `2024 roadmap` and numbered lists are content). Each round
/// removes, in order, a leading run of malformed hybrids, a leading run of
/// complete `§N§`, then a leading run of dangling `§N` (last, so a complete
/// pair is never cut into `§N` and an orphaned `§`), each with the
/// whitespace after it; rounds repeat until one changes nothing, at most
/// [`LEADING_STRIP_ROUNDS`] times.
///
/// Idempotent on every text it settles within those rounds: the round
/// that changed nothing would change nothing again.
pub fn strip_leading_tag_notation(text: &str) -> &str {
    let mut stripped = text;
    for _ in 0..LEADING_STRIP_ROUNDS {
        let before = stripped.len();
        stripped = strip_leading(stripped, malformed_prefix_item);
        stripped = strip_leading(stripped, complete_pair);
        stripped = strip_leading(stripped, dangling);
        if stripped.len() == before {
            break;
        }
    }
    stripped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javascript_whitespace_is_not_rust_whitespace() {
        assert!(is_js_whitespace('\u{FEFF}'));
        assert!(!'\u{FEFF}'.is_whitespace());
        assert!(!is_js_whitespace('\u{0085}'));
        assert!('\u{0085}'.is_whitespace());
        assert_eq!(clean_model_text("\u{FEFF}hi\u{0085}"), "hi\u{0085}");
    }

    #[test]
    fn a_decimal_after_any_digit_count_is_never_a_dangling_tag() {
        assert_eq!(clean_model_text("see §12.3 now"), "see 12.3 now");
        assert_eq!(clean_model_text("see §123.4 now"), "see 123.4 now");
        assert_eq!(clean_model_text("see §5.1 now"), "see 5.1 now");
        assert_eq!(strip_leading_tag_notation("§12.3 of it"), "§12.3 of it");
    }

    #[test]
    fn an_improvised_closer_is_consumed_whole() {
        assert_eq!(clean_model_text("§42\u{1F600} done"), "done");
    }
}
