# Model-batch rules, `v1`

Stability: **alpha**. This document states, once, the batch rules every
CortexKit model-batch role shares. A model-batch role takes a batch of items
and a schema (`classifier/v1`'s questions, `extractor/v1`'s fields), calls
one hosted model per item, and returns the provider's answers untouched.
The roles differ in their schema and answers; they share everything below.
Each role contract points here instead of restating these rules, and adds
only what is its own: its request shape, the parts of its body identity,
its `detail.field` paths, its catalog kind, and its provider mapping.

The code in this crate is these rules (`cortexkit-model-batch`), and
`test-vectors/model-batch-v1/` holds their vectors (§16). Every rule is
marked **[pinned]**: a module serving a model-batch role must do it, and a
caller may rely on it.

Terms: a **call** is one request to the module (`classify.run`,
`extract.run`); a **batch** is everything sent under one `batch_id`, over
one or more calls; a **provider call** is one request from the module to
the provider, for one item.

## 1. Batch requests

- [pinned] Every batch request carries `batch_id` (1 to 128 printable
  ASCII characters, space through tilde: `is_batch_id`), `model` (a catalog
  id), the role's schema, `items` (answered in request order) and optional
  `max_cost_usd`.
- [pinned] A catalog id is `<provider>/<model>`: models.dev's own provider
  key, then models.dev's model id. The first `/` splits provider from model,
  so `@` and any further `/` belong to the model part. A catalog id pins a
  model version, never a moving alias.
- [pinned] One provider call carries one item with the whole schema of the
  batch.

## 2. Batch identity

- [pinned] Idempotent by `batch_id`. The module records the admitted batch
  (model, frozen catalog row, body identity) and each item's outcome
  durably, before replying. The same `batch_id` with the same body returns
  the stored outcomes, and calls the provider only for items that need one
  (§4). The same `batch_id` with a different body is refused
  `batch_id_reuse`, with `detail.field` naming the first differing part.
- [pinned] **Body identity** is the model and the SHA-256 of each other
  part as RFC 8785 (JCS) canonical JSON (`identity::digest`), so the key
  order of an object, whitespace, and number spellings JSON treats as equal
  (`1.0` and `1`, `1e30` and `1E+30`) can't turn a retry into
  `batch_id_reuse`. Each role contract names its parts and their order;
  every role ends with `items` (a different item count) and then `items[i]`
  (the first differing item), each item hashed on its own
  (`identity::first_difference`).
- [pinned] An item's images are hashed in their canonical form (§13): the
  content type and the decoded bytes, as `{content_type, base64}` with
  standard padded base64 (`identity::item_digest`). The same image sent as a
  `data:` URL on one call and as `{content_type, base64}` on a re-send is the
  same batch.
- [pinned] An image reference is hashed in its canonical form `{handle}`,
  never the bytes it resolves to (§13). So deciding whether a re-sent
  `batch_id` is a replay or a reuse never needs a handle that may since have
  expired, and a reference and an inline copy of the same picture are
  different batches: a re-send that swaps one for the other is
  `batch_id_reuse` naming `items[i]`.
- [pinned] `max_cost_usd` is never part of the body identity (§5).

## 3. Single flight per `batch_id`

- [pinned] While one call holds a batch, a second call with that `batch_id`
  is refused `batch_in_progress` (transient, with `detail.retry_after_ms`),
  and writes and dispatches nothing. That covers a caller retrying after its
  own timeout while the first call still runs.
- [pinned] The caller re-sends after the first call ends, and gets the
  stored outcomes. No item is ever sent to the provider twice at once, and
  no call hangs waiting on another.

## 4. Stored outcomes on a re-send

- [pinned] A stored answer is never re-asked.
- [pinned] A stored permanent item error (`provider_refused`,
  `invalid_item`, `cost_exceeded`, `answer_too_large`) is returned as
  stored, with no provider call.
- [pinned] A stored transient item error (`rate_limited`, `provider_error`)
  is retried, and its new outcome (answer or error) replaces the stored one.
- [pinned] An item error of an unknown class is treated as permanent, so it
  is returned as stored (`ItemError::is_retried_on_resend`).
