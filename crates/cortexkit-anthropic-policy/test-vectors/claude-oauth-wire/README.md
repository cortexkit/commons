# Claude OAuth wire vectors (Claude Code 2.1.280 persona)

`vectors.json` holds 7 request vectors for an OAuth `POST /v1/messages`
as anthropic-auth's **OpenCode 1** path sends it. Every case is labelled
`"source": "reconstructed"`: it was produced by running this repository's
code, not captured from Claude Code. Shaped for
`crates/cortexkit-anthropic-policy/test-vectors/claude-oauth-wire/`; the
field-by-field explanation is `blueprint.md`.

## Cases

| id | model | what it covers |
|----|-------|----------------|
| `fable51-first-turn` | `claude-fable-5-1` | first turn: billing block with `cc_prompt_id` only; system sanitising and identity block; forced adaptive thinking; effort pinned; `fallbacks:"default"` + server-side-fallback beta; mid-conversation-output-config beta; `diagnostics.previous_message_id: null`; explicit cache (host breakpoints → `ttl:"1h"`) |
| `opus55-continuation-replayed-thinking` | `claude-opus-5-5` | continuation replaying a signed thinking block; `cc_prev_req` + `cc_prompt_id`; per-turn-control beta; `thinking.block_binding` + thinking-binding beta (account setting `drop_block`); hybrid cache anchors |
| `opus55-effort-change` | `claude-opus-5-5` | effort high → low on turn 2: inserted `{"role":"system","content":[],"output_config":{"effort":"low"}}`, top-level effort = baseline `high`, internal marker blocks removed; cache off (5-minute breakpoints) |
| `sonnet55-between-tools` | `claude-sonnet-5-5` | host `thinking: disabled` → `between_tools`, effort `xhigh` → `high`; no fallback; automatic cache (one top-level `cache_control`) |
| `haiku55-tools-continuation` | `claude-haiku-5-5` | tools: `mcp_Read`/`mcp_Bash` in `tools[]` and replayed `tool_use`; trailing whitespace text after `tool_use` removed; `temperature` removed; `tool_result` continuation |
| `sonnet55-full-agent-subagent` | `claude-sonnet-5-5` | body with `tools` + `context_management` → full-agent beta list, per-turn-control after mid-conversation-system; forced `tool_choice` removed; subagent: no 1h cache, no lineage |
| `opus48-structured-output-fast` | `claude-opus-4-8` | `json_schema` output → structured-output beta list; `speed:"fast"` + fast-mode beta; host budget thinking unchanged |

## Shape of each case

- `input`: host (`opencode-1`), route (`oauth`), `host_headers` (what the
  OpenCode 1 host puts on the request, including the session headers the
  plugin removes), `host_body` (the AI SDK body as a JSON string),
  `options` (plugin settings for the request). Effort-change cases also have
  `effort_markers` and `lowered_host_body`: OpenCode 1 adds internal marker
  text blocks to user messages before lowering, and `lowered_host_body` is the
  body that actually reaches the fetch wrapper.
- `output`: `url`; `request_line`; `headers_wire_order` (name/value pairs in
  the order Bun writes them, including Bun's own `Connection`, `Host`,
  `Accept-Encoding`, `Content-Length`); `body` (exact bytes sent, as a JSON
  string); `body_sha256`; `body_bytes`; `billing_header` (text of
  `system[0]`); `cc_version_suffix` (first user text, the 3 sampled
  characters, the suffix); `cch`; `cch_canonical_body` (the exact bytes fed
  to xxHash64: `model` set to `""`, `max_tokens` removed, `cch=00000`).
- `primitives`: the unit checks `xxHash64("hello world test message") → cc124`
  and `computeCcVersionSuffix("short", "2.1.258") → 587`.

## Held fixed

Each of these replaces a random, account-specific or machine-specific value
(listed again under `fixed` in the file):

| value | fixed as | real source |
|-------|----------|-------------|
| OAuth access token | `sk-ant-oat01-PLACEHOLDER` | account credential |
| account UUID (`metadata.user_id.account_uuid`) | `00000000-0000-4000-8000-000000000000` | verified account |
| device id | 64 × `0` | 32 random bytes per account identity |
| Claude Code session id (`x-claude-code-session-id`, `metadata.user_id.session_id`) | `11111111-1111-4111-8111-111111111111` | random UUID per account identity |
| `x-client-request-id` | `22222222-2222-4222-8222-222222222222` | `crypto.randomUUID()` per request; the generator overwrites the header after it is built |
| `cc_prompt_id` | `33333333-3333-4333-8333-333333333333` | random UUID per user prompt |
| `cc_prev_req` | `req_PLACEHOLDER0000000000000001` | previous response's `request-id` |
| `diagnostics.previous_message_id` | `msg_PLACEHOLDER0000000000000001` or `null` | previous response's message id |
| OpenCode session ids | `ses_PLACEHOLDER_…` | OpenCode session (`x-session-affinity`) |
| tool-use ids, thinking signature | `toolu_PLACEHOLDER_0001`, `SIGNATURE_PLACEHOLDER_0001` | model output |
| `x-stainless-os` / `x-stainless-arch` | `MacOS` / `arm64` | `process.platform` / `process.arch` |
| `Host` | `api.anthropic.com` | the capture targets a local socket, so the generator writes the production host back |

No time-dependent field exists in the request. Nothing else is
substituted: suffixes, `cch` values, beta lists and bodies are exactly what
the code produced from these inputs.

## How the outputs were generated

`generate-vectors.ts` (harness) and `cases.ts` (inputs) in this folder call
the real functions in the order `packages/opencode/src/index.ts`
`sendWithAccessToken` uses them: `mergeHeaders` and deletion of the internal
headers, `markOpenCodeEffortTransitions`/`encodeOpenCodeEffortPlan` for effort
plans, `rewriteRequestBody` (which signs the body), `setOAuthHeaders`,
`addFastModeBetaHeader`, `rewriteUrl`, then Bun's `fetch` to a local raw TCP
listener, which records the request line, headers and body bytes as written.
The harness checks that the captured body equals the signed body, that
recomputing `cch` from `cch_canonical_body` and re-signing reproduce it, and
that the suffix follows from the first user text.

```sh
bun run --cwd packages/core build
cd .cortexkit/internal-docs/research/wire-blueprint-2.1.280
bun cases.ts > vectors.json
```

Bun 1.4.2 on macOS and Bun 1.3.14 on Linux (the version CI pins) produced
byte-identical files (SHA-256
`32ee924e877ce47219bdb1e7c808a50a7014fa85603a354b76a55b7f3a503655`). Code
base: anthropic-auth `9234efe54cec0834813ecb923fdc7e890b28bf15`.

Not covered: Pi and OpenCode 2 (their differences are described in
`blueprint.md`), lane-start warm requests, and CacheKeep prewarms.
