# Changelog

## 0.3.0

- Move the exact `pulldown-cmark` pin from `=0.13.0` to `=0.13.5`.
- All samples retain the same results under `0.13.5`.

## 0.2.0

What a caller gains:

- **Provisional revisions while a reply streams in.** `StreamParser::feed`
  now returns `Option<Revision>`: a display-only picture of the received
  prefix, returned whenever it changes. A revision depends only on the bytes
  received, never on how they were chunked. It is inert: references are
  number-less placeholders, links carry no destination and tags no props.
  Text whose meaning is still open (a code span, a link, a tag, a possible
  reference, a line that may still be a table header or block marker) is
  held, within fixed caps, and not drawn until it resolves.
- **Work counts per feed.** `StreamParser::work` reports the bytes the last
  `feed` scanned and the held bytes it examined again, so the caller can
  charge provisional work separately from the final parse. Each byte is
  scanned once; the received prefix is never reparsed.
- `StreamParser::finish` still equals `parse_bytes` of the whole input for
  every chunking.
- **Sharper diagnostics.** A string with a raw line break or no closing
  quote reports `E_UNTERMINATED_STRING`, and an unknown escape or a bad
  `\u{…}` reports `E_BAD_ESCAPE`, each spanning the string or escape, instead
  of `E_SYNTAX` (or `E_DECLARATION_INCOMPLETE` inside a declaration). A
  declaration named `true`, `false`, `null`, `export`, `let` or `const`
  reports `E_RESERVED_NAME`.
- **Diagnostics name their subject.** The new `Diagnostic::subject` holds
  the declaration a reserved or duplicate name refused, the repeated prop or
  record key, the unclosed or mismatched tag, the prop a reply may not carry.
- The 64 KiB program limit measures raw input bytes. `parse_bytes` no longer
  refuses a program under 64 KiB because UTF-8 repair made it longer.
  `W_ENCODING` is still reported.

What breaks:

- `Diagnostic::code` is now the `DiagnosticCode` enum instead of a
  `&'static str`. `code.as_str()` (or `Display`) gives the same text as
  before, such as `"E_SYNTAX"`. `Diagnostic` has the new `subject` field.
- `StreamParser::push(&mut self, &[u8])` is replaced by
  `StreamParser::feed(&mut self, &[u8]) -> Option<Revision>`.
- `Mode`, `Kind`, `Expression`, `Markdown` and `DiagnosticCode` are
  `#[non_exhaustive]`: a `match` on them outside this crate needs a wildcard
  arm. Later releases may add variants without a major version.
- In `parse_bytes`, offsets past invalid input describe the repaired text, as
  before, but a reply's repaired text may now be longer than 256 KiB, since
  the limit applies before repair.

## 0.1.1

- Documentation only: `StreamParser` now states that in program mode `finish`
  refuses a source cut off at the size limit (`E_SOURCE_TOO_LARGE`, no nodes),
  as `parse_bytes` does, so a program never runs from a cut-off prefix.

## 0.1.0

- Publish the rich UI v1 syntax front end, including `scan_references` for
  reply and program sources and `check_token_slot` for exact token slots.
- Ship the original and extended reference-scanning sample files behind the
  `test-vectors` feature, with SHA-256 checksums for their packaged bytes.
