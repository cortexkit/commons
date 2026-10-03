# `classifier/v1`: role contract (draft)

Stability: **alpha, draft**. Nothing ships against it until the role's owner
and its consumers approve it. This document is the role definition; the Rust types
in this crate are its wire shapes, `tests/vectors/` holds its vectors, and
the sibling `cortexkit-role-classifier-conformance` crate holds its suite
(§11).

It is written from the role owner's draft and the owner's rulings on the
points that draft left open (stored errors on a re-send, the
batch spend ceiling, concurrent calls, the meaning of `noul`, the type of
`state`, and truncation), and on the readings an earlier version of this
crate chose where the draft was silent (the class on admission refusals,
the `batch_id_reuse` field order, the `detail.field` paths, the provider
status mapping, unknown question types, and recording a tightened
ceiling). The provider shapes are from the providers' documentation
(§10).

Every rule is marked **[pinned]**: the role states it. A module must do
it, and a caller may rely on it. No reading is left open (§12).

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
- [pinned] Unknown question types split by direction. In a request, a
  question whose `type` is not one of the three is refused
  `invalid_params` with `detail.field` `questions.<id>.type`, naming that
  question's key: the module can neither validate nor map a type it does
  not know. The type is checked before the rest of the question, which
  depends on it. The types may still model an open question type for
  decoding (`QuestionType::Other`); the request validator refuses it
  (`check_question`). In a reply, the type decodes open (§5).
- The role's question id (`[a-z0-9_-]`, 1-64) is narrower than clef's
  (letters, digits, `_`, `.` and `-`, up to 100), so every role id is a
  valid clef id.

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
- [pinned] A reply decodes open: an answer of a type this crate does not
  name keeps its type string, and a member the provider adds that this
  crate does not name is kept as given (`Answer::extra`), so a new provider
  field passes through.

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
- [pinned] `detail.field` of `batch_id_reuse` names the first field that
  differs: `model`, `questions`, `items` (a different item count) or
  `items[i]` (the first differing item), checked in that order
  (`BatchIdentity::first_difference`).

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
- [pinned] The one exception to storing item errors: `auth_failed` and
  `model_unavailable` (§8) are never stored. They say nothing about the
  item, so the item stays unanswered, and a re-send (after a re-login or a
  catalog fix) calls the provider for it again (`ItemError::is_stored`).
  The items a rate limit kept from being sent in a call (§8) are not
  stored either, so a re-send retries them.
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
- [pinned] A tightened ceiling is recorded. A re-send's lower
  `max_cost_usd` becomes the batch's recorded ceiling, durably, before that
  call sends anything, so the recorded ceiling after any call is
  `effective_ceiling(recorded, sent)`. Without this, a third re-send
  carrying the original higher value would loosen it again. A refusal at
  the call's first provider call (§7) does not undo it: the ceiling was
  recorded before anything was sent, and a ceiling never loosens.
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
  refusal happens only at admission, and writes nothing. A 401, 403 or 404
  on the call's first provider call counts as admission (§7, §8).

### 6.7 Validation before any provider call

- [pinned] Checked against the catalog row before anything is written or
  sent: question count, question ids, question types, choice options 2-255,
  score levels 2-10, `state` kinds, images, `max_items`,
  `max_request_bytes`
  (`parse_request`, `check_request_bytes`, `check_request`).
- [pinned] `detail.field` paths: `batch_id`, `model`, `questions`,
  `questions.<id>`, `questions.<id>.type`, `questions.<id>.instructions`,
  `questions.<id>.criteria`, `items`, `items[i]`, `items[i].state`,
  `items[i].images`, `items[i].images[j]`, `max_cost_usd`, and `params` for
  the request as a whole. A map key is joined with a dot and an array index
  is bracketed; a dot cannot be ambiguous, because a question id cannot
  contain one (§3).
- [pinned] `params` is used only when no narrower path applies: the body
  is not JSON or not an object, or it is over `max_request_bytes`. When a
  narrower path applies, the refusal uses it, even where the problem keeps
  the whole request from decoding: a malformed image names
  `items[i].images[j]`, and an object missing `batch_id` names `batch_id`
  (`parse_request`, `tests/vectors/field-paths.json`).
- [pinned] `detail.limit` is the bound broken, where there is one.

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
[pinned] Every refusal carries `detail.class`, the code's class, in the
same shape, so a caller that meets an unknown code still knows whether to
retry.

| Code | When | Detail | Class |
|---|---|---|---|
| `invalid_params` | malformed request or a limit exceeded | `field`, `limit?` | permanent |
| `batch_id_reuse` | same key, different body | `field` | permanent |
| `model_unknown` | model not in the catalog | `model` | permanent |
| `model_not_classifier` | catalog row is not a classifier | `model`, `kind` | permanent |
| `cost_exceeded` | estimate over `max_cost_usd` | `estimate_usd`, `max_cost_usd` | permanent |
| `account_walled` | the account is at its limit | `resets_at_ms?` | transient |
| `batch_in_progress` | another call holds this `batch_id` | `retry_after_ms` | transient |
| `auth_failed` | the call's first provider call answered 401 or 403 | — | permanent |
| `model_unavailable` | the call's first provider call answered 404 | `model` | permanent |

