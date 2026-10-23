# `tool-provider/v1` — role contract

Stability: **alpha**. This document is the role definition; the Rust types in
this crate are its wire shapes, and `test-vectors/tool-provider-v1/` inside
this crate holds the shared vectors. The `cortexkit-role-tool-provider-conformance`
crate is its suite. Every item below is **pinned**: a provider must do it, and a
consumer may rely on it. Nothing in this revision is open.

A tool provider is any module that serves tool definitions and executes calls
to them. Nothing here names a particular implementation.

## 1. Role identity and addressing

- A provider lists `tool-provider/v1` in its manifest's `capabilities.provides`
  (`PROVIDES`). That grammar covers roles only; a tool's capability tags never
  go in the manifest.
- Required ops: `role.describe` and `tool.catalog` (`REQUIRED_OPS`). A
  provider that can hold a call past its own reply also serves
  `tool.withdraw`. A provider that settles calls later also serves
  `late_results` and `late_results.ack`.
- Every role op is a named request on the provider's tool route,
  `{name: <op>, arguments: {...}}`, exactly like a tool call. Role ops are
  never model tools: no catalog lists them.
- A consumer refuses a module whose `role.describe` lacks a required op, by
  name, before routing anything to it.

## 2. `role.describe`

- The answer is `{majors: [{version, ops, stability}], implementation_version,
  capabilities?, tool_capabilities?}` (`RoleDescribe`). `majors` lists every
  major of the role the module serves, each with its own `ops` and `stability`; `version` is
  spelled as in the manifest, for example `tool-provider/v1`. The caller picks
  the highest major it understands for a new session.
- Decoded leniently: unknown fields, role majors, ops and module capabilities
  are ignored, except for the `tool-provider/v1` major and the known filter
  capability below. The v1 major and its required ops are checked strictly.
  `capabilities` and `tool_capabilities` may be omitted when the filter is not
  advertised. `stability` is `alpha`, `beta` or `stable`; an unknown level
  decodes.
- The answer describes the implementation, never the tools: it carries no
  tool list. It depends only on the module build, so a consumer may cache it
  for as long as it talks to the same module incarnation; two answers from one
  incarnation are identical.
- `tool-provider/v1` defines exactly one optional module-level capability:
  `tool-catalog-capability-filter/v1` (`CATALOG_CAPABILITY_FILTER`). A consumer
  sends `params.capabilities` only after `role.describe` advertises this token.
  An advertising provider also returns `tool_capabilities`: the nonempty union
  of capability tags on its catalog tools, sorted bytewise and without
  duplicates. Every tag passes `check_capability_tag`. A provider that does not
  advertise the filter omits `tool_capabilities` and is unchanged; the filter
  is additive and opt-in. Session-level capabilities travel in the catalog
  answer.
- `tool_capabilities` is an index of tags, not a tool list. A consumer
  intersects it with the tags it wants using
  `describe::select_tool_capabilities`, then builds canonical `params` with
  `catalog::params_for_capabilities`.

## 3. `tool.catalog`

- [pinned] A tool may declare `reference_slots: [{name, kind}]`: at most
  eight distinct, non-empty literal top-level argument names. Names are not
  paths. `kind` is `token` or `source:<dialect>` with a non-empty dialect.
  Providers refuse an oversized list naming `reference_slots`, an empty or
  duplicate name naming `reference_slots[].name`, and a malformed kind naming
  `reference_slots[].kind` (`catalog::check_reference_slots`). The latter two
  identify the offending name. Omit an empty list; catalogs without slots keep
  their bytes and digest. `show_ui`-style declarations are `data: token` and
  `program: source:rich-ui/1`; the declarations do not authorize scanning until
  the runner's frozen plan opts the provider into output references.

- Request: `{params, preset?, composition?, system_text?, digest_only?}`
  (`CatalogRequest`).
  - `params` is the plan item's params, and every key in it is per-provider:
    a provider accepts only the keys it documents, and refuses any other key
    or value, never guessing. A consumer sends a provider only the params of
    that provider's own plan item, never another provider's. `behavior`,
    `scope`, `tool_descs` and `exclude` are conventional names a provider
    reuses when it supports that axis, so the same idea is spelled the same
    way across providers; none is required of every provider.
    - An advertising provider that supports `tool-catalog-capability-filter/v1`
      accepts only `capabilities` in `params`. It must refuse any other key as
      `invalid_request` with `detail.field: "params.<key>"`.
    - `params.capabilities` is an array of strings in strictly increasing
      bytewise order (therefore sorted and without repeats). A non-array is
      refused with `detail.field: "params.capabilities"`; a non-string member
      is refused with its indexed field `params.capabilities[i]`; an unsorted
      or repeated list is refused naming `params.capabilities`. A requested tag
      the provider does not serve is refused as `invalid_request` with
      `detail: {field: "params.capabilities[i]", capability: "<tag>"}`.
    - A valid list selects exactly the tools carrying **any** requested tag,
      not only tools carrying every tag. An absent or empty list selects no
      tools. A consumer sends the field only to a provider that advertised the
      optional module capability; providers that do not advertise it retain
      their existing params and catalog behavior. The acceptance and refusal
      forms are pinned by `test-vectors/tool-provider-v1/catalog-capabilities.json`.
  - `preset` is the plan item's named variant, defined by the provider. It
    sits beside `params`, not inside it, so it never collides with a
    provider's own axis, and mirrors `system_text`'s `{preset, params}`.
    Absent means the provider's default variant; an encoder omits the member
    rather than sending `null`. A preset the provider does not define is
    refused as `invalid_request {field: "preset"}`, never guessed, like an
    unknown `params` value (`test-vectors/tool-provider-v1/catalog-requests.json`).
    A caller sends `preset` only to a provider whose preset-less answer lists
    it in `presets` (§4.3).
  - `composition` is the session's composition, carried verbatim as an opaque
    JSON object. Providers never interpret it beyond resolving their own text
    against it. Absent on a preflight call.
  - `system_text` is `{preset, params}` when the plan has a system-prompt item
    for this provider.
