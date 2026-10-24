use cortexkit_rich_ui_syntax::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

fn samples(name: &str) -> Vec<Value> {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-vectors")
            .join("rich-ui-syntax-v1")
            .join(name),
    )
    .unwrap();
    if name == "refs-scan-samples-v1.json" {
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            "1b042b7bfc2ba1542032fb6f6f226cdcb8f78e4147085740e3a4cbcc97037aaf"
        );
    }
    serde_json::from_slice::<Value>(&bytes).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn run_cases(name: &str) {
    for case in samples(name) {
        let source = case["source"].as_str().unwrap();
        let refs = match case["mode"].as_str().unwrap() {
            "token_slot" => check_token_slot(source).into_iter().collect(),
            "reply" => scan_references(source, Mode::Reply).references,
            "program" => scan_references(source, Mode::Program).references,
            _ => panic!("unknown mode"),
        };
        let actual: Vec<_> = refs
            .iter()
            .map(|r| (r.n.clone(), r.span.start, r.span.end))
            .collect();
        let expected: Vec<_> = case["references"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["n"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| r["n"].to_string()),
                    r["start"].as_u64().unwrap() as usize,
                    r["end"].as_u64().unwrap() as usize,
                )
            })
            .collect();
        let mode = if case["mode"] == "program" {
            Mode::Program
        } else {
            Mode::Reply
        };
        assert_eq!(
            actual,
            expected,
            "{}: {:?}",
            case["name"],
            parse(source, mode)
        );
        if let Some(diagnostics) = case.get("diagnostics") {
            let expected: Vec<_> = diagnostics
                .as_array()
                .unwrap()
                .iter()
                .map(|d| {
                    (
                        d["code"].as_str().unwrap().to_owned(),
                        d["start"].as_u64().unwrap() as usize,
                        d["end"].as_u64().unwrap() as usize,
                        d.get("subject").map(|s| s.as_str().unwrap().to_owned()),
                    )
                })
                .collect();
            let actual: Vec<_> = parse(source, mode)
                .diagnostics
                .into_iter()
                .map(|d| {
                    (
                        d.code.as_str().to_owned(),
                        d.span.start,
                        d.span.end,
                        d.subject,
                    )
                })
                .collect();
            assert_eq!(actual, expected, "{}: diagnostics", case["name"]);
        }
    }
}
#[test]
fn original_samples_and_digest() {
    assert_eq!(samples("refs-scan-samples-v1.json").len(), 27);
    run_cases("refs-scan-samples-v1.json");
}
#[test]
fn extended_samples() {
    run_cases("refs-scan-samples-v1.1.json");
}
#[test]
fn stream_and_diagnostic_samples() {
    run_cases("refs-scan-samples-v1.2.json");
}

/// The only diagnostic `parse` reports for `source`, as (code, span, subject).
fn only(source: &str, mode: Mode) -> (DiagnosticCode, usize, usize, Option<String>) {
    let doc = parse(source, mode);
    assert_eq!(doc.diagnostics.len(), 1, "{source:?}: {doc:?}");
    let d = doc.diagnostics.into_iter().next().unwrap();
    (d.code, d.span.start, d.span.end, d.subject)
}

#[test]
fn unterminated_string_names_the_string() {
    use DiagnosticCode::UnterminatedString as U;
    // A raw line break inside a declaration's string.
    let source = "export const a = 'abc\nexport const b = 1";
    assert_eq!(only(source, Mode::Program), (U, 17, 21, None));
    assert_eq!(&source[17..21], "'abc");
    // The construct ends before the closing quote.
    assert_eq!(only("{'abc}", Mode::Program), (U, 1, 6, None));
    // A line break after a backslash is still a raw line break.
    assert_eq!(only("{'a\\\nb'}", Mode::Program), (U, 1, 4, None));
    // A reply prop string.
    assert_eq!(only("<A x=\"abc\ndef\" />", Mode::Reply), (U, 5, 9, None));
    assert_eq!(U.as_str(), "E_UNTERMINATED_STRING");
}

