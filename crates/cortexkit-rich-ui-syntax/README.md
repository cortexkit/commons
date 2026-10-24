# cortexkit-rich-ui-syntax

A standalone Rust front end for **rich UI language v1**, with the reference
scanner used before storing tool-output snapshots. It has no host, catalog,
network, filesystem-resolution, or rendering dependency.

## Parsing and scanning

`parse(source, Mode::Reply | Mode::Program)` returns a spanned syntax tree and
bounded diagnostics. Spans are original UTF-8 byte offsets, start inclusive and
end exclusive, including any leading BOM. NUL is replaced in text/string values.
`parse_bytes` repairs invalid UTF-8 first; its offsets describe the repaired text.

The tree includes paragraphs, ATX headings, thematic breaks, fences, quotes,
lists/task markers, tables, emphasis/strong/strikethrough, code spans, links and
image-as-link syntax. Indented code, setext headings, raw HTML, reference-style
links, footnotes, math and front matter are not additional syntax. Tags have
named props, inline or block children, and source spans. Program expressions
include exact decimal literals, strings/escapes, references, literal-only
`Artifact`/`File` handles, names/state, calls, lists/ordered records, grouping,
postfix lookup/indexing, unary/binary operators and conditionals. Program
interpolations and declarations use the same expression parser.

This is a syntax front end, **not** a component/type checker or evaluator.
Addresses in link nodes are inert syntax, not authorization to open or fetch a
URL; a host must validate/canonicalize addresses and enforce its network policy.
Evaluation limits on values, actions, sites, allocation, fuel and rendered trees
belong to that later layer, not to source parsing.

```rust
use cortexkit_rich_ui_syntax::{check_token_slot, scan_references, Mode, DIALECT};

assert_eq!(DIALECT, "rich-ui/1");
let scan = scan_references("See $$tc(7). `$$tc(8)`", Mode::Reply);
assert_eq!(scan.references.len(), 1);
assert_eq!(scan.references[0].n, "7");
assert_eq!((scan.references[0].span.start, scan.references[0].span.end), (4, 11));
assert!(check_token_slot(" $$tc(7)").is_none());
```

`scan_references` calls `parse` and walks **only reference nodes in that tree**.
It returns occurrences in source order, preserving repeats, with no regex scan,
substitution, number resolution, or parsing of data strings. `N` is canonical
decimal stored as a string, so it cannot silently overflow a machine integer.

### Reference rule

`$$tc(N)` has no internal whitespace or leading zeros; `0` is legal. It counts
only where the syntax parser produces a reference node. In particular:

- Markdown prose, tag children, expression props and program expressions count.
- Fences, code spans, escaped markers and expression/prop string literals do not.
- A reply tag with a brace value outside the reply-value grammar becomes **one
  opaque literal span**, including its children. No reference inside survives.
  Reply values are literals, references, literal handles, lists/records of reply
  values, and `Parse(reference, string[, literal-record])`; no arbitrary calls,
  operators, names, state, grouping, or nested components are admitted as props.
- Crossed/missing closes invalidate the enclosing tag span. A child's forbidden
  prop is local: it does not invalidate a well-formed parent or its references.
- **Unknown component names are a catalog matter.** Valid reply values on an
  unknown component still count here. Runners must not substitute a catalog
  filter for this scanner.
- There is no recognition by decoding entities and then scanning the result.

## Runner integration

Only scan the source fields declared as reference slots by the tool provider;
never scan arbitrary arguments, tool output, snapshot data, or nested strings.

| Slot kind | Operation |
| --- | --- |
| `token` | `check_token_slot(value)`: the entire string must be exactly one token, with no trimming. `None` means `not_a_reference`. |
| `source:rich-ui/1` | `scan_references(value, Mode::Program)` |
| assistant reply source | `scan_references(value, Mode::Reply)` |

For every returned occurrence, bind the number to the immutable request that
produced the source, not to a later numbering. Store dialect and original byte
span with the record. This crate never looks up or copies tool output.

Occurrences after the first 64 have `excess = true`: record them as unavailable
(`too_many_refs`), **never materialize snapshots for them**. The caller must also
apply the 64-occurrence budget across multiple source/token slots in one call or
message, and enforce its separate snapshot-byte budget. `fallback` means the
whole parsed prefix became bounded literal text and has no reference nodes;
`shortened` means the unparsed tail must not be scanned by another implementation.
Program diagnostics require fallback before execution; the syntax tree is not an
execution approval.

## Bounds and streaming

Programs over 64 KiB are refused before parsing. Replies stop at a UTF-8 boundary
within 256 KiB. Parsing budgets are 8,192 AST allocations, 2,048 blocks, 65,536
text scalars, 64 reply tags, 256 declarations, expression nesting 64, tag nesting
32, Markdown quote/list nesting 16, emphasis nesting 64, and 50 listed diagnostics
(with a separate suppressed count). Reference and handle record budgets are 64
each; excess occurrences remain syntax rather than silently disappearing.

