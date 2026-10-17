# `tool-provider/v1` test vectors

The shared bytes every `tool-provider/v1` implementation and consumer checks.
The role document is `crates/cortexkit-role-tool-provider/CONTRACT.md`.

| File | What it pins | Checked by |
|---|---|---|
| `call-key.json` | the `call_key` bound (1 to 256 bytes, each 0x21–0x7E, no space): valid keys, and invalid keys with the validator's error | the wire crate's validator test; the conformance runner sends every key to a live provider, as a call key and as a withdraw target |
| `schema-digest.json` | the structural `schema_digest`: exact digests, schema pairs that differ only in description text or key order (same digest), and pairs that differ in structure (different digest) | the wire crate's digest tests; the runner recomputes every live tool's digest |
| `schema-pin.json` | the `tp1` schema-pin encoding (`tool`, `schema_digest`, `semantics`): canonical pins with their parts, and refused strings | the wire crate's pin test; the runner sends every refused pin to a live provider |
| `withdraw-answers.json` | `tool.withdraw` reply bodies (known, unclassified, malformed) and the caller policy for replies and route errors | the wire crate's decoder and policy tests |
| `role-describe.json` | `role.describe` answers a consumer accepts or refuses, including a provider that advertises the `tool-catalog-capability-filter/v1` capability and lists the tool capability tags it serves in `tool_capabilities` | the wire crate's `check_describe` test |
| `catalog-schemas.json` | flat and non-flat argument schemas | the wire crate's `check_flat_schema` test |
| `catalog-requests.json` | `tool.catalog` request bodies: without a `preset` (the encoding omits the member), with `preset: "default"` and params, and an unknown preset with the `invalid_request {field: "preset"}` refusal body | the wire crate's request round-trip and refusal tests; the runner's `catalog_unknown_preset_refused` sends an undefined preset to a live provider |
| `catalog-answers.json` | a full and a `digest_only` catalog answer (same `catalog_digest`), and tool entries a decoder refuses | the wire crate's catalog test |
| `content-v1.json` | ordered text and image replies, tool errors with stable codes, structured program data, unknown parts, and malformed content/error/image/size cases | the wire crate's named decode-and-validate tests; `tool_result_content_v1_valid` checks live replies only for tools declaring the format |
| `call-modes.json` | classic-only omission, pinned catalog-content digests for default and nondefault mode lists, and empty, duplicate and unknown declarations | the wire crate's mode tests; the runner's `catalog_modes_valid` checks every live tool with `check_modes` |
| `reply-deadline.json` | per-tool reply deadlines, integral and unusable hold arguments, fallback floors, saturating arithmetic, and valid, invalid and malformed declarations | the wire crate's reply tests; the runner's `catalog_reply_valid` checks every live tool with `check_reply` |
| `emits-events.json` | per-tool `emits_events`: `false`, `true` and absent entries that decode and round-trip, an explicit `null` that decodes as absent and encodes without the member, and non-boolean values a decoder refuses | the wire crate's `emits_events` tests, which also check that the existing catalog vectors, which never declare it, keep their bytes and digests; the runner's `catalog_emits_events_boolean` checks that every live declaration is a JSON boolean |
| `composition-digest.json` | the `composition_digest` rule: each composition with its exact JCS bytes and digest, and objects that differ only in key order (same digest) | the wire crate's `composition_digest` test |
| `late-results.json` | the `late_results` request, reply, ack, entry kinds (including an unknown one) and malformed entries | the wire crate's late-result tests |
| `catalog-capabilities.json` | accepted `params.capabilities` filters and every refusal: unknown params, malformed, unsorted or repeated tags, and unserved tags | the wire crate's capability-selection and refusal tests; the conformance runner keeps a vendored copy |

Changing a vector changes the contract: bump the role crate's version and say
why in the commit.

The oversize `content-v1.json` case uses `expand_text` to repeat the text at
the `part` index `repeat` times before serializing and decoding. This pins
the 3 MiB refusal without checking in a multi-MiB string. The result-format
omission test also compares encoded catalog bytes and the existing
`call-modes.json` digest pins; the older vectors remain unchanged.

The conformance crate compiles in copies of `call-key.json`, `schema-pin.json`
and `catalog-capabilities.json`
(`crates/cortexkit-role-tool-provider-conformance/vectors/`),
because a published crate cannot read files outside its own directory. Its
`vendored_vectors` test fails until a changed vector is copied there too.
