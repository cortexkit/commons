# `classifier/v1`: role contract (draft)

Owner: **BROCA**. Stability: **alpha, draft**. Nothing ships against it until
ALF and Ufuk approve it. This document is the role definition; the Rust types
in this crate are its wire shapes, `tests/vectors/` holds its vectors, and
the sibling `cortexkit-role-classifier-conformance` crate holds its suite
(§11).

It is written from BROCA's draft (`classifier-v1-contract.md`) and BROCA's
rulings on the points the draft left open (stored errors on a re-send, the
batch spend ceiling, concurrent calls, the meaning of `noul`, the type of
`state`, and truncation). The provider shapes are from the providers'
documentation (§10).

Every rule is marked:

- **[pinned]**: the role states it. A module must do it, and a caller may
  rely on it.
- **[crate]**: the draft is silent, and this crate writes one reading down
  so the types and vectors have something to pin. The owner may change it;
  each is listed in §12.

## 1. Role identity

- [pinned] A module serving the role lists `classifier/v1` (`PROVIDES`) in
  its manifest's `capabilities.provides`. It is a valid subc capability
  identifier (`subc_protocol::manifest::is_valid_capability_identifier`).
- [pinned] The module serves `role.describe` and `classify.run`
  (`REQUIRED_OPS`), each a request `{method, params}` on its management
  route.
- [pinned] The module takes a batch of items and a set of questions, calls
  one hosted classifier model for each item, and returns the provider's
  answers untouched. It does no inference itself. Local inference, when it
  exists, is another module under the same capability.
- [pinned] A caller picks the module by **model id**, through the catalog
  row's serving module, not by capability alone.
- [pinned] Starting providers: Cloudflare Workers AI `@cf/cloudflare/clef`
  (catalog ids `cloudflare/clef`, `cloudflare/clef-flash`) and TypeSafe
  `jev-latest` (`typesafe/jev-latest`, `POST
  https://api.typesafe.ai/v1/systemone`). Each provider call takes one
  item's `state` with every question of the batch.

## 2. `role.describe`

- [pinned] The answer is (`RoleDescribe`):

  ```
  {
    role: "classifier/v1",
    limits: { max_items, max_request_bytes },
    models: [ {
      model,                      // catalog id
      max_questions, max_items,   // per model: the tighter of catalog and module limits
      context_tokens,             // from the catalog; the provider truncates state beyond it
      images: { max: n } | null,  // null = no image support; always stated
      price?: { input_per_mtok_usd?, output_per_mtok_usd? },  // absent if unpriced
      egress_host                 // the provider host item text is sent to
    } ]
  }
  ```

- [pinned] It is resolved from the module's catalog plus its own routing
  pins, so a caller validates a manifest at install time against what this
  module will actually do, and can name the outside host on a card.
- [pinned] A model's `max_items` is never above `limits.max_items`
  (`check_describe`).
- [pinned] `images` is always present: `null` states that the model takes no
  images. A price direction the catalog does not price is absent, never
  zero.

## 3. `classify.run` request

- [pinned] The request is (`ClassifyRequest`):

  ```
  {
    batch_id: string,            // required; 1-128 printable ASCII; the idempotency key
    model: string,               // required; catalog id "<provider>/<model>"; never substituted
    questions: { <id>: Question },  // 1..model.max_questions; ids 1-64 [a-z0-9_-]
    items: [ { state, images? } ],  // 1..model.max_items, answered in this order
    max_cost_usd?: number        // optional spend ceiling for the batch (§6.4)
  }
  ```

- [pinned] `state` is a string, an object or an array, forwarded to the
  provider verbatim. Null, numbers and booleans are refused
  `invalid_params` with `detail.field` `items[i].state` (the real index).
- [pinned] `images` is accepted only where the model's catalog row allows
  it, up to its `images.max` (clef: at most 4). TypeSafe takes text only.
- [pinned] Truncation: the provider truncates a `state` longer than the
  model's `context_tokens` without saying so. Neither provider reports
  truncation, so the reply carries no truncation flag: a guessed flag would
  read as a measurement. A caller keeps `state` under `context_tokens`; the
  per-item `usage.input_tokens`, when the provider reports it, is how the
  caller sees an item ran near the limit. The module does not refuse a
  `state` by a byte heuristic.

## 4. Questions

