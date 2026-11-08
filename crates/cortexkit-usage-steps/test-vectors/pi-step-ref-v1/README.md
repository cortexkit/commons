# Pi step references, v1

These vectors pin `cortexkit_usage_steps::pi_step_ref`, the host record key
shared by live runner steps and durable Pi usage rows. The key does not
include a session path, so fork copies of the same call merge.

`cases.json` has two lists:

- `keys`: borrowed raw Pi fields in `entry`, and the exact expected key
  (`null` means no record). Omitted optional fields are absent, not zero;
  classification flags default to false. `provider` is already resolved.
  `timestamp` is the journal entry's own timestamp, not the message's.
- `chrono_timestamps`: timestamp spelling variants with a canonical spelling
  of the same instant in `canonical_timestamp`, or `null` when Chrono refuses
  the input. The test computes expected epoch milliseconds from the canonical
  spelling with `chrono::DateTime::parse_from_rfc3339(...).timestamp_millis()`;
  expected epoch values for these cases are not hand-transcribed. The
  `.1239` and longer fractions have canonical `.123` to pin truncation,
  not rounding. All other key components use the real sample in `keys`.

Rules are ordered: operations require a non-blank entry id and never fall
through; assistant response ids take precedence over message keys; otherwise
a non-blank entry id and a model are required. Ids stay verbatim after a
non-blank check. Components join with bare colons and counts use Rust's
`Option<u64>` Debug spelling. Message output excludes reasoning using checked
subtraction. Missing or invalid entry timestamps become `missing`.

Run `cargo test -p cortexkit-usage-steps --all-features --test pi`.
`SHA256SUMS` pins this README and `cases.json`, with paths relative to this
directory. The crate's contract registers this vector set.
