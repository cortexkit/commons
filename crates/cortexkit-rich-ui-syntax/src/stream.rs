//! Incremental reply parsing.
//!
//! [`StreamParser`] keeps every received byte (up to the reply limit) for one
//! whole-source parse at [`StreamParser::finish`], so the final result never
//! depends on how the source was chunked. While a reply streams in, it also
//! runs a separate *provisional* machine that turns each byte, once, into
//! bounded block and inline state, and reports a display-only [`Revision`]
//! whenever the drawable picture of the received prefix changes.
//!
//! The provisional machine never reparses the received prefix. Text whose
//! classification is still open (a code span waiting for its closing
//! backticks, a link waiting for its destination, a tag waiting for its
//! closing tag, a possible reference, the start of a line that may still
//! become a block marker) is held in a bounded buffer and is re-examined at
//! most a small constant number of times, when it resolves. Completed tags
//! and links are classified by the same parser [`crate::parse`] uses, run once
//! over just their buffered text.

use crate::parser::{parse_bytes, source_cap, Engine};
use crate::{parse, Document, Kind, Markdown, Mode, Node, Span, EMPHASIS_DEPTH, MARKDOWN_DEPTH};
use std::collections::VecDeque;
use std::sync::Arc;

/// The most source bytes one unresolved provisional context may hold: a code
/// span, a link, a whole tag, a table header line, or a line start that may
/// still become a block marker. At this size the held text is drawn as a
/// provisional literal and dropped. Final parsing has no such threshold.
pub const STREAM_CONTEXT_BYTES: usize = 32 * 1024;
/// The most source bytes a tag's opening (`<Name …>`) may hold before it is
/// drawn as a provisional literal.
pub const TAG_OPENING_BYTES: usize = 4 * 1024;
/// The most characters a possible `$$tc(N)` reference may hold before it is
/// drawn as a provisional literal. This is a streaming threshold, not a limit
/// on reference numbers: the final parse accepts any length.
pub const PENDING_REFERENCE_CHARS: usize = 24;

/// Text nodes are split at this many bytes so that drawing a revision copies
/// a bounded amount of text from the run still being written.
const RUN_BYTES: usize = 1024;
/// Link labels nest brackets at most this deep before the held link is drawn
/// as a provisional literal. This bounds how often a failed link's text is
/// re-examined.
const LABEL_DEPTH: usize = 4;
/// Quote and list containers open at once, counting a list and its item as
/// two.
const CONTAINERS: usize = 2 * MARKDOWN_DEPTH;

/// The work one [`StreamParser::feed`] call did, as plain counts. The caller
/// does its own fuel accounting: it reserves part of a reply's budget for the
/// final parse and charges provisional work separately.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Work {
    /// Received bytes consumed by the provisional machine. Each byte is
    /// scanned once; bytes past the reply limit are not scanned.
    pub scanned: usize,
    /// Bytes of held, unresolved text examined again: when a held context
    /// resolves, fails, or reaches its cap, or when a completed tag or link
    /// is classified.
    pub reexamined: usize,
}

/// A provisional picture of a streaming reply.
///
/// A revision is a pure function of the received prefix: the same bytes give
/// an equal revision however they were chunked. Revisions are **not**
/// monotone; a later byte may reclassify text drawn by an earlier revision
/// (a closing backtick, a link destination, a table delimiter row, the rest
/// of a line), so a caller drawing revisions replaces the whole drawn
/// revision with the next one. Text still held unresolved is not drawn at all.
///
/// A revision is inert. It carries display structure only: no reference
/// numbers (a reference is a [`Display::PendingReference`] placeholder), no
/// link destinations, no tag props, and nothing that names data to fetch or
/// an action to take. It is never a storage-time reference decision; use
/// [`StreamParser::finish`] or [`crate::scan_references`] for that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Revision {
    pub nodes: Vec<Arc<DisplayNode>>,
}

/// One node of a provisional display tree. Spans are byte offsets into the
/// received text after the same UTF-8 repair [`crate::parse_bytes`] applies:
/// each maximal invalid byte subsequence becomes one U+FFFD.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DisplayNode {
    pub span: Span,
    pub kind: Display,
    pub children: Vec<Arc<DisplayNode>>,
}

/// What a provisional display node draws.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Display {
    Paragraph,
    Heading(u8),
    ThematicBreak,
    /// Fenced code; its text is in `Text` children.
    CodeBlock {
        language: String,
    },
    Quote,
    List {
        /// The number of an ordered list's first item; `None` for a bullet
        /// list.
        start: Option<u64>,
    },
    Item,
    /// A task list marker. Only a completed tag's body, which the final
    /// parser classifies, produces one; while streaming, a list item's
    /// `[ ]` or `[x]` is drawn as text.
    Task(bool),
    Table,
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    Code(String),
    /// A link's or image's label, drawn as text. The destination is withheld.
    Link {
        image: bool,
    },
    Text(String),
    /// Opaque or provisional literal source text, control characters
    /// replaced.
    Literal(String),
    SoftBreak,
    HardBreak,
    /// A complete tag, by name only: props are withheld.
    Tag {
        name: String,
        block: bool,
    },
    /// Where a tool-output reference stands. Its number is withheld.
    PendingReference,
}

/// Parses a source that arrives in chunks.
///
/// [`finish`](Self::finish) returns exactly what [`parse_bytes`] returns for
/// the whole received input, for every chunking: tree, references,
/// diagnostics and spans. Bytes beyond the mode's size limit, plus a
/// three-byte margin for a split UTF-8 character, are counted but not kept;
/// `finish` then marks the document shortened, and in program mode refuses
/// it (`E_SOURCE_TOO_LARGE`, no nodes), so a program never runs from a
/// cut-off prefix.
///
/// In reply mode, [`feed`](Self::feed) also returns a provisional
/// [`Revision`] whenever the drawable picture changes. Programs are not drawn
/// while streaming: in program mode `feed` always returns `None`.
pub struct StreamParser {
    mode: Mode,
    bytes: Vec<u8>,
    machine: Option<Box<Machine>>,
    last: Revision,
    work: Work,
}

impl StreamParser {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            bytes: vec![],
            machine: (mode == Mode::Reply).then(|| Box::new(Machine::new())),
            last: Revision::default(),
            work: Work::default(),
        }
    }

    /// Accepts the next chunk. Returns the new revision when the provisional
    /// picture differs from the last one returned, and `None` otherwise.
    pub fn feed(&mut self, chunk: &[u8]) -> Option<Revision> {
        let keep = chunk
            .len()
            .min((source_cap(self.mode) + 3).saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&chunk[..keep]);
        self.work = Work::default();
        let machine = self.machine.as_mut()?;
        for &byte in chunk {
            if machine.stopped {
                break;
            }
            machine.byte(byte);
        }
        self.work = std::mem::take(&mut machine.work);
        if !std::mem::take(&mut machine.out.dirty) {
            return None;
        }
        let revision = machine.out.snapshot();
        if revision == self.last {
            return None;
        }
        self.last = revision.clone();
        Some(revision)
    }

    /// The current provisional picture: the last revision `feed` returned, or
    /// an empty one before any.
    pub fn revision(&self) -> &Revision {
        &self.last
    }

    /// The work the last `feed` call did.
    pub fn work(&self) -> Work {
        self.work
    }

    /// Parses everything received, exactly as [`parse_bytes`] parses it.
    pub fn finish(self) -> Document {
        parse_bytes(&self.bytes, self.mode)
    }
}

/// Replaces control characters other than line feed and tab, so provisional
/// text never carries terminal or layout controls. A carriage return becomes
/// a line feed; the builder drops the line feed of a CRLF pair.
fn sanitize(c: char) -> char {
    match c {
        '\r' => '\n',
        '\n' | '\t' => c,
        c if c.is_control() => '\u{fffd}',
        c => c,
    }
}

fn sanitize_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cr = false;
    for c in text.chars() {
        if !(cr && c == '\n') {
            out.push(sanitize(c));
        }
        cr = c == '\r';
    }
    out
}

/// A text run still being written into a frame.
struct Run {
    literal: bool,
    span: Span,
    text: String,
    cr: bool,
}

impl Run {
    fn node(&self) -> DisplayNode {
        DisplayNode {
            span: self.span,
            kind: if self.literal {
                Display::Literal(self.text.clone())
            } else {
                Display::Text(self.text.clone())
            },
            children: vec![],
        }
    }
}

/// An open display container.
struct Frame {
    kind: Display,
    span: Span,
    children: Vec<Arc<DisplayNode>>,
    run: Option<Run>,
}

/// The provisional display tree, built append-only except where a later
/// byte reclassifies the content of the innermost open frame. Finished nodes
/// are shared, so a snapshot copies only the open frames' child lists and
/// the current text run.
struct Builder {
    frames: Vec<Frame>,
    dirty: bool,
}

impl Builder {
    fn new() -> Self {
        Self {
            frames: vec![Frame {
                kind: Display::Paragraph,
                span: Span::default(),
                children: vec![],
                run: None,
            }],
            dirty: false,
        }
    }
    fn top(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("the root frame stays open")
    }
    fn top_kind(&self) -> &Display {
        &self.frames.last().expect("the root frame stays open").kind
    }
    fn flush_run(&mut self) {
        let top = self.top();
        if let Some(run) = top.run.take() {
            top.children.push(Arc::new(run.node()));
        }
    }
    fn extend(&mut self, end: usize) {
        let top = self.top();
        top.span.end = top.span.end.max(end);
    }
    fn open(&mut self, kind: Display, at: usize) {
        self.flush_run();
        self.frames.push(Frame {
            kind,
            span: Span::new(at, at),
            children: vec![],
            run: None,
        });
        self.dirty = true;
    }
    fn close(&mut self) {
        if self.frames.len() < 2 {
            return;
        }
        self.flush_run();
        let frame = self.frames.pop().expect("checked above");
        let node = DisplayNode {
            span: frame.span,
            kind: frame.kind,
            children: frame.children,
        };
        self.extend(node.span.end);
        self.top().children.push(Arc::new(node));
        self.dirty = true;
    }
    /// Drops the innermost frame and everything drawn in it.
    fn discard(&mut self) {
        if self.frames.len() > 1 {
            self.frames.pop();
            self.dirty = true;
        }
    }
    fn push(&mut self, node: DisplayNode) {
        self.flush_run();
        self.extend(node.span.end);
        self.top().children.push(Arc::new(node));
        self.dirty = true;
    }
    fn char(&mut self, c: char, start: usize, end: usize, literal: bool) {
        let top = self.top();
        let same = top
            .run
            .as_ref()
            .is_some_and(|run| run.literal == literal && run.text.len() < RUN_BYTES);
        if !same {
            if let Some(run) = top.run.take() {
                top.children.push(Arc::new(run.node()));
            }
            top.run = Some(Run {
                literal,
                span: Span::new(start, start),
                text: String::new(),
                cr: false,
            });
        }
        let run = top.run.as_mut().expect("opened above");
        if !(run.cr && c == '\n') {
            run.text.push(sanitize(c));
        }
        run.cr = c == '\r';
        run.span.end = end;
        top.span.end = top.span.end.max(end);
        self.dirty = true;
    }
    /// Removes trailing spaces and tabs from the text run being written.
    fn trim_run(&mut self) {
        let top = self.top();
        let Some(run) = &mut top.run else { return };
        let kept = run.text.trim_end_matches([' ', '\t']).len();
        if run.literal || kept == run.text.len() {
            return;
        }
        run.span.end -= run.text.len() - kept;
        run.text.truncate(kept);
        if run.text.is_empty() {
            top.run = None;
        }
        self.dirty = true;
    }
    fn snapshot(&self) -> Revision {
        let mut nested: Option<DisplayNode> = None;
        for (i, frame) in self.frames.iter().enumerate().rev() {
            let mut children = frame.children.clone();
            let mut end = frame.span.end;
            if let Some(run) = &frame.run {
                children.push(Arc::new(run.node()));
            }
            if let Some(node) = nested.take() {
                end = end.max(node.span.end);
                children.push(Arc::new(node));
            }
            if i == 0 {
                return Revision { nodes: children };
            }
            nested = Some(DisplayNode {
                span: Span::new(frame.span.start, end),
                kind: frame.kind.clone(),
                children,
            });
        }
        Revision::default()
    }
}

