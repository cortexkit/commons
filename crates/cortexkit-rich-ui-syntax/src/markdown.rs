use crate::expression::Result;
use crate::parser::Engine;
use crate::*;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::collections::VecDeque;

/// Extension spans are opaque to Markdown. Keeping byte lengths unchanged lets
/// the Markdown engine and the language nodes share original source offsets.
pub(crate) fn assemble(
    engine: &mut Engine<'_>,
    start: usize,
    end: usize,
    specials: Vec<Node>,
    inline: bool,
) -> Result<Vec<Node>> {
    let original = &engine.source[start..end];
    let mut masked = context_source(original).into_bytes();
    for node in &specials {
        let a = node.span.start - start;
        let b = node.span.end - start;
        masked[a..b].fill(b'x');
        // A consumed fence includes its final newline, which is still a block
        // boundary for the Markdown that follows the opaque code block.
        if matches!(node.kind, Kind::Markdown(Markdown::CodeBlock { .. })) {
            for i in (a..b)
                .rev()
                .take_while(|i| matches!(original.as_bytes()[*i], b'\n' | b'\r'))
            {
                masked[i] = original.as_bytes()[i];
            }
        }
    }
    let masked = String::from_utf8(masked).map_err(|_| "E_SYNTAX")?;
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(&masked, options).into_offset_iter();
    let mut pending: VecDeque<Node> = specials.into();
    let mut stack: Vec<Node> = Vec::new();
    let mut roots = Vec::new();
    let mut suppressed = Vec::new();
    let mut block_depth = 0;
    let mut emphasis_depth = 0;
    for (event, range) in events {
        engine.work(1)?;
        let span = Span::new(start + range.start, start + range.end);
        match event {
            Event::Start(tag) => {
                let kind = convert_tag(&tag);
                let is_block = matches!(tag, Tag::BlockQuote(_) | Tag::List(_));
                let is_emphasis = matches!(tag, Tag::Emphasis | Tag::Strong | Tag::Strikethrough);
                if is_block {
                    block_depth += 1;
                }
                if is_emphasis {
                    emphasis_depth += 1;
                }
                let skip = ((is_block || matches!(tag, Tag::Item)) && block_depth > MARKDOWN_DEPTH)
                    || (is_emphasis && emphasis_depth > EMPHASIS_DEPTH);
                let delimiter = match tag {
                    Tag::Strong | Tag::Strikethrough => 2,
                    Tag::Emphasis => 1,
                    _ => 0,
                };
                suppressed.push((skip, is_block, is_emphasis, delimiter));
                if skip {
                    let len = if is_emphasis {
                        delimiter
                    } else if matches!(tag, Tag::BlockQuote(_)) {
                        1
                    } else if matches!(tag, Tag::List(_)) {
                        engine.source[span.start..span.end]
                            .find(char::is_whitespace)
                            .unwrap_or(1)
                    } else {
                        0
                    };
                    if len > 0 {
                        let marker = Span::new(span.start, (span.start + len).min(span.end));
                        append(
                            engine.node(
                                marker,
                                Kind::Literal(engine.source[marker.start..marker.end].to_owned()),
                                vec![],
                            )?,
                            &mut stack,
                            &mut roots,
                        );
                    }
                    continue;
                }
                if matches!(
                    kind,
                    Markdown::Paragraph
                        | Markdown::Heading(_)
                        | Markdown::Quote
                        | Markdown::List { .. }
                        | Markdown::Item
                        | Markdown::Table
                        | Markdown::CodeBlock { .. }
                ) {
                    engine.blocks += 1;
                    if engine.blocks > BLOCKS {
                        engine.fatal = true;
                        return Err("E_LIMIT_BLOCKS");
                    }
                }
                stack.push(engine.node(span, Kind::Markdown(kind), vec![])?);
            }
            Event::End(_) => {
                let (skip, is_block, is_emphasis, delimiter) =
                    suppressed.pop().ok_or("E_SYNTAX")?;
                if is_block {
                    block_depth -= 1;
                }
                if is_emphasis {
                    emphasis_depth -= 1;
                }
                if skip {
                    if is_emphasis {
                        let marker =
                            Span::new(span.end.saturating_sub(delimiter).max(span.start), span.end);
                        append(
                            engine.node(
                                marker,
                                Kind::Literal(engine.source[marker.start..marker.end].to_owned()),
                                vec![],
                            )?,
                            &mut stack,
                            &mut roots,
                        );
                    }
                    continue;
                }
                let mut node = stack.pop().ok_or("E_SYNTAX")?;
                node.span.end = span.end;
                if inline && matches!(node.kind, Kind::Markdown(Markdown::Paragraph)) {
                    for child in node.children {
                        append(child, &mut stack, &mut roots);
                    }
                } else {
                    append(node, &mut stack, &mut roots);
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                emit_text(engine, span, &text, &mut pending, &mut stack, &mut roots)?;
            }
            Event::Code(text) => {
                append(
                    engine.node(
                        span,
                        Kind::Markdown(Markdown::Code(text.into_string())),
                        vec![],
                    )?,
                    &mut stack,
                    &mut roots,
                );
            }
            Event::Rule => append(
                engine.node(span, Kind::Markdown(Markdown::ThematicBreak), vec![])?,
                &mut stack,
                &mut roots,
            ),
            Event::SoftBreak | Event::HardBreak => {
                if pending
                    .front()
                    .is_some_and(|node| node.span.start <= span.start && node.span.end > span.start)
                {
                    emit_text(engine, span, "\n", &mut pending, &mut stack, &mut roots)?;
                } else {
                    let kind = if matches!(event, Event::SoftBreak) {
                        Markdown::SoftBreak
                    } else {
                        Markdown::HardBreak
                    };
                    append(
                        engine.node(span, Kind::Markdown(kind), vec![])?,
                        &mut stack,
                        &mut roots,
                    );
                }
            }
            Event::TaskListMarker(checked) => append(
                engine.node(span, Kind::Markdown(Markdown::Task(checked)), vec![])?,
                &mut stack,
                &mut roots,
            ),
            _ => {}
        }
    }
    // A Markdown structural event may consume a masked span (for example a
    // table delimiter). Such a span must never silently lose a language node.
    if !pending.is_empty() {
        return Err("E_SYNTAX");
    }
    split_blocks(engine, roots)
}

fn append(node: Node, stack: &mut [Node], roots: &mut Vec<Node>) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        roots.push(node);
    }
}

