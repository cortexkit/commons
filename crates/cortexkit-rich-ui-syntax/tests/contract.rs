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
        assert_eq!(
            actual,
            expected,
            "{}: {:?}",
            case["name"],
            parse(
                source,
                if case["mode"] == "program" {
                    Mode::Program
                } else {
                    Mode::Reply
                }
            )
        );
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
        doc.diagnostics.iter().any(|d| d.code == code),
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
        .any(|d| d.code == "W_ENCODING"));
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
                stream.push(&source.as_bytes()[..split]);
                stream.push(&source.as_bytes()[split..]);
                assert_eq!(stream.finish(), expected, "{source} at {split}");
            }
            let mut stream = StreamParser::new(mode);
            for byte in source.as_bytes() {
                stream.push(&[*byte]);
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
                stream.push(chunk);
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