- [pinned] The **batch-level codes** are never stored: `auth_failed`,
  `model_unavailable`, `account_limit_exhausted` and `account_not_entitled`
  (`errors::is_batch_level`). They say nothing about the item, so the item
  stays unanswered, and a re-send (after a re-login, a catalog fix, the
  allocation's reset or an operator's fix) calls the provider for it again
  (`ItemError::is_stored`). The items a rate limit kept from being sent in
  a call (§15) are not stored either, so a re-send retries them.

## 5. The spend ceiling and the cost estimate

- [pinned] `max_cost_usd` is a **batch** ceiling. It is recorded at first
  admission, and spend already recorded for the batch counts against it. A
  value that is not a number of USD, zero or more, is refused
  `invalid_params` naming `max_cost_usd` (`admission::check_max_cost_usd`).
- [pinned] It is not part of the body identity: a re-send carrying a
  different value is not `batch_id_reuse`.
- [pinned] The effective ceiling is the lower of the recorded one and the
  re-send's (`admission::effective_ceiling`): a caller can tighten it, never
  loosen it. A re-send's lower `max_cost_usd` becomes the batch's recorded
  ceiling, durably, before that call sends anything, so the recorded ceiling
  after any call is `effective_ceiling(recorded, sent)`. A refusal at the
  call's first provider call (§14) does not undo it.
- [pinned] Before each provider call the module checks recorded spend plus
  that call's estimate. An item that would cross the ceiling gets the
  per-item error `cost_exceeded` (permanent; a re-send with a higher value
  cannot raise the ceiling, so it cannot clear it).
