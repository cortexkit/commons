# rich-ui-syntax-v1 test vectors

This directory contains the reference-scanning sample sets used by
`cortexkit-rich-ui-syntax` tests and exposed by its `test-vectors` feature.

- `refs-scan-samples-v1.json` is the original 27-case sample set. The
  contract tests pin its SHA-256 (`1b042b7b…aaf`), so its bytes never change;
  new cases go in a new file.
- `refs-scan-samples-v1.1.json` adds 55 cases for failed tags, literal
  boundaries, links, Unicode and program mode.
- `refs-scan-samples-v1.2.json` adds 43 cases: streaming hazards (backticks
  around tags, links holding tags, table delimiter rows, blank lines inside
  tags, escaped markers, line endings, a reference longer than the streaming
  threshold) and the `E_UNTERMINATED_STRING`, `E_BAD_ESCAPE`,
  `E_RESERVED_NAME` and diagnostic-subject cases. Each of its cases also has a
  `diagnostics` list: the exact diagnostics `parse` returns, as `code`,
  `start`, `end` and, where the diagnostic names one, `subject`. Every case in
  every file must give the same final result under any chunking of a
  `StreamParser`.

`SHA256SUMS` pins every file in this directory except itself. The public
`vectors::FILES` and `vectors::SHA256SUMS` constants expose the same bytes to
consumers without requiring a repository checkout.