/// Converts a node from the final parser into inert display, mapping its
/// offsets (relative to `source`) through `map`.
fn convert(node: &Node, source: &str, map: &dyn Fn(usize) -> usize) -> DisplayNode {
    let span = Span::new(map(node.span.start), map(node.span.end));
    let mut children: Vec<Arc<DisplayNode>> = node
        .children
        .iter()
        .map(|child| Arc::new(convert(child, source, map)))
        .collect();
    let kind = match &node.kind {
        Kind::Markdown(markdown) => match markdown {
            Markdown::Paragraph => Display::Paragraph,
            Markdown::Heading(level) => Display::Heading(*level),
            Markdown::ThematicBreak => Display::ThematicBreak,
            Markdown::CodeBlock { language, text } => {
                children = vec![Arc::new(DisplayNode {
                    span,
                    kind: Display::Text(sanitize_str(text)),
                    children: vec![],
                })];
                Display::CodeBlock {
                    language: sanitize_str(language),
                }
            }
            Markdown::Code(text) => Display::Code(sanitize_str(text)),
            Markdown::Quote => Display::Quote,
            Markdown::List { start } => Display::List { start: *start },
            Markdown::Item => Display::Item,
            Markdown::Task(checked) => Display::Task(*checked),
            Markdown::Table => Display::Table,
            Markdown::TableHead => Display::TableHead,
            Markdown::TableRow => Display::TableRow,
            Markdown::TableCell => Display::TableCell,
            Markdown::Emphasis => Display::Emphasis,
            Markdown::Strong => Display::Strong,
            Markdown::Strikethrough => Display::Strikethrough,
            Markdown::Link { image, .. } => Display::Link { image: *image },
            Markdown::SoftBreak => Display::SoftBreak,
            Markdown::HardBreak => Display::HardBreak,
        },
        Kind::Text(text) => Display::Text(sanitize_str(text)),
        Kind::Literal(text) => Display::Literal(sanitize_str(text)),
        Kind::Reference { .. } => Display::PendingReference,
        Kind::Tag { name, block, .. } => Display::Tag {
            name: name.clone(),
            block: *block,
        },
        // Reply children never hold these; if one appears, draw its source.
        Kind::Handle { .. }
        | Kind::Interpolation { .. }
        | Kind::Declaration { .. }
        | Kind::Expression(_) => {
            children.clear();
            Display::Literal(sanitize_str(
                source
                    .get(node.span.start..node.span.end)
                    .unwrap_or_default(),
            ))
        }
    };
    DisplayNode {
        span,
        kind,
        children,
    }
}

/// Decodes UTF-8 one byte at a time, replacing each maximal invalid
/// subsequence with U+FFFD as `String::from_utf8_lossy` does.
#[derive(Default)]
struct Decoder {
    buf: [u8; 4],
    len: usize,
    need: usize,
}

impl Decoder {
    /// Feeds one byte; returns up to two decoded characters, each with the
    /// count of fed bytes (this one included) it does not cover.
    fn push(&mut self, byte: u8, out: &mut Vec<(char, usize)>) {
        if self.len > 0 {
            let valid = match (self.buf[0], self.len) {
                (0xE0, 1) => (0xA0..=0xBF).contains(&byte),
                (0xED, 1) => (0x80..=0x9F).contains(&byte),
                (0xF0, 1) => (0x90..=0xBF).contains(&byte),
                (0xF4, 1) => (0x80..=0x8F).contains(&byte),
                _ => (0x80..=0xBF).contains(&byte),
            };
            if valid {
                self.buf[self.len] = byte;
                self.len += 1;
                if self.len == self.need {
                    let c = std::str::from_utf8(&self.buf[..self.len])
                        .ok()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('\u{fffd}');
                    self.len = 0;
                    out.push((c, 0));
                }
                return;
            }
            self.len = 0;
            out.push(('\u{fffd}', 1));
        }
        let need = match byte {
            0x00..=0x7F => {
                out.push((byte as char, 0));
                return;
            }
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => {
                out.push(('\u{fffd}', 0));
                return;
            }
        };
        self.buf[0] = byte;
        self.len = 1;
        self.need = need;
    }
}

/// One decoded character, at its byte offset in the received text. `fresh`
/// characters have not yet been through line classification; the others are
/// held inline text being examined again.
#[derive(Clone, Copy, Debug)]
struct Item {
    at: usize,
    c: char,
    fresh: bool,
}

impl Item {
    fn end(self) -> usize {
        self.at + self.c.len_utf8()
    }
    fn inline(self) -> Self {
        Self {
            fresh: false,
            ..self
        }
    }
}

fn eol(c: char) -> bool {
    c == '\n' || c == '\r'
}

fn bytes(items: &[Item]) -> usize {
    items.iter().map(|item| item.c.len_utf8()).sum()
}

enum Container {
    Quote,
    List { ordered: bool },
    Item { width: usize },
}

/// The block whose content the current lines feed.
#[derive(Default)]
enum Leaf {
    #[default]
    None,
    Paragraph {
        /// The paragraph's first line, kept while it may still become a table
        /// header row.
        header: Option<Header>,
        lines: usize,
    },
    Heading,
    Fence {
        ch: char,
        len: usize,
        indent: usize,
    },
    Table,
}

#[derive(Default)]
struct Header {
    items: Vec<Item>,
    bytes: usize,
    pipe: bool,
}

enum Line {
    /// The line's start is still being classified.
    Start(Box<LineStart>),
    /// The rest of the line is inline content of the current leaf.
    Content { skip_ws: bool },
    /// The rest of the line is fenced code.
    Code,
}

impl Line {
    fn start() -> Self {
        Self::Start(Box::default())
    }
}

#[derive(Default)]
struct LineStart {
    /// Open containers this line continues so far.
    matched: usize,
    /// Columns of indentation since the last container marker.
    indent: usize,
    /// One space after `>` belongs to the quote marker.
    swallow: bool,
    /// Containers this line opens.
    opened: Vec<Opening>,
    phase: Phase,
    /// Characters this line has consumed so far.
    consumed: usize,
    /// The line was already tried as a table delimiter row.
    no_delim: bool,
    /// The line's content starts a reply declaration.
    declaration: bool,
}

enum Opening {
    Quote {
        at: usize,
    },
    Item {
        at: usize,
        ordered: bool,
        start: Option<u64>,
        width: usize,
    },
}

#[derive(Default)]
enum Phase {
    #[default]
    Match,
    Open,
    Bullet {
        ch: char,
        buf: Vec<Item>,
        indent: usize,
    },
    /// A thematic break so far. `item` is the list marker width when the
    /// line is a list item if it turns out not to be a break.
    Thematic {
        ch: char,
        count: usize,
        buf: Vec<Item>,
        item: Option<usize>,
    },
    Ordered {
        buf: Vec<Item>,
        value: u64,
        digits: usize,
        indent: usize,
    },
    OrderedDelim {
        buf: Vec<Item>,
        value: u64,
        width: usize,
    },
    ItemSpaces {
        at: usize,
        ordered: bool,
        start: Option<u64>,
        width: usize,
        spaces: usize,
    },
    Heading {
        buf: Vec<Item>,
        level: u8,
    },
    FenceRun {
        buf: Vec<Item>,
        ch: char,
        count: usize,
    },
    FenceInfo {
        buf: Vec<Item>,
        ch: char,
        count: usize,
        info: String,
    },
    Declaration {
        buf: Vec<Item>,
    },
    DelimRow {
        buf: Vec<Item>,
    },
    FenceIndent {
        stripped: usize,
    },
    FenceClose {
        buf: Vec<Item>,
        count: usize,
        lead: usize,
        trailing: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Paragraph,
    Heading,
    Cell,
}

/// The inline run in progress: a paragraph, a heading or a table cell.
#[derive(Default)]
struct Inline {
    target: Option<Target>,
    /// The run's frame is open in the builder.
    opened: bool,
    pending: Pending,
    delims: Vec<Delim>,
    /// The source character before the next one, for emphasis flanking.
    prev: char,
    /// Spaces at the end of the run's current line, for hard breaks.
    spaces: usize,
    /// Inside a bare `http(s)://` address, drawn as plain text.
    url: bool,
    recent: String,
    /// The next character is the first of a line, at a block position.
    block_start: bool,
    /// The next character starts a reply declaration.
    declaration: bool,
    /// In a table row: no cell content yet, so a `|` is the leading pipe.
    leading: bool,
}

/// Inline text held until its classification is known.
#[derive(Default)]
enum Pending {
    #[default]
    None,
    Escape(usize),
    Bang(usize),
    Angle {
        at: usize,
        block: bool,
    },
    Delim {
        ch: char,
        at: usize,
        len: usize,
    },
    Reference(Vec<Item>),
    Ticks {
        at: usize,
        len: usize,
    },
    Code(Code),
    Link(Link),
}

struct Code {
    at: usize,
    run: usize,
    buf: Vec<Item>,
    closing: usize,
    bytes: usize,
}

struct Link {
    image: bool,
    buf: Vec<Item>,
    bytes: usize,
    stage: Stage,
    escaped: bool,
}

enum Stage {
    Label { depth: usize },
    Paren,
    Destination { nesting: usize, quote: Option<char> },
}

/// An emphasis delimiter run that may still open emphasis. `index` is its
/// text node among the run frame's children.
struct Delim {
    ch: char,
    count: usize,
    orig: usize,
    close: bool,
    index: usize,
    start: usize,
    end: usize,
}

/// Text taken verbatim, bypassing line classification.
enum Raw {
    /// Reply prose braces: opaque literal through the matching `}`.
    Braces(Braces),
    /// A reply declaration: opaque literal through its last line.
    Declaration(Declaration),
    /// A tag, held whole until it is complete.
    Capture(Box<Capture>),
}

struct Braces {
    depth: usize,
    escaped: bool,
    cr: bool,
    /// Whitespace at the start of a new line, held until the line proves
    /// blank (which ends the braces) or not.
    held: Option<Vec<Item>>,
}

struct Declaration {
    scan: DeclarationScan,
    cr: bool,
    /// The line break and the start of the next line, held until it shows
    /// whether the declaration continues.
    held: Option<(Vec<Item>, String)>,
}

struct Machine {
    decoder: Decoder,
    decoded: Vec<(char, usize)>,
    raw: usize,
    offset: usize,
    stopped: bool,
    queue: VecDeque<Item>,
    out: Builder,
    containers: Vec<Container>,
    leaf: Leaf,
    line: Line,
    /// The end of the paragraph's last line, until the next line shows
    /// whether the paragraph continues.
    eol: Option<usize>,
    cr: bool,
    inline: Inline,
    raw_mode: Option<Raw>,
    work: Work,
}

impl Machine {
    fn new() -> Self {
        Self {
            decoder: Decoder::default(),
            decoded: vec![],
            raw: 0,
            offset: 0,
            stopped: false,
            queue: VecDeque::new(),
            out: Builder::new(),
            containers: vec![],
            leaf: Leaf::None,
            line: Line::start(),
            eol: None,
            cr: false,
            inline: Inline::default(),
            raw_mode: None,
            work: Work::default(),
        }
    }