The size limits measure raw input: `parse_bytes` cuts at the byte limit before
UTF-8 repair, so repair (which can triple an invalid byte) never makes a source
within the limit too large, and its repaired text, which offsets describe, may
be longer than the limit: at most three times it, since each invalid byte
grows to at most three. A caller that budgets by text size should charge the
repaired text's size, not the input's.

## Streaming replies

`StreamParser::new(mode)` accepts a source in arbitrary byte chunks:

```rust
use cortexkit_rich_ui_syntax::{parse, Display, Mode, StreamParser};

let source = "Look at `<Chart/>` and $$tc(3).";
let mut stream = StreamParser::new(Mode::Reply);
let mut drawn = None;
for chunk in source.as_bytes().chunks(5) {
    if let Some(revision) = stream.feed(chunk) {
        drawn = Some(revision); // replace whatever was drawn before
    }
    let work = stream.work(); // charge work.scanned + work.reexamined
    assert!(work.scanned <= chunk.len());
}
let last = drawn.unwrap();
assert!(matches!(last.nodes[0].children[1].kind, Display::Code(_)));
assert_eq!(stream.finish(), parse(source, Mode::Reply));
```

Its rules:

- **Equivalence.** `finish()` returns exactly `parse_bytes` of everything
  received, for every chunking: tree, references, diagnostics and spans. The
  parser keeps the received bytes (up to the limit plus three bytes) and parses
  them once, at `finish`. The final result never comes from provisional state.
- **Revisions.** In reply mode, `feed` returns a `Revision` whenever the drawable
  picture of the received prefix differs from the last one it returned, and
  `None` otherwise; `revision()` gives the current one. Programs are not drawn:
  in program mode `feed` always returns `None`.
- **Stable per prefix.** A revision is a pure function of the received prefix.
  The same bytes give an equal revision however they were chunked.
- **Not monotone.** A later byte may reclassify earlier provisional text: a
  closing backtick or emphasis delimiter, a link destination, a table delimiter
  row under a drawn line, or the rest of a line after a tag. A caller drawing
  revisions replaces the drawn revision with each new one.
- **Inert.** A revision carries display structure only. References are
  `Display::PendingReference` placeholders without their number, links and
  images carry their label but no destination, and tags carry their name but
  no props. Nothing in a revision names data to fetch or an action to take, and
  a revision is never a storage-time reference decision.
- **Linear work.** A provisional machine consumes each byte once into bounded
  block and inline state; it never reparses the received prefix. `work()`
  reports, for the last `feed`, the bytes `scanned` and the held bytes
  `reexamined`. Fuel accounting stays with the caller, which reserves part of a
  reply's budget for the final parse and charges provisional work separately.
- **Held context and caps.** Text whose classification is still open is held,
  not drawn: a code span until its closing backticks, a link until its
  destination closes, a tag until it is complete (a complete tag inside an
  unresolved code span or link is not complete for display), a possible
  `$$tc(N)`, an escape, and a line start that may still become a block marker.
  A paragraph's first line containing `|` is drawn as paragraph text, and a
  copy is kept until the next line arrives; when that line is a matching
  delimiter row (which is held, not drawn), the first line is redrawn as the
  table header.
  Each held region has its own cap, all within the 256 KiB reply limit:

  | Held region | Cap |
  | --- | --- |
  | code span, from its opening backticks | `STREAM_CONTEXT_BYTES`: 32 KiB, in bytes |
  | link or image, from `[` or `![` (labels also nest at most 4 brackets deep) | 32 KiB, in bytes |
  | tag opening, from `<` to its `>` | `TAG_OPENING_BYTES`: 4 KiB, in bytes |
  | whole tag, from `<` to the end of its closing tag | 32 KiB, in bytes |
  | possible reference, from `$` | `PENDING_REFERENCE_CHARS`: 24 characters |
  | thematic-break candidate line, fence info string | 32 KiB (ASCII marks, so bytes and characters agree; the info string counts bytes) |
  | kept copy of a possible table header line | 32 KiB, in bytes |

  At a cap the held text is drawn as a provisional `Display::Literal` with
  control characters replaced, and the machine continues from its bounded
  state after it; a table header copy over its cap is simply dropped, so that
  table is drawn as a paragraph until the final parse. The caps only shape the
  provisional picture: the final parse has none of them, and its own limits
  are the same for every chunking.