#[test]
fn bad_escape_names_the_escape() {
    use DiagnosticCode::BadEscape as B;
    for (source, start, end) in [
        ("{'a\\q'}", 3, 5),
        ("{'\\u{110000}'}", 2, 12),
        ("{'\\u{d800}'}", 2, 10),
        ("{'\\u{}'}", 2, 6),
        ("{'\\u{12'}", 2, 7),
        ("{'\\u{0000041}'}", 2, 13),
        ("{'\\u12'}", 2, 4),
    ] {
        assert_eq!(
            only(source, Mode::Program),
            (B, start, end, None),
            "{source}"
        );
        assert!(source[start..end].starts_with("\\"));
    }
    // Inside a declaration it is not reported as an incomplete declaration.
    assert_eq!(
        only("export const s = 'a\\q'", Mode::Program),
        (B, 19, 21, None)
    );
    assert_eq!(B.as_str(), "E_BAD_ESCAPE");
}

#[test]
fn keyword_declarations_are_reserved_names() {
    for name in ["true", "false", "null", "export", "let", "const"] {
        let source = format!("export const {name} = 1");
        assert_eq!(
            only(&source, Mode::Program),
            (
                DiagnosticCode::ReservedName,
                0,
                source.len(),
                Some(name.to_owned())
            ),
        );
        let source = format!("export let ${name} = 1");
        assert_eq!(
            only(&source, Mode::Program),
            (
                DiagnosticCode::ReservedName,
                0,
                source.len(),
                Some(name.to_owned())
            ),
        );
    }
}

#[test]
fn diagnostics_name_their_subject() {
    use DiagnosticCode as C;
    let cases: &[(&str, Mode, C, &str)] = &[
        (
            "export const data = 1",
            Mode::Program,
            C::ReservedName,
            "data",
        ),
        (
            "export const x = 1\nexport let $x = 2",
            Mode::Program,
            C::DuplicateName,
            "x",
        ),
        (
            "export let $s = $$tc(1)",
            Mode::Program,
            C::LiteralRequired,
            "s",
        ),
        ("{File(name)}", Mode::Program, C::LiteralRequired, "File"),
        ("{{a: 1, 'a': 2}}", Mode::Program, C::DuplicateKey, "a"),
        ("<A x={1} x={2} />", Mode::Reply, C::DuplicateProp, "x"),
        ("<A x={a}>b</A>", Mode::Reply, C::ReplyForbidden, "x"),
        ("<A>b", Mode::Reply, C::UnclosedTag, "A"),
        ("<A>b</B>", Mode::Reply, C::MismatchedClose, "A"),
    ];
    for (source, mode, code, subject) in cases {
        let (got, _, _, named) = only(source, *mode);
        assert_eq!((got, named.as_deref()), (*code, Some(*subject)), "{source}");
    }
    // Limit and nesting diagnostics about a tag name it.
    let tags = (0..65).map(|_| "<A x={1} />\n").collect::<String>();
    let doc = parse(&tags, Mode::Reply);
    let limit = doc
        .diagnostics
        .iter()
        .find(|d| d.code == C::LimitTags)
        .unwrap();
    assert_eq!(limit.subject.as_deref(), Some("A"));
    let deep = "<B>".repeat(33) + "x" + &"</B>".repeat(33);
    let doc = parse(&deep, Mode::Reply);
    let nesting = doc
        .diagnostics
        .iter()
        .find(|d| d.code == C::Nesting)
        .unwrap();
    assert_eq!(nesting.subject.as_deref(), Some("B"), "{doc:?}");
    // A refused duplicate still names what it refused, and the first stands.
    let doc = parse("export const x = 1\nexport const x = 2", Mode::Program);
    assert!(matches!(doc.nodes[0].kind, Kind::Declaration { ref name, .. } if name == "x"));
    assert_eq!(doc.diagnostics[0].subject.as_deref(), Some("x"));
}

