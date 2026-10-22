//! A bounded syntax front end for rich UI language v1.
//!
//! Reference discovery walks the returned syntax tree; it never scans strings
//! from data, Markdown destinations, or failed tags a second time.
mod expression;
mod markdown;
mod parser;
#[cfg(feature = "test-vectors")]
pub mod vectors;

pub use parser::{parse, parse_bytes, StreamParser};

/// Identifies the rich UI source dialect accepted by this parser.
pub const DIALECT: &str = "rich-ui/1";
pub const PROGRAM_BYTES: usize = 64 * 1024;
pub const REPLY_BYTES: usize = 256 * 1024;
pub const AST_NODES: usize = 8192;
pub const BLOCKS: usize = 2048;
pub const OUTPUT_SCALARS: usize = 65_536;
pub const REFERENCES: usize = 64;
pub const HANDLES: usize = 64;
pub const DECLARATIONS: usize = 256;
pub const EXPRESSION_DEPTH: usize = 64;
pub const TAG_DEPTH: usize = 32;
pub const MARKDOWN_DEPTH: usize = 16;
pub const EMPHASIS_DEPTH: usize = 64;
pub const DIAGNOSTICS: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Reply,
    Program,
}

/// Original UTF-8 byte offsets, including a leading BOM if one was supplied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub span: Span,
    pub kind: Kind,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Markdown(Markdown),
    Text(String),
    /// Opaque source, not reparsed as Markdown or expressions.
    Literal(String),
    Reference {
        n: String,
    },
    Handle {
        kind: String,
        path: String,
    },
    Tag {
        name: String,
        block: bool,
        props: Vec<Prop>,
    },
    Interpolation {
        block: bool,
    },
    Declaration {
        name: String,
        state: bool,
    },
    Expression(Expression),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prop {
    pub name: String,
    pub span: Span,
    pub value: Node,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expression {
    Number(String),
    String(String),
    Boolean(bool),
    Null,
    Name(String),
    State(String),
    Group,
    List,
    Record(Vec<String>),
    Call(String),
    Unary(String),
    Binary(String),
    Conditional,
    Member(String),
    Index,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Markdown {
    Paragraph,
    Heading(u8),
    ThematicBreak,
    CodeBlock {
        language: String,
        text: String,
    },
    Code(String),
    Quote,
    List {
        start: Option<u64>,
    },
    Item,
    Task(bool),
    Table,
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    /// An inert syntax address, not a validated or authorized navigation target.
    Link {
        destination: String,
        title: String,
        image: bool,
    },
    SoftBreak,
    HardBreak,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub mode: Mode,
    pub nodes: Vec<Node>,
    pub diagnostics: Vec<Diagnostic>,
    pub suppressed_diagnostics: usize,
    pub parsed_bytes: usize,
    pub shortened: bool,
    /// A whole-source limit selected bounded literal fallback; no refs survive.
    pub fallback: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// Canonical decimal, not a machine integer: the grammar sets no integer cap.
    pub n: String,
    pub span: Span,
    /// True for every occurrence after the first 64 in source order. A caller
    /// resolving references copies a tool's output only for occurrences where
    /// this is false.
    pub excess: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanResult {
    pub references: Vec<Reference>,
    pub diagnostics: Vec<Diagnostic>,
    pub suppressed_diagnostics: usize,
    pub parsed_bytes: usize,
    pub shortened: bool,
    pub fallback: bool,
}

impl Document {
    /// Lists all reference nodes, including repeated and over-budget occurrences.
    pub fn references(&self) -> Vec<Reference> {
        fn visit(node: &Node, refs: &mut Vec<Reference>) {
            if let Kind::Reference { n } = &node.kind {
                refs.push(Reference {
                    n: n.clone(),
                    span: node.span,
                    excess: false,
                });
            }
            if let Kind::Tag { props, .. } = &node.kind {
                for prop in props {
                    visit(&prop.value, refs);
                }
            }
            for child in &node.children {
                visit(child, refs);
            }
        }
        let mut refs = Vec::new();
        for node in &self.nodes {
            visit(node, &mut refs);
        }
        refs.sort_by_key(|r| r.span.start);
        for (i, reference) in refs.iter_mut().enumerate() {
            reference.excess = i >= REFERENCES;
        }
        refs
    }
}

pub fn scan_references(source: &str, mode: Mode) -> ScanResult {
    let doc = parse(source, mode);
    ScanResult {
        references: doc.references(),
        diagnostics: doc.diagnostics,
        suppressed_diagnostics: doc.suppressed_diagnostics,
        parsed_bytes: doc.parsed_bytes,
        shortened: doc.shortened,
        fallback: doc.fallback,
    }
}

/// Checks a `token` argument slot without trimming, unescaping, or substring search.
pub fn check_token_slot(value: &str) -> Option<Reference> {
    if value.len() > REPLY_BYTES {
        return None;
    }
    let end = reference_end(value, 0)?;
    (end == value.len()).then(|| Reference {
        n: value[5..end - 1].to_owned(),
        span: Span::new(0, end),
        excess: false,
    })
}

pub(crate) fn reference_end(source: &str, start: usize) -> Option<usize> {
    let tail = source.get(start..)?;
    let digits = tail.strip_prefix("$$tc(")?;
    let count = digits.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0
        || (count > 1 && digits.starts_with('0'))
        || digits.as_bytes().get(count) != Some(&b')')
    {
        return None;
    }
    Some(start + 6 + count)
}
