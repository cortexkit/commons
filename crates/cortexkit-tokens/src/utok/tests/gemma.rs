//! Gemma 4's tokenizer, which the Gemini family counts with: the engine's
//! merge rules on small hand-built tables, then counts against Hugging
//! Face's `tokenizers` on the pinned vocabulary (`fixtures/gemma4.json`,
//! made by `tools/gen-gemma-fixture.py`), and the counts Google's own
//! `countTokens` reported for Gemini.

use crate::utok::{
    Encoding,
    gemma::{Gemma, gemma4},
};

/// A table with the 256 byte tokens (ids 0..=255), `▁` (256), one empty
/// slot (257), then `pieces` from id 258, merged by `merges` in list order.
fn table(pieces: &[&str], merges: &[(&str, &str)]) -> (Gemma, Vec<String>) {
    let mut vocab: Vec<String> = (0..=255u8).map(|byte| format!("<0x{byte:02X}>")).collect();
    vocab.push("\u{2581}".to_owned());
    vocab.push(String::new());
    vocab.extend(pieces.iter().map(|piece| (*piece).to_owned()));
    let id = |piece: &str| {
        vocab
            .iter()
            .position(|p| p == piece)
            .expect("piece in vocab")
    };

    let mut vocab_raw = b"GVOCAB1\n".to_vec();
    vocab_raw.extend(u32::try_from(vocab.len()).unwrap().to_le_bytes());
    for piece in &vocab {
        vocab_raw.extend(piece.as_bytes());
        vocab_raw.push(0);
    }
    let mut merges_raw = b"GMERGE1\n".to_vec();
    merges_raw.extend(u32::try_from(merges.len()).unwrap().to_le_bytes());
    let mut previous = 0i64;
    for (left, right) in merges {
        let produced = i64::try_from(id(&format!("{left}{right}"))).unwrap();
        let delta = produced - previous;
        let zigzag = if delta >= 0 {
            delta << 1
        } else {
            ((-delta) << 1) - 1
        };
        push_varint(&mut merges_raw, u64::try_from(zigzag).unwrap());
        push_varint(&mut merges_raw, left.len() as u64);
        previous = produced;
    }
    let gemma = Gemma::parse(
        &zstd::encode_all(vocab_raw.as_slice(), 0).unwrap(),
        &zstd::encode_all(merges_raw.as_slice(), 0).unwrap(),
    );
    (gemma, vocab)
}

fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// `text` encoded with `gemma`, as pieces.
fn pieces(gemma: &Gemma, vocab: &[String], text: &str) -> Vec<String> {
    let ids = gemma.encode(text.as_bytes());
    assert_eq!(gemma.count(text.as_bytes()), ids.len(), "count of {text:?}");
    ids.into_iter()
        .map(|id| vocab[id as usize].clone())
        .collect()
}

#[test]
fn merge_priority_is_list_order_not_vocabulary_order() {
    // `bc` has the lower id, but `a + b` comes first in the merges list.
    let (gemma, vocab) = table(&["a", "b", "c", "bc", "ab"], &[("a", "b"), ("b", "c")]);
    assert_eq!(pieces(&gemma, &vocab, "abc"), ["ab", "c"]);
}

#[test]
fn a_lower_ranked_merge_wins_over_a_leftmost_one() {
    let (gemma, vocab) = table(&["a", "b", "c", "ab", "bc"], &[("b", "c"), ("a", "b")]);
    assert_eq!(pieces(&gemma, &vocab, "abc"), ["a", "bc"]);
}

#[test]
fn equal_ranks_merge_leftmost_first() {
    let (gemma, vocab) = table(&["a", "aa"], &[("a", "a")]);
    assert_eq!(pieces(&gemma, &vocab, "aaa"), ["aa", "a"]);
    assert_eq!(pieces(&gemma, &vocab, "aaaa"), ["aa", "aa"]);
}

