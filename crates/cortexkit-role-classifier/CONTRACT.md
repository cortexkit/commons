# `classifier/v1`: role contract

Stability: **alpha**. This document is the role definition; the Rust types
in this crate are its wire shapes, `test-vectors/classifier-v1/` holds its
vectors, and the sibling `cortexkit-role-classifier-conformance` crate holds its suite
(§11).

The provider shapes are from the providers' documentation (§10).

Every rule is marked **[pinned]**: the role states it. A module must do
it, and a caller may rely on it. §12 lists the readings this version leaves
open.

The batch rules `classifier/v1` shares with the other model-batch roles
(idempotency, single flight, stored outcomes, the spend ceiling, the error
vocabulary, provider status mapping, images, number text) are stated once,
in `cortexkit-model-batch`'s
[`RULES.md`](../cortexkit-model-batch/RULES.md), cited here as "RULES
§n". They are pinned for this role as written there; this document states
only what is this role's own. Route roles and model discovery follow RULES §18.

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
- [pinned] A caller opens a route for `classifier/v1` as RULES §18
  requires, then picks the module by **model id** using that section's
  discovery procedure, not by capability alone.
- [pinned] Starting providers: Cloudflare Workers AI `@cf/cloudflare/clef`
  (catalog ids `cloudflare-workers-ai/@cf/cloudflare/clef`,
  `cloudflare-workers-ai/@cf/cloudflare/clef-flash`) and TypeSafe
  `jev-latest` (catalog id `typesafe/jev-1.13.0`, `POST
  https://api.typesafe.ai/v1/systemone`). Each provider call takes one
  item's `state` with every question of the batch.
- [pinned] A catalog id is as RULES §1 states it.

## 2. `role.describe`

