//! Incremental reply parsing: chunking equivalence, provisional revisions,
//! buffering caps and linear work.

use cortexkit_rich_ui_syntax::*;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

/// The revision for `source` fed in one chunk.
fn oneshot(source: &[u8]) -> Revision {
    let mut stream = StreamParser::new(Mode::Reply);
    stream.feed(source);
    stream.revision().clone()
}

/// Feeds `source` in chunks of the given sizes. After every feed the current
/// revision must equal the revision of the same prefix fed in one chunk
/// (`revision_at`), `feed` must return a revision exactly when it changed,
/// and `finish` must equal the one-shot parse.
fn check_chunks(
    source: &[u8],
    mode: Mode,
    chunks: &[usize],
    revision_at: &mut dyn FnMut(usize) -> Option<Revision>,
    expected: &Document,
    label: &str,
) {
    let mut stream = StreamParser::new(mode);
    let mut fed = 0;
    for &size in chunks {
        let end = (fed + size).min(source.len());
        let before = stream.revision().clone();
        let got = stream.feed(&source[fed..end]);
        fed = end;
        let now = stream.revision();
        match &got {
            Some(revision) => {
                assert_eq!(revision, now, "{label}: returned revision is current");
                assert_ne!(
                    revision, &before,
                    "{label}: unchanged revision emitted at {fed}"
                );
            }
            None => assert_eq!(
                now, &before,
                "{label}: changed revision not emitted at {fed}"
            ),
        }
        if mode == Mode::Program {
            assert!(got.is_none(), "{label}: programs are not drawn");
            assert!(now.nodes.is_empty());
        } else if let Some(expected) = revision_at(fed) {
            assert_eq!(now, &expected, "{label}: revision after {fed} bytes");
        }
    }
    assert_eq!(fed, source.len(), "{label}: chunks cover the source");
    assert_eq!(&stream.finish(), expected, "{label}: final parse");
}

/// Deterministic chunk sizes from 1 to `max`.
fn random_chunks(len: usize, seed: u64, max: usize) -> Vec<usize> {
    let mut state = seed | 1;
    let mut chunks = vec![];
    let mut total = 0;
    while total < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let size = 1 + (state as usize) % max;
        chunks.push(size);
        total += size;
    }
    chunks
}

/// Every split position, one-byte chunks, and adversarial chunkings.
fn chunkings(len: usize) -> Vec<(String, Vec<usize>)> {
    let mut all: Vec<(String, Vec<usize>)> = (0..=len)
        .map(|split| (format!("split at {split}"), vec![split, len - split]))
        .collect();
    all.push(("one-byte chunks".into(), vec![1; len]));
    for seed in [3, 0x5eed, 0xdead_beef] {
        all.push((format!("random chunks {seed}"), random_chunks(len, seed, 7)));
    }
    all.push((
        "alternating 1 and 3".into(),
        (0..len).map(|i| 1 + 2 * (i % 2)).collect(),
    ));
    all.push((
        "empty chunks between bytes".into(),
        (0..2 * len).map(|i| i % 2).collect(),
    ));
    all
}

/// Every chunking of `source` is equivalent to feeding it whole.
fn check_equivalence(source: &[u8]) {
    let label = String::from_utf8_lossy(source);
    for mode in [Mode::Reply, Mode::Program] {
        let expected = parse_bytes(source, mode);
        if let Ok(text) = std::str::from_utf8(source) {
            assert_eq!(
                parse(text, mode),
                expected,
                "{label:?}: parse and parse_bytes agree"
            );
        }
        let revisions: Vec<Revision> = if mode == Mode::Reply {
            (0..=source.len()).map(|n| oneshot(&source[..n])).collect()
        } else {
            vec![]
        };
        for (name, chunks) in chunkings(source.len()) {
            let mut at = |n: usize| revisions.get(n).cloned();
            check_chunks(
                source,
                mode,
                &chunks,
                &mut at,
                &expected,
                &format!("{label:?} {mode:?} {name}"),
            );
        }
    }
}