#[test]
fn merges_build_on_earlier_merges_across_the_whole_text() {
    let (gemma, vocab) = table(
        &["a", "b", "ab", "abab", "\n", "\n\u{2581}", "\n\u{2581}ab"],
        &[
            ("a", "b"),
            ("ab", "ab"),
            ("\n", "\u{2581}"),
            ("\n\u{2581}", "ab"),
        ],
    );
    assert_eq!(pieces(&gemma, &vocab, "abab"), ["abab"]);
    // No pre-tokenizer: a merge may join a newline, a space and a word.
    assert_eq!(
        pieces(&gemma, &vocab, "abab\n ab"),
        ["abab", "\n\u{2581}ab"]
    );
}

#[test]
fn a_piece_without_a_merge_is_never_formed() {
    // `ac` is in the vocabulary, but no merge produces it.
    let (gemma, vocab) = table(&["a", "c", "ac"], &[]);
    assert_eq!(pieces(&gemma, &vocab, "ac"), ["a", "c"]);
}

/// The per-thread memo of segment counts is shared by every table a thread
/// counts with: a count cached under one table must never answer for
/// another, even for the same symbols.
#[test]
fn the_segment_memo_keeps_tables_apart() {
    let (merges, _) = table(&["a", "b", "ab"], &[("a", "b")]);
    let (no_merges, _) = table(&["a", "b", "ab"], &[]);
    for _ in 0..2 {
        assert_eq!(merges.count("ab ab".as_bytes()), 3);
        assert_eq!(no_merges.count("ab ab".as_bytes()), 5);
    }
}

#[test]
fn spaces_become_the_space_piece_and_unknown_characters_fall_back_to_bytes() {
    let (gemma, vocab) = table(&["a"], &[]);
    assert_eq!(pieces(&gemma, &vocab, " a"), ["\u{2581}", "a"]);
    assert_eq!(
        pieces(&gemma, &vocab, "é\r"),
        ["<0xC3>", "<0xA9>", "<0x0D>"]
    );
    assert_eq!(pieces(&gemma, &vocab, ""), Vec::<String>::new());
}

/// The texts behind the edge-case counts Gemini's `countTokens` returned
/// (`gemini-3.8-flash` and `gemini-3.1-pro-preview` agree), minus the one
/// token the API adds for the user turn.
#[test]
fn gemini_edge_texts_match_measured_counts() {
    let cases = [
        ("x", 1),
        (
            "Şimdi çalışıyoruz: ğüşıöç İIıi. 你好，世界！今日は良い天気ですね。مرحبا بالعالم 🚀👍🏽👨\u{200d}👩\u{200d}👧 Ünïcödé façade naïve — “quotes” ‘single’ … ¿Qué? ½ ∑ ∞ ≠ ≤",
            63,
        ),
        (
            "fn main() {\n\tlet  x =   1;\t\t// tabs\n\n\n    if x == 1 {\n        println!(\"{}\",  x);   \n    }\n}\n\n\n   trailing   \n \n",
            44,
        ),
        (
            r#"{"name":"ctx_search","description":"Search the archive \u2014 everything","input_schema":{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"number"}},"required":["query"],"additionalProperties":false}}"#,
            54,
        ),
        (
            "   leading spaces and  double  spaces and\r\nCRLF lines\r\n\u{a0}nbsp\u{200b}zero-width",
            23,
        ),
        (
            "1234567890 3.14159 0xDEADBEEF 2026-11-15T12:59:59.999Z 100,000 tokens 1e-9 v0.4.45",
            71,
        ),
    ];
    for (text, want) in cases {
        assert_eq!(Encoding::Gemma4.count(text), want, "{text:?}");
    }
}

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../fixtures/gemma4.json")).expect("fixture parses")
}

fn cases<'a>(fixture: &'a serde_json::Value, key: &str) -> Vec<(&'a str, u32)> {
    fixture[key]
        .as_array()
        .expect("case list")
        .iter()
        .map(|case| {
            let text = case[0].as_str().expect("text");
            let count = u32::try_from(case[1].as_u64().expect("count")).unwrap();
            (text, count)
        })
        .collect()
}