fn emit_text(
    engine: &mut Engine<'_>,
    span: Span,
    decoded: &str,
    pending: &mut VecDeque<Node>,
    stack: &mut [Node],
    roots: &mut Vec<Node>,
) -> Result<()> {
    let mut pos = span.start;
    let intersects = pending.front().is_some_and(|n| n.span.start < span.end);
    if !intersects {
        let text = decoded
            .replace('\u{1}', "<")
            .replace('\u{2}', " ")
            .replace('\u{3}', "=")
            .replace('\u{4}', "[")
            .replace('\0', "\u{fffd}");
        if !text.is_empty() {
            append(engine.node(span, Kind::Text(text), vec![])?, stack, roots);
        }
        return Ok(());
    }
    while let Some(node) = pending.front() {
        if node.span.start >= span.end {
            break;
        }
        if pos < node.span.start {
            append(
                engine.node(
                    Span::new(pos, node.span.start),
                    Kind::Text(engine.source[pos..node.span.start].replace('\0', "\u{fffd}")),
                    vec![],
                )?,
                stack,
                roots,
            );
        }
        let node = pending.pop_front().ok_or("E_SYNTAX")?;
        pos = node.span.end;
        append(node, stack, roots);
    }
    if pos < span.end {
        append(
            engine.node(
                Span::new(pos, span.end),
                Kind::Text(engine.source[pos..span.end].replace('\0', "\u{fffd}")),
                vec![],
            )?,
            stack,
            roots,
        );
    }
    Ok(())
}

fn convert_tag(tag: &Tag<'_>) -> Markdown {
    match tag {
        Tag::Paragraph => Markdown::Paragraph,
        Tag::Heading { level, .. } => Markdown::Heading(*level as u8),
        Tag::BlockQuote(_) => Markdown::Quote,
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => Markdown::CodeBlock {
            language: info.split_whitespace().next().unwrap_or("").into(),
            text: String::new(),
        },
        Tag::CodeBlock(_) => Markdown::Paragraph,
        Tag::List(start) => Markdown::List { start: *start },
        Tag::Item => Markdown::Item,
        Tag::Table(_) => Markdown::Table,
        Tag::TableHead => Markdown::TableHead,
        Tag::TableRow => Markdown::TableRow,
        Tag::TableCell => Markdown::TableCell,
        Tag::Emphasis => Markdown::Emphasis,
        Tag::Strong => Markdown::Strong,
        Tag::Strikethrough => Markdown::Strikethrough,
        Tag::Link {
            dest_url, title, ..
        }
        | Tag::Image {
            dest_url, title, ..
        } => Markdown::Link {
            destination: dest_url.to_string(),
            title: title.to_string(),
            image: matches!(tag, Tag::Image { .. }),
        },
        _ => Markdown::Paragraph,
    }
}