/// For inputs too long to try every split: every split up to `head`, every
/// split within `window` bytes of each listed `edge`, and `spread` more
/// evenly spaced; one-byte chunks are compared at the same positions.
fn check_sampled(source: &[u8], head: usize, edges: &[usize], window: usize, spread: usize) {
    let len = source.len();
    let mut positions: Vec<usize> = (0..=head.min(len)).collect();
    for &edge in edges {
        positions.extend(edge.saturating_sub(window)..=(edge + window).min(len));
    }
    positions.extend((1..=spread).map(|i| i * len / (spread + 1)));
    positions.push(len);
    positions.sort_unstable();
    positions.dedup();
    let revisions: std::collections::BTreeMap<usize, Revision> = positions
        .iter()
        .map(|&n| (n, oneshot(&source[..n])))
        .collect();
    let mut at = |n: usize| revisions.get(&n).cloned();
    let expected = parse_bytes(source, Mode::Reply);
    for &split in &positions {
        check_chunks(
            source,
            Mode::Reply,
            &[split, len - split],
            &mut at,
            &expected,
            &format!("split at {split}"),
        );
    }
    check_chunks(
        source,
        Mode::Reply,
        &vec![1; len],
        &mut at,
        &expected,
        "one-byte chunks",
    );
    check_chunks(
        source,
        Mode::Reply,
        &random_chunks(len, 7, 4096),
        &mut at,
        &expected,
        "random chunks",
    );
    let program = parse_bytes(source, Mode::Program);
    check_chunks(
        source,
        Mode::Program,
        &random_chunks(len, 11, 4096),
        &mut |_| None,
        &program,
        "program",
    );
}