#[test]
fn program_limit_measures_raw_bytes() {
    // Every invalid byte repairs to three bytes; the limit applies before that.
    let doc = parse_bytes(&vec![0xff; PROGRAM_BYTES], Mode::Program);
    assert!(!doc.fallback && !doc.shortened, "{:?}", doc.diagnostics);
    assert_eq!(
        doc.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
        [DiagnosticCode::Encoding]
    );
    assert_eq!(doc.parsed_bytes, 3 * PROGRAM_BYTES);
    let doc = parse_bytes(&vec![0xff; PROGRAM_BYTES + 1], Mode::Program);
    assert!(doc.fallback && doc.shortened);
    assert_eq!(
        doc.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
        [DiagnosticCode::SourceTooLarge, DiagnosticCode::Encoding]
    );
    // A valid scalar straddling the limit is left out whole, as by `parse`.
    let mut source = vec![b'x'; PROGRAM_BYTES - 1];
    source.extend_from_slice("é".as_bytes());
    let doc = parse_bytes(&source, Mode::Program);
    assert!(
        doc.fallback
            && doc
                .diagnostics
                .iter()
                .all(|d| d.code != DiagnosticCode::Encoding)
    );
    assert_eq!(
        doc,
        parse(std::str::from_utf8(&source).unwrap(), Mode::Program)
    );
}

fn refs(source: &str, mode: Mode) -> Vec<String> {
    scan_references(source, mode)
        .references
        .into_iter()
        .map(|r| r.n)
        .collect()
}
fn diagnostic(source: &str, mode: Mode, code: &str) {
    let doc = parse(source, mode);
    assert!(
        doc.diagnostics.iter().any(|d| d.code.as_str() == code),
        "missing {code}: {doc:?}"
    );
}
fn visit<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
    for node in nodes {
        out.push(node);
        visit(&node.children, out);
        if let Kind::Tag { props, .. } = &node.kind {
            for prop in props {
                visit(std::slice::from_ref(&prop.value), out);
            }
        }
    }
}

#[test]
fn expression_grammar_and_precedence() {
    let doc = parse(
        "{[-2 ** 3 ** 2, a.b[1], x ? y : z, (a || b) ?? c, {a: true, b: null}, File('p')]}",
        Mode::Program,
    );
    assert!(doc.diagnostics.is_empty(), "{doc:?}");
    let mut all = Vec::new();
    visit(&doc.nodes, &mut all);
    let unary = all
        .iter()
        .find(|n| matches!(&n.kind, Kind::Expression(Expression::Unary(op)) if op == "-"))
        .unwrap();
    assert!(
        matches!(&unary.children[0].kind, Kind::Expression(Expression::Binary(op)) if op == "**")
    );
    assert!(
        matches!(&unary.children[0].children[1].kind, Kind::Expression(Expression::Binary(op)) if op == "**")
    );
    for (source, code) in [
        ("{a < b < c}", "E_NONASSOC"),
        ("{a == b == c}", "E_NONASSOC"),
        ("{a ?? b || c}", "E_MIXED_COALESCE"),
        ("{a && b ?? c}", "E_MIXED_COALESCE"),
        ("{{a: 1, 'a': 2}}", "E_DUPLICATE_KEY"),
        ("{File(name)}", "E_LITERAL_REQUIRED"),
    ] {
        diagnostic(source, Mode::Program, code);
    }
}

#[test]
fn exact_decimal_literals() {
    for token in [
        "0",
        "18446744073709551615",
        "99999999999999999999.999999999999999999",
        "1e-18",
        "100e-20",
        "0e999",
    ] {
        let doc = parse(&format!("{{{token}}}"), Mode::Program);
        assert!(doc.diagnostics.is_empty(), "{token}: {doc:?}");
    }
    for token in ["01", "1e20", "1e-19", "1.2e-18", "1e99999999", "1."] {
        diagnostic(&format!("{{{token}}}"), Mode::Program, "E_NUMBER_LITERAL");
    }
}

#[test]
fn declarations_and_handles() {
    let source = "export const rows = [\n $$tc(1),\n]\nexport let $state = {x: -1}\nexport const value = rows\n .x\n\n<A>\nexport const b = $$tc(2)\n</A>";
    let doc = parse(source, Mode::Program);
    assert!(doc.diagnostics.is_empty(), "{doc:?}");
    assert_eq!(refs(source, Mode::Program), ["1", "2"]);
    diagnostic(
        "export const x = 1\nexport let $x = 2",
        Mode::Program,
        "E_DUPLICATE_NAME",
    );
    diagnostic("export const data = 1", Mode::Program, "E_RESERVED_NAME");
    diagnostic(
        "export let $s = $$tc(1)",
        Mode::Program,
        "E_LITERAL_REQUIRED",
    );
    assert!(refs("<A file={File('$$tc(4)')} />", Mode::Reply).is_empty());
}