    fn byte(&mut self, byte: u8) {
        self.work.scanned += 1;
        self.raw += 1;
        let mut decoded = std::mem::take(&mut self.decoded);
        self.decoder.push(byte, &mut decoded);
        for (c, uncovered) in decoded.drain(..) {
            // The final parse keeps no character that ends past the limit.
            if self.raw - uncovered > crate::REPLY_BYTES {
                self.stopped = true;
                break;
            }
            self.char(c);
        }
        self.decoded = decoded;
        if self.raw >= crate::REPLY_BYTES + 3 {
            self.stopped = true;
        }
    }

    fn char(&mut self, c: char) {
        let at = self.offset;
        self.offset += c.len_utf8();
        if at == 0 && c == '\u{feff}' {
            return;
        }
        let mut queue = std::mem::take(&mut self.queue);
        queue.push_back(Item { at, c, fresh: true });
        while let Some(item) = queue.pop_front() {
            self.step(item, &mut queue);
        }
        self.queue = queue;
    }

    fn step(&mut self, item: Item, q: &mut VecDeque<Item>) {
        if self.raw_mode.is_some() {
            self.raw_step(item, q);
        } else if item.fresh {
            self.block_step(item, q);
        } else {
            self.inline_step(item, q);
        }
    }

    /// Queues held characters to be examined again, ahead of anything else.
    fn rerun(&mut self, q: &mut VecDeque<Item>, items: &[Item]) {
        for item in items.iter().rev() {
            self.work.reexamined += item.c.len_utf8();
            q.push_front(*item);
        }
    }

    /// Examines held inline characters again now, to completion, outside the
    /// line machine. Used when a block ends and its held text must resolve.
    fn run_now(&mut self, items: &[Item]) {
        let mut q = VecDeque::new();
        self.rerun(&mut q, items);
        while let Some(item) = q.pop_front() {
            if self.raw_mode.is_some() {
                self.raw_step(item, &mut q);
            } else {
                self.inline_step(item.inline(), &mut q);
            }
        }
    }
}

/// What a line start resolved to, when it was not inline content.
enum Block {
    Heading {
        at: usize,
        level: u8,
    },
    Thematic {
        span: Span,
    },
    Fence {
        at: usize,
        ch: char,
        len: usize,
        indent: usize,
        language: String,
    },
}

impl Machine {
    fn block_step(&mut self, item: Item, q: &mut VecDeque<Item>) {
        let c = item.c;
        let crlf = c == '\n' && self.cr;
        self.cr = c == '\r';
        if crlf {
            return;
        }
        match &mut self.line {
            Line::Content { skip_ws } => {
                if eol(c) {
                    self.content_eol(item.at);
                    return;
                }
                if *skip_ws && (c == ' ' || c == '\t') {
                    return;
                }
                *skip_ws = false;
                if let Leaf::Paragraph {
                    header: Some(header),
                    lines: 1,
                } = &mut self.leaf
                {
                    header.items.push(item);
                    header.bytes += c.len_utf8();
                    header.pipe |= c == '|';
                    if header.bytes > STREAM_CONTEXT_BYTES {
                        self.drop_header();
                    }
                }
                self.inline_step(item.inline(), q);
            }
            Line::Code => {
                self.out.char(c, item.at, item.end(), false);
                if eol(c) {
                    self.out.flush_run();
                    self.line = Line::start();
                }
            }
            Line::Start(_) => self.line_char(item, q),
        }
    }

    fn drop_header(&mut self) {
        if let Leaf::Paragraph { header, .. } = &mut self.leaf {
            *header = None;
        }
    }

    /// A line of inline content ends.
    fn content_eol(&mut self, at: usize) {
        match &mut self.leaf {
            Leaf::Paragraph { header, .. } => {
                if header.as_ref().is_some_and(|header| !header.pipe) {
                    *header = None;
                }
                self.eol = Some(at);
            }
            Leaf::Heading => {
                self.end_run();
                self.leaf = Leaf::None;
            }
            Leaf::Table => {
                self.end_run();
                self.out.close();
            }
            Leaf::None | Leaf::Fence { .. } => {}
        }
        self.line = Line::start();
    }

    /// Classifies the start of a line, one character at a time.
    fn line_char(&mut self, item: Item, q: &mut VecDeque<Item>) {
        let Line::Start(mut ls) = std::mem::replace(&mut self.line, Line::Code) else {
            return;
        };
        let decided = self.classify(&mut ls, item, q);
        if !decided {
            ls.consumed += 1;
            self.line = Line::Start(ls);
        }
    }

