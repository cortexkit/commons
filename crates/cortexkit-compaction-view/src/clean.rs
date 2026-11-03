//! A cleaned candidate for an assistant reply, with the caller's route policy.
//!
//! A model sees `§N§ ` prefixes in front of text and tool results, and starts
//! its own reply with the next number, the way the runner will number it. It
//! also copies the notation where it does not belong: after heavy drops,
//! models start writing numbers as delimiters, including broken forms such as
//! `§40827">`, `§103012$` or a lone `§`. Left in the transcript, those
//! reinforce themselves in every later request. So the runner cleans a reply
//! when it maps it to neutral text and when it renders it. Never apply this
//! cleaning to user, notice, or tool text. Whether stored host text is also
//! cleaned is the runner's choice.
//!
//! The patterns use JavaScript regular-expression semantics so Rust and
//! JavaScript runners agree: `\s` is JavaScript's whitespace set
//! ([`is_js_space`]) and `\w` is ASCII letters, digits and underscore.
//!
//! The function returns a candidate that may be empty, such as for a reply
//! containing only its prefix. The caller applies the compaction contract's
//! empty-text policy (W4.6): use an empty candidate only on a route that accepts
//! empty text; otherwise retain the original.

const SECTION: char = '§';

/// A cleaned reply candidate, with the number the model wrote in front of it.
/// The original whitespace is not retained.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CleanedReply {
    /// The first number among the prefixes the model wrote at the start that
    /// fits a `u64`, if any. Every leading prefix is stripped, including one
    /// whose number overflows, so after an oversized first prefix this is the
    /// next prefix's number. Re-prefixing with the same number reproduces the original
    /// bytes only when the reply already uses the canonical prefix and cleaning
    /// leaves the remainder unchanged. For the contract's broader original-byte
    /// preservation rule, retain and conditionally reuse the original reply.
    pub model_tag: Option<u64>,
    /// The text with every piece of tag notation removed, trimmed.
    pub text: String,
}

/// Return a cleaned candidate for an assistant reply. The caller applies the
/// empty-text policy described in this module; storage cleaning is optional.
/// In order: the leading prefixes are split off;
/// every remaining well-formed `§N§` is removed; then the malformed hybrids
/// `§N">`, `§N">§` and `§N">§M§`; then a dangling `§N` with at most one
/// improvised closer that is not a word character (never followed by a digit,
/// or by `.` and a digit, so a section reference such as `§5.1` keeps its
/// number); then any stray `§`; then surrounding whitespace.
pub fn clean_reply(text: &str) -> CleanedReply {
    let (model_tag, rest) = split_leading(text);
    let rest = remove_matches(rest, well_formed_pair);
    let rest = remove_matches(&rest, malformed_hybrid);
    let rest = remove_matches(&rest, dangling_open);
    let rest: String = rest.chars().filter(|&c| c != SECTION).collect();
    CleanedReply {
        model_tag,
        text: rest.trim_matches(is_js_space).to_owned(),
    }
}

/// Split the well-formed prefixes a model wrote at the start (`§N§`, each
/// followed by any whitespace, as often as repeated) from the text. Returns
/// the number of the first prefix, the model's own tag (W4.6), and the rest.
/// When that first number does not fit a `u64`, the model named no valid
/// tag and the result is `None`: a later prefix never stands in for it.
/// Every leading prefix is stripped either way.
pub fn split_leading(text: &str) -> (Option<u64>, &str) {
    let mut rest = text;
    let mut first = None;
    let mut seen = false;
    while let Some(length) = well_formed_pair(rest) {
        if !seen {
            seen = true;
            first = rest[SECTION.len_utf8()..length - SECTION.len_utf8()]
                .parse()
                .ok();
        }
        rest = rest[length..].trim_start_matches(is_js_space);
    }
    (first, rest)
}