#[test]
fn markdown_subset_tree() {
    let source = "# title &amp; text\n\n> quote\n\n- [x] **strong** *em* ~~gone~~\n\n| a | b |\n|---|---|\n| x | y |\n\n[link](https://example.test)\n\n---\n\n    not code\n\nnot setext\n===\n\n<div>ordinary</div>";
    let doc = parse(source, Mode::Reply);
    assert!(doc.diagnostics.is_empty(), "{doc:?}");
    let mut all = Vec::new();
    visit(&doc.nodes, &mut all);
    for kind in [
        Markdown::Heading(1),
        Markdown::Quote,
        Markdown::List { start: None },
        Markdown::Task(true),
        Markdown::Strong,
        Markdown::Emphasis,
        Markdown::Strikethrough,
        Markdown::Table,
        Markdown::ThematicBreak,
    ] {
        assert!(
            all.iter().any(|n| n.kind == Kind::Markdown(kind.clone())),
            "{kind:?}: {doc:?}"
        );
    }
    assert!(!all
        .iter()
        .any(|n| matches!(n.kind, Kind::Markdown(Markdown::CodeBlock { .. }))));
}

#[test]
fn source_limits_and_unicode_boundaries() {
    let program = "x".repeat(PROGRAM_BYTES + 1) + "$$tc(1)";
    let doc = parse(&program, Mode::Program);
    assert!(doc.fallback);
    assert!(doc.references().is_empty());
    diagnostic(&program, Mode::Program, "E_SOURCE_TOO_LARGE");
    let source = "\n".repeat(REPLY_BYTES - 1) + "é$$tc(2)";
    let doc = parse(&source, Mode::Reply);
    assert_eq!(doc.parsed_bytes, REPLY_BYTES - 1);
    assert!(doc.shortened);
    assert!(doc.references().is_empty());
    assert_eq!(
        refs("\u{feff}é $$tc(3)\r\n$$tc(4)\r$$tc(5)", Mode::Reply),
        ["3", "4", "5"]
    );
    assert_eq!(
        scan_references("\u{feff}$$tc(1)", Mode::Reply).references[0].span,
        Span::new(3, 10)
    );
    assert!(parse_bytes(b"\xff$$tc(1)", Mode::Program)
        .diagnostics
        .iter()
        .any(|d| d.code == DiagnosticCode::Encoding));
}

#[test]
fn tag_and_record_budgets() {
    let source = (0..65)
        .map(|i| format!("<A value={{$$tc({i})}} />\n"))
        .collect::<String>();
    assert_eq!(refs(&source, Mode::Reply).len(), 64);
    diagnostic(&source, Mode::Reply, "E_LIMIT_TAGS");
    let source = "$$tc(1) ".repeat(65);
    let scan = scan_references(&source, Mode::Reply);
    assert_eq!(scan.references.len(), 65);
    assert!(!scan.references[63].excess);
    assert!(scan.references[64].excess);
    diagnostic(&source, Mode::Program, "E_REFERENCE_LIMIT");
    let source = "{[".to_owned() + &"File('p'),".repeat(65) + "]}";
    diagnostic(&source, Mode::Program, "E_REFERENCE_LIMIT");
    let source = (0..257)
        .map(|i| format!("export const v{i} = 1\n"))
        .collect::<String>();
    diagnostic(&source, Mode::Program, "E_LIMIT_DECLARATIONS");
}

