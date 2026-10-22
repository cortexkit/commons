//! Number text (`RULES.md` §12): a provider's number reaches the reply with
//! the provider's own spelling.
//!
//! A runner that decodes a number into binary floating point and encodes
//! it again can change its spelling (`0.10` becomes `0.1`, `1E+30` becomes
//! `1e30`) and, past 17 significant digits, its value. Callers threshold on these numbers, so a role keeps
//! the text: each known numeric member is a [`NumberText`], serde_json's
//! [`RawValue`], which holds the exact bytes the provider sent.
//!
//! The serde_json features `arbitrary_precision` and `float_roundtrip`
//! would also keep spellings, but they change number parsing for every
//! crate in a workspace build. No role crate enables them.

use serde_json::{value::RawValue, Number};

/// A JSON value kept as the exact text it arrived as.
pub type NumberText = Box<RawValue>;

/// Whether `text` is one JSON number literal: an optional `-`, an integer
/// part without leading zeros, an optional fraction and an optional
/// exponent, with no surrounding whitespace.
pub fn is_number_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    let digits = |at: &mut usize| {
        let start = *at;
        while *at < bytes.len() && bytes[*at].is_ascii_digit() {
            *at += 1;
        }
        *at - start
    };
    if bytes.get(at) == Some(&b'-') {
        at += 1;
    }
    match bytes.get(at) {
        Some(b'0') => at += 1,
        Some(b'1'..=b'9') => {
            digits(&mut at);
        }
        _ => return false,
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        if digits(&mut at) == 0 {
            return false;
        }
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        if digits(&mut at) == 0 {
            return false;
        }
    }
    at == bytes.len()
}

/// Whether `raw` holds one JSON number.
pub fn is_number_text(raw: &RawValue) -> bool {
    is_number_literal(raw.get())
}

/// `text` as number text, or why it is not one JSON number literal.
pub fn number_text(text: &str) -> Result<NumberText, String> {
    if !is_number_literal(text) {
        return Err(format!("{text:?} is not a JSON number"));
    }
    RawValue::from_string(text.to_owned()).map_err(|e| e.to_string())
}

/// `number` as number text, in serde_json's spelling of it. For building a
/// value a runner computed itself; a provider's number is kept as it came.
pub fn number_text_of(number: &Number) -> NumberText {
    RawValue::from_string(number.to_string()).expect("a Number's text is JSON")
}

/// Whether two optional raw values hold the same text.
pub fn same_text(a: Option<&RawValue>, b: Option<&RawValue>) -> bool {
    a.map(RawValue::get) == b.map(RawValue::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_literals() {
        for good in ["0", "-0", "0.10", "1e-7", "1E+30", "12.500", "-3"] {
            assert!(is_number_literal(good), "{good}");
        }
        for bad in [
            "", "-", "01", "1.", ".5", "1e", "+1", " 1", "1 ", "NaN", "\"1\"", "null",
        ] {
            assert!(!is_number_literal(bad), "{bad}");
        }
    }

    #[test]
    fn number_text_keeps_the_spelling_through_a_decode() {
        let decoded: Vec<NumberText> = serde_json::from_str("[0.10,1e-7,1E+30,12.500]").unwrap();
        let texts: Vec<&str> = decoded.iter().map(|raw| raw.get()).collect();
        assert_eq!(texts, ["0.10", "1e-7", "1E+30", "12.500"]);
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            "[0.10,1e-7,1E+30,12.500]"
        );
        assert!(number_text("0.10").is_ok());
        assert!(number_text("\"x\"").is_err());
    }
}