- Answer (`CatalogAnswer`): `{generation, catalog_digest, composition_digest?,
  tools: [...], system_text?, capabilities?, user_commands?, presets?}`.
  `user_commands` is §4.2; `presets` is §4.3.
  - `generation` is an opaque string that changes whenever the full advertised
    definitions change, independent of a capability filter.
  - `catalog_digest` is an opaque digest of the answer's content, including
    only the tools selected by the request's capability filter. A full answer
    and a `digest_only` answer to the same inputs carry the same value, so a
    caller holding a full answer can check it later with one cheap `digest_only`
    fetch. When the request asks for `system_text`, that item is part of the
    answer's content, so `catalog_digest` covers it too: a change to the text
    alone changes `catalog_digest`, and a `digest_only` fetch with the same
    `system_text` request detects it.
  - `composition_digest` is SHA-256 over the RFC 8785 (JCS) canonical JSON
    of the request's `composition` object exactly as sent, written as 64
    lowercase hex characters (`composition_digest`). Nothing is stripped
    first. The runner computes the same value over the composition it sent
    and compares (`test-vectors/tool-provider-v1/composition-digest.json`
    carries each composition, its JCS bytes and the digest). Absent when the
    request carried no composition.
  - `capabilities` is a top-level object of session-level capabilities, keyed
    by name: `host_params`, `late_results`. A capability is declared by the
    value `true`; any other value is not a declaration. The runner freezes
    them with the tools.
  - `tool_capabilities` in `role.describe` is the sorted, unique union of the
    per-tool capability tags in the full catalog. An advertising provider's
    filtered answer still contains exactly the tools selected by any requested
    tag.
  - `system_text`, when requested, is `{text, item_digest, preflight_digest,
    composition_digest, tool_names}`, from the same configuration resolution
    as the catalog in the same reply. `item_digest` is the SHA-256 of the
    exact UTF-8 bytes of the text this answer resolved to, as 64 lowercase
    hex characters (`system_text_digest` in this crate), never a digest of a
    JSON object wrapping it, and is present whether or not `text` is. A
    runner that receives `text` checks it by hashing the bytes it received.
    `preflight_digest` is provider-defined, written as 64 lowercase hex
    characters: it must change whenever the text the same request resolves to
    would change, and may change more often (a provider may digest its inputs
    rather than its output). A runner compares
    it only with an earlier `preflight_digest` from the same provider, never
    with `item_digest`. `tool_names` lists, sorted and without
    duplicates, the model-facing names of the tools the text was composed
    for; it is required whenever `text` is present. A runner compares it with
    the names of the tools it fetched from the same provider and refuses the
    plan as `plan_stale` with a `text_tool_names` difference (`missing`,
    `unexpected`) when they differ, so text written for one tool set is
    never served beside another.
  - A provider with only system text answers an empty `tools` list.
