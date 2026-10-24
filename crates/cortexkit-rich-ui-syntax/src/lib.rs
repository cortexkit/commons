//! A bounded syntax front end for rich UI language v1.
//!
//! Reference discovery walks the returned syntax tree; it never scans strings
//! from data, Markdown destinations, or failed tags a second time.
mod expression;
mod markdown;
mod parser;
mod stream;
#[cfg(feature = "test-vectors")]
pub mod vectors;

pub use parser::{parse, parse_bytes};
pub use stream::{
    Display, DisplayNode, Revision, StreamParser, Work, PENDING_REFERENCE_CHARS,
    STREAM_CONTEXT_BYTES, TAG_OPENING_BYTES,
};

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

/// Which grammar a source follows.
///
/// Later releases may add modes, so a `match` outside this crate needs a
/// wildcard arm:
///
/// ```
/// use cortexkit_rich_ui_syntax::Mode;
/// fn name(mode: Mode) -> &'static str {
///     match mode {
///         Mode::Reply => "reply",
///         Mode::Program => "program",
///         _ => "other",
///     }
/// }
/// assert_eq!(name(Mode::Reply), "reply");
/// ```
///
/// ```compile_fail,E0004
/// use cortexkit_rich_ui_syntax::Mode;
/// fn name(mode: Mode) -> &'static str {
///     match mode {
///         Mode::Reply => "reply",
///         Mode::Program => "program",
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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

/// A diagnostic's stable code. New codes may be added in minor releases, so
/// a `match` needs a wildcard arm; [`DiagnosticCode::as_str`] gives the code
/// as it is written in documentation and logs, such as `E_SYNTAX`.
///
/// Naming every code without a wildcard arm does not compile outside this
/// crate:
///
/// ```compile_fail,E0004
/// use cortexkit_rich_ui_syntax::DiagnosticCode as C;
/// fn rank(code: C) -> u8 {
///     match code {
///         C::Syntax => 0,
///         C::UnterminatedString => 1,
///         C::BadEscape => 2,
///         C::ReservedName => 3,
///         C::DuplicateName => 4,
///         C::DeclarationIncomplete => 5,
///         C::LiteralRequired => 6,
///         C::LimitDeclarations => 7,
///         C::DuplicateProp => 8,
///         C::DuplicateKey => 9,
///         C::MismatchedClose => 10,
///         C::UnclosedTag => 11,
///         C::ReplyForbidden => 12,
///         C::LimitTags => 13,
///         C::Nesting => 14,
///         C::NonAssoc => 15,
///         C::MixedCoalesce => 16,
///         C::NumberLiteral => 17,
///         C::ParseWork => 18,
///         C::LimitAst => 19,
///         C::LimitBlocks => 20,
///         C::LimitText => 21,
///         C::SourceTooLarge => 22,
///         C::ReferenceLimit => 23,
///         C::Encoding => 24,
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// `E_SYNTAX`: source that fits no rule of the grammar.
    Syntax,
    /// `E_UNTERMINATED_STRING`: a string literal that reaches a raw line
    /// break, or the end of its construct, before its closing quote.
    UnterminatedString,
    /// `E_BAD_ESCAPE`: an unknown escape in a string literal, or a `\u{…}`
    /// escape that is malformed or names no Unicode scalar value.
    BadEscape,
    /// `E_RESERVED_NAME`: a declaration whose name is reserved.
    ReservedName,
    /// `E_DUPLICATE_NAME`: a second declaration of the same name.
    DuplicateName,
    /// `E_DECLARATION_INCOMPLETE`: a declaration whose value expression is
    /// missing or cut short.
    DeclarationIncomplete,
    /// `E_LITERAL_REQUIRED`: a state initializer or handle path that is not
    /// a literal.
    LiteralRequired,
    /// `E_LIMIT_DECLARATIONS`: more than [`DECLARATIONS`] declarations.
    LimitDeclarations,
    /// `E_DUPLICATE_PROP`: a tag that sets the same prop twice.
    DuplicateProp,
    /// `E_DUPLICATE_KEY`: a record literal that repeats a key.
    DuplicateKey,
    /// `E_MISMATCHED_CLOSE`: a closing tag that does not close the open tag.
    MismatchedClose,
    /// `E_UNCLOSED_TAG`: a paired tag without its closing tag.
    UnclosedTag,
    /// `E_REPLY_FORBIDDEN`: a reply tag prop outside the reply-value grammar.
    ReplyForbidden,
    /// `E_LIMIT_TAGS`: more than 64 tags in a reply.
    LimitTags,
    /// `E_NESTING`: a nesting limit was exceeded.
    Nesting,
    /// `E_NONASSOC`: chained comparison or equality operators.
    NonAssoc,
    /// `E_MIXED_COALESCE`: `??` mixed with `&&` or `||` without grouping.
    MixedCoalesce,
    /// `E_NUMBER_LITERAL`: a number outside the exact decimal grammar.
    NumberLiteral,
    /// `E_PARSE_WORK`: the deterministic parsing work allowance ran out.
    ParseWork,
    /// `E_LIMIT_AST`: more than [`AST_NODES`] syntax nodes.
    LimitAst,
    /// `E_LIMIT_BLOCKS`: more than [`BLOCKS`] Markdown blocks.
    LimitBlocks,
    /// `E_LIMIT_TEXT`: more than [`OUTPUT_SCALARS`] text scalars in a reply.
    LimitText,
    /// `E_SOURCE_TOO_LARGE`: a program longer than [`PROGRAM_BYTES`].
    SourceTooLarge,
    /// `E_REFERENCE_LIMIT`: more than [`REFERENCES`] references or
    /// [`HANDLES`] handles.
    ReferenceLimit,
    /// `W_ENCODING`: invalid UTF-8 was replaced before parsing.
    Encoding,
}