    /// Returns true when the line start is decided and `self.line` is set.
    fn classify(&mut self, ls: &mut LineStart, item: Item, q: &mut VecDeque<Item>) -> bool {
        let c = item.c;
        loop {
            if ls.swallow {
                ls.swallow = false;
                if c == ' ' {
                    return false;
                }
            }
            match &mut ls.phase {
                Phase::Match => {
                    while matches!(
                        self.containers.get(ls.matched),
                        Some(Container::List { .. })
                    ) {
                        ls.matched += 1;
                    }
                    if ls.matched == self.containers.len() {
                        ls.phase = if matches!(self.leaf, Leaf::Fence { .. }) {
                            Phase::FenceIndent { stripped: 0 }
                        } else {
                            Phase::Open
                        };
                        continue;
                    }
                    if eol(c) {
                        return self.blank_line(ls, item);
                    }
                    let fits = match self.containers[ls.matched] {
                        Container::Quote => {
                            if c == '>' && ls.indent <= 3 {
                                ls.matched += 1;
                                ls.indent = 0;
                                ls.swallow = true;
                                return false;
                            }
                            (c == ' ' || c == '\t') && ls.indent < 3
                        }
                        Container::Item { width } => {
                            if c == ' ' || c == '\t' {
                                ls.indent += if c == '\t' { 4 - ls.indent % 4 } else { 1 };
                                if ls.indent >= width {
                                    ls.indent -= width;
                                    ls.matched += 1;
                                }
                                return false;
                            }
                            false
                        }
                        Container::List { .. } => unreachable!("lists match through their items"),
                    };
                    if fits {
                        ls.indent += if c == '\t' { 4 - ls.indent % 4 } else { 1 };
                        return false;
                    }
                    // The line leaves an open container. Fenced code has no
                    // lazy continuation, so it ends here.
                    if matches!(self.leaf, Leaf::Fence { .. }) {
                        self.close_containers(ls.matched);
                    }
                    ls.phase = Phase::Open;
                }
                Phase::Open => {
                    if c == ' ' || c == '\t' {
                        ls.indent += if c == '\t' { 4 - ls.indent % 4 } else { 1 };
                        return false;
                    }
                    if eol(c) {
                        return self.blank_line(ls, item);
                    }
                    let header = matches!(
                        &self.leaf,
                        Leaf::Paragraph { header: Some(h), lines: 1 } if h.pipe
                    );
                    if header
                        && !ls.no_delim
                        && ls.opened.is_empty()
                        && ls.matched == self.containers.len()
                        && matches!(c, '|' | '-' | ':')
                    {
                        ls.phase = Phase::DelimRow { buf: vec![item] };
                        return false;
                    }
                    if ls.indent >= 4 {
                        return self.content(ls, &[item], q);
                    }
                    let buf = vec![item];
                    let indent = ls.indent;
                    ls.phase = match c {
                        '>' => {
                            ls.opened.push(Opening::Quote { at: item.at });
                            ls.indent = 0;
                            ls.swallow = true;
                            return false;
                        }
                        '-' | '+' | '*' => Phase::Bullet { ch: c, buf, indent },
                        '_' => Phase::Thematic {
                            ch: c,
                            count: 1,
                            buf,
                            item: None,
                        },
                        '0'..='9' => Phase::Ordered {
                            buf,
                            value: u64::from(c as u8 - b'0'),
                            digits: 1,
                            indent,
                        },
                        '#' => Phase::Heading { buf, level: 1 },
                        '`' | '~' => Phase::FenceRun {
                            buf,
                            ch: c,
                            count: 1,
                        },
                        'e' if ls.consumed == 0 => Phase::Declaration { buf },
                        _ => return self.content(ls, &buf, q),
                    };
                    return false;
                }
                Phase::Bullet { ch, buf, indent } => {
                    let (ch, indent) = (*ch, *indent);
                    if c == ' ' || c == '\t' || eol(c) {
                        if ch == '+' {
                            ls.phase = Phase::ItemSpaces {
                                at: buf[0].at,
                                ordered: false,
                                start: None,
                                width: indent + 1,
                                spaces: 0,
                            };
                        } else {
                            ls.phase = Phase::Thematic {
                                ch,
                                count: 1,
                                buf: std::mem::take(buf),
                                item: Some(indent + 1),
                            };
                        }
                        continue;
                    }
                    if c == ch && ch != '+' {
                        buf.push(item);
                        ls.phase = Phase::Thematic {
                            ch,
                            count: 2,
                            buf: std::mem::take(buf),
                            item: None,
                        };
                        return false;
                    }
                    let mut buf = std::mem::take(buf);
                    buf.push(item);
                    return self.content(ls, &buf, q);
                }
                Phase::Thematic {
                    ch,
                    count,
                    buf,
                    item: marker,
                } => {
                    if c == *ch || c == ' ' || c == '\t' {
                        // Marks and blanks are ASCII, so the length is in bytes.
                        if buf.len() >= STREAM_CONTEXT_BYTES {
                            let mut buf = std::mem::take(buf);
                            buf.push(item);
                            return self.content_with(ls, &buf, q, true);
                        }
                        *count += usize::from(c == *ch);
                        buf.push(item);
                        return false;
                    }
                    if eol(c) && *count >= 3 {
                        let span = Span::new(buf[0].at, item.at);
                        return self.block(ls, Block::Thematic { span }, Some(item), q);
                    }
                    let mut buf = std::mem::take(buf);
                    buf.push(item);
                    if let Some(width) = *marker {
                        // Not a break: a list item whose content follows the
                        // marker.
                        ls.phase = Phase::ItemSpaces {
                            at: buf[0].at,
                            ordered: false,
                            start: None,
                            width,
                            spaces: 0,
                        };
                        self.rerun(q, &buf[1..]);
                        return false;
                    }
                    return self.content(ls, &buf, q);
                }
                Phase::Ordered {
                    buf,
                    value,
                    digits,
                    indent,
                } => {
                    if c.is_ascii_digit() && *digits < 9 {
                        *value = *value * 10 + u64::from(c as u8 - b'0');
                        *digits += 1;
                        buf.push(item);
                        return false;
                    }
                    let mut buf = std::mem::take(buf);
                    buf.push(item);
                    if c == '.' || c == ')' {
                        ls.phase = Phase::OrderedDelim {
                            width: *indent + *digits + 1,
                            value: *value,
                            buf,
                        };
                        return false;
                    }
                    return self.content(ls, &buf, q);
                }
                Phase::OrderedDelim { buf, value, width } => {
                    if c == ' ' || c == '\t' || eol(c) {
                        ls.phase = Phase::ItemSpaces {
                            at: buf[0].at,
                            ordered: true,
                            start: Some(*value),
                            width: *width,
                            spaces: 0,
                        };
                        continue;
                    }
                    let mut buf = std::mem::take(buf);
                    buf.push(item);
                    return self.content(ls, &buf, q);
                }
                Phase::ItemSpaces {
                    at,
                    ordered,
                    start,
                    width,
                    spaces,
                } => {
                    if c == ' ' || c == '\t' {
                        *spaces += if c == '\t' {
                            4 - (*width + *spaces) % 4
                        } else {
                            1
                        };
                        return false;
                    }
                    let content_spaces = if *spaces == 0 || *spaces > 4 {
                        1
                    } else {
                        *spaces
                    };
                    ls.indent = spaces.saturating_sub(content_spaces);
                    ls.opened.push(Opening::Item {
                        at: *at,
                        ordered: *ordered,
                        start: *start,
                        width: *width + content_spaces,
                    });
                    ls.phase = Phase::Open;
                }
                Phase::Heading { buf, level } => {
                    if c == '#' && *level < 6 {
                        *level += 1;
                        buf.push(item);
                        return false;
                    }
                    if c == ' ' || c == '\t' || eol(c) {
                        let block = Block::Heading {
                            at: buf[0].at,
                            level: *level,
                        };
                        return self.block(ls, block, eol(c).then_some(item), q);
                    }
                    let mut buf = std::mem::take(buf);
                    buf.push(item);
                    return self.content(ls, &buf, q);
                }
                Phase::FenceRun { buf, ch, count } => {
                    if c == *ch {
                        *count += 1;
                        buf.push(item);
                        return false;
                    }
                    if *count < 3 {
                        let mut buf = std::mem::take(buf);
                        buf.push(item);
                        return self.content(ls, &buf, q);
                    }
                    ls.phase = Phase::FenceInfo {
                        buf: std::mem::take(buf),
                        ch: *ch,
                        count: *count,
                        info: String::new(),
                    };
                }
                Phase::FenceInfo {
                    buf,
                    ch,
                    count,
                    info,
                } => {
                    if eol(c) {
                        let block = Block::Fence {
                            at: buf[0].at,
                            ch: *ch,
                            len: *count,
                            indent: ls.indent,
                            language: info.split_whitespace().next().unwrap_or("").to_owned(),
                        };
                        return self.block(ls, block, None, q);
                    }
                    buf.push(item);
                    // A backtick fence's info string has no backticks; a held
                    // line stops at the context cap.
                    if *ch == '`' && c == '`' {
                        let buf = std::mem::take(buf);
                        return self.content(ls, &buf, q);
                    }
                    if info.len() >= STREAM_CONTEXT_BYTES {
                        let buf = std::mem::take(buf);
                        return self.content_with(ls, &buf, q, true);
                    }
                    info.push(c);
                    return false;
                }
                Phase::Declaration { buf } => {
                    let mut text: String = buf.iter().map(|item| item.c).collect();
                    text.push(c);
                    let mut buf = std::mem::take(buf);
                    buf.push(item);
                    match declaration_prefix(&text) {
                        Some(true) => {
                            ls.declaration = true;
                            return self.content(ls, &buf, q);
                        }
                        Some(false) => {
                            ls.phase = Phase::Declaration { buf };
                            return false;
                        }
                        None => return self.content(ls, &buf, q),
                    }
                }
                Phase::DelimRow { buf } => {
                    if matches!(c, '|' | '-' | ':' | ' ' | '\t') && buf.len() < STREAM_CONTEXT_BYTES
                    {
                        buf.push(item);
                        return false;
                    }
                    let mut buf = std::mem::take(buf);
                    if eol(c) && self.delimiter_row(&buf) {
                        self.make_table(&buf);
                        self.line = Line::start();
                        return true;
                    }
                    // Not a delimiter row: classify the line again as usual.
                    ls.no_delim = true;
                    ls.phase = Phase::Open;
                    buf.push(item);
                    self.rerun(q, &buf);
                    return false;
                }
                Phase::FenceIndent { stripped } => {
                    let indent = match self.leaf {
                        Leaf::Fence { indent, .. } => indent,
                        _ => 0,
                    };
                    if c == ' ' && *stripped < indent {
                        *stripped += 1;
                        return false;
                    }
                    ls.phase = Phase::FenceClose {
                        buf: vec![],
                        count: 0,
                        lead: 0,
                        trailing: false,
                    };
                }
                Phase::FenceClose {
                    buf,
                    count,
                    lead,
                    trailing,
                } => {
                    let (ch, len) = match self.leaf {
                        Leaf::Fence { ch, len, .. } => (ch, len),
                        _ => ('`', usize::MAX),
                    };
                    if eol(c) && *count >= len {
                        self.close_leaf();
                        self.line = Line::start();
                        return true;
                    }
                    let held = if c == ' ' && *count == 0 && *lead < 3 {
                        *lead += 1;
                        true
                    } else if c == ch && !*trailing {
                        *count += 1;
                        true
                    } else if (c == ' ' || c == '\t') && *count > 0 {
                        *trailing = true;
                        true
                    } else {
                        false
                    };
                    if held && !eol(c) {
                        buf.push(item);
                        return false;
                    }
                    let mut buf = std::mem::take(buf);
                    buf.push(item);
                    self.line = Line::Code;
                    self.rerun(q, &buf);
                    return true;
                }
            }
        }
    }

    /// The line is a blank line (only whitespace and container markers).
    fn blank_line(&mut self, ls: &mut LineStart, item: Item) -> bool {
        // A blank line continues open list items but not quotes.
        let mut keep = ls.matched;
        while keep < self.containers.len() && !matches!(self.containers[keep], Container::Quote) {
            keep += 1;
        }
        if ls.opened.is_empty()
            && keep == self.containers.len()
            && matches!(self.leaf, Leaf::Fence { .. })
        {
            self.out.char('\n', item.at, item.end(), false);
            self.out.flush_run();
        } else if ls.opened.is_empty() {
            self.close_containers(keep);
        } else {
            self.settle(ls, false);
        }
        self.line = Line::start();
        true
    }

    /// Closes and opens containers for a line that is not a paragraph
    /// continuation. Returns true when the line continues the open paragraph.
    fn settle(&mut self, ls: &mut LineStart, content: bool) -> bool {
        if content && ls.opened.is_empty() && matches!(self.leaf, Leaf::Paragraph { .. }) {
            return true;
        }
        let mut keep = ls.matched;
        if keep > 0 {
            if let Container::List { ordered } = self.containers[keep - 1] {
                let same = matches!(ls.opened.first(), Some(Opening::Item { ordered: o, .. }) if *o == ordered);
                if !same {
                    keep -= 1;
                }
            }
        }
        self.close_containers(keep);
        for opening in std::mem::take(&mut ls.opened) {
            if self.containers.len() + 2 > CONTAINERS {
                break;
            }
            match opening {
                Opening::Quote { at } => {
                    self.containers.push(Container::Quote);
                    self.out.open(Display::Quote, at);
                }
                Opening::Item {
                    at,
                    ordered,
                    start,
                    width,
                } => {
                    let reuse = matches!(self.containers.last(), Some(Container::List { ordered: o }) if *o == ordered);
                    if !reuse {
                        if matches!(self.containers.last(), Some(Container::List { .. })) {
                            self.containers.pop();
                            self.out.close();
                        }
                        self.containers.push(Container::List { ordered });
                        self.out.open(Display::List { start }, at);
                    }
                    self.containers.push(Container::Item { width });
                    self.out.open(Display::Item, at);
                }
            }
        }
        false
    }

    fn close_leaf(&mut self) {
        match std::mem::take(&mut self.leaf) {
            Leaf::Paragraph { .. } | Leaf::Heading => self.end_run(),
            Leaf::Fence { .. } => self.out.close(),
            Leaf::Table => {
                self.end_run();
                while matches!(
                    self.out.top_kind(),
                    Display::TableRow | Display::TableHead | Display::TableCell
                ) {
                    self.out.close();
                }
                self.out.close();
            }
            Leaf::None => {}
        }
        self.eol = None;
    }