- [pinned] `auth_failed` and `model_unavailable` are decided by the first
  provider call the call makes, so they come after validation and the
  spend guard. The call sends no further item, records no item outcome,
  and for a batch's first call records no batch, so the refusal writes
  nothing (a ceiling the call tightened stays recorded, §6.4).

Per-item errors (`ItemError`):

| Code | When | Class |
|---|---|---|
| `rate_limited` | provider 429 or 529 after retries, or not sent because an earlier item in the call ended `rate_limited`; `retry_after_ms` when given | transient |
| `provider_error` | provider 5xx or 408, or a transport failure, after retries | transient |
| `provider_refused` | the provider refused the content (no status maps here yet, §8) | permanent |
| `invalid_item` | the provider rejected this item's input (400, 413, 422, any other 4xx not listed in §8) | permanent |
| `cost_exceeded` | answering this item would cross the batch ceiling | permanent |
| `auth_failed` | provider 401 or 403, or not sent because an earlier item in the call met one; never stored | permanent |
| `model_unavailable` | provider 404, or not sent because an earlier item in the call met one; never stored | permanent |

- [pinned] `class` is one of the shared classes, and the set is fixed for
  `classifier/v1`: `transient` or `permanent`. A new class means a new role
  version. A caller that meets an unknown value treats it as `permanent`,
  the safe direction (`ErrorClass::effective`).

## 8. Provider status mapping

- [pinned] Neither provider documents an error body, so the module maps a
  provider's HTTP status, once its own retries are spent
  (`item_code_for_status`):

  | Status | Code | Class | Stops the call | Stored |
  |---|---|---|---|---|
  | 401, 403 (TypeSafe's 401, clef's 403) | `auth_failed` | permanent | yes | never |
  | 404 (clef's 404) | `model_unavailable` | permanent | yes | never |
  | 429 (clef's codes 3036 account-limited and 3040 out-of-capacity; TypeSafe's rate limit), 529 (TypeSafe's Overloaded) | `rate_limited` | transient | yes | the item that met it, as transient |
  | 408 (clef's timeout), every 5xx | `provider_error` | transient | no | as transient |
  | 400, 413, 422, and every other 4xx | `invalid_item` | permanent | no | as permanent |

- [pinned] 401, 403 and 404 are never `invalid_item`: they say nothing
  about the item, and a caller reading `invalid_item` drops the item as
  bad.
- [pinned] `auth_failed` and `model_unavailable` stop the call. No further
  item is sent to the provider in that call; items already answered stay
  recorded; every item left unanswered carries that code. Neither is
  stored (§6.3), so a re-send after a re-login or a catalog fix calls the
  provider again for exactly those items. If the status comes on the
  call's first provider call, the whole call is refused at admission with
  the same code instead (§7).
- [pinned] A rate limit stops the rest of the call too. Once an item ends
  `rate_limited` after its retries, the items after it that would need a
  provider call are not sent in that call: they get `rate_limited` without
  a provider call. They are transient and not stored, so a re-send retries
  them, as it retries the stored `rate_limited` of the item that met the
  limit.
- [pinned] An item that needs no provider call (a stored answer, a stored
  permanent error) is returned as stored whatever stopped the call.
- [pinned] Nothing maps to `provider_refused`: neither provider documents a
  content-refusal status, so the code is reserved for a provider that
  reports one.

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
- Cloudflare Workers AI, clef-flash model page:
  <https://developers.cloudflare.com/workers-ai/models/clef-flash/>
- TypeSafe, Jev API reference (request and answer shapes, `jev-latest`):
  <https://jevtypesafeai.com/docs>; the endpoint is `POST
  https://api.typesafe.ai/v1/systemone`
  (<https://www.jevtypesafeai.com/how-to-use>).

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
  - `item-errors.json`: one per-item error per code, with `auth_failed`
    and `model_unavailable` showing the item the stop left unsent;
  - `field-paths.json`: an unknown question type refused naming
    `questions.<id>.type`, and a refusal naming the narrowest path that
    applies rather than `params`;
  - `replay-retry.json`: a retry returning the stored answers with the same
    `cost_usd`;
  - `replay-batch-id-reuse.json`: a reuse refused naming the field;
  - `role-describe.json`: two models, one with images and one with
    `images: null`.
- The conformance suite (`cortexkit-role-classifier-conformance`) drives a
  live module against a call-counting stand-in provider; its `README.md`
  lists the checks.

## 12. Open readings

None remain. The owner ruled on each reading an earlier version of this
crate had chosen where the draft was silent, and each ruling is a pinned
rule in its section: `detail.class` on admission refusals (§7), the
`batch_id_reuse` field names and order (§6.1), the `detail.field` paths and
the limit on `params` (§6.7), the provider status mapping (§8), unknown
question types (§4, §5), and recording a tightened ceiling (§6.4).