- `digest_only: true` answers `{generation, catalog_digest}` with no tools.
- Each tool (`CatalogTool`): `{name, schema_digest, semantics, result_ops?,
  result_format?, capabilities, description?, input_schema, reply?, modes?,
  emits_events?}`.
  - `name` is the exact name the user tier disables the tool by.
  - `schema_digest` is the digest of the **structure** of `input_schema`
    (see "Schema digest" below): 64 lowercase hex characters. Description
    text never changes it.
  - `semantics` is an integer, bumped whenever the tool's behaviour changes
    without a schema change.
  - `modes` lists the call modes this tool serves: `classic`, `codemode` and
    `flow`. Absent means `["classic"]`. An explicit `["classic"]` decodes to
    absence (`None`), and both forms encode without `modes`, including a
    directly assigned default list. A provider hashes the encoded catalog,
    not a raw input spelling, so writing the default explicitly cannot change
    its digest. Other lists retain their order and spelling; unknown strings
    decode as `CallMode::Unknown` and round-trip unchanged.
    - A provider declares only modes it handles: `check_modes` rejects empty
      lists, duplicates and unknown modes. A consumer refuses an invalid
      provider catalog as `invalid_answer`, naming the tool and `modes`, as
      with invalid reply metadata.
    - A provider declaring `CALL_MODES_CAPABILITY` (`call-modes/v1`) reads the
      daemon's mode on every route bind. The daemon derives it from scope;
      a request body cannot choose it. Absent bind mode means classic.
      `serves_call_mode` checks every tool call against that bind mode before
      execution: an unlisted mode or `Unknown` is never served. The refusal is
      a terminal `ERROR` frame with code `call_mode_unsupported`
      (`errors::CALL_MODE_UNSUPPORTED`) and detail `{tool, mode}`, so every
      caller handles it the same way; the wording is the provider's own.
    - Nondefault lists are catalog content, covered by `catalog_digest`, not
      the argument schema digest or schema pin. The optional default emits no
      new bytes, preserving existing catalog digests.
    - `test-vectors/tool-provider-v1/call-modes.json` pins invalid declarations
      and default/nondefault content digests for one provider-defined scheme.
  - `result_ops` lists the hook result operations the tool accepts, from
    `prepend`, `append`, `replace`. Absent means all three.
  - `result_format` opts this tool into a reply body format. It is an open
    string enum (`ResultFormat`), with known value `"content/v1"`; unknown
    strings decode and round-trip unchanged. Absence leaves today's opaque
    provider JSON body unchanged and emits no new catalog bytes, preserving
    existing catalogs and digests. A present format is catalog content,
    covered by `catalog_digest`, never by `schema_digest` or the schema pin.
  - `capabilities` lists capability tags: unprefixed (`code.outline/v1`,
    defined only by this document) or namespaced (`acme:code.callgraph/v1`,
    free for anyone). Data in this answer only. This document defines these
    unprefixed tags, each a promise any provider can make about a tool:

    | Tag | The tool |
    |---|---|
    | `shell.exec/v1` | runs a shell command in the session's workspace and returns its output and exit status |
    | `code.read/v1` | returns file contents, whole or by range |
    | `code.edit/v1` | changes files in the workspace |
    | `code.search/v1` | finds code in the session's own workspace by text, regular expression or meaning |
    | `code.files/v1` | lists or finds files by path pattern |
    | `code.outline/v1` | returns the structure of a file or directory (symbols, headings) |
    | `code.callgraph/v1` | answers caller, callee and impact questions about code |
    | `code.diagnostics/v1` | returns compiler or linter diagnostics |
    | `browser.use/v1` | drives a web browser session: navigates, reads the page and acts on it |
    | `computer.use/v1` | drives desktop applications: reads their state and acts on them |
    | `git.history/v1` | reads repository history: commits, file contents at a revision, and diffs between revisions, without changing the repository |
    | `web.search/v1` | searches the general web and returns results |
    | `github.read/v1` | reads GitHub issues, pull requests and repository data, without changing anything on GitHub |
    | `code.search.public/v1` | searches code across public code hosts, outside the session's own workspace |

    A defined tag promises only the activity in its row. Everything else is
    the provider's own: its arguments, permission grants, consent flow,
    element or object identifiers, output format and refusal wording. A
    consumer that matched a defined tag must not depend on any of those. A
    provider that wants consumers to match its specific contract adds its own
    namespaced tag beside the defined one (for example
    `cerebellum:browser.use/v1`), and a consumer that depends on that contract
    pins the namespaced tag.

    Any other tag is `<namespace>:<name>/v<N>`: a namespace of lowercase letters
    and digits in `-`-separated words, a name of `.`-separated words of
    lowercase letters, digits and `_`, and a version with no leading zero
    (`acme:code.callgraph/v1`). The crate lists the defined tags as
    `DEFINED_CAPABILITY_TAGS` and checks a tag with `check_capability_tag`;
    the conformance suite refuses a catalog with any tag that fails it.
  - `input_schema` is the argument schema; `description` its description.
  - `reply` is optional per-tool reply deadline metadata:
    `{max_ms, hold_argument?: {name, default_ms, grace_ms}}`. There is no
    catalog-wide default. A provider knows how long its tools may hold a call;
    declaring that time lets a runner wait without guessing a deadline or
    maintaining a provider-specific table.
    - `max_ms` is a required integer from 1 through 86,400,000 (24 hours), in
      milliseconds from a provider receiving a request to its terminal frame.
    - `hold_argument`, when present, names the tool argument that controls its
      hold. `name` must name a top-level property of `input_schema` whose
      `type` is `integer` or `number`, not a union or a property that allows a
      string. `default_ms` and `grace_ms` are required integers from 1 through
      `max_ms`. `default_ms` is the hold for an absent or unusable argument;
      `grace_ms` allows time for the reply to arrive after the hold.
    - A runner refuses a catalog whose `reply` violates these rules as
      `invalid_answer`, naming the tool and the member. `check_reply` validates
      the metadata beside `check_flat_schema`; the conformance suite checks it.
      Missing required members or non-integer metadata also make an answer
      invalid.
    - A runner computes the deadline for one call from its coerced arguments
      (`reply_deadline_ms`):
      1. No `reply`: use the runner's own fallback.
      2. `reply` without `hold_argument`: use `max_ms`.
      3. With `hold_argument`: use the named argument as the hold only if it is
         a JSON integer or an integral number at least 1. Absence, zero,
         strings, fractional numbers, negative numbers, null and other values
         use `default_ms`. Add `grace_ms` with saturating arithmetic, then take
         `min(hold + grace_ms, max_ms)`.
      4. Never use a deadline below the runner's own fallback, even if `max_ms`
         is smaller. The fallback floor applies after the maximum is applied.
    - The deadline bounds how long a runner waits; it never changes the call's
      arguments or a provider's own limits. A provider exceeding its declared
      `max_ms` violates the contract; a runner records `outcome_unknown` and
      never re-sends that call. When the reply deadline expires without
      a terminal frame, a runner records `outcome_unknown` and never re-sends
      the call.
    - A runner saves `reply` together with the tool when it admits the tool
      into a run's plan. A resumed run uses that saved value, never a value
      fetched again from the provider.
    - `reply` is omitted from serialization when absent: existing catalog
      bytes, digests and pins do not change. When present, it is covered by
      `catalog_digest`, never by `schema_digest` or `schema_pin`. Changing only
      a deadline therefore never invalidates a pinned call. Old runners may
      ignore the optional member; old providers may omit it.
    - `test-vectors/tool-provider-v1/reply-deadline.json` pins the deadline
      arithmetic and valid and invalid reply declarations.
  - `emits_events` is an optional boolean saying whether calling the tool can
    make this provider emit an event. A consumer that reacts to this
    provider's events (a flow triggered by them, say) uses it to tell whether
    calling the tool could loop back into a new trigger.
    - `false` is the provider's promise that **no call to this tool, with any
      arguments and any op, causes this provider to emit an event**. That
      covers events it publishes directly, and events it relays from an
      upstream system (for example a webhook the upstream sends because of
      the call).
    - `true` means it may.
    - **Absent means unknown**, and consumers MUST treat unknown like `true`.
    - Only `false` lets a consumer skip a loop check. The declaration covers
      the whole tool, so a tool whose ops differ must not declare `false`.
    - The member is omitted from serialization when absent, so existing
      catalog bytes and digests do not change. When present it is catalog
      content, covered by `catalog_digest`, never by `schema_digest` or the
      schema pin. An explicit `null` decodes as absent, as for every other
      optional member; any other non-boolean value does not decode, so a
      consumer refuses the catalog as `invalid_answer`, as for any other
      malformed member.
    - `test-vectors/tool-provider-v1/emits-events.json` pins `false`, `true`,
      absent, `null` and refused non-boolean declarations.