    fn close_containers(&mut self, keep: usize) {
        self.close_leaf();
        while self.containers.len() > keep {
            self.containers.pop();
            self.out.close();
        }
    }

    /// The line is inline content; `items` are its characters so far.
    fn content(&mut self, ls: &mut LineStart, items: &[Item], q: &mut VecDeque<Item>) -> bool {
        self.content_with(ls, items, q, false)
    }

    /// Like [`Self::content`]; when `capped`, the held line start reached its
    /// cap and is drawn as a provisional literal instead of examined again.
    fn content_with(
        &mut self,
        ls: &mut LineStart,
        items: &[Item],
        q: &mut VecDeque<Item>,
        capped: bool,
    ) -> bool {
        let block_start = ls.indent <= 3 && !capped;
        let row = ls.opened.is_empty()
            && ls.matched == self.containers.len()
            && matches!(self.leaf, Leaf::Table);
        self.line = Line::Content { skip_ws: false };
        let mut newline = None;
        if row {
            self.out.open(Display::TableRow, items[0].at);
            self.start_run(Target::Cell);
        } else if self.settle(ls, true) {
            if let Leaf::Paragraph { header, lines } = &mut self.leaf {
                *lines += 1;
                *header = None;
            }
            newline = self.eol.take().map(|at| Item {
                at,
                c: '\n',
                fresh: false,
            });
        } else {
            self.leaf = Leaf::Paragraph {
                header: Some(Header::default()),
                lines: 1,
            };
            self.start_run(Target::Paragraph);
        }
        self.inline.block_start = block_start && !row;
        self.inline.declaration = ls.declaration;
        if capped {
            if let Some(newline) = newline {
                self.run_now(&[newline]);
            }
            self.drop_header();
            self.cap_flush(items, "", items[0].at);
        } else {
            // Both go to the front of the queue: the line's characters first,
            // then the paragraph's line break ahead of them, so the break is
            // examined before the characters that follow it.
            self.rerun(q, items);
            if let Some(newline) = newline {
                q.push_front(newline);
            }
        }
        true
    }

    /// The line starts a block that is not a paragraph.
    fn block(
        &mut self,
        ls: &mut LineStart,
        block: Block,
        eol_item: Option<Item>,
        q: &mut VecDeque<Item>,
    ) -> bool {
        self.settle(ls, false);
        match block {
            Block::Thematic { span } => {
                self.out.push(DisplayNode {
                    span,
                    kind: Display::ThematicBreak,
                    children: vec![],
                });
                self.line = Line::start();
            }
            Block::Heading { at, level } => {
                self.out.open(Display::Heading(level), at);
                self.leaf = Leaf::Heading;
                self.start_run(Target::Heading);
                self.inline.opened = true;
                self.line = Line::Content { skip_ws: true };
                if let Some(item) = eol_item {
                    self.rerun(q, &[item]);
                }
            }
            Block::Fence {
                at,
                ch,
                len,
                indent,
                language,
            } => {
                self.out.open(
                    Display::CodeBlock {
                        language: sanitize_str(&language),
                    },
                    at,
                );
                self.leaf = Leaf::Fence { ch, len, indent };
                self.line = Line::start();
            }
        }
        true
    }

    /// Whether a held line is a GFM delimiter row matching the held header.
    fn delimiter_row(&self, buf: &[Item]) -> bool {
        let Leaf::Paragraph {
            header: Some(header),
            ..
        } = &self.leaf
        else {
            return false;
        };
        let row: String = buf.iter().map(|item| item.c).collect();
        let head: String = header.items.iter().map(|item| item.c).collect();
        let cells = row_cells(&row);
        !cells.is_empty()
            && cells.len() == row_cells(&head).len()
            && cells.iter().all(|cell| {
                let cell = cell.trim();
                let dashes = cell.trim_start_matches(':').trim_end_matches(':');
                !dashes.is_empty()
                    && dashes.bytes().all(|b| b == b'-')
                    && cell.len() - dashes.len() <= 2
            })
    }

    /// Redraws the held paragraph line as a table header.
    fn make_table(&mut self, delimiter: &[Item]) {
        let header = match &mut self.leaf {
            Leaf::Paragraph { header, .. } => header.take().unwrap_or_default(),
            _ => Header::default(),
        };
        self.discard_run();
        self.leaf = Leaf::Table;
        let at = header.items.first().map_or(delimiter[0].at, |item| item.at);
        self.out.open(Display::Table, at);
        self.out.open(Display::TableHead, at);
        self.start_run(Target::Cell);
        self.run_now(&header.items);
        self.finish_raw();
        self.end_run();
        self.out.close();
    }
}

/// Splits a table row into cells at unescaped pipes, ignoring one leading
/// and one trailing pipe.
fn row_cells(row: &str) -> Vec<String> {
    let row = row.trim();
    let row = row.strip_prefix('|').unwrap_or(row);
    let row = match row.strip_suffix('|') {
        Some(rest) if !rest.ends_with('\\') => rest,
        _ => row,
    };
    let mut cells = vec![String::new()];
    let mut escaped = false;
    for c in row.chars() {
        if c == '|' && !escaped {
            cells.push(String::new());
        } else {
            cells.last_mut().expect("starts non-empty").push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    cells
}

/// Whether `text` is a reply declaration's start (`export`, whitespace,
/// `let` or `const`, whitespace): `Some(true)` when it is, `Some(false)` when
/// it still may be, `None` when it cannot be.
fn declaration_prefix(text: &str) -> Option<bool> {
    if text.len() > 64 {
        return None;
    }
    let Some(rest) = text.strip_prefix("export") else {
        return "export".starts_with(text).then_some(false);
    };
    if rest.is_empty() {
        return Some(false);
    }
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start_matches([' ', '\t']);
    for keyword in ["let", "const"] {
        if let Some(tail) = rest.strip_prefix(keyword) {
            return match tail.chars().next() {
                None => Some(false),
                Some(' ' | '\t') => Some(true),
                Some(_) => None,
            };
        }
        if keyword.starts_with(rest) {
            return Some(false);
        }
    }
    None
}

fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace())
}

/// A provisional literal for held text, control characters replaced.
fn literal_node(items: &[Item], prefix: &str, at: usize) -> DisplayNode {
    let mut text = String::from(prefix);
    text.extend(items.iter().map(|item| item.c));
    DisplayNode {
        span: Span::new(
            at,
            items.last().map_or(at + prefix.len(), |item| item.end()),
        ),
        kind: Display::Literal(sanitize_str(&text)),
        children: vec![],
    }
}

impl Machine {
    fn start_run(&mut self, target: Target) {
        self.inline = Inline {
            target: Some(target),
            prev: ' ',
            leading: target == Target::Cell,
            ..Inline::default()
        };
    }

    /// Opens the run's frame before its first drawn content.
    fn ensure_frame(&mut self, at: usize) {
        if self.inline.opened {
            return;
        }
        let kind = match self.inline.target {
            Some(Target::Cell) => Display::TableCell,
            Some(Target::Heading) => Display::Heading(1),
            Some(Target::Paragraph) => Display::Paragraph,
            None => {
                // The inline run already ended, yet inline text arrives on
                // the same line: the rest of a line after a block tag ended
                // the paragraph, or held text the final parser left
                // unconsumed after a tag. That text starts a new paragraph.
                self.leaf = Leaf::Paragraph {
                    header: None,
                    lines: 1,
                };
                self.inline.target = Some(Target::Paragraph);
                Display::Paragraph
            }
        };
        self.out.open(kind, at);
        self.inline.opened = true;
    }

    fn emit_char(&mut self, c: char, start: usize, end: usize) {
        self.ensure_frame(start);
        self.out.char(c, start, end, false);
        let inline = &mut self.inline;
        inline.prev = c;
        inline.spaces = if c == ' ' { inline.spaces + 1 } else { 0 };
        inline.recent.push(c);
        if inline.recent.len() > 8 {
            inline.recent.remove(0);
        }
        if inline.recent.ends_with("http://") || inline.recent.ends_with("https://") {
            inline.url = true;
        }
    }

    fn emit_items(&mut self, items: &[Item]) {
        for item in items {
            self.emit_char(item.c, item.at, item.end());
        }
    }

    fn literal_char(&mut self, item: Item) {
        self.ensure_frame(item.at);
        self.out.char(item.c, item.at, item.end(), true);
        self.inline.prev = item.c;
        self.inline.spaces = 0;
        self.inline.recent.clear();
    }

    fn emit_node(&mut self, node: DisplayNode, prev: char) {
        self.ensure_frame(node.span.start);
        self.out.push(node);
        self.inline.prev = prev;
        self.inline.spaces = 0;
        self.inline.recent.clear();
    }

    /// Draws held text as a provisional literal because its context reached a
    /// buffering cap, and forgets the context.
    fn cap_flush(&mut self, items: &[Item], prefix: &str, at: usize) {
        self.work.reexamined += bytes(items) + prefix.len();
        let prev = items.last().map_or(' ', |item| item.c);
        self.emit_node(literal_node(items, prefix, at), prev);
    }

    /// Ends the inline run: held text resolves as the block ends.
    fn end_run(&mut self) {
        let Some(target) = self.inline.target else {
            return;
        };
        let pending = std::mem::take(&mut self.inline.pending);
        let again = self.resolve_end(pending);
        if !again.is_empty() {
            self.run_now(&again);
        }
        // Anything held again by that pass is drawn as text, without another.
        let pending = std::mem::take(&mut self.inline.pending);
        self.flush_pending(pending);
        self.finish_raw();
        if self.inline.opened {
            self.out.trim_run();
            if target == Target::Heading {
                self.trim_closing_hashes();
            }
            self.out.close();
        }
        self.inline = Inline::default();
    }

    /// Drops the inline run and everything it drew.
    fn discard_run(&mut self) {
        if self.inline.opened {
            self.out.discard();
        }
        self.inline = Inline::default();
        self.eol = None;
    }

    /// A heading's optional closing sequence of `#` is not drawn.
    fn trim_closing_hashes(&mut self) {
        let top = self.out.top();
        let Some(run) = &mut top.run else { return };
        let kept = run.text.trim_end_matches('#');
        if kept.len() < run.text.len() && (kept.is_empty() || kept.ends_with([' ', '\t'])) {
            let kept = kept.trim_end_matches([' ', '\t']).len();
            run.span.end -= run.text.len() - kept;
            run.text.truncate(kept);
            if run.text.is_empty() {
                top.run = None;
            }
            self.out.dirty = true;
        }
    }

