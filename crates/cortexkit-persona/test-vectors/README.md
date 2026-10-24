# persona/v1 golden vectors

Each directory under `persona-v1/cases/` is one render:

- `case.json`: the module id, role, project key, registered extension keys
  (each registered as an untyped JSON value with the `replace` rule), the
  tier ladder (name, trust class, root relative to `persona-v1/`, roles path),
  the approval set, the context file and the supplied fragment files;
- `expected.txt`: the exact rendered bytes;
- `expected-report.json`: the full resolution report. A tier's `root` is
  `test-vectors/persona-v1/<root>`, because the tests run from the crate root,
  and it is joined with `/` rather than the platform separator so the expected
  report matches on every OS.

The fixtures the cases point at:

- `scribe/`: a three-tier role (embedded-style base, user, repository project)
  with bundle fragments, a fragment override, a supplied qualified fragment
  `mc/memory-profile@1`, block overrides, `disabled_blocks` from two layers and
  a registered `limits` extension. `scribe-approved` applies the project layer;
  `scribe-changed` approves an older digest, and `scribe-unapproved` approves
  it for a different project key, so both skip it whole.
- `echo-lf/` and `echo-crlf/`: the same one-tier role with LF and CRLF line
  endings, which must give identical bytes and digests.

`SHA256SUMS` pins every file here (except itself) byte for byte.
`crates/cortexkit-persona/.gitattributes` marks `test-vectors/**` as `-text`,
so git never converts line endings here and the CRLF fixture keeps its CRLF.

## Regenerating

`generate.py` writes the echo fixtures, every `cases/*` directory and
`SHA256SUMS`; the scribe fixtures are hand-written and only read. It is a
second implementation of SPEC.md's digest and report rules, written in Python
without the Rust code, so the golden test checks the crate against it rather
than against the crate's own earlier output. It is not a template engine: the
expected rendered text of each case is written out in the script. Run it with
Python 3.8 or later, then the tests:

```sh
python3 crates/cortexkit-persona/test-vectors/generate.py
cargo test -p cortexkit-persona --test golden --test vector_sums
```

On an unchanged checkout it rewrites every file byte for byte, so
`git status` stays clean. Changing any expected byte, digest or report is a
breaking change of the crate (SPEC.md, "Bytes and digests").
