# `extractor/v1`: role contract

Stability: **draft**. This document is the role definition; the Rust types
in this crate are its wire shapes, `test-vectors/extractor-v1/` holds its
vectors, and the sibling `cortexkit-role-extractor-conformance` crate holds
its suite (§11). The request side is pinned from Cloudflare's
documentation; the reply side (the envelope, every answer shape, spans,
billing) is pinned from a live probe of Clairic on 2026-10-22, whose
reply bodies are this crate's `provider-replies.json` (§10). §12 lists what
that probe left unmeasured.

Every rule is marked **[pinned]**: the role states it, a module must do it,
and a caller may rely on it. A point the provider's documentation leaves
open is never guessed: it is listed in §12 as not yet measured, and until it
is, a request that depends on it is refused. Accepting something later is an
additive change; refusing something the role used to accept is a breaking
one.

The batch rules `extractor/v1` shares with `classifier/v1` (idempotency,
single flight, stored outcomes, the spend ceiling and its estimate, the
error vocabulary, provider status mapping, images, number text) are stated
once, in `cortexkit-model-batch`'s
[`RULES.md`](../cortexkit-model-batch/RULES.md), cited here as "RULES §n".
They are pinned for this role as written there; this document states only
what is this role's own. Route roles and model discovery follow RULES §18.

## 1. Role identity

- [pinned] A module serving the role lists `extractor/v1` (`PROVIDES`) in
  its manifest's `capabilities.provides`, and serves `role.describe` and
  `extract.run` (`REQUIRED_OPS`), each a request `{method, params}` on its
  management route.
- [pinned] The module takes a batch of items and a set of typed fields. It
  makes one provider call per item, sending that item and every field of
  the batch, and returns the provider's answers untouched. It does no
  inference itself.
- [pinned] A caller opens a route for `extractor/v1` as RULES §18
  requires, then picks the module by **model id** using that section's
  discovery procedure, not by capability alone. The module serves only rows
  whose catalog `kind` is `extraction` (§6.8). A catalog id is as RULES §1
  states it.
- [pinned] Starting provider: Cloudflare Workers AI Clairic, catalog id
  `cloudflare-workers-ai/@cf/cloudflare/clairic`, called at `POST
  https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/run/@cf/cloudflare/clairic`
  with `Authorization: Bearer <scoped Workers AI token>`
  (`provider::clairic_endpoint`).

## 2. `role.describe`

- [pinned] The answer is (`RoleDescribe`):

  ```
  {
    role: "extractor/v1",
    limits: { max_items, max_request_bytes, max_instruction_bytes },
    models: [ {
      model,                         // catalog id
      max_items,                     // never above limits.max_items
      max_fields,                    // the module's cap; the provider states none
      field_types: [ "text", ... ],  // exactly the types extract.run accepts for this model
      pass_tokens,                   // the provider's per-pass budget (Clairic: 8192)
      max_answer_bytes,              // the module's bound on one encoded answer
      context_tokens?,               // absent where the provider states no total
      images: { max, max_bytes_each, max_bytes_total, formats } | null,
      price?: { input_per_mtok_usd?, output_per_mtok_usd? },
      egress_host
    } ]
  }
  ```

- [pinned] `images` is always present: `null` states that the model takes
  no images. `formats` is a subset of `image/png`, `image/jpeg`,
  `image/webp`.
- [pinned] A price direction the catalog states as free is `0`; a direction
  it does not price is absent (RULES §10).
- [pinned] `field_types` lists exactly what `extract.run` accepts for that
  model, so a caller validates a manifest against it at install time.