    /// Resolves a held context at the end of its block. Returns held
    /// characters to examine once more.
    fn resolve_end(&mut self, pending: Pending) -> Vec<Item> {
        match pending {
            Pending::Code(code) if code.closing == code.run => {
                self.resolve_code(code);
                vec![]
            }
            Pending::Code(code) => {
                for i in 0..code.run {
                    self.emit_char('`', code.at + i, code.at + i + 1);
                }
                code.buf
            }
            Pending::Link(link) => {
                let opener = if link.image { 2 } else { 1 };
                self.emit_items(&link.buf[..opener]);
                link.buf[opener..].to_vec()
            }
            other => {
                self.flush_pending(other);
                vec![]
            }
        }
    }

    /// Draws a held context as plain text.
    fn flush_pending(&mut self, pending: Pending) {
        match pending {
            Pending::None => {}
            Pending::Escape(at) => self.emit_char('\\', at, at + 1),
            Pending::Bang(at) => self.emit_char('!', at, at + 1),
            Pending::Angle { at, .. } => self.emit_char('<', at, at + 1),
            Pending::Delim { ch, at, len } => self.resolve_delim(ch, at, len, ' '),
            Pending::Ticks { at, len } => {
                for i in 0..len {
                    self.emit_char('`', at + i, at + i + 1);
                }
            }
            Pending::Reference(buf) => {
                self.work.reexamined += bytes(&buf);
                self.emit_items(&buf);
            }
            Pending::Code(code) => {
                self.work.reexamined += code.bytes;
                for i in 0..code.run {
                    self.emit_char('`', code.at + i, code.at + i + 1);
                }
                self.emit_items(&code.buf);
            }
            Pending::Link(link) => {
                self.work.reexamined += link.bytes;
                self.emit_items(&link.buf);
            }
        }
    }

    fn inline_step(&mut self, item: Item, q: &mut VecDeque<Item>) {
        let c = item.c;
        // GFM splits table cells at unescaped pipes before inline parsing.
        if self.inline.target == Some(Target::Cell)
            && c == '|'
            && !matches!(self.inline.pending, Pending::Escape(_))
        {
            self.cell_boundary(item);
            return;
        }
        match std::mem::take(&mut self.inline.pending) {
            Pending::None => self.plain(item, q),
            Pending::Escape(at) => {
                if c == '\n' {
                    let node = DisplayNode {
                        span: Span::new(at, item.end()),
                        kind: Display::HardBreak,
                        children: vec![],
                    };
                    self.emit_node(node, '\n');
                } else if c.is_ascii_punctuation() {
                    self.emit_char(c, at, item.end());
                } else {
                    self.emit_char('\\', at, at + 1);
                    q.push_front(item);
                }
            }
            Pending::Bang(at) => {
                if c == '[' {
                    self.inline.pending = Pending::Link(Link {
                        image: true,
                        buf: vec![
                            Item {
                                at,
                                c: '!',
                                fresh: false,
                            },
                            item,
                        ],
                        bytes: 2,
                        stage: Stage::Label { depth: 1 },
                        escaped: false,
                    });
                } else {
                    self.emit_char('!', at, at + 1);
                    q.push_front(item);
                }
            }
            Pending::Angle { at, block } => {
                if c.is_ascii_uppercase() {
                    self.drop_header();
                    self.raw_mode = Some(Raw::Capture(Box::new(Capture::new(at, block))));
                    self.raw_step(item, q);
                } else {
                    self.emit_char('<', at, at + 1);
                    q.push_front(item);
                }
            }
            Pending::Delim { ch, at, len } => {
                if c == ch {
                    self.inline.pending = Pending::Delim {
                        ch,
                        at,
                        len: len + 1,
                    };
                } else {
                    self.resolve_delim(ch, at, len, c);
                    q.push_front(item);
                }
            }
            Pending::Reference(buf) => self.reference(buf, item, q),
            Pending::Ticks { at, len } => {
                if c == '`' {
                    self.inline.pending = Pending::Ticks { at, len: len + 1 };
                } else {
                    self.inline.pending = Pending::Code(Code {
                        at,
                        run: len,
                        buf: vec![],
                        closing: 0,
                        bytes: len,
                    });
                    q.push_front(item);
                }
            }
            Pending::Code(code) => self.code_char(code, item, q),
            Pending::Link(link) => self.link_char(link, item, q),
        }
    }

    fn plain(&mut self, item: Item, q: &mut VecDeque<Item>) {
        let (c, at) = (item.c, item.at);
        if c == '\n' {
            self.newline(at);
            return;
        }
        if std::mem::take(&mut self.inline.declaration) {
            self.drop_header();
            self.raw_mode = Some(Raw::Declaration(Declaration {
                scan: DeclarationScan::default(),
                cr: false,
                held: None,
            }));
            self.raw_step(item, q);
            return;
        }
        if self.inline.url {
            if c.is_whitespace() || c == '<' || c == '>' {
                self.inline.url = false;
            } else {
                self.emit_char(c, at, item.end());
                return;
            }
        }
        let block = std::mem::take(&mut self.inline.block_start);
        if self.inline.target == Some(Target::Cell)
            && !self.inline.opened
            && (c == ' ' || c == '\t')
        {
            return;
        }
        self.inline.leading = false;
        self.inline.pending = match c {
            '\\' => Pending::Escape(at),
            '`' => Pending::Ticks { at, len: 1 },
            '[' => Pending::Link(Link {
                image: false,
                buf: vec![item],
                bytes: 1,
                stage: Stage::Label { depth: 1 },
                escaped: false,
            }),
            '!' => Pending::Bang(at),
            '*' | '_' | '~' => Pending::Delim { ch: c, at, len: 1 },
            '$' => Pending::Reference(vec![item]),
            '<' => Pending::Angle { at, block },
            '{' => {
                // Reply prose braces are opaque literal text.
                self.drop_header();
                self.literal_char(item);
                self.raw_mode = Some(Raw::Braces(Braces {
                    depth: 1,
                    escaped: false,
                    cr: false,
                    held: None,
                }));
                Pending::None
            }
            _ => {
                self.emit_char(c, at, item.end());
                Pending::None
            }
        };
    }

    /// A line break inside a paragraph.
    fn newline(&mut self, at: usize) {
        let hard = self.inline.spaces >= 2;
        if self.inline.opened {
            self.out.trim_run();
            let node = DisplayNode {
                span: Span::new(at, at + 1),
                kind: if hard {
                    Display::HardBreak
                } else {
                    Display::SoftBreak
                },
                children: vec![],
            };
            self.out.push(node);
        }
        self.inline.prev = '\n';
        self.inline.spaces = 0;
        self.inline.url = false;
        self.inline.recent.clear();
    }

    fn cell_boundary(&mut self, item: Item) {
        if std::mem::take(&mut self.inline.leading) {
            return;
        }
        let opened = self.inline.opened;
        self.end_run();
        if !opened {
            self.out.open(Display::TableCell, item.at);
            self.out.close();
        }
        self.start_run(Target::Cell);
        self.inline.leading = false;
    }

    /// Resolves a run of `*`, `_` or `~` once the character after it is
    /// known, following CommonMark's flanking and matching rules.
    fn resolve_delim(&mut self, ch: char, at: usize, len: usize, next: char) {
        let prev = self.inline.prev;
        let left = !next.is_whitespace()
            && (!is_punctuation(next) || prev.is_whitespace() || is_punctuation(prev));
        let right = !prev.is_whitespace()
            && (!is_punctuation(prev) || next.is_whitespace() || is_punctuation(next));
        let (open, close) = if ch == '_' {
            (
                left && (!right || is_punctuation(prev)),
                right && (!left || is_punctuation(next)),
            )
        } else {
            (left, right)
        };
        if ch == '~' && len != 2 {
            for i in 0..len {
                self.emit_char('~', at + i, at + i + 1);
            }
            return;
        }
        let mut count = len;
        let mut pos = at;
        while close && count > 0 {
            let found = self.inline.delims.iter().rposition(|d| {
                d.ch == ch
                    && (ch != '~' || d.count == count)
                    && !((d.close || open)
                        && (d.orig + len).is_multiple_of(3)
                        && !(d.orig.is_multiple_of(3) && len.is_multiple_of(3)))
            });
            let Some(j) = found else { break };
            let used = if ch == '~' || (self.inline.delims[j].count >= 2 && count >= 2) {
                2
            } else {
                1
            };
            let kind = match (ch, used) {
                ('~', _) => Display::Strikethrough,
                (_, 2) => Display::Strong,
                _ => Display::Emphasis,
            };
            self.wrap(j, used, pos, kind);
            count -= used;
            pos += used;
        }
        if count > 0 {
            self.ensure_frame(pos);
            self.out.push(DisplayNode {
                span: Span::new(pos, at + len),
                kind: Display::Text(std::iter::repeat_n(ch, count).collect()),
                children: vec![],
            });
            if open && self.inline.delims.len() < EMPHASIS_DEPTH {
                let index = self.out.top().children.len() - 1;
                self.inline.delims.push(Delim {
                    ch,
                    count,
                    orig: len,
                    close,
                    index,
                    start: pos,
                    end: at + len,
                });
            }
        }
        self.inline.prev = ch;
        self.inline.spaces = 0;
        self.inline.recent.clear();
    }

    /// Wraps everything drawn after opener `j` into an emphasis node closed
    /// by `used` delimiters at `pos`.
    fn wrap(&mut self, j: usize, used: usize, pos: usize, kind: Display) {
        self.out.flush_run();
        let opener = &mut self.inline.delims[j];
        opener.count -= used;
        opener.end -= used;
        let (index, count, start, end, ch) = (
            opener.index,
            opener.count,
            opener.start,
            opener.end,
            opener.ch,
        );
        let span = Span::new(end, pos + used);
        let top = self.out.top();
        let inner = top.children.split_off(index + 1);
        if count == 0 {
            top.children.pop();
        } else {
            top.children[index] = Arc::new(DisplayNode {
                span: Span::new(start, end),
                kind: Display::Text(std::iter::repeat_n(ch, count).collect()),
                children: vec![],
            });
        }
        top.children.push(Arc::new(DisplayNode {
            span,
            kind,
            children: inner,
        }));
        top.span.end = top.span.end.max(span.end);
        self.inline.delims.truncate(j + 1);
        if count == 0 {
            self.inline.delims.pop();
        }
        self.out.dirty = true;
    }