`Question` is the providers' shared shape, forwarded as the caller wrote it,
with the providers' own field names (`question` module):

- [pinned] Every question has `type` (`"noul"`, `"choice"` or `"score"`)
  and `instructions` (a non-empty string, or an object or array).
- [pinned] `noul`: optional `criteria: {"true": string, "false": string}`.
- [pinned] `choice`: required `criteria`, an object from option id to its
  description (a string, object, array or null), with 2 to 255 options.
- [pinned] `score`: required `criteria`, an array of level descriptions,
  lowest first, indexed from 0, with 2 to 10 levels. The request has no
  levels or legend field.
- [pinned] Images: a `data:` URL string, or `{content_type, base64}` with
  `content_type` one of `image/png`, `image/jpeg`, `image/webp`. At most 4
  MiB and 16 megapixels each, 8 MiB in total. No remote URLs.
- [crate] Question type decodes open: an unknown type is kept as its string
  and refused `invalid_params` naming `questions.<id>.type`.
- [crate] The role's question id (`[a-z0-9_-]`, 1-64) is narrower than
  clef's (letters, digits, `_`, `.` and `-`, up to 100), so every role id is
  a valid clef id.

## 5. Reply and answers

- [pinned] The reply is (`ClassifyReply`):

  ```
  {
    batch_id, model,
    items: [ {
      index: n,                         // position in the request
      answers?: { <id>: Answer },       // present iff the item succeeded
      error?: { code, class, message, retry_after_ms? },  // present iff it failed
      usage?: { input_tokens?, output_tokens? }
    } ],
    usage: { input_tokens?, output_tokens? },   // summed over answered items
    cost_usd?: number                           // the BATCH's cumulative cost
  }
  ```