- [pinned] The answer is (`RoleDescribe`):

  ```
  {
    role: "classifier/v1",
    limits: { max_items, max_request_bytes },
    models: [ {
      model,                      // catalog id
      max_questions, max_items,   // per model: the tighter of catalog and module limits
      context_tokens?,            // from the catalog; absent when it states no exact figure
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
- [pinned] `context_tokens` is the catalog's exact context figure, and the
  provider truncates `state` beyond it. It is absent when the catalog states
  no exact figure, under the same rule as `price`: absent is not zero, and
  the module never guesses one (a provider that says only "64k" gets
  neither 64,000 nor 65,536). A stated `context_tokens` is never zero
  (`check_describe`).

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
  model's context without saying so. Neither provider reports truncation,
  so the reply carries no truncation flag: a guessed flag would read as a
  measurement. A caller keeps `state` under `context_tokens` where the model
  states one; the per-item `usage.input_tokens`, when the provider reports
  it, is how the caller sees an item ran near the limit. The module does not
  refuse a `state` by a byte heuristic.

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
- [pinned] Images are the shared item image shape, validated and sized as
  RULES §13 states (Workers AI's limits: 4 MiB and 16 megapixels each, 8
  MiB in total); clef takes at most 4 per item.
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
- [pinned] Each number reaches the reply with the provider's spelling
  (RULES §12); forward the answer object's text.
- [pinned] A reply decodes open: an answer of a type this crate does not
  name keeps its type string, and a member the provider adds that this
  crate does not name is kept as given (`Answer::extra`), so a new provider
  field passes through.

## 6. Pinned rules

The shared batch rules apply as RULES §2 to §11 state them. The subsections
keep their numbers, and say what is this role's own.

### 6.1 Idempotent by `batch_id`

- [pinned] As RULES §2. **Body identity** is the model, the questions, and
  each item (its images in canonical form) (`BatchIdentity`).
- [pinned] `detail.field` of `batch_id_reuse` names the first field that
  differs: `model`, `questions`, `items` (a different item count) or
  `items[i]` (the first differing item), checked in that order
  (`BatchIdentity::first_difference`).

### 6.2 Single flight per `batch_id`

- [pinned] As RULES §3.

### 6.3 Stored errors on a re-send

- [pinned] As RULES §4.

### 6.4 `max_cost_usd` is a batch ceiling

- [pinned] As RULES §5, with the estimate this role's §6.9 states.

### 6.5 `cost_usd` is the batch total

- [pinned] As RULES §6. It is present whenever the model is priced.

### 6.6 Never all-or-nothing after admission

- [pinned] As RULES §7: each item gets `answers` or `error`, never both.

### 6.7 Validation before any provider call

- [pinned] Checked against the catalog row before anything is written or
  sent: question count, question ids, question types, choice options 2-255,
  score levels 2-10, `state` kinds, images (RULES §13), `max_items`,
  `max_request_bytes`
  (`parse_request`, `check_request_bytes`, `check_request`).
- [pinned] `detail.field` paths: `batch_id`, `model`, `questions`,
  `questions.<id>`, `questions.<id>.type`, `questions.<id>.instructions`,
  `questions.<id>.criteria`, `items`, `items[i]`, `items[i].state`,
  `items[i].images`, `items[i].images[j]`, `max_cost_usd`, and `params` for
  the request as a whole, under RULES §8. A dot cannot be ambiguous,
  because a question id cannot contain one (§3).
- [pinned] A malformed image names `items[i].images[j]`, and an object
  missing `batch_id` names `batch_id` (`parse_request`,
  `test-vectors/classifier-v1/field-paths.json`).

### 6.8 Model pinned

- [pinned] As RULES §9: a missing row is `model_unknown`; a row whose
  `kind` is not `classifier` is `model_not_classifier`, with `detail.model`
  and `detail.kind`; a row that states no `kind` is that same code with
  `detail.model` only, never an invented kind (`check_catalog_row`).

### 6.9 Spend guard

- [pinned] An estimate of items x input tokens x catalog price above
  `max_cost_usd`, or an account at its limit, is refused before dispatch
  (`cost_exceeded`, `account_walled`). The estimate is the module's own; see
  §12.

### 6.10 Absent is not zero

- [pinned] As RULES §10 (`sum_usage`).

### 6.11 Synchronous, bounded

- [pinned] As RULES §11.

## 7. Error codes

The vocabulary, the refusal and item error shapes, every code's meaning,
detail and class, and which codes are stored and stop the call are RULES
§14 (`errors`). `classifier/v1` answers with these codes:

- [pinned] Admission refusals (`ADMISSION_CODES`): `invalid_params`,
  `batch_id_reuse`, `model_unknown`, `model_not_classifier`,
  `cost_exceeded`, `account_walled`, `batch_in_progress`, `auth_failed`,
  `model_unavailable`.
- [pinned] Per-item errors (`ITEM_CODES`): `rate_limited`,
  `provider_error`, `provider_refused`, `invalid_item`, `cost_exceeded`,
  `auth_failed`, `model_unavailable`.
- [pinned] The batch-level account codes clef's errors map to (§8,
  `BATCH_LEVEL_CODES`): `account_limit_exhausted` and
  `account_not_entitled`, as an admission refusal when the call's first
  provider call meets one, and otherwise as the per-item error of every
  item the call leaves unanswered.
- `answer_too_large` is in the shared vocabulary, but this role states no
  answer bound and never answers with it.

## 8. Provider status mapping

- [pinned] TypeSafe documents no error codes, so its errors map by status
  (RULES §15, the first table; `item_code_for_status`): its 401 is
  `auth_failed`, 422 `invalid_item`, 429 and 529 `rate_limited`.
- [pinned] clef is a Cloudflare Workers AI model, so its errors map by
  Workers AI code first (RULES §15, the second table;
  `workers_ai_item_code`). Against 0.1.0's status-only mapping that changes
  four clef errors: 429 `3036` is `account_limit_exhausted`, not
  `rate_limited`; 403 `5035`, `5016` and `3023` are `account_not_entitled`,
  not `auth_failed`; 400 `5007` is `model_unavailable`, not `invalid_item`
  (`test-vectors/classifier-v1/workers-ai-codes.json`).
- [pinned] A stop, a stored error and an item that needs no provider call
  behave as RULES §14 states.

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
- Errors: clef answers Workers AI errors, an HTTP status with
  `errors[].code` (the codes RULES §15 maps); there is no documented 529
  and no content-refusal code. TypeSafe answers 401, 422 (validation, naming
  the field), 429, and 529 Overloaded, with no documented error body.

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

`test-vectors/classifier-v1/provider-examples.json` holds the providers'
examples verbatim.

## 11. Vectors and conformance

- `test-vectors/classifier-v1/` holds the role's vectors, pinned by its
  `SHA256SUMS` file. They ship in this crate's package, and the
  `test-vectors` feature compiles them into the `vectors` module
  (`vectors::FILES`, plus a constant per file), so a module's tests can
  read the same bytes. Each is decoded into this crate's types and
  re-encoded byte for byte by `tests/vectors.rs`, which needs
  `--features test-vectors`:
  - `provider-examples.json`: the providers' documented examples;
  - `question-noul.json`, `question-choice.json`, `question-score.json`:
    per question type, per provider, the provider request and response and
    the `classify.run` request and reply that wrap them;
  - `state-object.json`, `state-array.json`: structured `state`;
  - `admission-refusals.json`: one refusal per admission code, plus a
    kindless catalog row refused as `model_not_classifier`;
  - `item-errors.json`: one per-item error per code, with `auth_failed`
    and `model_unavailable` showing the item the stop left unsent;
  - `field-paths.json`: an unknown question type refused naming
    `questions.<id>.type`, and a refusal naming the narrowest path that
    applies rather than `params`;
  - `image-identity.json`: one image as a `data:` URL and re-sent as
    `{content_type, base64}` with one identity, and malformed base64
    refused naming `items[0].images[0]`;
  - `workers-ai-codes.json`: clef's errors by Workers AI code, each with
    its class, storage and stop, and the 0.1.0 code where it differs;
    TypeSafe's by status, unchanged; and the account codes' first-call
    refusals;
  - `replay-retry.json`: a retry returning the stored answers with the same
    `cost_usd`;
  - `replay-batch-id-reuse.json`: a reuse refused naming the field;
  - `role-describe.json`: three models, one with images, one with
    `images: null`, and one with no `context_tokens`.
- The conformance suite (`cortexkit-role-classifier-conformance`) drives a
  live module against a call-counting stand-in provider; its `README.md`
  lists the checks.

## 12. Open readings

- The spend guard's estimate (§6.9) is the module's own. This role may
  adopt the shared conservative estimate (RULES §5) in a later version.

Every other point likely to be read more than one way is a pinned rule in
its section: every admission refusal carries `detail.class` (RULES §14);
`batch_id_reuse` names the first differing field in the order model,
questions, items, `items[i]` (§6.1); `detail.field` is `params` only when
no narrower path applies (RULES §8); 401 and 403 map to `auth_failed` and
404 to `model_unavailable`, never `invalid_item` (RULES §15); an unknown
question type is refused in a request and kept in a reply (§4, §5); and a
lower `max_cost_usd` on a re-send becomes the recorded ceiling (RULES §5).