- [pinned] **The shared estimate** (`estimate::estimate_item`), for a role
  whose contract adopts it, is a conservative upper bound on one item's
  input cost:

  ```
  input tokens = schema + max_passes x (state + images x image_tokens_each + overhead)
  cost         = input tokens x input rate
  ```

  This is the model Cloudflare's Clairic was measured to bill (a live
  probe of 2026-10-22, against `@cf/cloudflare/clairic`): on every
  successful reply, `input_tokens = schema_tokens + passes x state_tokens +
  image_tokens`, with no fixed overhead. Probe 05 sent 40 text fields and a
  state of 85 tokens: 40 passes, `schema_tokens` 26,488, `input_tokens`
  29,888 = 26,488 + 40 x 85. Probe 07 sent the same fields in reverse order
  and was billed the same. Where each term comes from:

  - `schema` bills **once** across all passes: the fields or questions,
    split between passes by the provider.
  - `state` bills on **every** pass: the provider billed the whole state
    again for each pass it made.
  - `max_passes` is the module's **upper bound** on the passes the provider
    makes for the item (`ItemCostInput::with_max_passes`), at least 1; a
    zero is refused (`EstimateError::NoPasses`). It defaults to 1, which is
    right for a one-pass provider. A role whose provider splits its schema
    states the bound in its contract. The provider's per-pass token budget
    does not predict the passes: probe 05's 40 small fields ran in 40
    passes where a budget of 8,192 tokens would predict 4, and probe 01's
    nine fields ran in two. Deriving passes from the budget underestimates
    whenever the provider makes more passes than the budget predicts.
  - `images x image_tokens_each` and `overhead` are charged on every pass.
    That overstates them: the measured overhead was zero, and an image was
    measured only in a one-pass request (probe 12: one 64x64 PNG, reported
    `image_tokens` 64, and `schema_tokens` 41 above the same request without
    the image), so whether a provider bills images once or per pass is not
    known. A module's `image_tokens_each` covers both figures.
  - Text is bounded by its UTF-8 byte length: a byte-level tokenizer never
    makes more tokens than bytes. The state's bound is its byte length, the
    schema's the byte length of its canonical JSON (`Tokens::Bound`). A
    module with the provider's own tokenizer may pass exact counts instead
    (`Tokens::Exact`). Never a characters-divided-by-four rule: it can
    underestimate.
  - `image_tokens_each` (the tokens one image costs) and `overhead` (the
    module's prompt template and any system text) are bounds the module
    supplies. Both are required; neither defaults to zero.
  - The provider's per-pass budget (`pass_tokens`) is used only to check
    that the state fits in a pass. With an **exact** count at or past it
    (`state >= pass_tokens`), the item cannot fit: it gets `invalid_item`
    before any call, its message naming the bound
    (`EstimateError::DoesNotFit`). A **byte bound** at or past it is not a
    count, so the module does not refuse the item by it; the item is
    estimated like any other, since the formula bounds its cost whether the
    provider refuses the state or truncates it.
  - Money is integer nanodollars, the catalog's unit
    (`RateNanosPerMtok`, nanodollars per million tokens). A cost rounds up
    to a whole nanodollar; a USD ceiling converts by decimal text, rounding
    down (`estimate::usd_to_nanos`). Arithmetic is checked: an overflow
    makes the estimate cross any ceiling, never wrap.
  - A rate the catalog states as zero (stated free) costs nothing; a
    direction the catalog does not price is absent (§10).

## 6. `cost_usd` is the batch total

- [pinned] A reply's `cost_usd` is the batch's cumulative cost, the same on
  every reply to the batch, a replay included. It covers every answered
  item of the batch, whichever call answered it.
- [pinned] A caller sets the batch's charge to this value rather than
  adding it, so a crashed and replayed run is charged exactly once.

## 7. Never all-or-nothing after admission

- [pinned] Each item gets its answers or an `error`, never both. A
  whole-batch refusal happens only at admission, and writes nothing. A
  batch-level code (§4) on the call's first provider call counts as
  admission (§14).

## 8. Validation before any provider call

- [pinned] Everything the role contract lists is checked against the
  catalog row and the module's limits before anything is written or sent.
- [pinned] A refusal names the offending field in `detail.field`, as a
  path: a map key joined with a dot, an array index in brackets
  (`items[2].images[0]`). `params` names the request as a whole, and is used
  only when no narrower path applies: the body is not JSON or not an object,
  or it is over `max_request_bytes` (`admission::check_request_size`). When
  a narrower path applies, the refusal uses it, even where the problem keeps
  the whole request from decoding.
- [pinned] `detail.limit` is the bound broken, where there is one
  (`admission::bounded`).

## 9. Model pinned

- [pinned] The module checks the model's catalog row in three cases
  (`admission::CatalogRow`): no row is `model_unknown`; a row whose `kind`
  is not the role's is the role's own "not my kind" code, with
  `detail.model` and `detail.kind`; a row that states no kind is that same
  code with `detail.model` only, never an invented kind. A role contract
  that does not yet state how it handles a row with no `kind` must list
  that case among its unresolved questions.
- [pinned] The model is never substituted.

## 10. Absent is not zero

- [pinned] Usage the provider didn't report stays absent, per item and in
  the batch sum (`absent::sum_reported`). A price direction the catalog
  doesn't price is absent too; a direction it states as free is `0`.
- [pinned] An input-priced cost is absent when any answered item did not
  report the tokens it is priced by (`absent::input_cost_nanos`): a partial
  sum would read as the whole cost.

## 11. Synchronous, bounded

- [pinned] `max_items` is chosen so a batch fits the call deadline. A
  caller whose call timed out sends the same `batch_id` again and gets every
  outcome recorded so far, with the rest completed.

## 12. Provider number text

- [pinned] A module never rounds, rescales or renames a provider's
  numbers, because callers threshold on them: each number reaches the reply
  with the provider's own spelling. A module that decodes and re-encodes a
  provider answer through binary floating point breaks this.
- [pinned] A role keeps a provider's number as its text: serde_json's
  `RawValue` (`number::NumberText`), never the `arbitrary_precision` or
  `float_roundtrip` features, which change number parsing for every crate in
  a workspace build.

## 13. Item images

- [pinned] An image is a `data:` URL string,
  `data:<content type>;base64,<payload>`, `{content_type, base64}`, or a
  reference `{handle}` to an image another module holds (§13.1). The content
  type is one of `image/png`, `image/jpeg`, `image/webp`, or the fewer a
  model states. Remote URLs are refused.
- [pinned] Both object forms are closed: an object with any other member,
  or with members of both forms (`{content_type, base64, handle}`), is no
  image, and is refused `invalid_params` naming `items[i].images[j]`. So each
  JSON shape is exactly one form (`Image`, `InlineImage`, `ImageReference`).
- [pinned] Validation decodes the base64 (standard alphabet, padding
  optional, no whitespace), so malformed base64, a `data:` URL without
  `;base64`, and an image of no bytes are refused before any provider call.
  Sizes are decoded bytes (`image::check_image_with`, `image::check_images`).
- [pinned] Cloudflare Workers AI's image-taking batch models state 4 MiB
  per image and 8 MiB per item in total (`ImageLimits::workers_ai`), and 16
  megapixels per image, which is a module's check: the role crates do not
  decode image formats.
- [pinned] A refusal names `items[i].images` for the count or the total,
  and `items[i].images[j]` for one image.
- [pinned] The canonical form is `{content_type, base64}` with the decoded
  bytes as standard padded base64 (`Image::canonical`); it is what batch
  identity hashes (§2). A module forwards the image to the provider as the
  caller wrote it.

### 13.1 Image references

A caller that already handed an image to another module (a screenshot, say)
sends a short opaque handle instead of copying the image's bytes into every
request. The host resolves the handle to bytes through the module that
issued it, just before it calls the model.

- [pinned] A reference is `{handle}`. The handle is opaque: 1 to 128
  characters of `[A-Za-z0-9._:-]` (`image::is_image_handle`). Anything else
  is refused like any malformed image, `invalid_params` naming
  `items[i].images[j]`.
- [pinned] **Declared support.** A host resolves references only where its
  role's `role.describe` declares the `image_reference` group. A caller sends
  a reference only to a host that declares it. A host that does not declare
  it refuses a reference as an unsupported image: `invalid_params` naming
  `items[i].images[j]` (`image::check_images`). That is also how a host built
  before references existed answers one, so a caller can roll references out
  host by host.
- [pinned] **Resolution before anything else.** A declaring host resolves
  every reference of the request before any model call and before it writes
  anything (`image::resolve_images`). It then applies exactly the checks an
  inline image gets to the resolved bytes, with the same refusals and the
  same `detail.field`: content type, size per image, the item's total, the
  module's pixel limit, and the model's image support
  (`image::check_unresolved_images` before resolution,
  `image::check_resolved_images` after it).
- [pinned] **Two refusals for a reference that cannot be resolved**, both
  admission refusals naming `items[i].images[j]` and writing nothing:
  - `image_unavailable`, permanent: the handle is unknown, expired, or not
    usable by this caller. These are one answer, with one message, so a
    refusal never tells a caller whether a handle it may not use exists.
  - `image_source_unavailable`, transient, with `detail.retry_after_ms`:
    the issuing module is unreachable or timed out.
- [pinned] A reference has no bytes until it is resolved, and the code
  never pretends otherwise: `Image::decode` and `Image::approx_bytes` are
  `None` for one, which is neither a decode failure nor zero bytes.
- [pinned] The canonical form of a reference is `{handle}` as sent (§2). A
  host forwards the resolved bytes to the provider
  (`InlineImage::from_decoded`); a provider never sees a handle.

## 14. Error vocabulary

One vocabulary serves every model-batch role (`errors`). Codes are open
strings on the wire; a code a caller does not know is kept as sent. A role
contract's tables say which codes that role answers with.

- [pinned] An admission refusal is an `ERROR` frame whose body is `{code,
  message, detail}` (`Refusal`), and writes nothing. Every refusal carries
  `detail.class`, the code's class, so a caller that meets an unknown code
  still knows whether to retry.
- [pinned] A per-item error is `{code, class, message, retry_after_ms?,
  detail?}` (`ItemError`). `detail` is present only on a code that defines
  one (`answer_too_large`).
- [pinned] `class` is `transient` or `permanent`. The set is fixed for
  `v1`; a new class means a new role version. A caller that meets an
  unknown value treats it as `permanent`, the safe direction
  (`ErrorClass::effective`).

Admission refusals:

| Code | When | Detail | Class |
|---|---|---|---|
| `invalid_params` | malformed request or a limit exceeded | `field`, `limit?` | permanent |
| `batch_id_reuse` | same key, different body | `field` | permanent |
| `model_unknown` | model not in the catalog | `model` | permanent |
| `model_not_classifier` | `classifier/v1`: the row is not a classifier | `model`, `kind?` | permanent |
| `model_not_extractor` | `extractor/v1`: the row is not an extraction model, or states no kind | `model`, `kind?` | permanent |
| `cost_exceeded` | estimate over `max_cost_usd` | `estimate_usd`, `max_cost_usd` | permanent |
| `account_walled` | the module's own spend guard knows the account is at its limit | `resets_at_ms?` | transient |
| `batch_in_progress` | another call holds this `batch_id` | `retry_after_ms` | transient |
| `auth_failed` | the call's first provider call met an auth failure | `credential?` | permanent |
| `model_unavailable` | the call's first provider call met "no such model" | `model` | permanent |
| `account_limit_exhausted` | the call's first provider call met Workers AI 3036 | `resets_at_ms?` | transient |
| `account_not_entitled` | the call's first provider call met Workers AI 5035, 5016 or 3023 | — | permanent |
| `image_unavailable` | an image reference is unknown, expired or not usable by this caller (§13.1) | `field` | permanent |
| `image_source_unavailable` | the module that issued an image reference did not answer (§13.1) | `field`, `retry_after_ms` | transient |
| `model_unserved` | caller found no module that lists the model | `model` | permanent (caller-raised; not a module error) |
| `model_ambiguous` | caller found more than one module that lists the model | `model`, `modules` (sorted module ids) | permanent (caller-raised; not a module error) |

- [pinned] A batch-level code met on the call's first provider call comes
  after validation and the spend guard. The call sends no further item,
  records no item outcome, and for a batch's first call records no batch,
  so the refusal writes nothing, except that a lower `max_cost_usd` the call
  sent stays the batch's recorded ceiling (§5). Answers an earlier call
  stored stay stored and come back on the next re-send; a re-send whose
  items are all already answered calls no provider, so it cannot meet this
  refusal.
- [pinned] `auth_failed` carries `credential`, the id of the credential the
  runner presented (for example `apikey:cloudflare`), when the runner knows
  it. It is always an id, never a token or key.

Per-item errors:

| Code | When | Class | Stored | Stops the call |
|---|---|---|---|---|
| `rate_limited` | provider 429 or 529 after retries, or not sent because an earlier item in the call ended `rate_limited`; `retry_after_ms` when given | transient | the item that met it | yes |
| `provider_error` | provider 5xx or 408, or a transport failure, after retries | transient | yes | no |
| `provider_refused` | the provider refused the content (no status maps here yet) | permanent | yes | no |
| `invalid_item` | the provider rejected this item's input, or the item cannot fit (§5) | permanent | yes | no |
| `cost_exceeded` | answering this item would cross the batch ceiling | permanent | yes | no |
| `answer_too_large` | an answer is larger than the module's stated bound; `detail.field`, `detail.limit`, `detail.size` | permanent | yes | no |
| `auth_failed` | provider auth failure, or not sent after one | permanent | never | yes |
| `model_unavailable` | provider "no such model", or not sent after one | permanent | never | yes |
| `account_limit_exhausted` | Workers AI 3036, or not sent after one | transient | never | yes |
| `account_not_entitled` | Workers AI 5035, 5016 or 3023, or not sent after one | permanent | never | yes |

- [pinned] A code that stops the call: no further item is sent to the
  provider in that call; items already answered stay recorded; every item
  left unanswered that would need a provider call carries that code. An
  item that needs no provider call (a stored answer, a stored permanent
  error) is returned as stored whatever stopped the call.

## 15. Provider status mapping

The module maps a provider's error once its own retries are spent.

- [pinned] **By status**, for a provider that documents no error codes
  (`status::item_code_for_status`):

  | Status | Code |
  |---|---|
  | 401, 403 | `auth_failed` |
  | 404 | `model_unavailable` |
  | 429, 529 | `rate_limited` |
  | 408, every 5xx | `provider_error` |
  | 400, 413, 422, every other 4xx | `invalid_item` |

  401, 403 and 404 are never `invalid_item`: they say nothing about the
  item, and a caller reading `invalid_item` drops the item as bad. Nothing
  maps to `provider_refused`.

- [pinned] **Cloudflare Workers AI** replies carry `errors[].code`. A code
  this table lists decides, whatever the status
  (`status::workers_ai_item_code`):

  | HTTP / code | Code |
  |---|---|
  | 401, or 403 with no account code | `auth_failed` |
  | 400 `5004` invalid data, `3003` incomplete request | `invalid_item` |
  | 400 `5007`, 404 `3042` no such model | `model_unavailable` |
  | 403 `5035` needs a paid plan, `5016` model agreement, `3023` account blocked | `account_not_entitled` |
  | 413 `3006` request too large | `invalid_item` |
  | 408 `3007` timeout, `3008` aborted | `provider_error` |
  | 429 `3036` daily free allocation used | `account_limit_exhausted` |
  | 429 `3040` out of capacity, any other 429 | `rate_limited`, with `retry_after_ms` if given |
  | 5xx | `provider_error` |

  A code the table doesn't list keeps the status mapping, and the module
  puts the code into the item error's `message`.

## 16. Vectors

`test-vectors/model-batch-v1/`, pinned by its `SHA256SUMS`, ships in this
crate's package; the `test-vectors` feature compiles it into the `vectors`
module. `tests/vectors.rs` decodes and re-encodes every file byte for byte,
then checks it against the code:

- `vocabulary.json`: every code with its class, whether it is stored, and
  whether it stops the call;
- `status-mapping.json`: the Workers AI table and the status mapping;
- `images.json`: one image in three spellings with one canonical form and
  one identity, and the images validation refuses; a reference with its
  canonical form `{handle}`, handles the grammar refuses, and objects that
  mix the two forms or add a member and so decode as no image;
- `estimates.json`: the estimate, including probe 05's exact counts (which
  estimate exactly what Clairic billed) and byte bounds (at least that),
  images and overhead on every pass, a byte-bound state past the pass
  budget, an item that does not fit, and a stated-free rate.