/// JavaScript's `\s` (and what `String.prototype.trim` removes): ECMAScript
/// WhiteSpace and LineTerminator. It differs from Rust's `char::is_whitespace`
/// by including U+FEFF and leaving out U+0085.
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// JavaScript's `\w`.
fn is_js_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Remove every non-overlapping match, scanning left to right as a global
/// JavaScript `replace` does. Each pattern starts with `§`, so only positions
/// holding one are tried; `matcher` gets the text from there and returns the
/// match's length in bytes.
fn remove_matches(text: &str, matcher: fn(&str) -> Option<usize>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(SECTION) {
        out.push_str(&rest[..start]);
        let at = &rest[start..];
        match matcher(at) {
            Some(length) => rest = &at[length..],
            None => {
                out.push(SECTION);
                rest = &at[SECTION.len_utf8()..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The length of a run of ASCII digits at the start of `text`.
fn digits(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

/// `§\d+§`
fn well_formed_pair(text: &str) -> Option<usize> {
    let after_open = text.strip_prefix(SECTION)?;
    let count = digits(after_open);
    if count == 0 {
        return None;
    }
    after_open[count..].strip_prefix(SECTION)?;
    Some(2 * SECTION.len_utf8() + count)
}

/// `§\d+">(?:§(?:\d+§)?)?`
fn malformed_hybrid(text: &str) -> Option<usize> {
    let after_open = text.strip_prefix(SECTION)?;
    let count = digits(after_open);
    if count == 0 {
        return None;
    }
    let after_quote = after_open[count..].strip_prefix("\">")?;
    let mut length = SECTION.len_utf8() + count + 2;
    if let Some(after_section) = after_quote.strip_prefix(SECTION) {
        length += SECTION.len_utf8();
        let inner = digits(after_section);
        if inner > 0 && after_section[inner..].starts_with(SECTION) {
            length += inner + SECTION.len_utf8();
        }
    }
    Some(length)
}

/// `§\d+(?!\d)(?!\.\d)[^\s§\w.]?`
fn dangling_open(text: &str) -> Option<usize> {
    let after_open = text.strip_prefix(SECTION)?;
    let count = digits(after_open);
    if count == 0 {
        return None;
    }
    let after_digits = &after_open[count..];
    let mut chars = after_digits.chars();
    let next = chars.next();
    // A decimal section reference (`§5.1`) is not a tag: backtracking over
    // fewer digits would leave a digit next, which the pattern also refuses.
    if next == Some('.') && chars.next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let closer = next
        .filter(|&c| !is_js_space(c) && c != SECTION && !is_js_word(c) && c != '.')
        .map_or(0, char::len_utf8);
    Some(SECTION.len_utf8() + count + closer)
}

#[cfg(test)]
mod tests {
    //! Compatibility cases for JavaScript-style reply cleaning, including
    //! replies with empty text.

    use super::*;

    fn clean(text: &str) -> String {
        clean_reply(text).text
    }

    #[test]
    fn dangling_openers_go_with_their_improvised_closer() {
        assert_eq!(clean("§103012$ Fixed it"), "Fixed it");
        assert_eq!(clean("§11865\u{04A9} done"), "done");
        assert_eq!(clean("§42 files changed"), "files changed");
        assert_eq!(clean("§42important"), "important");
        assert_eq!(clean("mid §11865\u{04A9} text"), "mid  text");
    }

    #[test]
    fn decimal_section_references_keep_their_numbers() {
        assert_eq!(
            clean("§12.3 cites the specification; see §123.45 and §5.1."),
            "12.3 cites the specification; see 123.45 and 5.1."
        );
        assert_eq!(
            clean("§123.45 begins this citation"),
            "123.45 begins this citation"
        );
        assert_eq!(clean("§12."), ".");
        assert_eq!(clean("end §12"), "end");
        assert_eq!(clean("§12 before"), "before");
    }

    #[test]
    fn well_formed_pairs_go_anywhere() {
        assert_eq!(clean("§42§ hi"), "hi");
        assert_eq!(clean("before §12§ after"), "before  after");
        assert_eq!(clean("§2030§ §2030§ Run"), "Run");
        assert_eq!(
            clean("Looking at §40827§ the result is X"),
            "Looking at  the result is X"
        );
        assert_eq!(clean("2030  §42§ Hello"), "2030   Hello");
    }

    #[test]
    fn malformed_hybrids_go() {
        assert_eq!(
            clean("Hello §40827\">Result confirmed"),
            "Hello Result confirmed"
        );
        assert_eq!(clean("§15298\">§15298§ "), "");
    }

    #[test]
    fn bare_digits_stay() {
        assert_eq!(
            clean("2030  2030  2030\u{00B0} Run clippy"),
            "2030  2030  2030\u{00B0} Run clippy"
        );
        assert_eq!(
            clean("99  Actually executing now. Running fmt:"),
            "99  Actually executing now. Running fmt:"
        );
    }

    #[test]
    fn a_reply_of_only_its_prefix_cleans_to_empty() {
        assert_eq!(clean("§58§"), "");
        assert_eq!(clean("§42§ "), "");
        assert_eq!(clean("   "), "");
    }

    #[test]
    fn the_model_tag_is_the_first_leading_number() {
        let cleaned = clean_reply("§12§ §14§ Done. §3§ was the old read.");
        assert_eq!(cleaned.model_tag, Some(12));
        assert_eq!(cleaned.text, "Done.  was the old read.");
        assert_eq!(clean_reply("Done §12§").model_tag, None);
        assert_eq!(clean_reply("§12 open").model_tag, None);
    }

    #[test]
    fn a_correct_self_tag_renders_back_byte_for_byte() {
        let written = "§12§ Done.";
        let cleaned = clean_reply(written);
        assert_eq!(
            format!("{}{}", crate::tags::tag_prefix(12), cleaned.text),
            written
        );
    }

    #[test]
    fn javascript_whitespace_is_trimmed_and_nel_is_not() {
        assert_eq!(clean("\u{FEFF}§3§\u{FEFF}hi\u{FEFF}"), "hi");
        assert_eq!(clean("hi\u{0085}"), "hi\u{0085}");
    }
}