- The answer is a pure function of its inputs (the plan item's preset and
  params, the composition, user and project configuration, host facts), so
  `catalog_digest` covers the preset: a `digest_only` request with the same
  preset carries the full answer's `catalog_digest`. Scope, owner and
  agent on the route are attribution only. Tool names and schemas never depend
  on the composition; descriptions and system text may. The same inputs give
  the same bytes.
- A tool the user or project tier disables is absent from the answer.
- **Schema digest** (`schema_digest`, `structural_schema`). The digest covers
  the argument schema's structure (property names, types, `required`, enums,
  bounds and every other keyword) and never description text:
  1. Remove the `description` keyword from every schema object: the root and
     every subschema reached through `properties`, `patternProperties`,
     `dependentSchemas`, `$defs`, `definitions` (the subschemas under each
     name), `items`, `additionalItems`, `additionalProperties`,
     `unevaluatedItems`, `unevaluatedProperties`, `not`, `if`, `then`,
     `else`, `contains`, `propertyNames`, and each element of `prefixItems`,
     `anyOf`, `oneOf`, `allOf`. Property names stay, even a property called
     `description`; data values (`enum`, `const`, `default`, `examples`) and
     unknown keywords stay verbatim.
  2. Serialize the result as RFC 8785 (JCS) canonical JSON.
  3. Take SHA-256 of those bytes, written as 64 lowercase hex characters.

  Two catalogs that differ only in description text give every tool the same
  digest; any structural change gives a different one
  (`test-vectors/tool-provider-v1/schema-digest.json`).
- Argument schemas are flat (`check_flat_schema`): a JSON object with no
  root-level `anyOf`, `oneOf` or `allOf`, and no root-level `type` array (a
  type array is a union). Unions below the root are allowed.
- A host-only parameter carries `"x-ck-audience": "host"`, is always
  optional, and is refused in a call from the model.

## 4. Calls

- A call is a route `REQUEST` whose body is subc-protocol's
  `ToolCallRequest`: `{name, arguments, tool_call_id?, progress_token?,
  call_key?, schema_pin?}`.
- `call_key` (subc-protocol 0.26.0) is opaque, 1 to 256 bytes, each printable
  ASCII from 0x21 to 0x7E; space is not allowed. A present but malformed key
  is refused as `invalid_request` with `detail: {field: "call_key"}`. A call
  without a key executes but has no deduplication and no withdrawal, by
  construction; nothing is added to its reply.
- Every record shared between parties keys on `(carrier principal,
  call_key)`. The model's `tool_call_id` is display-only.
- `schema_pin` (subc-protocol 0.27.0, top-level, omitted when `None`) has the
  same byte bound as `call_key`. Its content is this role's: the tool name,
  the tool's `schema_digest` and its `semantics`, as the runner froze them.
  A pin never holds the catalog `generation` or any description text, so it
  survives a description-only deploy. It is encoded canonically as **`tp1`**
  (`SchemaPin`):

  ```text
  tp1:<pct(tool)>:<schema_digest>:<semantics>
  ```

  `pct` keeps the RFC 3986 unreserved bytes (`A-Z a-z 0-9 - . _ ~`) and
  writes every other byte of the UTF-8 string as `%` plus two uppercase hex
  digits, so `:` inside the tool name is always `%3A`. `schema_digest` is 64
  lowercase hex characters. `semantics` is decimal, with no sign and no
  leading zeros. The tool is non-empty. Exactly one string encodes each pin;
  anything else is refused. A pin that is
  malformed, or names a tool other than the call's `name`, is refused as
  `invalid_request {field: "schema_pin"}`. The field and its bound are
  subc-protocol 0.27.0's, re-exported by this crate (`call.rs`).
- A call whose pinned `schema_digest` the provider can no longer honour is
  refused `tool_schema_changed {tool, expected, current}`; one whose pinned
  `semantics` it can no longer honour, `tool_semantics_changed {tool,
  expected, current}`. Providers keep argument changes additive so old
  schemas stay servable.
- Every request gets exactly one terminal frame (`RESPONSE`, `STREAM_END` or
  `ERROR`), and it is the last frame: on success, refusal, cancel, drain, and
  when the provider stops tracking the request.
- A `RESPONSE` body is the bare answer object (a catalog answer, a call's
  reply, a `WithdrawAnswer`), never wrapped in `{"result": ...}` as
  `llm-runner/v1` replies are. A refusal is an `ERROR` frame whose body is
  `{code, message, detail?}`: every other refusal field (`tool`, `expected`,
  `reason` and the rest) lives inside `detail`, never at the top level of
  the body. `reply-shape.json` pins one of each.
- Refusal codes: `unknown_tool {tool}` for a tool the provider does not serve;
  `tool_disabled {tool}` for a disabled tool; and, recorded by the runner as
  the call's error result and never re-dispatched, `tool_schema_changed`,
  `tool_semantics_changed`, `tool_unavailable {tool, reason: retired |
  under_review}` and `capability_not_admitted`.
- Model-shaped arguments are coerced before anything records, hashes or
  checks them. Scope, owner and agent are never arguments; they come from the
  route's stamp.

## 4.1. Opt-in tool results: `content/v1`

A consumer that decodes `content/v1` result bodies declares the string
`content/v1` in its `route.open` `consumer_capabilities`. The daemon relays
that list verbatim on the provider's route bind as
`RouteBindRequest.consumer_capabilities`. This declaration is the consumer's
unverified statement; it grants no permission or other authority.

For a tool whose catalog entry declares `result_format: "content/v1"`, a
provider sends a `content/v1` body only on a route whose bind declared
`content/v1`. On every other route, including one whose bind has no
`consumer_capabilities`, the provider sends that tool's legacy body exactly as
before. The decision is made separately for each route from its bind, never
from anything in a call. The catalog lists `result_format` regardless of who
asks: it does not change schema digests, and older consumers ignore it.

A consumer must not decode a body as `content/v1` unless it both declared
`content/v1` on that route and the tool's catalog entry opts in with
`result_format: "content/v1"`.