#[test]
fn nesting_and_whole_tree_limits() {
    diagnostic(
        &("{".to_owned() + &"(".repeat(65) + "1" + &")".repeat(65) + "}"),
        Mode::Program,
        "E_NESTING",
    );
    let tags = "<A>".repeat(33) + "$$tc(1)" + &"</A>".repeat(33);
    diagnostic(&tags, Mode::Reply, "E_NESTING");
    assert!(refs(&tags, Mode::Reply).is_empty());
    diagnostic(&"x\n\n".repeat(2049), Mode::Reply, "E_LIMIT_BLOCKS");
    diagnostic(&"a".repeat(65_537), Mode::Reply, "E_LIMIT_TEXT");
    diagnostic(&"$$tc(1) ".repeat(5000), Mode::Reply, "E_LIMIT_AST");
    let quote = "> ".repeat(40) + "$$tc(1)";
    let doc = parse(&quote, Mode::Reply);
    let mut all = Vec::new();
    visit(&doc.nodes, &mut all);
    assert!(
        all.iter()
            .filter(|n| n.kind == Kind::Markdown(Markdown::Quote))
            .count()
            <= MARKDOWN_DEPTH
    );
    let source = "*a **b ".repeat(90) + &" ** *".repeat(90);
    let doc = parse(&source, Mode::Reply);
    fn emphasis_depth(nodes: &[Node], depth: usize) -> usize {
        nodes
            .iter()
            .map(|n| {
                emphasis_depth(
                    &n.children,
                    depth
                        + usize::from(matches!(
                            n.kind,
                            Kind::Markdown(
                                Markdown::Emphasis | Markdown::Strong | Markdown::Strikethrough
                            )
                        )),
                )
            })
            .max()
            .unwrap_or(depth)
    }
    assert!(emphasis_depth(&doc.nodes, 0) <= EMPHASIS_DEPTH);
    let doc = parse(&"<A x={a} /> ".repeat(100), Mode::Reply);
    assert_eq!(doc.diagnostics.len(), DIAGNOSTICS);
    assert!(doc.suppressed_diagnostics > 0);
}

#[test]
fn scanner_is_tree_walk() {
    for source in [
        "**$$tc(1)** <Unknown x={Parse($$tc(2),'csv')} />",
        "<A x={$$tc(1)+1}>$$tc(2)</A>",
        "export const a = $$tc(1)\n{a}",
    ] {
        for mode in [Mode::Reply, Mode::Program] {
            assert_eq!(
                scan_references(source, mode).references,
                parse(source, mode).references()
            );
        }
    }
}

#[test]
fn chunking_does_not_change_final_parse() {
    for source in [
        "é $$tc(1)",
        "`<A x={$$tc(2)} />` $$tc(3)",
        "[<A x={$$tc(4)} />](https://example.test)",
        "| a |\n|---|\n| $$tc(1) |",
        "<A\n\nx={$$tc(1)} />",
        "<A>\n# title\n</A>",
    ] {
        for mode in [Mode::Reply, Mode::Program] {
            let expected = parse(source, mode);
            for split in 0..=source.len() {
                let mut stream = StreamParser::new(mode);
                stream.feed(&source.as_bytes()[..split]);
                stream.feed(&source.as_bytes()[split..]);
                assert_eq!(stream.finish(), expected, "{source} at {split}");
            }
            let mut stream = StreamParser::new(mode);
            for byte in source.as_bytes() {
                stream.feed(&[*byte]);
            }
            assert_eq!(stream.finish(), expected);
        }
    }
}

#[test]
fn random_and_truncated_inputs_are_bounded_and_boundary_safe() {
    let alphabet = [
        "é",
        "🦀",
        "\r",
        "\n",
        "\\",
        "'",
        "\"",
        "`",
        "<A>",
        "</B>",
        "$$tc(1)",
        "{",
        "}",
        "[",
        "]",
        "*",
        "_",
        " ",
        "export const a = ",
        "\0",
    ];
    let mut state = 0x5647_389a_0102_0203_u64;
    for _ in 0..1200 {
        let mut source = String::new();
        for _ in 0..80 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            source.push_str(alphabet[state as usize % alphabet.len()]);
        }
        for mode in [Mode::Reply, Mode::Program] {
            for end in source
                .char_indices()
                .map(|(i, _)| i)
                .step_by(11)
                .chain(std::iter::once(source.len()))
            {
                let source = &source[..end];
                let doc = parse(source, mode);
                let mut nodes = Vec::new();
                visit(&doc.nodes, &mut nodes);
                assert!(nodes.len() <= AST_NODES);
                for span in nodes
                    .iter()
                    .map(|n| n.span)
                    .chain(doc.diagnostics.iter().map(|d| d.span))
                {
                    assert!(
                        span.start <= span.end
                            && span.end <= source.len()
                            && source.is_char_boundary(span.start)
                            && source.is_char_boundary(span.end),
                        "{span:?} {source:?}"
                    );
                }
                let references = doc.references();
                for pair in references.windows(2) {
                    assert!(pair[0].span.end <= pair[1].span.start);
                }
                for reference in references {
                    assert_eq!(
                        &source[reference.span.start..reference.span.end],
                        format!("$$tc({})", reference.n)
                    );
                }
            }
        }
    }
}