- [pinned] `max_instruction_bytes` bounds a field's `instructions` (§4).
- [pinned] `max_answer_bytes` bounds one answer: the module never forwards
  an answer larger than it. That item gets `answer_too_large` (permanent,
  stored, with `detail.field` naming `fields.<id>`, `detail.limit` the bound
  and `detail.size` the answer's size), so the reply-size estimate (§3) is a
  real upper bound and a batch can never fail on the frame limit after it
  was admitted.
- [pinned] `check_describe` refuses an answer with a zero limit, a model
  allowing more items than the module, an empty, unknown or repeated field
  type, a zero `pass_tokens`, `max_answer_bytes` or `context_tokens`, image
  limits it cannot meet, or no egress host.

## 3. `extract.run` request

- [pinned] The request is (`ExtractRequest`):

  ```
  {
    batch_id: string,                     // 1-128 printable ASCII; idempotency key (§6.1)
    model: string,                        // catalog id; never substituted
    fields: { <id>: Field },              // 1..model.max_fields, in the caller's order
    items: [ {
      state: string,                      // "" only when images are present
      images?: [ Image ],                 // 1..model.images.max
      reference_date?: "YYYY-MM-DD"       // anchors dates written without a year, per item
    } ],                                  // 1..model.max_items, answered in this order
    options?: { quantiles?: bool, state_text?: bool },   // default false, false
    max_cost_usd?: number                 // batch ceiling (RULES §5)
  }
  ```

- [pinned] Every object of the request is closed. A member the contract
  does not name is refused `invalid_params` naming its path (`<member>`,
  `items[i].<member>`, `options.<member>`, `fields.<id>.<option>`), so a
  misspelt member is never silently dropped.
- [pinned] `state` is a string; a non-string is refused naming
  `items[i].state`. An empty `state` with no images is refused the same
  way: the provider requires a non-empty state unless images are sent.
- [pinned] `Image` is the shared item image shape (RULES §13). The module
  checks the decoded sizes against the model's `images` (Clairic: 4 MiB
  each, 8 MiB in total) before any call. `images: []` is refused naming
  `items[i].images`.
- [pinned] `reference_date` is a real calendar date, `YYYY-MM-DD`, years
  0001 to 9999 (`is_calendar_date`); `2026-02-30` or any other non-date is
  refused naming `items[i].reference_date`.
- [pinned] The provider request for each item is checked against the
  provider's body limit (Clairic: 16 MiB) before any call; an oversize one
  is refused naming `items[i]` with the limit
  (`provider::check_provider_bodies`).
- [pinned] **Reply size.** With `options.state_text` the reply echoes each
  item's state text, so admission refuses a batch whose estimated reply would
  not fit the daemon's 4 MiB module-to-client frame (`REPLY_FRAME_BYTES`,
  4,194,304 bytes), before any call: `invalid_params` naming `items`, with
  `detail.limit` the frame size (`check_reply_size`). The estimate
  (`estimated_reply_bytes`) is

  ```
  512 + json(batch_id) + json(model)
    + per item: 1024 + json(state) when options.state_text
                + per field: json(id) + 1 + max_answer_bytes
  ```

  where `json(s)` is the JSON-escaped length of `s`, quotes included; 512
  and 1024 are allowances for the reply's and each item's own members, an
  item error included.
- [pinned] `options.quantiles` maps to Clairic's `include_quantiles`, and
  `options.state_text` to `return_state_text`. They apply to every item.
- [pinned] Truncation: Clairic reports it (`truncated`), and the role
  forwards it per item (§5). The module never refuses a `state` by a byte
  heuristic; the one exception the shared estimate makes is with an exact
  token count (RULES §5, §6.9).

## 4. Fields

`Field` is `{type, instructions?, ...options}` (`field` module), forwarded to
the provider under its own key names, in the caller's order.

- [pinned] A field id matches `^[A-Za-z_][A-Za-z0-9_-]{0,63}$`
  (`is_field_id`): the provider's id grammar without `.`, because
  `detail.field` paths are dotted (`fields.<id>.type`) and a dot in an id
  would make them ambiguous. Every role id is a valid provider id.
- [pinned] `type` is one of `text`, `number`, `date`, `time`, `datetime`,
  `duration`, `list`, `estimate`, `forecast`, and one of the model's
  `field_types`. Anything else is refused naming `fields.<id>.type`,
  checked before the rest of the field. In a reply, types decode open (§5).
