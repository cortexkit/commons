# `tool-provider/v1` test vectors

The shared bytes every `tool-provider/v1` implementation and consumer checks.
The role document is `crates/cortexkit-role-tool-provider/CONTRACT.md`.

| File | What it pins | Checked by |
|---|---|---|
| `call-key.json` | the `call_key` bound (1 to 256 bytes, each 0x21–0x7E, no space): valid keys, and invalid keys with the validator's error | the wire crate's validator test; the conformance runner sends every key to a live provider, as a call key and as a withdraw target |
| `schema-digest.json` | the structural `schema_digest`: exact digests, schema pairs that differ only in description text or key order (same digest), and pairs that differ in structure (different digest) | the wire crate's digest tests; the runner recomputes every live tool's digest |
| `schema-pin.json` | the `tp1` schema-pin encoding (`tool`, `schema_digest`, `semantics`): canonical pins with their parts, and refused strings | the wire crate's pin test; the runner sends every refused pin to a live provider |
| `withdraw-answers.json` | `tool.withdraw` reply bodies (known, unclassified, malformed) and the caller policy for replies and route errors | the wire crate's decoder and policy tests |
| `role-describe.json` | `role.describe` answers a consumer accepts or refuses, with the problem | the wire crate's `check_describe` test |
| `catalog-schemas.json` | flat and non-flat argument schemas | the wire crate's `check_flat_schema` test |
| `catalog-answers.json` | a full and a `digest_only` catalog answer (same `catalog_digest`), and tool entries a decoder refuses | the wire crate's catalog test |
| `late-results.json` | the `late_results` request, reply, ack, entry kinds (including an unknown one) and malformed entries | the wire crate's late-result tests |

Changing a vector changes the contract: bump the role crate's version and say
why in the commit.