On a route with both declarations, the tool returns the following body
(`result::ToolResult`), with MCP's CallToolResult member names exactly:

```json
{
  "content": [{"type": "text", "text": "Model-facing result."}],
  "structuredContent": {"program_facing": "data"},
  "isError": true,
  "errorCode": "provider_error"
}
```

- `content` is required and must hold at least one `ContentPart`. A reply
  with only `structuredContent` is malformed, even if that object is useful.
- `structuredContent` is an optional object for programs. Consumers build
  model input from `content` only, **never** from `structuredContent`.
- `isError` is an optional boolean. When it is true, `errorCode` is required:
  a stable snake_case code owned by the provider (lowercase letters and
  digits in nonempty underscore-separated words, starting with a letter).
  `isError: true` without a code is malformed. A code without `isError: true`
  is malformed too, including with an absent or false `isError`.
- Content parts are tagged by `type`:
  - `{"type": "text", "text": "..."}`: text the model sees verbatim.
  - `{"type": "image", "mimeType": "image/png", "data": "AA==", "width": 1,
    "height": 1, "alt": "...", "id": "..."}`: `data` is raw standard base64,
    not a URL. Refuse an empty value, a value starting with `data:` and invalid
    base64 (including an invalid alphabet, padding or trailing bits). `mimeType`
    must be `image/png`, `image/jpeg`, `image/webp` or `image/gif`. `width` and
    `height` are positive integer pixel dimensions. Optional `alt` is the
    provider's own wording for a consumer that cannot deliver the image to
    its model. Optional `id` is an opaque provider string, unique among image
    IDs in the reply when present.
  - Any other string `type` is preserved as raw JSON (`ContentPart::Unknown`).
    A consumer renders it as a one-line "unsupported content" note and never
    drops it silently.
- Order is significant: consumers keep the exact order of `content`. Any
  number of image parts is allowed. Harness-specific layouts (one text plus
  attachments versus separate blocks, for example) are the consumer's
  mapping, never the provider's.
- The entire serialized reply must be **under 3 MiB** (3 × 1024 × 1024 bytes),
  counting every member, including structured and unknown content. This
  leaves headroom under the daemon's 4 MiB per-connection limit, as for
  step-transform hooks. A provider that would exceed the bound downsizes or
  refuses by name; it never truncates silently.
- Providers validate before sending; consumers decode and validate on receipt.
  `result::validate` returns a typed `ResultProblem` naming the member path
  and reason. Missing required fields or wrong JSON member types do not
  decode. Tools without the opt-in keep their opaque bodies; consumers never
  guess `content/v1` from a reply's appearance.

`test-vectors/tool-provider-v1/content-v1.json` pins valid and malformed bodies.
The existing catalog vectors pin unchanged encoded bytes and digests when
`result_format` is absent.

### Held first replies: `held-reply/v1`

A consumer that understands held first replies declares the string
`held-reply/v1` alongside `content/v1` in its `route.open`
`consumer_capabilities`. The daemon relays that list verbatim on the provider's
route bind as `RouteBindRequest.consumer_capabilities`. Like `content/v1`, this
is an unverified consumer statement and grants no permission or authority.

A provider sends a held reply only on a route whose bind declared **both**
`held-reply/v1` and `content/v1`, for a tool whose catalog entry opts into
`result_format: "content/v1"`. On every other route the provider sends today's
body unchanged: a `content/v1` body without the marker for a content-only
consumer, or the legacy body when `content/v1` is not declared. The decision
is made separately for each route from its bind, never from anything in a call
and never from `call_mode`. `HELD_REPLY_V1` names the consumer capability;
`consumer_reads_held_reply_v1` checks the two bind declarations. Providers also
check the tool opt-in and the call/provider prerequisites below.

`ToolResult` gains the optional top-level `held` member:

```json
{
  "content": [{"type": "text", "text": "Please approve the consent card."}],
  "held": {"reason": "consent", "detail": {"ask_id": "provider-owned-id"}}
}
```

- `content` remains required, nonempty model-facing interim content. A consumer
  may show it, but **must not treat it as the call's final answer**.
- With `held` present, `isError` must be absent or false. `held` with
  `isError: true` is malformed.
- `reason` is an open snake_case string with exactly the `errorCode` grammar:
  lowercase letters and digits in nonempty underscore-separated words,
  starting with a letter. Unknown reasons are kept, not refused. The documented
  values are `consent`, `login_handoff`, `login_capture` and `running`.
- `detail`, when present, is a JSON object, owned by the provider and opaque
  to the role. A non-object, including explicit `null`, is malformed.
- A provider may send `held` only for a call that carried a `call_key`, and
  only if it declares the `late_results` session capability. The held reply is
  **not** the call's answer: the answer is the §7 `late_results` entry for that
  `(carrier, call_key)`, of kind `result`, `not_started` or `expired`.
  `tool.withdraw` applies unchanged (§5).
- `held` means the call's answer comes later through `late_results`. The
  reason says why:
  - `consent`, `login_handoff` and `login_capture` are the person-wait
    reasons. A person must act before the answer can arrive.
  - `running` means the call's work continues and no person needs to act. The
    interim `content` says so and says that the result will arrive later.
- The prerequisites above apply to every reason, `running` included: a
  `call_key`, the `late_results` capability, and a route whose bind declared
  both `held-reply/v1` and `content/v1`.
- A consumer may show a person a prompt to act, or count the call as waiting
  on a person, only for the three person-wait reasons
  (`result::is_person_wait`). For `running` and for any reason it does not
  know, it shows the provider's interim content only. It still pulls
  `late_results` for every held reply, whatever the reason.
- Refusals no person can fix, such as busy, validation failure or unavailable
  consent, are ordinary final replies and never carry `held`.

`result::validate` returns typed `ResultProblem`s for held/error conflicts,
invalid reason grammar and non-object detail. It does not have route, catalog
or call context; providers enforce those prerequisites before sending.
`result::held_reasons` names the documented reasons.
`test-vectors/tool-provider-v1/held-reply-v1.json` pins the valid reasons,
malformed bodies, route-bind gating decision table and which reasons are
person waits.

