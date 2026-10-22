# rich-ui-syntax-v1 test vectors

This directory contains the reference-scanning sample sets used by
`cortexkit-rich-ui-syntax` tests and exposed by its `test-vectors` feature.

- `refs-scan-samples-v1.json` is the original 27-case sample set. The
  contract tests pin its SHA-256 (`1b042b7b…aaf`), so its bytes never change;
  new cases go in a new file.
- `refs-scan-samples-v1.1.json` adds 55 cases for failed tags, literal
  boundaries, links, Unicode and program mode.

`SHA256SUMS` pins every file in this directory except itself. The public
`vectors::FILES` and `vectors::SHA256SUMS` constants expose the same bytes to
consumers without requiring a repository checkout.