- [pinned] `instructions`, when present, is a non-empty string of at most
  `limits.max_instruction_bytes` bytes. Clairic accepted 20,000 characters
  (probe 10); its own bound above that is §12's.
- [pinned] The role never reorders fields.
- [pinned] Options, with the constraints the provider's schema states. An
  option outside this table, or on a type the table doesn't list for it, is
  refused naming `fields.<id>.<option>`:

  | option | types | value |
  |---|---|---|
  | `integer` | number, duration | bool |
  | `decimals` | number, duration, list (with `items: number`) | integer 0–6 |
  | `unit` | number, duration | a non-empty string; for duration one of `seconds minutes hours days weeks months years` |
  | `items` | list (required) | `text`, `number` or `date` |
  | `min`, `max` | estimate, forecast (both required) | number, or on an estimate an ISO date `YYYY-MM-DD` (both or neither); `min < max` |
  | `step` | estimate, forecast | positive number; on a date estimate `month` (required there) |
  | `scale` | estimate, forecast | `linear` or `log`; `log` requires `min > 0`; not on a date estimate |
  | `null_threshold` | all | number in (0, 1] |
  | `horizon` | forecast (required) | integer 1–64 |
  | `interval` | forecast (required) | `step minute hour day week month quarter year` |

- [pinned] A field is checked in this order (`check_field`): its id; its
  type; `instructions`; each option in the order written, its name then its
  value; each option its type requires, in the table's order; then the
  options together: `min` and `max` both numbers or both dates (naming
  `max`); for dates, `min < max` (naming `max`), `step: month` (naming
  `step`) and no `scale` (naming `scale`); for numbers, `min < max`
  (naming `max`), `scale: log` with `min > 0` (naming `scale`) and no
  `step: month` (naming `step`); and `decimals` on a list only with
  `items: number` (naming `decimals`). Clairic applies `decimals` to a list
  of numbers (probe 03).
- [pinned] Required-ness of `horizon` and `interval` is the role's choice:
  the provider documents no default for either.
- [pinned] A date estimate is an `estimate` with `min` and `max` as ISO
  dates and `step: month`, the one step it was measured with (probe 04);
  its answer is ISO dates throughout (§5). The `year` step, and date bounds
  on a `forecast`, are refused until they are measured (§12).

## 5. Reply and answers

The reply is (`ExtractReply`):

```
{
  batch_id, model,
  items: [ {
    index: n,
    answers?: { <id>: Answer },          // present iff the item succeeded
    truncated?: bool,                    // the provider's flag, iff it succeeded
    state_text?: string,                 // iff options.state_text and the item succeeded
    error?: { code, class, message, retry_after_ms?, detail? },
    usage?: { input_tokens?, output_tokens?, state_tokens?, image_tokens?, schema_tokens?, passes? }
  } ],
  usage: { input_tokens?, output_tokens? },   // summed over answered items
  cost_usd?: number                            // the batch's cumulative cost; input-priced only
}
```

- [pinned] Items come back in request order; `items[i].index == i`.
- [pinned] `Answer` is the provider's answer object as returned. It always
  has `type`. `value` is `null` when the input doesn't determine it
  ("insufficient", not "absent"), and `p_null`, when present, is the
  provider's probability of that.
- [pinned] The members this role names are `value`, `unit`, `guess`,
  `p_null`, `confidence`, `confidences`, `span`, `spans`, `mean`,
  `median`, `p10`, `p90`, `p_below`, `p_above`, `quantiles` and `steps`
  (`ANSWER_MEMBERS`). Each is kept as the provider's own text, a
  `Box<RawValue>` per member (RULES §12), so a number reaches the reply
  with the provider's spelling. A member this role does not name is kept as
  given (`Answer::extra`), and an unknown `type` keeps its string: decoding
  stays open. An encoded answer writes `type`, the named members in that
  order, then the others.