- [pinned] Items come back in request order; `items[i].index == i`.
- [pinned] `Answer` is the provider's answer object as returned:
  - `noul`: `{type, noul}`. `noul` is the probability that the answer is
    yes, from 0 (no) to 1 (yes) (providers' docs: "Probability the answer is
    yes").
  - `choice`: `{type, choice, probabilities: {<option>: number},
    confidence}`.
  - `score`: `{type, score, legend: {"0": desc, ...}, probabilities: {"0":
    number, ...}, confidence}`. `score` is fractional, from 0 to levels - 1,
    and can land between levels.
- [pinned] The module never rounds, rescales or renames these numbers,
  because callers threshold on them: each number reaches the reply with the
  provider's spelling. A module that decodes and re-encodes a provider
  answer through binary floating point breaks this; forward the answer
  object's text.
- [crate] `Answer` keeps a member the provider adds that this crate does not
  name (`Answer::extra`), so a new provider field passes through.

## 6. Pinned rules

### 6.1 Idempotent by `batch_id`

- [pinned] The module records the admitted batch (model, frozen catalog
  row, body identity) and each item's outcome durably, before replying.
- [pinned] The same `batch_id` with the same body returns the stored
  answers, and calls the provider only for items that have none.
- [pinned] The same `batch_id` with a different body is refused
  `batch_id_reuse`, with `detail.field` naming the first differing field.
- [pinned] **Body identity** is the model, the questions, and each item,
  the questions and every item hashed as RFC 8785 (JCS) canonical JSON
  (`BatchIdentity`). The key order of an object `state`, whitespace, and
  number spellings JSON treats as equal (`1.0` and `1`, `1e30` and `1E+30`)
  can't turn a retry into `batch_id_reuse`.
- [crate] `detail.field` of `batch_id_reuse` is `model`, `questions`,
  `items` (a different item count) or `items[i]` (the first differing
  item), checked in that order (`BatchIdentity::first_difference`).

### 6.2 Single flight per `batch_id`

- [pinned] While one call holds a batch, a second call with that `batch_id`
  is refused `batch_in_progress` (transient, with `detail.retry_after_ms`),
  and writes and dispatches nothing. That covers a caller retrying after its
  own timeout while the first call still runs.
- [pinned] The caller re-sends after the first call ends, and gets the
  stored answers. No item is ever sent to the provider twice at once, and no
  call hangs waiting on another.

### 6.3 Stored errors on a re-send

- [pinned] A stored permanent item error (`provider_refused`,
  `invalid_item`, `cost_exceeded`) is returned as stored, with no provider
  call.
- [pinned] A stored transient item error (`rate_limited`,
  `provider_error`) is retried, and its new outcome (answer or error)
  replaces the stored one.
- [pinned] A stored answer is never re-asked.
- [pinned] An item error of an unknown class is treated as permanent, so it
  is returned as stored (`ItemError::is_retried_on_resend`).

### 6.4 `max_cost_usd` is a batch ceiling

- [pinned] It is recorded at first admission, and spend already recorded
  for the batch counts against it.
- [pinned] It is not part of the body identity: a re-send carrying a
  different value is not `batch_id_reuse`.
- [pinned] The effective ceiling is the lower of the recorded one and the
  re-send's (`effective_ceiling`): a caller can tighten it, never loosen it.
- [pinned] Before each provider call the module checks recorded spend plus
  that call's estimate. An item that would cross the ceiling gets the
  per-item error `cost_exceeded` (permanent; a re-send with a higher value
  cannot raise the ceiling, so it cannot clear it).

### 6.5 `cost_usd` is the batch total

- [pinned] `cost_usd` is the batch's cumulative cost, the same on every
  reply to the batch, a replay included. It is present whenever the model is
  priced, and covers every answered item of the batch, whichever call
  answered it.
- [pinned] A caller sets the batch's charge to this value rather than adding
  it, so a crashed and replayed run is charged exactly once.

### 6.6 Never all-or-nothing after admission

- [pinned] Each item gets `answers` or `error`, never both. A whole-batch
  refusal happens only at admission, and writes nothing.

### 6.7 Validation before any provider call

- [pinned] Checked against the catalog row before anything is written or
  sent: question count, question ids, choice options 2-255, score levels
  2-10, `state` kinds, images, `max_items`, `max_request_bytes`
  (`parse_request`, `check_request_bytes`, `check_request`).
- [crate] `detail.field` paths: `batch_id`, `model`, `questions`,
  `questions.<id>`, `questions.<id>.type`, `questions.<id>.instructions`,
  `questions.<id>.criteria`, `items`, `items[i]`, `items[i].state`,
  `items[i].images`, `items[i].images[j]`, `max_cost_usd`, and `params` for
  the request as a whole (malformed JSON, or over `max_request_bytes`).
  `detail.limit` is the bound broken, where there is one.

### 6.8 Model pinned

- [pinned] Refused `model_unknown` if absent from the catalog, and
  `model_not_classifier` if the row's `kind` isn't `classifier`
  (`check_catalog_row`). Never substituted.

### 6.9 Spend guard

- [pinned] An estimate of items x input tokens x catalog price above
  `max_cost_usd`, or an account at its limit, is refused before dispatch
  (`cost_exceeded`, `account_walled`).

### 6.10 Absent is not zero

- [pinned] Usage the provider didn't report stays absent, per item and in
  the batch sum (`sum_usage`). A price direction the catalog doesn't price
  is absent too.

### 6.11 Synchronous, bounded

- [pinned] `max_items` is chosen so a batch fits the call deadline. A caller
  whose call timed out sends the same `batch_id` again and gets every answer
  recorded so far, with the rest completed.

## 7. Error codes

Codes are open strings on the wire (`errors` module); a code this crate does
not list is kept as sent.

Admission refusals, which write nothing. [pinned] An admission refusal is an
`ERROR` frame whose body is `{code, message, detail}` (`Refusal`).
[crate] `detail.class` carries the code's class, so a caller that meets an
unknown code still knows whether to retry.

| Code | When | Detail | Class |
|---|---|---|---|
| `invalid_params` | malformed request or a limit exceeded | `field`, `limit?` | permanent |
| `batch_id_reuse` | same key, different body | `field` | permanent |
| `model_unknown` | model not in the catalog | `model` | permanent |
| `model_not_classifier` | catalog row is not a classifier | `model`, `kind` | permanent |
| `cost_exceeded` | estimate over `max_cost_usd` | `estimate_usd`, `max_cost_usd` | permanent |
| `account_walled` | the account is at its limit | `resets_at_ms?` | transient |
| `batch_in_progress` | another call holds this `batch_id` | `retry_after_ms` | transient |

Per-item errors (`ItemError`):

| Code | When | Class |
|---|---|---|
| `rate_limited` | provider 429 or 529 after retries; `retry_after_ms` when given | transient |
| `provider_error` | provider 5xx or a transport failure after retries | transient |
| `provider_refused` | the provider refused the content | permanent |
| `invalid_item` | the provider rejected this item's input (4xx) | permanent |
| `cost_exceeded` | answering this item would cross the batch ceiling | permanent |

- [pinned] `class` is one of the shared classes, and the set is fixed for
  `classifier/v1`: `transient` or `permanent`. A new class means a new role
  version. A caller that meets an unknown value treats it as `permanent`,
  the safe direction (`ErrorClass::effective`).

## 8. Provider status mapping

- [crate] Neither provider documents an error body, so the module maps a
  provider's HTTP status, once its own retries are spent
  (`item_code_for_status`):
  - 429 (clef's codes 3036 account-limited and 3040 out-of-capacity;
    TypeSafe's rate limit) and 529 (TypeSafe's Overloaded): `rate_limited`;
  - 408 (clef's timeout) and every 5xx: `provider_error`;
  - every other 4xx (clef's 400, 403, 404 and 413; TypeSafe's 401 and 422):
    `invalid_item`.
- [crate] Neither provider documents a content-refusal status, so nothing
  maps to `provider_refused` yet; it is reserved for a provider that reports
  one.

## 9. Provider shapes

Both providers share one shape; Cloudflare calls clef "fully Jev-API
compatible" (`provider` module).

- Request: `{model, state, questions, images?}`. clef takes `model: "clef"`
  (or `"clef-flash"`); TypeSafe takes `"jev-latest"`. The module maps the
  catalog id to the provider's own value. `state` is a string or structured
  data (an object or array); the provider truncates a long state silently.
  `questions` maps id to question (clef: 1 to 64 entries). `images` is clef
  only: at most 4, each a `data:` URL or `{content_type, base64}`; 4 MiB and
  16 megapixels each, 8 MiB in total, a body of at most 13 MiB, no remote
  URLs.
- Response: `{model, answers: {<id>: Answer}, usage: {input_tokens,
  output_tokens}}`.
- Errors: clef (generic Workers AI) answers 400, 403 and 404 for bad input
  or model, 408 for a timeout, 413 for too large, and 429 for codes 3036
  (account limited) and 3040 (out of capacity); there is no documented 529
  and no content-refusal code. TypeSafe answers 401, 422 (validation, naming
  the field), 429, and 529 Overloaded.

## 10. Sources

- Cloudflare Workers AI, clef model page:
  <https://developers.cloudflare.com/workers-ai/models/clef/>
- Cloudflare Workers AI, errors:
  <https://developers.cloudflare.com/workers-ai/platform/errors/>
- TypeSafe, classification endpoint: `POST
  https://api.typesafe.ai/v1/systemone` (`jev-latest`), as documented by
  TypeSafe at <https://api.typesafe.ai>.

`tests/vectors/provider-examples.json` holds the providers' examples
verbatim.

## 11. Vectors and conformance

- `tests/vectors/` holds the role's vectors; each is decoded into this
  crate's types and re-encoded byte for byte by `tests/vectors.rs`:
  - `provider-examples.json`: the providers' documented examples;
  - `question-noul.json`, `question-choice.json`, `question-score.json`:
    per question type, per provider, the provider request and response and
    the `classify.run` request and reply that wrap them;
  - `state-object.json`, `state-array.json`: structured `state`;
  - `admission-refusals.json`: one refusal per admission code;
  - `item-errors.json`: one per-item error per code;
  - `replay-retry.json`: a retry returning the stored answers with the same
    `cost_usd`;
  - `replay-batch-id-reuse.json`: a reuse refused naming the field;
  - `role-describe.json`: two models, one with images and one with
    `images: null`.
- The conformance suite (`cortexkit-role-classifier-conformance`) drives a
  live module against a call-counting stand-in provider; its `README.md`
  lists the checks.

## 12. Readings this crate chose ([crate])

The owner may change any of these; each is one type or function:

1. `detail.class` on admission refusals (§7).
2. The `batch_id_reuse` field names and their order (§6.1).
3. The `detail.field` paths, including `params` for the request as a whole
   (§6.7).
4. The status mapping, including 401 and 403 as `invalid_item` (§8).
5. Open question types and `Answer::extra` (§4, §5).
6. A tightened ceiling on a re-send applies to that call; whether it is also
   recorded for later re-sends is not stated by the draft, and this crate's
   `effective_ceiling` takes the recorded and sent values only.