## 4.2. User commands

- `CatalogAnswer.user_commands` is a list of `{name, description,
  argument_hint?}` (`user_command::UserCommand`). Empty lists are omitted:
  old catalogs keep their bytes and digests. A present list is catalog
  content covered by `catalog_digest`. Names are unique within the answer,
  1–32 bytes matching `^[a-z][a-z0-9]*(-[a-z0-9]+)*$`; descriptions are one
  line of 1–120 characters with no line break, and hints are 1–60 characters.
  The runner freezes commands alongside tools at admission. A provider must
  serve every listed command for the life of every session that froze it.
- Commands are never tools. `user_command.invoke` is a reserved op; listing
  it as a tool makes a catalog malformed (`user_command::check_catalog`).
- The request is `{name: "user_command.invoke", arguments: {command, input},
  call_key}`. `input` is the person's verbatim text after the name, possibly
  empty, at most 4,096 bytes. `call_key` is required and validated as in §4.
  A repeat of `(carrier, call_key)` returns the stored reply byte for byte
  and never runs the command twice.
- Only a session-scoped route with neither `attributes.run_id` nor
  `attributes.flow_id` is admitted (`user_command::admitted`). Authority comes
  from the daemon's route stamp, not arguments or the call-mode declaration.
  Every other route is refused `user_command_not_admitted`. An unlisted
  command is refused `unknown_user_command {command}`; malformed requests
  are refused `invalid_request {field}`.
- `UserCommandReply` is `{state, text, job?}`. Providers emit `done`,
  `accepted` or `failed`; decoders read `state` open and show an unknown
  state's text. `text` is plain text with light markdown, at most 64 KiB;
  `job` is an optional opaque id. Bad command arguments yield `failed` with
  usage text, not a refusal. The first reply arrives within 5 s.
- Longer work replies `accepted`, which requires the `late_results` session
  capability. The provider writes zero or more non-terminal `progress`
  entries with `{text}`, then exactly one `result` whose `result` is a
  `UserCommandReply` in `done` or `failed`, under the same `(carrier,
  call_key)`. Replies, progress and final results are for the person only,
  never model history. Older consumers record and ack unknown kinds without
  retrying, so the added `progress` kind is safe for them.

## 4.3. Served presets

- `CatalogAnswer.presets` lists the names of the presets the provider
  serves. It appears only in the answer to a `tool.catalog` request without
  a `preset`, and is omitted when the provider serves none. Empty lists are
  omitted: old catalogs keep their bytes and digests. A present list is
  catalog content covered by `catalog_digest`, so a `digest_only` request
  without a preset detects a change to it. A `digest_only` answer still
  carries only `generation` and `catalog_digest`.
- Names are unique within the list, 1–32 bytes matching
  `^[a-z][a-z0-9_-]*$`, and the list has at most 16 entries. Every such name
  also passes subc-protocol's `validate_preset`, so it can be sent on a tool
  call unchanged. A list in the answer to a request that named a preset is
  malformed. `catalog::check_presets` refuses a malformed list as
  `invalid_request`: `presets` for too many entries or a list in a preset
  answer, and `presets[]` with `detail.preset` for a bad or repeated name. A
  consumer refuses such an answer as `invalid_answer`, as for any other
  malformed member.
- A caller sends `preset`, on `tool.catalog` or on a tool call
  (`ToolCallRequest.preset`), only to a provider whose preset-less catalog
  answer lists that preset (`CatalogAnswer::lists_preset`).
- An absent list means the provider serves no presets, so callers send it
  none.
- A provider must serve every listed preset, on `tool.catalog` and on tool
  calls, for as long as it lists it. It keeps refusing any other preset as
  `invalid_request {field: "preset"}`, as today.
- `test-vectors/tool-provider-v1/presets.json` pins catalogs with and without
  a list (the absent one is the `user-commands.json` catalog, byte for byte),
  valid lists, and malformed ones: a bad name, a duplicate, 17 entries, and a
  list in a preset answer.

Adoption. Providers declare first. A caller relying on the list, and so
sending no preset to a provider that lists none, needs every provider that
serves presets to declare its list. Until then, a caller may keep its
existing behaviour for a provider whose preset-less answer lists no presets.

## 5. `tool.withdraw`

Served by a provider that can hold a call past its reply.
This includes a call whose first reply carries `held` (§4.1,
`held-reply/v1`); the marker changes neither withdrawal nor caller policy.

- The request is `{name: "tool.withdraw", arguments: {call_key, carrier?,
  scope?}}` (`WithdrawArguments`). It has no top-level `call_key`; one that is
  present is refused as `invalid_request {field: "call_key"}`. A malformed
  `arguments.call_key` is refused as `invalid_request {field:
  "arguments.call_key"}`.
- The record is keyed `(carrier, call_key)`; the carrier is the principal
  that raised the call.
- Caller rules (`resolve_carrier`, `check_record_scope`). Each failure is a
  **route error**, never an answer, and terminal for the caller:

  | Situation | Route error |
  |---|---|
  | `arguments.scope` present and not the route's stamped scope | `withdraw_scope_mismatch` |
  | a caller other than the scope's owner names a carrier other than itself | `withdraw_carrier_mismatch` |
  | the scope's owner names no carrier and holds no call of its own under the key | `withdraw_carrier_required` |
  | the record found is under a different scope than the route's stamp | `withdraw_not_permitted` |

  - A caller other than the scope's owner is the carrier: the record is keyed
    on the route's stamped principal, and `arguments.carrier`, if present,
    must equal it. Naming anyone else is `withdraw_carrier_mismatch`, whether
    or not the caller carries calls of its own, so `withdraw_not_permitted`
    is reached only when the record found is under a different scope than the
    route's stamp.
  - The scope's owner may withdraw any carrier's call by naming
    `arguments.carrier`. An owner that is also the carrier is treated as the
    carrier: `carrier` is optional and must match if present. An owner
    naming no carrier that holds no call of its own under the key gets
    `withdraw_carrier_required`, not `unknown_call`.
  - On an unscoped route, only the carrier may withdraw, keyed on the route's
    principal.