- [pinned] Per-type shapes, as Clairic returned them (probe 01, which asked
  for every type with `include_quantiles` and `return_state_text`; members
  in the provider's order):

  | type | members | `value` |
  |---|---|---|
  | `text` | `type, p_null, span, guess, confidence, value` | string, or `null` |
  | `number` | `type, p_null, guess, confidence, value` | number |
  | `date` | `type, p_null, guess, confidence, value` | `"2026-03-14"` |
  | `time` | `type, p_null, guess, confidence, value` | `"09:15:00"` |
  | `datetime` | `type, p_null, guess, confidence, value` | `"2026-03-06T16:40:00"`, no offset |
  | `duration` | `type, p_null, unit, guess, confidence, value` | number, in `unit` (`"minutes"`) |
  | `list` | of text: `type, value, spans, confidence`; of numbers: `type, value, confidences, confidence`, one confidence per element and no `spans` (probe 03) | array; empty when nothing matches, never null (Cloudflare's documentation) |
  | `estimate` | `type, value, mean, median, p10, p90, quantiles, p_below, p_above` | number; no `p_null` and no `confidence` |
  | `forecast` | `type, value, median, p10, p90, steps` | array, one per step |

  - `quantiles`, when requested, is 99 values in nondecreasing order; the
    10th, 50th and 90th equal `p10`, `median` and `p90`, as the 1st to the
    99th percentiles would (probes 01 and 04).
  - A `forecast`'s `value`, `median`, `p10` and `p90` are arrays parallel to
    `steps`, one entry per `horizon` step. Each `steps[]` entry is shaped
    like an estimate without `type`: `{value, mean, median, p10, p90,
    quantiles, p_below, p_above}`.
  - A date estimate (§4) answers ISO dates in every statistic and every
    quantile: `value`, `mean`, `median`, `p10`, `p90` and the 99
    `quantiles` (probe 04).
- [pinned] **Spans** count Unicode scalar values of the state, and `end` is
  exclusive: `span: [start, end]`, and `spans` an array of them. Probe 02
  put a precomposed é, an emoji outside the Basic Multilingual Plane and
  four CJK characters before its target, and Clairic answered `[53,61]`:
  65 in UTF-8 bytes, 54 in UTF-16 units. Spans are forwarded untouched;
  the role never converts offsets. `reply::span_byte_range` turns one span
  into a UTF-8 byte range of the state for a caller that wants to slice
  it, `None` when the span is malformed or past the state's end.
- [pinned] `state_text`, when requested, is the caller's `state` byte for
  byte (probes 01 and 02), so a span indexes into the caller's own
  `state`. The reply-size estimate (§3) counts it as the caller's `state`.
- [pinned] `usage` members are forwarded as given (RULES §10). `cost_usd`
  is `input_tokens` x the catalog input price: output is stated free, so it
  adds nothing, and an answered item without reported `input_tokens` makes
  `cost_usd` absent (`absent::input_cost_nanos`).

## 6. Pinned rules

The shared batch rules apply as RULES §2 to §11 state them. The subsections
say what is this role's own.

### 6.1 Idempotent by `batch_id`

- [pinned] As RULES §2. **Body identity** is the model, the fields, the
  options in force, and each item (its images in canonical form, its
  `reference_date` included) (`BatchIdentity`). The options are hashed as in
  force (`EffectiveOptions`), so an absent option and the same option sent
  as `false` are one batch; they are part of the identity because they
  change what the provider answers.
- [pinned] `detail.field` of `batch_id_reuse` names the first part that
  differs: `model`, `fields`, `options`, `items` (a different item count) or
  `items[i]`, checked in that order (`BatchIdentity::first_difference`).
- [pinned] Fields are hashed as canonical JSON, so the order fields are
  written in is not part of the identity. The provider agrees: probe 07
  sent probe 05's 40 fields in reverse order and got the same answers and
  the same usage.

### 6.2 Single flight per `batch_id`

- [pinned] As RULES §3.

### 6.3 Stored outcomes on a re-send

- [pinned] As RULES §4.

### 6.4 `max_cost_usd` is a batch ceiling

- [pinned] As RULES §5, with the shared estimate (§6.9).

### 6.5 `cost_usd` is the batch total

- [pinned] As RULES §6, input-priced (§5).

### 6.6 Never all-or-nothing after admission

- [pinned] As RULES §7: each item gets `answers` or `error`, never both.

### 6.7 Validation before any provider call

- [pinned] Admission checks, in this order (`admit`), each refusal writing
  nothing: the request size against `max_request_bytes` (`params`); the
  decode, naming the first malformed member (`parse_request`); the catalog
  row (§6.8); the batch id, the field count, each field (§4), the item
  count, each item's state, images and `reference_date`, and
  `max_cost_usd` (`check_request`); each provider request's size
  (`check_provider_bodies`); and the reply size (`check_reply_size`).
- [pinned] Admission refuses, before any provider call, the three requests
  Clairic was measured to refuse (probes 08, 09 and 11; their bodies are in
  `provider-replies.json`):
  - a field of unknown type: `invalid_params` naming `fields.<id>.type`
    (§4). Clairic answers HTTP 400, Workers AI code 5006;
  - an option the §4 table does not list for the type, such as
    `decimalz`: `invalid_params` naming `fields.<id>.<option>`. Clairic
    refuses an unknown option with HTTP 422, code 3030, rather than
    ignoring it, so the role refuses it too;
  - an empty `state` with no images: `invalid_params` naming
    `items[i].state` (§3). Clairic answers HTTP 422, code 3030.

  The provider's codes stay as a backstop for a module that sends such a
  request anyway: neither 5006 nor 3030 is in the Workers AI table, so
  both map by status (400, 422) to `invalid_item`, permanent (§8).
- [pinned] `detail.field` paths: `batch_id`, `model`, `fields`,
  `fields.<id>`, `fields.<id>.type`, `fields.<id>.instructions`,
  `fields.<id>.<option>`, `items`, `items[i]`, `items[i].state`,
  `items[i].images`, `items[i].images[j]`, `items[i].reference_date`,
  `items[i].<member>`, `options`, `options.<member>`, `max_cost_usd`, a
  top-level `<member>`, and `params` for the request as a whole, under
  RULES §8.

### 6.8 Model pinned

- [pinned] As RULES §9, with three cases (`check_catalog_row`,
  `CatalogRow`): no row is `model_unknown`; a row whose `kind` is not
  `extraction` is `model_not_extractor` with `detail.model` and
  `detail.kind`; a row that states no kind is `model_not_extractor` with
  `detail.model` only, never an invented kind. A model whose row is an
  extraction model but which `role.describe` does not list is
  `model_unknown`: this module does not serve it. A caller does not send such
  a model to this module (RULES §18); when no module lists the model, the
  caller refuses `model_unserved` before any call.

### 6.9 Spend guard

- [pinned] The spend guard works per item, before each item's provider
  call, with the shared conservative estimate of RULES §5 (`spend` module):

  ```
  input tokens = schema + max_passes x (state + images x image_tokens_each + overhead)
  cost         = input tokens x input price
  max_passes   = the number of fields in the request
  ```

  `state` is the item's state and `schema` the canonical JSON of the
  batch's fields, each the byte bound unless the module passes exact
  counts. `image_tokens_each` and `overhead` are the module's bounds
  (`ModuleBounds`), required. The rule for `max_passes` is the role's:
  Clairic runs a request's fields in one or more passes, never more passes
  than fields, and bills the schema once and the state on every pass, with
  no fixed overhead. Probe 05 sent 40 text fields: 40 passes,
  `input_tokens` 29,888 = `schema_tokens` 26,488 + 40 x `state_tokens` 85.
  Probe 01 sent nine fields of mixed types: two passes, 1,150 = 776 + 2 x
  187. The per-pass budget (`pass_tokens`) does not predict the passes: it
  predicts 4 for probe 05. Images and overhead are charged on every pass,
  which can only overstate them (§12).
- [pinned] An item that would cross the effective ceiling gets the
  per-item error `cost_exceeded` and is not sent (`check_before_call`). A
  state counted exactly at or past `pass_tokens` gets `invalid_item`. A
  state whose byte bound reaches `pass_tokens` is not refused for it: it
  is estimated like any other state, and is sent while the estimate stays
  within the ceiling.
- [pinned] `cost_exceeded` is never an admission refusal in this role: a
  batch whose items all cross the ceiling is answered with an item error on
  each. `account_walled` is: the module's own guard knows the account is at
  its limit.

### 6.10 Absent is not zero

- [pinned] As RULES §10 (`sum_usage`).

### 6.11 Synchronous, bounded

- [pinned] As RULES §11.

## 7. Error codes

The vocabulary, the refusal and item error shapes, every code's meaning,
detail and class, and which codes are stored and stop the call are RULES
§14 (`errors`). `extractor/v1` answers with these codes:

- [pinned] Admission refusals (`ADMISSION_CODES`): `invalid_params`,
  `batch_id_reuse`, `model_unknown`, `model_not_extractor`,
  `account_walled`, `batch_in_progress`, and, when the call's first provider
  call meets one, `auth_failed`, `model_unavailable`,
  `account_limit_exhausted`, `account_not_entitled`.
- [pinned] Per-item errors (`ITEM_CODES`): `rate_limited`,
  `provider_error`, `provider_refused`, `invalid_item`, `cost_exceeded`,
  `answer_too_large`, `auth_failed`, `model_unavailable`,
  `account_limit_exhausted`, `account_not_entitled`.

## 8. Provider status mapping

- [pinned] Clairic is a Cloudflare Workers AI model: its errors map by
  Workers AI code first, then by status (RULES §15, the second table;
  `provider::workers_ai_item_code`). In particular 413 `3006` is
  `invalid_item`, and 408 `3007` and `3008` are `provider_error`. The two
  codes Clairic was measured to answer a bad request with are not in the
  table, so the status decides: 400 `5006` (unknown field type) and 422
  `3030` (unknown option, empty state) are `invalid_item`, permanent. The
  item error's `message` carries the status, the code and the provider's
  message (RULES §15).
- [pinned] The reply envelope is `{result, success, errors: [{message,
  code}], messages}` (`Envelope`), as Clairic was measured to send it:
  - a success: HTTP 200, the members in the order `result, success,
    errors, messages`, with `success: true`, `errors: []`, `messages: []`,
    and the answer object under `result` (§9);
  - an error: the members in the order `errors, success, result,
    messages`, with `success: false`, `messages: []`, and `result: {}`, an
    empty object, not `null`. No error reply carries usage.

  The module reads the envelope whatever its member order, takes an empty
  `result` object (or `null`) as no result, and never forwards the
  envelope. `provider::read_reply` reads one call's status and body: a 2xx
  with `success: true` and a result is answered (`ProviderResult::into_item`),
  any other status is the mapped item error, and a 2xx without a usable
  result is `provider_error`.

## 9. Provider shape (Clairic)

- Request per item (`ProviderRequest::for_item`): `{model:
  "@cf/cloudflare/clairic", state, images?, reference_date?, fields,
  options: {include_quantiles, return_state_text}}`, with `fields` passed
  through key by key and both options always sent.
- Reply: the envelope's `result`, `{model, answers, truncated, usage,
  state_text?}` in that order (`ProviderResult`): `model` is the
  provider's own name (`"clairic"`), not forwarded; `state_text` is present
  when `return_state_text` asked for it; `usage` is `{state_tokens,
  image_tokens, schema_tokens, input_tokens, output_tokens, passes}`, with
  `output_tokens` 0 on every measured reply.
- Billing, exact on every measured success: `input_tokens = schema_tokens
  + passes x state_tokens + image_tokens`. Passes follow the fields, at
  most one per field (§6.9).
- Limits: 8,192 tokens per pass (`clairic_limits::PASS_TOKENS`), the
  documented budget, which the role uses only to refuse an exactly counted
  state that cannot fit; images 4 MiB each and 8 MiB in total; a request
  body of at most 16 MiB.

## 10. Sources

- Cloudflare Workers AI, Clairic model page, with its input and output
  schemas: <https://developers.cloudflare.com/workers-ai/models/clairic/>
- Cloudflare Workers AI, errors:
  <https://developers.cloudflare.com/workers-ai/platform/errors/>
- A live probe of `@cf/cloudflare/clairic`, 2026-10-22: twelve
  requests, each reply captured byte for byte. The bodies and statuses are
  `provider-replies.json` (no headers); each reading this contract pins
  from it names its probe:
  - `01-all-types`: every answer type with quantiles and state text (§5);
    nine fields in two passes (§6.9);
  - `02-span-offsets`: span unit and exclusive end, `state_text` equal to
    the state (§5);
  - `03-list-decimals`: `decimals` applied to a list of numbers, and
    `confidences` (§4, §5);
  - `04-date-estimate`: a date estimate with the `month` step (§4, §5);
  - `05-multi-pass`, `06-one-pass-control`, `07-multi-pass-reversed`: 40
    fields in 40 passes, the state billed per pass, no fixed overhead, and
    field order changing neither answers nor usage (§6.1, §6.9);
  - `08-unknown-type`, `09-unknown-option`, `11-empty-state`: the error
    envelope and codes 5006 and 3030 (§6.7, §8);
  - `10-long-instructions`: 20,000 characters of `instructions` accepted
    (§4);
  - `12-one-image`: one 64x64 PNG, `image_tokens` 64 and 41 more
    `schema_tokens` (§12).

## 11. Vectors and conformance

- `test-vectors/extractor-v1/` holds the role's vectors, pinned by its
  `SHA256SUMS` file. They ship in this crate's package, and the
  `test-vectors` feature compiles them into the `vectors` module. Each is
  decoded and re-encoded byte for byte by `tests/vectors.rs`, which needs
  `--features test-vectors`, then checked against the code:
  - `role-describe.json`: one model, Clairic, with stated-free output and no
    context;
  - `requests.json`: admitted requests using all nine field types, an
    image with an empty state, and a date estimate, each item with the
    Clairic request it maps to;
  - `field-paths.json`: every `invalid_params` admission builds, naming the
    narrowest path;
  - `admission-refusals.json`: one refusal per admission code, and the
    kindless catalog row;
  - `identity.json`: replays (two image spellings, absent options, a
    different `max_cost_usd`, another field order) and each differing part;
  - `reply-size.json`: a batch whose estimated reply is exactly the frame,
    admitted, and one byte over, refused;
  - `spend.json`: the pre-call check, including an exact state that cannot
    fit, a byte-bound state past the pass budget under a ceiling and
    without one (both estimated and sent), and a batch whose three fields
    bill the state three times;
  - `provider-replies.json`: every probe reply (§10), its status and body
    byte for byte, each read into the item a module answers with, with the
    probe's billing and answer shapes checked against this contract;
  - `answer-too-large.json`: an answer over `max_answer_bytes` as a stored
    item error.
- The conformance suite (`cortexkit-role-extractor-conformance`) drives a
  live module against a call-counting stand-in provider: its request side,
  and its reply side with the stand-in replaying probe replies; its
  `README.md` lists the checks.

## 12. Open readings

Each is not yet measured. Until it is, the role states nothing about it,
and where a request would depend on it, the request is refused.

1. The `year` step of a date estimate, and date bounds on a `forecast`;
   both are refused (§4).
2. Whether images bill once or on every pass. Images were measured in a
   one-pass request only; the estimate charges them on every pass (§6.9).
3. The tokens an image larger than 64x64 costs. One point is measured: a
   64x64 PNG reported `image_tokens` 64 and raised `schema_tokens` by 41
   (probe 12). The module supplies `image_tokens_each` as a bound covering
   both.
4. Which image spellings Clairic accepts (a `data:` URL, the object form,
   or both), and how many images an item may carry; the module forwards an
   image as the caller wrote it.
5. The provider's own bound on `instructions` above 20,000 characters,
   which it accepted (probe 10); `limits.max_instruction_bytes` is the
   module's.