impl DiagnosticCode {
    /// The code as written in documentation and logs, such as `E_SYNTAX`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Syntax => "E_SYNTAX",
            Self::UnterminatedString => "E_UNTERMINATED_STRING",
            Self::BadEscape => "E_BAD_ESCAPE",
            Self::ReservedName => "E_RESERVED_NAME",
            Self::DuplicateName => "E_DUPLICATE_NAME",
            Self::DeclarationIncomplete => "E_DECLARATION_INCOMPLETE",
            Self::LiteralRequired => "E_LITERAL_REQUIRED",
            Self::LimitDeclarations => "E_LIMIT_DECLARATIONS",
            Self::DuplicateProp => "E_DUPLICATE_PROP",
            Self::DuplicateKey => "E_DUPLICATE_KEY",
            Self::MismatchedClose => "E_MISMATCHED_CLOSE",
            Self::UnclosedTag => "E_UNCLOSED_TAG",
            Self::ReplyForbidden => "E_REPLY_FORBIDDEN",
            Self::LimitTags => "E_LIMIT_TAGS",
            Self::Nesting => "E_NESTING",
            Self::NonAssoc => "E_NONASSOC",
            Self::MixedCoalesce => "E_MIXED_COALESCE",
            Self::NumberLiteral => "E_NUMBER_LITERAL",
            Self::ParseWork => "E_PARSE_WORK",
            Self::LimitAst => "E_LIMIT_AST",
            Self::LimitBlocks => "E_LIMIT_BLOCKS",
            Self::LimitText => "E_LIMIT_TEXT",
            Self::SourceTooLarge => "E_SOURCE_TOO_LARGE",
            Self::ReferenceLimit => "E_REFERENCE_LIMIT",
            Self::Encoding => "W_ENCODING",
        }
    }
}

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub span: Span,
    /// The name the diagnostic is about, where it has one: the declaration a
    /// reserved or duplicate name refused, the repeated prop or record key
    /// (`children` when a tag both sets that prop and has children), the tag
    /// left unclosed or wrongly closed, the prop a reply may not carry, the
    /// state or handle that needs a literal, the tag past the reply's tag
    /// limit, and the tag nested too deep. Expression nesting, parse-work,
    /// size and count limits name nothing.
    pub subject: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub span: Span,
    pub kind: Kind,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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