#[test]
fn exact_parse_limit_boundaries() {
    let expression = "{".to_owned() + &"(".repeat(64) + "1" + &")".repeat(64) + "}";
    assert!(parse(&expression, Mode::Program).diagnostics.is_empty());
    let tags = "<A>".repeat(32) + "$$tc(1)" + &"</A>".repeat(32);
    assert_eq!(refs(&tags, Mode::Reply), ["1"]);
    let declarations = (0..256)
        .map(|i| format!("export const v{i} = 1\n"))
        .collect::<String>();
    assert!(parse(&declarations, Mode::Program).diagnostics.is_empty());
    let references = "$$tc(1) ".repeat(64);
    assert!(scan_references(&references, Mode::Reply)
        .references
        .iter()
        .all(|r| !r.excess));
    assert!(!parse(&"x".repeat(65_536), Mode::Reply).fallback);
    assert!(!parse(&"\n".repeat(PROGRAM_BYTES), Mode::Program).shortened);
    assert!(!parse(&"\n".repeat(REPLY_BYTES), Mode::Reply).shortened);
}

#[test]
fn streaming_cap_preserves_valid_utf8() {
    for mode in [Mode::Reply, Mode::Program] {
        let cap = if mode == Mode::Reply {
            REPLY_BYTES
        } else {
            PROGRAM_BYTES
        };
        for offset in 1..4 {
            let source = "\n".repeat(cap - offset) + "🦀$$tc(1)";
            let expected = parse(&source, mode);
            assert_eq!(parse_bytes(source.as_bytes(), mode), expected);
            let mut stream = StreamParser::new(mode);
            for chunk in source.as_bytes().chunks(37) {
                stream.feed(chunk);
            }
            assert_eq!(stream.finish(), expected);
        }
    }
}

#[test]
fn large_adversarial_inputs_terminate() {
    for source in [
        "[".repeat(REPLY_BYTES),
        "<http://".repeat(20_000),
        "> ".repeat(50_000) + "$$tc(1)",
        "<A x={".repeat(1000),
        "**_a ".repeat(10_000),
    ] {
        for mode in [Mode::Reply, Mode::Program] {
            let doc = parse(&source, mode);
            assert!(doc.diagnostics.len() <= DIAGNOSTICS);
            let mut nodes = Vec::new();
            visit(&doc.nodes, &mut nodes);
            assert!(nodes.len() <= AST_NODES);
        }
    }
    let mixed = ("<A x={".to_owned() + &"(".repeat(50)).repeat(32) + "$$tc(1)";
    assert!(parse(&mixed, Mode::Program).references().is_empty());
}

#[test]
fn markdown_payloads_and_nesting() {
    let source = "&amp; \\* $$tc(1)\n\n- outer\n    - inner $$tc(2)\n\nplain\n---\n";
    let doc = parse(source, Mode::Reply);
    assert!(doc.diagnostics.is_empty(), "{doc:?}");
    let mut all = Vec::new();
    visit(&doc.nodes, &mut all);
    assert!(
        all.iter()
            .filter(|n| matches!(n.kind, Kind::Markdown(Markdown::List { .. })))
            .count()
            == 2,
        "{doc:?}"
    );
    assert!(
        !all.iter()
            .any(|n| matches!(n.kind, Kind::Markdown(Markdown::Heading(_)))),
        "{doc:?}"
    );
    let text: String = all
        .iter()
        .filter_map(|n| {
            if let Kind::Text(text) = &n.kind {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(text.starts_with("& * "), "{text:?}");
    let doc = parse("```rs\nlet x = 1;\n```", Mode::Reply);
    assert!(
        matches!(&doc.nodes[0].kind, Kind::Markdown(Markdown::CodeBlock { text, language }) if text == "let x = 1;\n" && language == "rs"),
        "{doc:?}"
    );
    assert!(
        matches!(&parse("` x `", Mode::Reply).nodes[0].children[0].kind, Kind::Markdown(Markdown::Code(text)) if text == "x")
    );
}
