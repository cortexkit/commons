//! Deterministic neutralization of the sole permitted inbox envelope tag.

pub const ENVELOPE_TAG: &str = "system-reminder";

fn whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// Replace only a marker's ASCII `<` with `‹` (U+2039). See CONTRACT.md
/// for the prefix grammar; all remaining UTF-8 bytes are preserved.
pub fn neutralize(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'<' {
            continue;
        }
        let mut end = index + 1;
        while bytes.get(end).is_some_and(|b| whitespace(*b)) {
            end += 1;
        }
        if bytes.get(end) == Some(&b'/') {
            end += 1;
        }
        while bytes.get(end).is_some_and(|b| whitespace(*b)) {
            end += 1;
        }
        if bytes
            .get(end..end + ENVELOPE_TAG.len())
            .is_some_and(|name| name.eq_ignore_ascii_case(ENVELOPE_TAG.as_bytes()))
        {
            out.push_str(&text[copied..index]);
            out.push('‹');
            copied = index + 1;
        }
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutralize_marker_forms() {
        for input in [
            "<system-reminder>",
            "</system-reminder>",
            "< SYSTEM-REMINDER role='x'>",
            "</ system-reminder>",
            "<\nsystem-reminder",
            "<\t/\r\nSyStEm-ReMiNdErmore",
        ] {
            assert_eq!(neutralize(input), format!("‹{}", &input[1..]), "{input:?}");
        }
    }

    #[test]
    fn neutralize_nested_markers_and_preserves_non_markers() {
        assert_eq!(
            neutralize("λ<system-reminder>< / system-reminder>尾"),
            "λ‹system-reminder>‹ / system-reminder>尾"
        );
        for input in [
            "",
            "<",
            "</",
            "<system-reminde",
            "<system-\nreminder>",
            "＜system-reminder＞",
            "‹system-reminder>",
            "<ѕystem-reminder>",
            "<\u{a0}system-reminder>",
            "<\u{b}system-reminder>",
            "<other>",
        ] {
            assert_eq!(neutralize(input), input);
        }
    }

    #[test]
    fn neutralize_is_idempotent() {
        let fragments = [
            "<",
            "</",
            "< \n/\t",
            "system-reminder",
            "SYSTEM-REMINDER>",
            "‹system-reminder>",
            "<system-\nreminder>",
            "λ",
            "\r\n",
        ];
        for a in fragments {
            for b in fragments {
                for c in fragments {
                    let text = format!("{a}{b}{c}");
                    let once = neutralize(&text);
                    assert_eq!(neutralize(&once), once, "{text:?}");
                }
            }
        }
    }
}