- The reply is a `RESPONSE` whose body is the answer (`WithdrawAnswer`):
  - `{answer: "withdrawn"}`: the action is guaranteed never to run;
  - `{answer: "already_started", outcome: "<code>" | "unknown"}`;
  - `{answer: "completed", result: <recorded reply>, result_retained: true}`,
    or `{answer: "completed", outcome: "<code>", result_retained: false}`;
  - `{answer: "refused", reason: "denied" | "expired" | "scope_ended" | <open>}`;
  - `{answer: "unknown_call"}`: no record has that `(carrier, call_key)`. A
    provider answers this for a key it never held, and never refuses such a
    key (the caller checks aside). It never means "not sent".
- The answer is read from the stored record, so it is identical, byte for
  byte, for every permitted caller and every repeat.
- The caller policy (`caller_verdict`):

  | Reply | Caller concludes | Retries |
  |---|---|---|
  | `withdrawn`, `refused` | the call never ran | no |
  | `completed` | the call ran | no |
  | `already_started` with an outcome | that outcome | no |
  | `already_started`, `unknown` | `outcome_unknown` | no |
  | `unknown_call` | nothing was held | no |
  | any other `answer`, or a body that is not an answer | final, unclassified; recorded verbatim | no |
  | `withdraw_not_permitted` and the three errors above | terminal refusal | no |
  | a transient route refusal: `scope_not_synced`, or any code subc itself retries (`module_reloading`, `module_warming`, `target_unavailable`, `module_timeout`; `errors::is_transient`) | nothing yet | yes |
  | any other route error | terminal refusal | no |

## 6. Approval execution on the provider

A provider that executes approval-gated calls itself moves each through four
durable states, each synced before the step it permits: Prepared,
Authorized, DispatchStarted, Settled.

- A call that is Prepared, or Authorized without DispatchStarted, when the
  provider restarts is never run automatically. The provider reports it to the
  call's custodian as a `late_results` entry of kind `not_started` with
  `reason: "restart_before_dispatch"`, and `tool.withdraw` for it answers
  `withdrawn` or `refused`.
- A call that had started when the provider restarted reports what the
  provider observes of it; when it cannot tell whether the call settled, it
  says `unknown`, never a guessed success. It is never re-sent.

## 7. Late results

A provider that settles calls later declares the `late_results` session
capability. The custodian is always the carrier, the principal that raised
the call, including a call raised inside a run whose scope another principal
owns. For example, Broca collects held results for calls it raises inside a
worker run that core owns. The custodian pulls; the provider never pushes, and serves an
entry only to a caller whose verified principal is its custodian.
For a held first reply (§4.1, `held-reply/v1`), this entry, not the interim
`content`, is the call's answer, correlated by `(carrier, call_key)`.

- Request: `late_results {since: cursor | null, limit?}`
  (`LateResultsRequest`). `since: null` reads from the start.
- Reply: `{entries, cursor: {provider_incarnation, seq}, more: bool}`
  (`LateResultsReply`). `more` says entries beyond `limit` are waiting.
  `provider_incarnation` is an opaque string that changes on every provider
  start; `seq` is a `u64`, `0` before any entry; `limit` is a `u32`.
- Entry (`LateEntry`): `{kind, owner, ref, scope_epoch, custodian, call_key,
  invocation_id?, event_id, settled_at, reduced, result? | outcome?}`, plus
  `reason` on a `not_started` entry.
  - `kind` is `result`, `not_started` or `expired`, decoded open. An entry of
    an unknown kind is recorded and acked like any other, never retried.
  - `not_started` carries `{call_key, event_id, settled_at, reason}`; `reason`
    is an open string whose first value is `restart_before_dispatch`.
  - `reduced: true` means the provider kept only the outcome, in `outcome`,
    rather than the full result.
  - `call_key` is qualified by the carrier; `invocation_id` is present only
    for an onward request.
  - `event_id` is deterministic: the same late result always carries the same
    `event_id` across restarts and lost acks.
  - `settled_at` is milliseconds since the Unix epoch (`u64`). `reduced`
    defaults to `false`.
  - Every entry, of any kind including an unknown one, carries `owner`,
    `ref`, `scope_epoch`, `custodian`, `call_key`, `event_id` and
    `settled_at`, because the caller's dedupe key needs them. An entry
    missing one does not decode.
- Ack: `late_results.ack {through: cursor}` (`AckRequest`), per custodian,
  answered with an empty `RESPONSE` body `{}`.
  Each custodian has its own cursor and acks only its own entries.
- A cursor from another provider incarnation, or past the entries the
  provider has, is refused `cursor_incarnation_changed` (`check_since`). The
  caller re-reads from `null` and deduplicates on `(provider, custodian,
  call_key, event_id)` (`IntakeKey`).
- Retention: an unacked result is kept until acked or 24 h after it settled,
  and a scope ending does not delete it. On expiry an `expired` entry stays
  until acked. At most 1,000 unacked results per session and 1,000 expired
  entries per scope owner are kept, oldest dropped first.

## 8. Crash harness and kill points

Implementations supply a `cortexkit-role-harness` `Harness`. The suite calls
`kill_at` once per point in a run, each time on a module it spawned on a fresh
state root, and never kills one trigger at several points; `CrashDriver`
refuses anything else. This role's kill points (`points.rs`):

| Point | Durable state after the kill | Used by the v1 suite |
|---|---|---|
| `Prepared` | the held call and its filed question, nothing authorized or sent | yes |
| `Authorized` | the approval decision recorded, nothing sent | yes |
| `DispatchStarted` | sending may have begun | declared, not cut yet |

A provider whose approval state lives in a store where truncation is not
meaningful (SQLite, say) reaches these points with a fault hook followed by a
real process kill, and declares that.