#[test]
fn gemma4_matches_hugging_face_counts() {
    let fixture = fixture();
    let cases = cases(&fixture, "cases");
    assert!(cases.len() > 3000, "the fixture holds a few thousand texts");
    let wrong: Vec<_> = cases
        .iter()
        .filter_map(|&(text, want)| {
            let got = Encoding::Gemma4.count(text);
            (got != want).then_some((text, want, got))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} of {} texts count differently, first ones (text, want, got): {:?}",
        wrong.len(),
        cases.len(),
        &wrong[..wrong.len().min(5)]
    );
}

/// Splitting the text into segments, rescanning short ones and memoizing
/// their counts are shortcuts; merging the whole text as one word with the
/// heap is the plain algorithm. Both must agree with the reference.
#[test]
fn merging_the_whole_text_as_one_word_gives_the_same_counts() {
    let fixture = fixture();
    for (text, want) in cases(&fixture, "cases") {
        assert_eq!(
            gemma4().count_as_one_word(text.as_bytes()),
            want as usize,
            "{text:?}"
        );
    }
}

/// `tokenizers` turns a special token written in the text into that one
/// token; this crate counts it as plain text, like the other families. The
/// fixture counts these with `tokenizers`' plain-text mode.
#[test]
fn special_token_literals_count_as_plain_text() {
    let fixture = fixture();
    for (text, want) in cases(&fixture, "special_token_literals") {
        assert_eq!(Encoding::Gemma4.count(text), want, "{text:?}");
    }
}

#[test]
fn gemma4_utf16_utf32_parity() {
    let fixture = fixture();
    for (text, want) in cases(&fixture, "cases") {
        let u16s: Vec<u16> = text.encode_utf16().collect();
        let u32s: Vec<u32> = text.chars().map(u32::from).collect();
        assert_eq!(
            Encoding::Gemma4.count(u16s.as_slice()),
            want,
            "utf16 {text:?}"
        );
        assert_eq!(
            Encoding::Gemma4.count(u32s.as_slice()),
            want,
            "utf32 {text:?}"
        );
    }
}

#[test]
fn encode_returns_hugging_face_ids() {
    // Ids from `tokenizers` on the pinned file.
    let ids = Encoding::Gemma4
        .encode("Hello world\n")
        .expect("Gemma encodes");
    assert_eq!(ids, [9259, 1902, 107]);
}

/// The calibration texts whose counts Gemini's `countTokens` returned: a
/// system prompt, documentation, conversation history, memories, tool
/// declarations and a Rust source file, about 360 KB. They are not shipped
/// with the crate; point `CORTEXKIT_TOKENS_GEMINI_CALIBRATION` at a
/// directory holding them and run with `--ignored`. Gemini's API reported
/// one more token for the documentation, history and memories: the probe
/// appended each to a one-word baseline message and subtracted the
/// baseline's count, which leaves one token where the two join. The system
/// prompt went in as the system instruction, and there the API's count
/// equals the text's own, as the counts below are. The tool declarations are
/// counted as JSON text here; Gemini counts declarations its own way
/// (20,722 for the same tools), so tool counts stay an estimate.
#[test]
#[ignore = "needs the calibration texts, which are not shipped"]
fn gemini_calibration_texts_match_measured_counts() {
    let dir = std::env::var("CORTEXKIT_TOKENS_GEMINI_CALIBRATION")
        .expect("set CORTEXKIT_TOKENS_GEMINI_CALIBRATION to the calibration directory");
    let dir = std::path::Path::new(&dir);
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name)).unwrap_or_else(|err| panic!("{name}: {err}"))
    };
    for (name, bytes, want) in [
        ("calib-system.txt", 64_931, 15_717),
        ("calib-docs.txt", 43_227, 10_545),
        ("calib-history.txt", 141_148, 24_462),
        ("calib-memory.txt", 12_736, 2_573),
        ("calib-tools.json", 88_193, 21_545),
    ] {
        let text = read(name);
        assert_eq!(text.len(), bytes, "{name} is not the measured text");
        assert_eq!(Encoding::Gemma4.count(&text), want, "{name}");
    }
    let edge: serde_json::Value = serde_json::from_str(&read("edge-texts.json")).unwrap();
    let code = edge["code"].as_str().expect("the Rust source text");
    assert_eq!(Encoding::Gemma4.count(code), 3_629, "edge-texts.json code");
}