## 17. Sources

- A live probe of Cloudflare Workers AI Clairic
  (`@cf/cloudflare/clairic`), 2026-10-22: twelve requests, each reply
  captured byte for byte. The estimate's model (§5) rests on probes 05 and
  07 (40 fields, 40 passes), 06 (the same state with one field), 01 (nine
  fields in two passes) and 12 (one 64x64 image).

- Cloudflare Workers AI, errors:
  <https://developers.cloudflare.com/workers-ai/platform/errors/>
- RFC 8785, JSON Canonicalization Scheme:
  <https://www.rfc-editor.org/rfc/rfc8785>
- RFC 2397, the `data:` URL scheme:
  <https://www.rfc-editor.org/rfc/rfc2397>

## 18. Routes and discovery

- [pinned] A module serves a model-batch role (`classifier/v1` or
  `extractor/v1`) only on a management route whose `route.open` declares that
  role in `role_versions` (`{"classifier": "v1"}` or
  `{"extractor": "v1"}`). `role_versions` is the subc route field
  (`subc_protocol::session::ROLE_VERSIONS_FIELD`). The daemon validates it
  and forwards it unchanged on the module's `route.bind`.
- [pinned] A module serving more than one role on its management surface
  serves exactly one role per route: the role that route declared. A route
  that declares no role keeps whatever role the module served there before
  (Broca's `llm-runner/v1` routes are unchanged), and a model-batch role is
  never served on it. On a route of one role, ops of another role are refused
  `unknown_method`.
- [pinned] A bind that declares two of the module's roles, or a version of a
  role the module does not serve, is refused at bind, naming `role_versions`
  in `detail.field`.
- [pinned] A caller checks for the daemon's `route-role-versions/v1`
  capability (`subc_protocol::scope::CAP_ROUTE_ROLE_VERSIONS_V1`) before
  relying on forwarded route roles. The daemon advertises this capability in
  `HELLO_ACK` and `server.describe` when it forwards `role_versions`. A daemon
  without it drops the field silently. The caller also checks that `role` in
  the `role.describe` answer is the role it declared.
- [pinned] Which module serves a model is a fact about one install, not about
  the model, so it is not in the catalog. From the daemon's `catalog.list`, a
  caller lists the modules whose manifest `capabilities.provides` names the
  role. It opens one route of that role to each, reads `role.describe`, and
  picks the module whose `models` lists the model's catalog id. It may cache
  model id to module until the daemon's module catalog changes.
- [pinned] If no module lists the model, the caller refuses before any call
  with `model_unserved` and `detail.model`.
- [pinned] If more than one module lists the model, the caller refuses
  `model_ambiguous` with `detail.model` and `detail.modules` (the module ids,
  sorted). It never picks one, so an operator who installs a second provider
  of the same model sees the ambiguity at once.