fn split_blocks(engine: &mut Engine<'_>, nodes: Vec<Node>) -> Result<Vec<Node>> {
    let mut result = Vec::new();
    for mut node in nodes {
        if matches!(node.kind, Kind::Markdown(Markdown::Paragraph))
            && node.children.iter().any(is_block_extension)
        {
            let mut run = Vec::new();
            for child in node.children {
                if is_block_extension(&child) {
                    flush_paragraph(engine, &mut run, &mut result)?;
                    result.push(child);
                } else {
                    run.push(child);
                }
            }
            flush_paragraph(engine, &mut run, &mut result)?;
        } else {
            node.children = split_blocks(engine, node.children)?;
            result.push(node);
        }
    }
    Ok(result)
}
fn is_block_extension(node: &Node) -> bool {
    matches!(
        node.kind,
        Kind::Tag { block: true, .. }
            | Kind::Declaration { .. }
            | Kind::Interpolation { block: true }
            | Kind::Markdown(Markdown::CodeBlock { .. } | Markdown::ThematicBreak)
    )
}
fn flush_paragraph(
    engine: &mut Engine<'_>,
    run: &mut Vec<Node>,
    result: &mut Vec<Node>,
) -> Result<()> {
    if let (Some(first), Some(last)) = (run.first(), run.last()) {
        result.push(engine.node(
            Span::new(first.span.start, last.span.end),
            Kind::Markdown(Markdown::Paragraph),
            std::mem::take(run),
        )?);
    }
    Ok(())
}

/// Disable syntax outside the dialect before asking CommonMark for contexts.
/// Each replacement occupies one original byte, including lone-CR newlines.
pub(crate) fn context_source(original: &str) -> String {
    let mut masked = original.as_bytes().to_vec();
    for i in 0..masked.len() {
        if masked[i] == b'<' {
            masked[i] = 1;
        }
        if masked[i] == b'\r' && masked.get(i + 1) != Some(&b'\n') {
            masked[i] = b'\n';
        }
    }
    let mut line = 0;
    while line < masked.len() {
        let stop = masked[line..]
            .iter()
            .position(|b| matches!(b, b'\n' | b'\r'))
            .map_or(masked.len(), |i| line + i);
        let spaces = masked[line..stop]
            .iter()
            .take_while(|b| **b == b' ')
            .count();
        let content = &masked[line + spaces..stop];
        let setext = !content.is_empty() && content.iter().all(|b| *b == b'=');
        let definition = content.starts_with(b"[") && content.windows(2).any(|w| w == b"]:");
        if setext {
            masked[line + spaces] = 3;
        }
        if definition {
            masked[line + spaces] = 4;
        }
        line = stop + 1;
    }
    // Neutralize only actual indented-code regions, not the indentation that
    // CommonMark needs to recognize nested lists and their continuation lines.
    let temporary = String::from_utf8(masked).expect("ASCII substitutions preserve UTF-8");
    let indented: Vec<_> = Parser::new(&temporary)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::Start(Tag::CodeBlock(CodeBlockKind::Indented))).then_some(range)
        })
        .collect();
    let mut masked = temporary.into_bytes();
    for range in indented {
        let mut line = original[..range.start]
            .rfind(['\r', '\n'])
            .map_or(0, |i| i + 1);
        while line < range.end {
            let stop = masked[line..]
                .iter()
                .position(|b| matches!(b, b'\n' | b'\r'))
                .map_or(masked.len(), |i| line + i);
            let spaces = masked[line..stop]
                .iter()
                .take_while(|b| **b == b' ')
                .count();
            if spaces >= 4 {
                masked[line + spaces - 4] = 2;
            }
            line = stop + 1;
        }
    }
    String::from_utf8(masked).expect("ASCII substitutions preserve UTF-8")
}