- **Block ends.** A blank line, or the end of a block, resolves what it holds.
  An unclosed code span or link has its opening marker drawn as text and the
  rest examined once more; whatever that pass would hold again is drawn
  without a further pass (as text, or a tag as a `Display::Literal`). A held
  escape, `!`, `<`, delimiter run or possible reference is drawn as text. A
  blank line inside a tag's opening or an inline tag's body ends the tag, which
  the final parser then classifies (usually as literal text); a block tag's
  body continues. Reply declarations and prose braces are never held: they are
  drawn as literals as they arrive and end where the final parser ends them
  (the declaration's last line, the matching `}` or a blank line).

The provisional machine draws paragraphs, ATX headings, thematic breaks, fenced
code, quotes, lists (with an ordered list's start number), GFM tables,
emphasis, code spans, links and images, hard and soft breaks, reply prose
braces and declarations as literals, complete tags and reference placeholders.
Task list markers are drawn as text (`[ ] task`), except inside a completed
tag's body, which the final parser classifies. Completed tags and links are classified by running
the final parser over just their held text. Its picture is a preview: where it
differs from the final tree, the final tree is authoritative.

All display text replaces control characters other than line feed and tab with
U+FFFD; carriage returns become line feeds. That is a floor, not a guarantee
that the text is safe to display: a caller drawing it still applies its own
display sanitizing. Display spans are byte offsets in
the received text after the UTF-8 repair `parse_bytes` applies: each maximal
invalid byte subsequence becomes one U+FFFD. A possibly incomplete scalar at
the end of the received bytes is held until the next byte decides it.

## Choices

These choices resolve underspecified boundaries conservatively; they are part of
this scanner's pinned behavior, not catalog rules:

1. A failed opening candidate is **not a tag**. Its prefix is opaque through the
   next `>` outside strings/braces, the first blank line, or end of source,
   whichever comes first. Parsing resumes as Markdown after that boundary. This
   prevents a blank line in an unfinished opener from consuming later paragraphs,
   without copying output mentioned in the failed prefix. A valid but forbidden
   reply tag instead makes its **whole paired/self-closing span** literal.
2. Reply declarations (including continuation lines) and brace-delimited prose
   are opaque literal text, so references in program examples are not copied.
   Unclosed prose braces stop at
   a blank line/end. Ordinary quotation marks in prose are not expression strings;
   use code spans/fences for quoted programs. This also means braces in a tag
   written as text inside a link label stay literal in reply mode.
3. Link labels/alt text are inline content, but never components. Destinations,
   titles and HTTP(S) autolink addresses are not visible reference nodes. Markdown
   body references count in program mode too, not just declaration expressions.
4. Parenthesized reply values are rejected, even around a reference: grouping is
   not in the reply-value grammar. `Parse` options are literal records; their
   literal lists contain scalars only, as specified by that narrower grammar.
5. There is no maximum numeric value for reference `N`. The pending-reference
   streaming threshold is not a final token-length limit. Exact-token slots do
   have the same 256 KiB hard input ceiling as reply sources, to bound allocation.
6. Work and stack safety apply to all modes: the AST/block budgets also protect
   programs; mixed expression/tag parsing shares 65 active expression frames;
   recursive inline contexts cap at 64. A deterministic work allowance of
   `max(1024, 128 * bounded_source_bytes)` prevents repeated failed lookaheads
   from making adversarial parsing superlinear. Exhaustion selects literal
   fallback or a literal failed construct, never extra references. These extra
   safety bounds can reject deeply mixed constructs before their independent
   expression/tag limits would be reached.
7. Diagnostics carry a `subject` where the failure has a name: reserved and
   duplicate declarations, duplicate props and record keys (`children` when a
   tag sets that prop and also has children), unclosed and mismatched tags
   (the opening tag's name), forbidden reply props, non-literal state
   initializers and handles, the tag that passes the reply tag limit
   (`E_LIMIT_TAGS`), and a tag nested too deep (`E_NESTING`). Expression
   nesting, parse-work, size and count limits have no subject. String diagnostics span the
   string from its opening quote to where it broke (`E_UNTERMINATED_STRING`) or
   the escape as far as it was read (`E_BAD_ESCAPE`); other diagnostics keep
   the span of their construct. A string that reaches the end of its construct
   without a closing quote is unterminated, like one that reaches a line break.

The original 27-case sample file is byte-identical and SHA-256 pinned in tests.
`refs-scan-samples-v1.1.json` adds failed-tag, literal-boundary, link, Unicode and
program-mode cases. `refs-scan-samples-v1.2.json` adds streaming hazards and the
string, reserved-name and subject diagnostics; its cases may also pin the exact
`diagnostics` list. Tests load all three from `CARGO_MANIFEST_DIR`; no checkout
layout or platform-specific service is required. Generated boundary/adversarial
tests cover parser budgets and randomized/truncated UTF-8 source, and every
sample is checked under every split position, one-byte chunks and adversarial
chunkings.

The sample files ship in `test-vectors/rich-ui-syntax-v1/`. With the
`test-vectors` feature, the public `vectors` module exposes their contents and
`SHA256SUMS`, so callers can test against the exact same bytes without a
repository checkout.

## Development

```text
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

A reference must parse identically in every build that scans or renders it.
Patch releases of `pulldown-cmark` have changed output on edge cases, so its
version is pinned exactly. The pin moves only in a minor release of this crate,
with every sample rerun against the new parser version.

The `pulldown-cmark` dependency supplies CommonMark delimiter behavior,
GFM tables/task/strike syntax, entity decoding and source offsets. A byte-length
preserving shielding layer keeps rich UI syntax and failed literals opaque to
Markdown. `serde_json` (sample loading) and `sha2` (immutable fixture digest) are
test-only dependencies. The workspace lockfile pins the exact dependency graph;
the crate inherits common package metadata from the workspace.