    /// A possible `$$tc(N)` reference, held at most
    /// [`PENDING_REFERENCE_CHARS`] characters.
    fn reference(&mut self, mut buf: Vec<Item>, item: Item, q: &mut VecDeque<Item>) {
        let c = item.c;
        let i = buf.len();
        let fits = match i {
            1 => c == '$',
            2 => c == 't',
            3 => c == 'c',
            4 => c == '(',
            _ => (c.is_ascii_digit() && !(i > 5 && buf[5].c == '0')) || (c == ')' && i > 5),
        };
        if !fits {
            // Not a reference here; one may still start at the next `$`.
            self.emit_char('$', buf[0].at, buf[0].end());
            q.push_front(item);
            self.rerun(q, &buf[1..]);
            return;
        }
        if c == ')' {
            let node = DisplayNode {
                span: Span::new(buf[0].at, item.end()),
                kind: Display::PendingReference,
                children: vec![],
            };
            self.emit_node(node, ')');
            return;
        }
        if buf.len() == PENDING_REFERENCE_CHARS {
            self.cap_flush(&buf, "", buf[0].at);
            q.push_front(item);
            return;
        }
        buf.push(item);
        self.inline.pending = Pending::Reference(buf);
    }

    fn code_char(&mut self, mut code: Code, item: Item, q: &mut VecDeque<Item>) {
        if item.c == '`' {
            code.closing += 1;
        } else if code.closing == code.run {
            self.resolve_code(code);
            q.push_front(item);
            return;
        } else {
            code.closing = 0;
        }
        code.buf.push(item);
        code.bytes += item.c.len_utf8();
        if code.bytes > STREAM_CONTEXT_BYTES {
            let ticks = "`".repeat(code.run);
            self.cap_flush(&code.buf, &ticks, code.at);
            return;
        }
        self.inline.pending = Pending::Code(code);
    }

    /// Draws a closed code span, normalized as CommonMark does.
    fn resolve_code(&mut self, code: Code) {
        self.work.reexamined += code.bytes;
        let content = &code.buf[..code.buf.len() - code.run];
        let mut text: String = content
            .iter()
            .map(|item| if eol(item.c) { ' ' } else { item.c })
            .collect();
        if text.len() >= 2
            && text.starts_with(' ')
            && text.ends_with(' ')
            && text.chars().any(|c| c != ' ')
        {
            text = text[1..text.len() - 1].to_owned();
        }
        let end = code
            .buf
            .last()
            .map_or(code.at + code.run, |item| item.end());
        let node = DisplayNode {
            span: Span::new(code.at, end),
            kind: Display::Code(sanitize_str(&text)),
            children: vec![],
        };
        self.emit_node(node, '`');
    }

    fn link_char(&mut self, mut link: Link, item: Item, q: &mut VecDeque<Item>) {
        let c = item.c;
        link.buf.push(item);
        link.bytes += c.len_utf8();
        if link.bytes > STREAM_CONTEXT_BYTES {
            self.cap_flush(&link.buf, "", link.buf[0].at);
            return;
        }
        if std::mem::take(&mut link.escaped) {
            self.inline.pending = Pending::Link(link);
            return;
        }
        match &mut link.stage {
            Stage::Label { depth } => match c {
                '\\' => link.escaped = true,
                '[' => {
                    *depth += 1;
                    if *depth > LABEL_DEPTH {
                        self.cap_flush(&link.buf, "", link.buf[0].at);
                        return;
                    }
                }
                ']' => {
                    *depth -= 1;
                    if *depth == 0 {
                        link.stage = Stage::Paren;
                    }
                }
                _ => {}
            },
            Stage::Paren => {
                if c != '(' {
                    self.link_failed(link, q);
                    return;
                }
                link.stage = Stage::Destination {
                    nesting: 1,
                    quote: None,
                };
            }
            Stage::Destination { nesting, quote } => match (*quote, c) {
                (_, '\\') => link.escaped = true,
                (Some(open), c) if c == open => *quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => *quote = Some(c),
                (None, '(') => *nesting += 1,
                (None, ')') => {
                    *nesting -= 1;
                    if *nesting == 0 {
                        self.link_complete(link, q);
                        return;
                    }
                }
                _ => {}
            },
        }
        self.inline.pending = Pending::Link(link);
    }

    /// Not a link: the brackets are text and what they held is examined
    /// again.
    fn link_failed(&mut self, link: Link, q: &mut VecDeque<Item>) {
        let opener = if link.image { 2 } else { 1 };
        self.emit_items(&link.buf[..opener]);
        self.rerun(q, &link.buf[opener..]);
    }

