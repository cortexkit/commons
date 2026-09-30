# `tool-provider/v1` test vectors

The shared bytes every `tool-provider/v1` implementation and consumer checks.
The role document is `crates/cortexkit-role-tool-provider/CONTRACT.md`.

| File | What it pins | Checked by |
|---|---|---|
| `call-key.json` | `call_key` bounds: valid keys, and invalid keys with the validator's error | the wire crate's validator test; the conformance runner sends every key to a live provider |
| `withdraw-answers.json` | `tool.withdraw` reply bodies: known answers, unknown answers (final but unclassified) and malformed replies | the wire crate's decoder tests |
| `role-describe.json` | `role.describe` answers a consumer accepts or refuses, with the problem | the wire crate's `check_describe` test |
| `catalog-schemas.json` | flat and non-flat argument schemas | the wire crate's `check_flat_schema` test |

Changing a vector changes the contract: bump the role crate's version and say
why in the commit.