fn sample_sources(name: &str) -> Vec<(String, String)> {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-vectors")
            .join("rich-ui-syntax-v1")
            .join(name),
    )
    .unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            (
                case["name"].as_str().unwrap().to_owned(),
                case["source"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn every_vector_is_equivalent_under_every_chunking() {
    for file in [
        "refs-scan-samples-v1.json",
        "refs-scan-samples-v1.1.json",
        "refs-scan-samples-v1.2.json",
    ] {
        for (_, source) in sample_sources(file) {
            check_equivalence(source.as_bytes());
        }
    }
}

/// Inputs built to break incremental parsing, each named for the hazard.
const ADVERSARIAL: &[&str] = &[
    // Unmatched backticks enclosing complete tags.
    "`<A x={$$tc(1)} />",
    "before `<A/>` after $$tc(2)",
    "`` <B>$$tc(2)</B> ` still open\n\nnext $$tc(3)",
    "```<A/>``\n<A/>",
    // Links containing tags.
    "[<A x={$$tc(1)} />](https://x.test) $$tc(2)",
    "[see <A/> here](u) and ![alt <B/>](img.png)",
    "[unclosed <A/> label\n\n$$tc(3)",
    "[a](b <A/>) [c]d [[e]](f) [g] (h)",
    // GFM tables.
    "| a | b |\n|---|---|\n| $$tc(1) | `x|y` |\n| \\| | <A/> |\n",
    "| a |\n| not delim\n",
    "a | b\n-- | --\n1 | 2\n\nafter",
    "| a | b |\n|---|\nmismatch",
    "> | q |\n> |:-:|\n> | $$tc(4) |",
    // Block and inline transitions.
    "text\n# heading\n- item\n> quote\n```\ncode\n```\n<A>\nblock\n</A>\nafter",
    "para <A>inline</A> more\n<B/>\n<C/> trailing",
    "- item\n\n  continued\n\nnot item\n1. one\n2) two\n10. ten",
    "***\n---\n___\n* * *\n- - -\n-- x\n**bold** *em* _u_ ~~s~~",
    "# h1 #\n###### h6\n####### seven\n#nospace",
    "~~~ info `x`\ncode $$tc(1)\n~~~~\n``` a`b\n",
    "  > nested\n  > > deeper\nlazy\n\n- a\n  - b\n    - c\n- d",
    "export const x = [\n  $$tc(1)\n]\nexport let $y = 1\n  ?? 2\ntext",
    "{a\n{b}\n\n} $$tc(1) {c} \\{ $$tc(2)",
    "https://example.test/$$tc(1) <https://x.test/$$tc(2)> $$tc(3)",
    // Escaped markers.
    "\\$$tc(1) \\`code\\` \\[x](y) \\<A/> \\*em\\* $\\$tc(2) \\\nnext",
    "$$$tc(3) $$tc(04) $$tc(0) $$tc() $$tc(12",
    // Malformed tags.
    "<A x={(1)} /> <A [x] /> $$tc(1)",
    "<A x=\"unterminated />\n\n$$tc(2)",
    "</A> stray <a> lower <A>unclosed\n\n$$tc(3)",
    "<A><B>$$tc(1)</A>$$tc(2)</B>",
    "<A x={Other($$tc(2))}>$$tc(3)</A> $$tc(4)",
    "<A x='a\\q' /> <B x={'\\u{110000}'} />",
    // A blank line inside an open tag.
    "<A x={$$tc(1)}\n\nthen $$tc(2) />",
    "<A>\n\npara $$tc(1)\n\n</A>\nafter",
    "text <A>open\n\nnext $$tc(2)",
    "<A>\n  \n</A>",
    // Line endings, BOM, multibyte characters.
    "\u{feff}é $$tc(3)\r\n$$tc(4)\r$$tc(5)\r\n\r\n| a |\r\n|---|\r\n",
    "🦀 *ü* `é` [ß](ç) <A t='🦀'/>",
];

#[test]
fn adversarial_inputs_are_equivalent_under_every_chunking() {
    for source in ADVERSARIAL {
        check_equivalence(source.as_bytes());
    }
    // Invalid UTF-8 is repaired the same way for every chunking.
    check_equivalence(b"a\xffb\xe2\x82 $$tc(1) \xf0\x9f\xa6");
    check_equivalence(b"\xed\xa0\x80`x\xc3`");
}

fn texts(nodes: &[Arc<DisplayNode>], out: &mut Vec<Display>) {
    for node in nodes {
        out.push(node.kind.clone());
        texts(&node.children, out);
    }
}

fn kinds(revision: &Revision) -> Vec<Display> {
    let mut out = vec![];
    texts(&revision.nodes, &mut out);
    out
}

fn has(revision: &Revision, f: impl Fn(&Display) -> bool) -> bool {
    kinds(revision).iter().any(f)
}

fn literal_text(revision: &Revision) -> String {
    kinds(revision)
        .into_iter()
        .filter_map(|kind| match kind {
            Display::Literal(text) => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn held_text_is_not_drawn_until_classified() {
    assert!(oneshot(b"`code").nodes.is_empty());
    // A closing run is known only once the character after it arrives.
    assert!(oneshot(b"`code`").nodes.is_empty());
    assert!(has(
        &oneshot(b"`code` "),
        |k| matches!(k, Display::Code(t) if t == "code")
    ));
    assert!(oneshot(b"[label](https://x").nodes.is_empty());
    assert!(has(&oneshot(b"[label](https://x)"), |k| matches!(
        k,
        Display::Link { image: false }
    )));
    assert!(oneshot(b"$$tc(1").nodes.is_empty());
    assert!(has(&oneshot(b"$$tc(1)"), |k| *k == Display::PendingReference));
    assert!(oneshot(b"<A x={1}").nodes.is_empty());
    assert!(oneshot(b"<A>body").nodes.is_empty());
    assert!(has(
        &oneshot(b"x <A>body</A>"),
        |k| matches!(k, Display::Tag { name, block: false } if name == "A")
    ));
}

#[test]
fn ordered_lists_keep_their_start_number() {
    let ordered = kinds(&oneshot(b"3. a\n4. b\n"));
    assert!(
        ordered.contains(&Display::List { start: Some(3) }),
        "{ordered:?}"
    );
    let bullets = kinds(&oneshot(b"- a\n- b\n"));
    assert!(
        bullets.contains(&Display::List { start: None }),
        "{bullets:?}"
    );
}

#[test]
fn a_continued_paragraph_draws_its_line_break_first() {
    // The second line's start is held (it may be a heading), then examined
    // again; the line break before it must still come first.
    for (source, second) in [("a\n#b", "#b"), ("a\n-x", "-x"), ("a\n12x", "12x")] {
        let revision = oneshot(source.as_bytes());
        let paragraph = &revision.nodes[0];
        let drawn: Vec<Display> = paragraph.children.iter().map(|n| n.kind.clone()).collect();
        assert_eq!(
            drawn,
            [
                Display::Text("a".into()),
                Display::SoftBreak,
                Display::Text(second.into())
            ],
            "{source:?}"
        );
    }
}

#[test]
fn display_text_repairs_utf8_like_parse_bytes() {
    // Each maximal invalid subsequence becomes one U+FFFD, as in
    // `String::from_utf8_lossy`, so display offsets match `parse_bytes`.
    let source = b"a\xffb\xe2\x82c\xe0\x80d\xf0\x9f\xa6e\xed\xa0\x80f\xc0\xafg\xf4\x90h";
    let repaired = String::from_utf8_lossy(source);
    let revision = oneshot(source);
    let paragraph = &revision.nodes[0];
    let text: String = paragraph
        .children
        .iter()
        .map(|n| match &n.kind {
            Display::Text(t) => t.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(text, repaired);
    assert_eq!(paragraph.span, Span::new(0, repaired.len()));
    let doc = parse_bytes(source, Mode::Reply);
    assert_eq!(doc.nodes[0].span, paragraph.span);
}

#[test]
fn blank_line_resolves_held_text() {
    // Held text that never resolved is drawn as text once its block ends,
    // and the next block is not swallowed by it.
    let revision = oneshot(b"`open [x $$tc(1\n\nnext");
    let all = kinds(&revision);
    assert!(
        all.iter()
            .any(|k| matches!(k, Display::Text(t) if t == "`open [x $$tc(1")),
        "{all:?}"
    );
    assert!(
        all.iter()
            .any(|k| matches!(k, Display::Text(t) if t == "next")),
        "{all:?}"
    );
    assert_eq!(revision.nodes.len(), 2, "{all:?}");
}

#[test]
fn complete_tag_inside_unresolved_link_or_code_is_not_drawn() {
    let is_tag = |k: &Display| matches!(k, Display::Tag { .. });
    assert!(has(&oneshot(b"x <A/>"), is_tag));
    for held in [&b"[see <A/> "[..], b"`<A/> ", b"![alt <A/> ", b"[x](<A/> "] {
        let revision = oneshot(held);
        assert!(
            !has(&revision, is_tag),
            "{:?}: {revision:?}",
            String::from_utf8_lossy(held)
        );
    }
    // Once the link resolves, its label is text: labels never hold components.
    let revision = oneshot(b"[see <A/>](u) ");
    assert!(!has(&revision, is_tag), "{revision:?}");
    assert!(has(&revision, |k| matches!(k, Display::Link { .. })));
}

#[test]
fn later_bytes_reclassify_earlier_provisional_text() {
    // A closing delimiter turns drawn text into emphasis.
    assert!(!has(&oneshot(b"**bold"), |k| *k == Display::Strong));
    assert!(has(&oneshot(b"**bold** "), |k| *k == Display::Strong));
    // A delimiter row turns a drawn paragraph line into a table header.
    let before = oneshot(b"| a | b |\n");
    assert!(has(&before, |k| *k == Display::Paragraph));
    let after = oneshot(b"| a | b |\n|---|---|\n");
    assert!(!has(&after, |k| *k == Display::Paragraph));
    assert!(has(&after, |k| *k == Display::TableHead));
    // The rest of a line decides between a block and an inline tag.
    assert!(oneshot(b"<A/>").nodes.is_empty());
    assert!(has(&oneshot(b"<A/>\n"), |k| matches!(
        k,
        Display::Tag { block: true, .. }
    )));
    assert!(has(&oneshot(b"<A/> x"), |k| matches!(
        k,
        Display::Tag { block: false, .. }
    )));
}

#[test]
fn provisional_output_is_inert() {
    let source = "[click](https://dest.test/secret) <Button action={Parse($$tc(7), 'csv')} label=\"secret-prop\">Go $$tc(8)</Button> ![i](https://img.test/x.png) $$tc(9)\n";
    let revision = oneshot(source.as_bytes());
    let all = kinds(&revision);
    let strings: Vec<&str> = all
        .iter()
        .filter_map(|kind| match kind {
            Display::Text(s) | Display::Literal(s) | Display::Code(s) => Some(s.as_str()),
            Display::Tag { name, .. } => Some(name.as_str()),
            Display::CodeBlock { language } => Some(language.as_str()),
            _ => None,
        })
        .collect();
    for withheld in [
        "dest.test",
        "img.test",
        "secret-prop",
        "action",
        "Parse",
        "$$tc",
        "csv",
    ] {
        assert!(
            strings.iter().all(|s| !s.contains(withheld)),
            "{withheld} leaked: {strings:?}"
        );
    }
    assert!(all.contains(&Display::PendingReference));
    assert!(all.contains(&Display::Link { image: true }));
    assert!(all
        .iter()
        .any(|k| matches!(k, Display::Tag { name, .. } if name == "Button")));
    // The final parse still has every reference.
    let mut stream = StreamParser::new(Mode::Reply);
    stream.feed(source.as_bytes());
    let refs: Vec<String> = stream
        .finish()
        .references()
        .into_iter()
        .map(|r| r.n)
        .collect();
    assert_eq!(refs, ["7", "8", "9"]);
}

/// A held context that overflows its cap is drawn as a sanitized literal,
/// later text is drawn without waiting, and the final parse is unaffected.
fn check_cap(source: &str, context_start: usize, overflow_at: usize, literal_prefix: &str) {
    let bytes = source.as_bytes();
    assert_eq!(
        oneshot(&bytes[..overflow_at - 1]),
        oneshot(&bytes[..context_start]),
        "held, not drawn, below the cap"
    );
    let revision = oneshot(&bytes[..overflow_at]);
    let literal = literal_text(&revision);
    assert!(
        literal.starts_with(literal_prefix),
        "{:?}",
        &literal[..literal.len().min(40)]
    );
    assert!(
        literal.contains('\u{fffd}') && !literal.contains('\u{1}'),
        "control sanitized"
    );
    // The bounded state is kept, not the prefix: the next bytes are drawn.
    let more = oneshot(&[&bytes[..overflow_at], b" tail"].concat());
    assert!(
        kinds(&more)
            .iter()
            .any(|k| matches!(k, Display::Text(t) if t.contains("tail"))),
        "text after the cap is drawn"
    );
    check_sampled(bytes, 32, &[overflow_at], 3, 8);
}

#[test]
fn each_buffer_cap_overflows_to_a_provisional_literal() {
    let filler = |n: usize| "\u{1}".to_owned() + &"a".repeat(n - 1);
    // Code span: held up to the context cap.
    let code = format!("`{}` $$tc(1)", filler(STREAM_CONTEXT_BYTES + 8));
    check_cap(&code, 0, STREAM_CONTEXT_BYTES + 1, "`\u{fffd}a");
    // Link label.
    let link = format!("[{}](u) $$tc(1)", filler(STREAM_CONTEXT_BYTES + 8));
    check_cap(&link, 0, STREAM_CONTEXT_BYTES + 1, "[\u{fffd}a");
    // A tag opening holds at most its own, smaller cap.
    let opening = format!("x <A t=\"{}\" />", filler(TAG_OPENING_BYTES + 8));
    check_cap(&opening, 2, 2 + TAG_OPENING_BYTES + 1, "<A t=\"\u{fffd}a");
    // A whole tag.
    let tag = format!("x <A>{}</A>", filler(STREAM_CONTEXT_BYTES + 8));
    check_cap(&tag, 2, 2 + STREAM_CONTEXT_BYTES + 1, "<A>\u{fffd}a");
    // A pending reference holds 24 characters; the final reference is longer.
    let reference = format!("$$tc({})", "1".repeat(30));
    let revision = oneshot(&reference.as_bytes()[..PENDING_REFERENCE_CHARS]);
    assert!(revision.nodes.is_empty());
    let revision = oneshot(&reference.as_bytes()[..PENDING_REFERENCE_CHARS + 1]);
    assert_eq!(
        literal_text(&revision),
        &reference[..PENDING_REFERENCE_CHARS]
    );
    check_equivalence(reference.as_bytes());
    let refs = scan_references(&reference, Mode::Reply).references;
    assert_eq!(refs[0].n.len(), 30);
    // A line start that may still be a thematic break.
    let rule = format!("{}\u{1}\n", "-".repeat(STREAM_CONTEXT_BYTES + 4));
    let revision = oneshot(&rule.as_bytes()[..STREAM_CONTEXT_BYTES + 1]);
    assert!(literal_text(&revision).starts_with("---"));
    check_sampled(rule.as_bytes(), 16, &[STREAM_CONTEXT_BYTES], 3, 4);
    // A table header line stops being held as a header at the cap.
    let table = format!("| {} |\n|---|\n", "b".repeat(STREAM_CONTEXT_BYTES + 4));
    assert!(!has(&oneshot(table.as_bytes()), |k| *k == Display::Table));
    assert!(parse(&table, Mode::Reply)
        .nodes
        .iter()
        .any(|n| n.kind == Kind::Markdown(Markdown::Table)));
    check_sampled(table.as_bytes(), 16, &[STREAM_CONTEXT_BYTES], 3, 4);
}

#[test]
fn reply_cap_is_equivalent_for_sampled_chunkings() {
    // A two-byte scalar straddles nothing; a reference straddles the cap.
    let source = "word ".repeat((REPLY_BYTES - 10) / 5) + "ééé $$tc(1) $$tc(2)";
    assert!(source.len() > REPLY_BYTES);
    let doc = parse(&source, Mode::Reply);
    assert!(doc.shortened && doc.references().is_empty());
    check_sampled(source.as_bytes(), 64, &[REPLY_BYTES], 12, 24);
    // Bytes past the cap are not scanned.
    let mut stream = StreamParser::new(Mode::Reply);
    stream.feed(source.as_bytes());
    assert!(stream.work().scanned <= REPLY_BYTES + 3);
}

#[test]
fn programs_are_not_drawn_while_streaming() {
    let source = b"export const a = 1\n{a} $$tc(1)";
    let mut stream = StreamParser::new(Mode::Program);
    for byte in source {
        assert!(stream.feed(&[*byte]).is_none());
    }
    assert_eq!(stream.work(), Work::default());
    assert_eq!(stream.finish(), parse_bytes(source, Mode::Program));
}

/// A long, varied reply, built deterministically.
fn long_reply(target: usize) -> String {
    let parts = [
        "# Heading $$tc(1)\n\n",
        "Plain words with *emphasis*, **strong**, `code`, a [link](https://x.test) and $$tc(2).\n",
        "- item one\n- item `two`\n  - nested $$tc(3)\n\n",
        "> quoted line\n> with <A x={$$tc(4)} /> inline\n\n",
        "| a | b |\n|---|---|\n| 1 | $$tc(5) |\n\n",
        "```rust\nfn main() {}\n```\n\n",
        "<Card title=\"t\">\nbody text $$tc(6)\n</Card>\n\n",
        "braces {literal $$tc(7)} and \\*escaped\\* text_with_underscores.\n\n",
    ];
    let mut out = String::new();
    let mut i = 0;
    while out.len() < target {
        out.push_str(parts[i % parts.len()]);
        i += 1;
    }
    out
}

#[test]
fn linear_work_feeding_one_byte_at_a_time() {
    let source = long_reply(48 * 1024);
    let mut stream = StreamParser::new(Mode::Reply);
    let (mut scanned, mut reexamined) = (0usize, 0usize);
    for (i, byte) in source.as_bytes().iter().enumerate() {
        stream.feed(&[*byte]);
        let work = stream.work();
        scanned += work.scanned;
        reexamined += work.reexamined;
        let fed = i + 1;
        // Each byte is scanned once; held text is examined again only a
        // bounded number of times, plus at most one held context's worth.
        assert!(
            scanned <= fed && scanned + reexamined <= 4 * fed + STREAM_CONTEXT_BYTES,
            "linear work: {scanned} scanned and {reexamined} re-examined after {fed} bytes"
        );
    }
    assert_eq!(scanned, source.len());
    assert!(
        reexamined <= 3 * source.len(),
        "re-examined {reexamined} of {}",
        source.len()
    );
    assert_eq!(stream.finish(), parse(&source, Mode::Reply));
}