    /// Classifies a complete `[label](destination)` with the final parser.
    fn link_complete(&mut self, link: Link, q: &mut VecDeque<Item>) {
        let text: String = link.buf.iter().map(|item| item.c).collect();
        self.work.reexamined += text.len();
        let doc = parse(&text, Mode::Reply);
        let found = match doc.nodes.as_slice() {
            [Node {
                kind: Kind::Markdown(Markdown::Paragraph),
                children,
                ..
            }] => match children.as_slice() {
                [node @ Node {
                    kind: Kind::Markdown(Markdown::Link { .. }),
                    ..
                }] if node.span == Span::new(0, text.len()) => Some(node.clone()),
                _ => None,
            },
            _ => None,
        };
        let Some(node) = found else {
            self.link_failed(link, q);
            return;
        };
        // Offsets in the link's text map back to where each character arrived.
        let mut offsets = Vec::with_capacity(text.len() + 1);
        for item in &link.buf {
            offsets.extend((0..item.c.len_utf8()).map(|i| item.at + i));
        }
        offsets.push(link.buf.last().map_or(0, |item| item.end()));
        let display = convert(&node, &text, &|o| offsets[o.min(offsets.len() - 1)]);
        self.emit_node(display, ')');
    }
}

/// Tracks a reply declaration's text to tell, at each line end, whether the
/// next line continues it. Mirrors the final parser's rule one character at
/// a time: an open bracket, tag or string, or a trailing operator, continues.
struct DeclarationScan {
    depth: i32,
    tags: i32,
    header: bool,
    closing: bool,
    header_braces: i32,
    quote: Option<char>,
    escaped: bool,
    operand: bool,
    previous: char,
    /// A `<` waiting for the next character to show whether it opens a tag.
    angle: bool,
}

impl Default for DeclarationScan {
    fn default() -> Self {
        Self {
            depth: 0,
            tags: 0,
            header: false,
            closing: false,
            header_braces: 0,
            quote: None,
            escaped: false,
            operand: true,
            previous: '\0',
            angle: false,
        }
    }
}

impl DeclarationScan {
    fn feed(&mut self, c: char) {
        if std::mem::take(&mut self.angle) {
            if c.is_ascii_uppercase() || c == '/' {
                self.closing = c == '/';
                self.tags += if self.closing { -1 } else { 1 };
                self.header = true;
                self.header_braces = 0;
            } else if self.tags == 0 {
                self.operand = true;
            }
            self.previous = '<';
        }
        if self.escaped {
            self.escaped = false;
            return;
        }
        if c == '\\' {
            self.escaped = true;
            return;
        }
        if let Some(quote) = self.quote {
            if c == quote {
                self.quote = None;
                self.operand = false;
            }
            return;
        }
        if (self.header || self.tags == 0) && (c == '\'' || c == '"') {
            self.quote = Some(c);
            return;
        }
        if c == '<' && !self.header && (self.tags > 0 || self.operand) {
            self.angle = true;
            return;
        }
        if self.header {
            match c {
                '{' => self.header_braces += 1,
                '}' => self.header_braces -= 1,
                '>' if self.header_braces == 0 => {
                    if !self.closing && self.previous == '/' {
                        self.tags -= 1;
                    }
                    self.header = false;
                    self.operand = false;
                }
                _ => {}
            }
        } else if self.tags == 0 {
            match c {
                '(' | '[' | '{' => {
                    self.depth += 1;
                    self.operand = true;
                }
                ')' | ']' | '}' => {
                    self.depth -= 1;
                    self.operand = false;
                }
                '+' | '-' | '*' | '/' | '%' | '?' | ':' | ',' | '=' | '!' | '<' | '>' | '&'
                | '|' => self.operand = true,
                c if !c.is_whitespace() => self.operand = false,
                _ => {}
            }
        }
        if !c.is_whitespace() {
            self.previous = c;
        }
    }
    fn continues(&self) -> bool {
        self.depth > 0 || self.tags > 0 || self.header || self.operand || self.angle
    }
}

/// Scans a tag opening as the final parser bounds a failed one: through the
/// first `>` outside strings and braces.
#[derive(Default)]
struct TagScan {
    quote: Option<char>,
    braces: usize,
    escaped: bool,
    prev: char,
}

impl TagScan {
    /// Returns whether the opening self-closes once its `>` arrives.
    fn feed(&mut self, c: char) -> Option<bool> {
        let prev = std::mem::replace(&mut self.prev, c);
        if self.escaped {
            self.escaped = false;
        } else if c == '\\' {
            self.escaped = true;
        } else if let Some(quote) = self.quote {
            if c == quote {
                self.quote = None;
            }
        } else if c == '\'' || c == '"' {
            self.quote = Some(c);
        } else if c == '{' {
            self.braces += 1;
        } else if c == '}' {
            self.braces = self.braces.saturating_sub(1);
        } else if c == '>' && self.braces == 0 {
            return Some(prev == '/');
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CapturePhase {
    /// The outer `<Name …>`.
    Opening,
    /// After the outer opening: the rest of its line decides between a block
    /// and an inline tag.
    Suffix {
        self_closing: bool,
    },
    Body,
    /// A block tag's close was seen; its line must end before the tag is
    /// complete.
    CloseLine,
}

/// Where the body scan stands inside one tag-like construct.
enum Inner {
    Plain,
    Angle,
    Open(TagScan),
    Close { named: bool, spaced: bool },
}

/// A tag held whole until complete, then classified by the final parser.
/// The scan only decides where the tag ends; the final parser decides what
/// it is.
struct Capture {
    base: usize,
    text: String,
    /// Leading bytes of `text` that already passed line classification.
    inline_bytes: usize,
    block_candidate: bool,
    phase: CapturePhase,
    opener: TagScan,
    opener_end: usize,
    block: bool,
    depth: usize,
    braces: usize,
    escaped: bool,
    inner: Inner,
    line_start: usize,
    line_blank: bool,
    first_line: bool,
    cr: bool,
}

impl Capture {
    fn new(base: usize, block_candidate: bool) -> Self {
        Self {
            base,
            text: "<".to_owned(),
            inline_bytes: 1,
            block_candidate,
            phase: CapturePhase::Opening,
            opener: TagScan {
                prev: '<',
                ..TagScan::default()
            },
            opener_end: 0,
            block: false,
            depth: 0,
            braces: 0,
            escaped: false,
            inner: Inner::Plain,
            line_start: 0,
            line_blank: false,
            first_line: true,
            cr: false,
        }
    }

    /// Feeds one body character; returns true at the outer tag's close.
    fn body(&mut self, c: char) -> bool {
        if std::mem::take(&mut self.escaped) {
            return false;
        }
        match &mut self.inner {
            Inner::Plain => match c {
                '\\' => self.escaped = true,
                '<' if self.braces == 0 => self.inner = Inner::Angle,
                '{' => self.braces += 1,
                '}' => self.braces = self.braces.saturating_sub(1),
                _ => {}
            },
            Inner::Angle => {
                if c.is_ascii_uppercase() {
                    self.inner = Inner::Open(TagScan::default());
                } else if c == '/' {
                    self.inner = Inner::Close {
                        named: false,
                        spaced: false,
                    };
                } else {
                    self.inner = Inner::Plain;
                    return self.body(c);
                }
            }
            Inner::Open(scan) => {
                if let Some(self_closing) = scan.feed(c) {
                    self.depth += usize::from(!self_closing);
                    self.inner = Inner::Plain;
                }
            }
            Inner::Close { named, spaced } => {
                if c.is_ascii_alphanumeric() && !*spaced && (*named || c.is_ascii_uppercase()) {
                    *named = true;
                } else if c.is_whitespace() && *named {
                    *spaced = true;
                } else if c == '>' && *named {
                    self.inner = Inner::Plain;
                    if self.depth == 0 {
                        return true;
                    }
                    self.depth -= 1;
                } else {
                    self.inner = Inner::Plain;
                }
            }
        }
        false
    }
}

impl Machine {
    fn raw_step(&mut self, item: Item, q: &mut VecDeque<Item>) {
        match self.raw_mode.take() {
            Some(Raw::Braces(braces)) => self.braces_step(braces, item, q),
            Some(Raw::Declaration(declaration)) => self.declaration_step(declaration, item, q),
            Some(Raw::Capture(capture)) => self.capture_step(capture, item, q),
            None => {}
        }
    }

    /// The paragraph's current line ended inside verbatim text; the next
    /// character starts a line.
    fn resume_line_start(&mut self) {
        self.line = Line::start();
        self.eol = None;
    }

    fn braces_step(&mut self, mut b: Braces, item: Item, q: &mut VecDeque<Item>) {
        let c = item.c;
        if let Some(held) = &mut b.held {
            if c == '\n' && b.cr {
                b.cr = false;
                self.literal_char(item);
                self.raw_mode = Some(Raw::Braces(b));
                return;
            }
            if c == ' ' || c == '\t' {
                held.push(item);
                self.raw_mode = Some(Raw::Braces(b));
                return;
            }
            let mut held = std::mem::take(held);
            b.held = None;
            if eol(c) {
                // A blank line ends unclosed braces before it.
                held.push(item);
                self.resume_line_start();
                self.rerun(q, &held);
                return;
            }
            for item in held {
                self.literal_char(item);
            }
        }
        self.literal_char(item);
        b.cr = c == '\r';
        if eol(c) {
            b.escaped = false;
            b.held = Some(vec![]);
        } else if b.escaped {
            b.escaped = false;
        } else if c == '\\' {
            b.escaped = true;
        } else if c == '{' {
            b.depth += 1;
        } else if c == '}' {
            b.depth -= 1;
            if b.depth == 0 {
                return;
            }
        }
        self.raw_mode = Some(Raw::Braces(b));
    }

    fn declaration_step(&mut self, mut d: Declaration, item: Item, q: &mut VecDeque<Item>) {
        let c = item.c;
        if let Some((held, prefix)) = &mut d.held {
            if c == '\n' && d.cr && held.len() == 1 {
                d.cr = false;
                held.push(item);
                self.raw_mode = Some(Raw::Declaration(d));
                return;
            }
            if prefix.is_empty() && (c == ' ' || c == '\t') {
                held.push(item);
                self.raw_mode = Some(Raw::Declaration(d));
                return;
            }
            let continues = if prefix.is_empty() && eol(c) {
                false
            } else if prefix.is_empty() && d.scan.continues() {
                true
            } else {
                prefix.push(c);
                match prefix.as_str() {
                    "&" | "|" => {
                        held.push(item);
                        self.raw_mode = Some(Raw::Declaration(d));
                        return;
                    }
                    "&&" | "||" => true,
                    p => p.starts_with(['?', ':', '.']),
                }
            };
            let mut held = std::mem::take(held);
            d.held = None;
            if !continues {
                // The declaration ended at its line break; the held text is
                // ordinary input again.
                held.push(item);
                self.rerun(q, &held);
                return;
            }
            for item in held {
                self.literal_char(item);
                d.scan.feed(item.c);
            }
        }
        if eol(c) {
            d.held = Some((vec![item], String::new()));
            d.cr = c == '\r';
        } else {
            self.literal_char(item);
            d.scan.feed(c);
        }
        self.raw_mode = Some(Raw::Declaration(d));
    }

    fn capture_step(&mut self, mut cap: Box<Capture>, item: Item, q: &mut VecDeque<Item>) {
        let c = item.c;
        let crlf = c == '\n' && cap.cr;
        let line_end = eol(c) && !crlf;
        let blank = |c: char| c == ' ' || c == '\t';
        match cap.phase {
            CapturePhase::Suffix { self_closing } if line_end => {
                if self_closing {
                    let (end, block) = (cap.text.len(), cap.block_candidate);
                    return self.complete_capture(&cap, end, block, item, q);
                }
                cap.block = cap.block_candidate;
                cap.phase = CapturePhase::Body;
            }
            CapturePhase::Suffix { self_closing } if !blank(c) => {
                if self_closing {
                    let end = cap.opener_end;
                    return self.complete_capture(&cap, end, false, item, q);
                }
                cap.block = false;
                cap.phase = CapturePhase::Body;
            }
            CapturePhase::CloseLine if line_end => {
                let (end, block) = (cap.text.len(), cap.block_candidate);
                return self.complete_capture(&cap, end, block, item, q);
            }
            _ => {}
        }
        if line_end && !cap.first_line && cap.line_blank {
            // A blank line ends a failed opening and an inline tag's body,
            // but not a block tag's body.
            let ends = match cap.phase {
                CapturePhase::Opening => true,
                CapturePhase::Body => !cap.block,
                _ => false,
            };
            if ends {
                let (end, block) = (cap.line_start, cap.block_candidate);
                return self.complete_capture(&cap, end, block, item, q);
            }
            cap.braces = 0;
            cap.inner = Inner::Plain;
        }
        cap.text.push(c);
        if !item.fresh {
            cap.inline_bytes = cap.text.len();
        }
        if line_end || crlf {
            cap.line_start = cap.text.len();
            cap.line_blank = true;
            cap.first_line = false;
        } else if !blank(c) {
            cap.line_blank = false;
        }
        cap.cr = c == '\r';
        match cap.phase {
            CapturePhase::Opening => {
                if let Some(self_closing) = cap.opener.feed(c) {
                    cap.opener_end = cap.text.len();
                    // Only a tag at a block position waits for the rest of
                    // its line to decide between block and inline.
                    if cap.block_candidate {
                        cap.phase = CapturePhase::Suffix { self_closing };
                    } else if self_closing {
                        let end = cap.text.len();
                        return self.complete_capture_now(&cap, end, false, q);
                    } else {
                        cap.phase = CapturePhase::Body;
                    }
                } else if cap.text.len() > TAG_OPENING_BYTES {
                    return self.capture_cap(&cap);
                }
            }
            CapturePhase::Body => {
                if cap.body(c) {
                    if cap.block {
                        cap.phase = CapturePhase::CloseLine;
                    } else {
                        let (end, block) = (cap.text.len(), cap.block_candidate);
                        return self.complete_capture_now(&cap, end, block, q);
                    }
                }
            }
            CapturePhase::Suffix { .. } | CapturePhase::CloseLine => {}
        }
        if cap.text.len() > STREAM_CONTEXT_BYTES {
            return self.capture_cap(&cap);
        }
        self.raw_mode = Some(Raw::Capture(cap));
    }

    /// A held tag reached its cap: draw it as a provisional literal.
    fn capture_cap(&mut self, cap: &Capture) {
        self.work.reexamined += cap.text.len();
        let node = DisplayNode {
            span: Span::new(cap.base, cap.base + cap.text.len()),
            kind: Display::Literal(sanitize_str(&cap.text)),
            children: vec![],
        };
        let prev = cap.text.chars().last().unwrap_or('<');
        self.emit_node(node, prev);
        if eol(prev) {
            self.resume_line_start();
        }
    }

    fn complete_capture(
        &mut self,
        cap: &Capture,
        end: usize,
        block_candidate: bool,
        next: Item,
        q: &mut VecDeque<Item>,
    ) {
        q.push_front(next);
        self.complete_capture_now(cap, end, block_candidate, q);
    }

    /// Classifies the held tag `text[..end]` with the final parser, draws the
    /// result, and queues whatever the tag did not consume.
    fn complete_capture_now(
        &mut self,
        cap: &Capture,
        end: usize,
        block_candidate: bool,
        q: &mut VecDeque<Item>,
    ) {
        let text = &cap.text[..end];
        self.work.reexamined += end;
        let mut engine = Engine::new(text, end, Mode::Reply);
        let (node, stop) = match engine.tag(0, end, 1, block_candidate) {
            Ok((node, stop, _)) if stop > 0 && stop <= end => (node, stop),
            _ => (
                Node {
                    span: Span::new(0, end),
                    kind: Kind::Literal(text.to_owned()),
                    children: vec![],
                },
                end,
            ),
        };
        let display = convert(&node, text, &|o| cap.base + o);
        let prev = text[..stop].chars().last().unwrap_or('>');
        if matches!(node.kind, Kind::Tag { block: true, .. }) {
            // A block tag ends the paragraph it interrupts.
            if matches!(self.leaf, Leaf::Paragraph { .. }) {
                self.close_leaf();
            }
            self.out.push(display);
        } else {
            self.emit_node(display, prev);
        }
        if eol(prev) {
            self.resume_line_start();
        }
        let rest: Vec<Item> = cap.text[stop..]
            .char_indices()
            .map(|(i, c)| Item {
                at: cap.base + stop + i,
                c,
                fresh: stop + i >= cap.inline_bytes,
            })
            .collect();
        self.rerun(q, &rest);
    }

    /// Ends verbatim text left open when its block ends.
    fn finish_raw(&mut self) {
        match self.raw_mode.take() {
            Some(Raw::Capture(cap)) => self.capture_cap(&cap),
            Some(Raw::Declaration(Declaration {
                held: Some((held, _)),
                ..
            })) => {
                for item in held {
                    self.literal_char(item);
                }
            }
            _ => {}
        }
    }
}
