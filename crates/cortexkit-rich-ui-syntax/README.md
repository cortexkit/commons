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

`StreamParser` retains a bounded source prefix plus at most three UTF-8 lookahead
bytes, accepts arbitrary byte chunks, and parses **once at finalization**. It does
not expose provisional display frames or provisional reference lists. All split
positions and single-byte chunking have the same final result as a whole-source
parse, including truncation through a multibyte scalar. A UI needing provisional
frames must not treat them as storage-time reference decisions.

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

The original 27-case sample file is byte-identical and SHA-256 pinned in tests.
`refs-scan-samples-v1.1.json` adds failed-tag, literal-boundary, link, Unicode and
program-mode cases. Tests load both from `CARGO_MANIFEST_DIR`; no checkout layout
or platform-specific service is required. Generated boundary/adversarial tests
cover parser budgets and randomized/truncated UTF-8 source.

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

The pinned `pulldown-cmark` dependency supplies CommonMark delimiter behavior,
GFM tables/task/strike syntax, entity decoding and source offsets. A byte-length
preserving shielding layer keeps rich UI syntax and failed literals opaque to
Markdown. `serde_json` (sample loading) and `sha2` (immutable fixture digest) are
test-only dependencies. The workspace lockfile pins the exact dependency graph;
the crate inherits common package metadata from the workspace.