## 9. Conformance suite

Crate `cortexkit-role-tool-provider-conformance`. A provider runs it in its own
CI against its real module over a real route; never against a double.

| Case | Requires |
|---|---|
| `role_describe_shape`, `role_describe_cacheable` | — |
| `catalog_schemas_flat`, `catalog_reply_valid`, `catalog_modes_valid`, `catalog_emits_events_boolean`, `catalog_schema_digest_stable`, `catalog_digest_only`, `catalog_unknown_preset_refused` | — |
| `catalog_capability_filter_matches_describe` | `tool-catalog-capability-filter/v1` advertised in `role.describe` |
| `catalog_listed_presets_served` | `presets` listed in the live preset-less catalog answer |
| `call_mode_matches_catalog` | `call_modes` (restricted tool probe and routes with known bind modes) |
| `system_text_digests_match_text`, `system_text_preflight_digest_stable` | `system_text` |
| `catalog_disabled_tool_absent`, `call_disabled_tool_refused_by_name` | `disable_tool` |
| `terminal_frame_on_success`, `terminal_frame_on_refusal` | — |
| `tool_result_content_v1_valid` | a catalog tool declaring `result_format: "content/v1"` |
| `tool_result_held_reply_route_gating` | `held-reply/v1` suite fixture, `held_calls`, `late_results`, `scope_stamp`, `call_key`, `approval_execution`, a catalog tool declaring `content/v1` |
| `terminal_frame_on_cancel` | `cancellation` |
| `call_key_malformed_refused`, `call_key_well_formed_accepted` | `call_key` |
| `schema_pin_current_accepted`, `schema_pin_malformed_refused` | `schema_pin` |
| `withdraw_unknown_call`, `withdraw_answer_identical_on_repeat_and_owner`, `withdraw_owner_as_carrier_needs_no_carrier`, `withdraw_owner_without_carrier_refused`, `withdraw_carrier_naming_other_carrier_refused`, `withdraw_scope_mismatch_refused`, `withdraw_top_level_call_key_refused`, `withdraw_malformed_target_key_refused`, `withdraw_not_permitted_is_route_error` | `held_calls`, `scope_stamp`, `call_key` |
| `late_results_cursor_round_trip`, `late_results_ack` | `late_results`, `scope_stamp`, `held_calls`, `call_key`, `approval_execution` |
| `late_results_incarnation_change_refused` | `late_results`, `scope_stamp` |
| `crash_after_prepared_not_started`, `crash_after_authorized_not_started` | `approval_execution`, `held_calls`, `scope_stamp`, `call_key`, `late_results` |

The result case infers its `content/v1` requirement from the catalog, not
from an opt-out flag on the subject (the adapter a provider writes to run
this suite against itself). For every tool that declares `content/v1`, it
makes the calls the subject lists in `content_v1_calls` on a route with no
consumer declaration and on a route whose bind declares `content/v1`. The
first reply must not decode as `content/v1`; the second must decode and
validate, including the whole serialized body's size. The catalog must list
`result_format: "content/v1"` on both routes. By default `content_v1_calls`
is the subject's `quick_call`, a `(tool name, arguments)` pair that completes
promptly; a subject overrides it when other tools declare `content/v1`, and
may list several calls per tool. Subjects must open the declared route through
their real route-opening path. Missing probes for a declared tool fail by
tool name. With no declared tool, `tool_result_content_v1_valid` is skipped by
name with missing `content/v1`, never passed. Unknown result formats do not
opt in.

The `system_text` cases request full catalog answers. They check `item_digest`
against the returned text's exact UTF-8 bytes, both digests' 64-lowercase-hex
form, and that `tool_names` is present, sorted and deduplicated. Identical
requests must return the same `item_digest` and the same `preflight_digest`,
but the provider-defined `preflight_digest` need not equal the text digest.
There is no digest-only comparison of `preflight_digest`: a `digest_only`
answer carries only `generation` and `catalog_digest`, not `system_text`.

The `call_modes` case requires a restricted tool probe and a harness that
opens real routes with known bind modes. The subject supplies the call and
one known mode its tool does not list; the suite chooses a listed mode from
the catalog. The same body must be refused on the unlisted-mode route and
served on the listed-mode route. If no restricted tool fixture exists,
`call_mode_matches_catalog` is skipped with missing `call_modes`, never
passed. See the conformance crate's README for the subject hooks. The current
daemon stamps only known bind modes; unknown-mode refusal is checked in the
helper's unit tests instead.

The `catalog_capability_filter_matches_describe` case is gated by the live
`role.describe` advertisement, never a subject flag. It requests all of the
advertised `tool_capabilities` and checks that the returned catalog's tag union
matches exactly. It also checks that missing and empty lists select no tools,
filtering leaves `generation` unchanged, and a changed selection changes
`catalog_digest`. A provider that does not advertise the capability skips this
case by name.

The `catalog_listed_presets_served` case is gated by the live catalog, never
a subject flag: it runs when the answer to the subject's catalog request,
sent without `preset`, `digest_only` or `system_text`, lists `presets`. The
case checks that list with `check_presets`, then requests the catalog once
per listed preset. Each must be answered, not refused, and must carry no
`presets` list of its own; a refused preset fails the case by name. A
provider that lists no presets skips the case with missing `presets`.

Caller-side rules that no live provider can be made to exercise (an unknown
withdraw `answer`, an unknown late-result `kind`, the withdraw caller policy)
are tested against the vectors in this crate.

Verdict:

- A case whose requirements the subject does not declare is skipped, with the
  missing capabilities, never passed.
- **Passed**: every case ran and passed, and at least one kill ended a real
  process.
- **Conforming for declared capabilities** (naming the skipped tags): nothing
  failed, and every skip is a capability the subject does not declare. Never
  a plain pass.
- **Failed**: a case failed, or the run killed anything and no kill ended a
  real process (a run whose kills are all simulated cannot catch a provider
  that keeps something only in memory).
