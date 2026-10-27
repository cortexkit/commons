# LLM tools v1

This crate defines the single-call wire shapes shared by callers and tool
providers. It implements types, schemas and validation only, not routing,
provider calls, storage, consent, scope stamping or billing.

## Identity and invocation

The tool tags are **`llm.classify/v1`**, **`llm.extract/v1`** and
**`llm.complete/v1`**, each with **`modes: ["codemode","flow"]`**. They never
appear on an agent's direct tool list. The request/result objects below are the
tool's arguments and successful data, not a second tool-provider envelope.
`call_key` and the daemon-stamped scope come from the invocation envelope, never
from model-visible arguments. Unknown top-level argument/result members are
refused. Optional members are omitted when absent, not written as null (an
explicit `json: null` answer is the exception).

**Replay:** every call is keyed on the tool call's `call_key`. A repeat returns
the stored result and is never billed twice. Its `cost_usd` is the original
call's cost, not zero and not a newly estimated price. A provider stores each
call's result durably, and concurrent calls with the same `call_key` share one
execution; role batching does not replace this replay.

**No tools:** a call never uses tools. Its result is data the caller treats as
untrusted input. It grants nothing: it never stands in for a consent card or a
grant. Neither prompt text nor returned data can grant authority.

**Billing:** spend is billed to the daemon-stamped run or flow scope, never to
the serving module. A flow is never treated as its owner agent. These tools
route only to models priced per token, **never to subscription plans**. A model
with no known applicable per-token rate is refused before the call. Every
request to any of the three tools requires a non-negative numeric `max_cost_usd`;
the caller supplies its remaining budget as this per-call ceiling. The member
belongs to the tool request; the role types are unchanged. A missing ceiling is
refused `invalid_params` naming
`max_cost_usd` before anything is written. Every successful result of every tool
requires a non-negative numeric `cost_usd`,
even when the provider omits token counts. The serving module must determine
the billed cost independently when usage is incomplete; missing measurements
must not be fabricated to derive a cost. A known free rate and cost may be zero.
When nothing gives the billed cost, the call fails `cost_unmeasured` (below)
instead of succeeding.

## Common tool-admission refusals (all three tools)

These checks run **before anything is written and before anything reaches a
role or provider**. The flow carrier is the module that runs flows and calls
tools on their behalf. A call from it must carry a flow scope, not merely an
owner-agent or run scope. Check this rule first, so a flow carrier with no scope
is always refused `flow_scope_required`.

| Code | Class | Meaning |
|---|---|---|
| `scope_required` | permanent | No daemon-stamped run or flow scope. |
| `flow_scope_required` | permanent | A flow carrier called without a daemon-stamped flow scope. |
| `cost_unpriceable` | permanent | The chosen model has no known applicable per-token rate. Refused before the call. |

## Common post-call failure (all three tools)

| Code | Class | Meaning |
|---|---|---|
| `cost_unmeasured` | permanent | The provider answered, but nothing gives the billed cost: a count the per-token price rests on is missing and there is no provider billing figure. |

- The answer is withheld. A result is never returned without a measured cost,
  and a cost is never derived from a missing count.
- The usage the provider did report is recorded as reported; an absent count
  stays absent.
- The provider may still have charged for the call, so the caller charges the
  request's `max_cost_usd` against its budget for it. That bound is safe: the
  call was admitted only because its estimate fit that ceiling.
- A retry with the same call key replays this failure and never calls the
  provider again.
- A serving module should not offer these tools a model whose provider is known
  to omit a count its price needs, so this failure stays rare.

These are tool-admission codes, not replacements for role failure codes.
Refusals reuse `cortexkit_model_batch::errors::Refusal`:
`{code, message, detail: {class, ...}}`. Item failures retain the role's
`ItemError` vocabulary and shape; they are not successful results with a
fabricated answer. The tool-provider error envelope carries the failure.

## `llm.classify/v1`

Input (`classify::Request`):

```text
{model, questions: {<id>: Question}, item: Item, max_cost_usd}
```

`model` is the role's catalog ID `<provider>/<model>`, never substituted.
`Item` is **`cortexkit_role_classifier::request::Item`** (`{state, images?}`),
`Question` is **`cortexkit_role_classifier::question::Question`**. This is one
item of the published [classifier/v1 contract](../cortexkit-role-classifier/CONTRACT.md),
not a batch: no `batch_id`, `items` array, index, or renamed question fields.
`questions` is non-empty, retains caller order, and follows the role's IDs,
question types, instructions and criteria rules. Structured state is preserved.
The ceiling `max_cost_usd` follows the common billing rule above.

Result (`classify::Result`):

```text
{answers: {<id>: Answer}, usage: Usage, cost_usd: number}
```

`Answer` is **`cortexkit_role_classifier::reply::Answer`**, including its open
types and extra provider fields. There is one answer per requested question,
with the matching question type. Do not normalize provider answers.

Model/item errors are exactly the classifier role's codes, with shared classes:

| Codes | Class |
|---|---|
| `invalid_params`, `batch_id_reuse`, `model_unknown`, `model_not_classifier`, `cost_exceeded`, `auth_failed`, `model_unavailable`, `account_not_entitled`, `image_unavailable`, `provider_refused`, `invalid_item` | permanent |
| `account_walled`, `batch_in_progress`, `account_limit_exhausted`, `image_source_unavailable`, `rate_limited`, `provider_error` | transient |

The role's admission/item distinctions and provider mappings remain unchanged;
`answer_too_large` is not a classifier error. There is no second vocabulary.

## `llm.extract/v1`

Input (`extract::Request`):

```text
{model, fields: {<id>: Field}, item: Item, options?, max_cost_usd}
```

`Item` is **`cortexkit_role_extractor::request::Item`**
(`{state, images?, reference_date?}`), `Field` is
**`cortexkit_role_extractor::field::Field`**, and `Options` is
**`cortexkit_role_extractor::request::Options`** (`{quantiles?, state_text?}`).
The published [extractor/v1 contract](../cortexkit-role-extractor/CONTRACT.md)
governs these objects unchanged: fields are non-empty and ordered, dates are
real calendar dates, state is non-empty unless images are supplied, and field
options are validated by type. There is exactly one item, no batch wrapper.
`model` retains the role's meaning; `max_cost_usd` follows the common billing rule.

Result (`extract::Result`):

```text
{answers: {<id>: Answer}, truncated?, state_text?, usage: Usage, cost_usd: number}
```

`Answer` is **`cortexkit_role_extractor::reply::Answer`**, preserving raw JSON
number spelling, unknown types and extra members. Each requested field has its
matching answer type. `truncated` is only the provider's report, never guessed;
`state_text` is returned only if requested, as the role specifies.

Model/item errors are exactly the extractor role's codes:

| Codes | Class |
|---|---|
| `invalid_params`, `batch_id_reuse`, `model_unknown`, `model_not_extractor`, `cost_exceeded`, `auth_failed`, `model_unavailable`, `account_not_entitled`, `image_unavailable`, `provider_refused`, `invalid_item`, `answer_too_large` | permanent |
| `account_walled`, `batch_in_progress`, `account_limit_exhausted`, `image_source_unavailable`, `rate_limited`, `provider_error` | transient |

The role's admission/item distinctions remain unchanged (in particular,
`cost_exceeded` is an item error, not an extractor admission refusal).

## Image references (`llm.classify/v1`, `llm.extract/v1`)

An item image may be a reference `{handle}` to an image another module
already holds, as the shared model-batch rules state it
(`cortexkit-model-batch` `RULES.md` §13.1): a caller sends a short handle
instead of the image's bytes, and the serving module resolves the handle
through the module that issued it before it calls the model. The handle is
1-128 characters of `[A-Za-z0-9._:-]`.

- A provider's tool input schema lists the `{handle}` form only in the
  release where the provider resolves references
  (`input_schema_with_image_references`); before that it exports
  `input_schema`, which lists only a `data:` URL and `{content_type,
  base64}`. A caller sends a reference only to a tool whose schema lists it.
- A provider that does not resolve references refuses one as an
  unsupported image, `invalid_params` naming `item.images[j]`
  (`validate_request`). One that does checks the handle
  (`validate_request_with_image_references`), resolves it, and gives the
  resolved bytes exactly the checks an inline image gets.
- A reference it cannot resolve is refused `image_unavailable` (permanent:
  unknown, expired or not usable by this caller, one answer for all three)
  or `image_source_unavailable` (transient, with `detail.retry_after_ms`:
  the issuing module did not answer), naming `item.images[j]`.

## `llm.complete/v1`

Input (`complete::Request`):

```text
{
  demand: {iq, eq?}, prompt, system?, output_schema?,
  max_output_tokens, max_cost_usd
}
```

- `demand` is required. `iq` and optional `eq` are integers in **0–100**.
  There is **no `model` member**: the module selects a model satisfying the
  demand under the caller's class. These are caller-supplied routing demand
  scores, not measurements reported by a model. Their scoring scales and the
  interpretation of the two axes belong to daemon routing policy, not this
  wire contract. The caller's class is its daemon-supplied routing category,
  outside the tool arguments; a prompt cannot set or override that category.
  The prompt and optional system message are text-only in v1; completion may
  still return JSON when `output_schema` is supplied.
- `prompt` is non-empty text; `system` is optional text.
- `output_schema`, when present, is a JSON Schema **object**, at most **16 KiB
  (16,384 bytes)** in compact UTF-8 JSON serialization. Whitespace in the
  transport encoding is not part of this bound. The serving module checks that
  the object is a valid schema before dispatch. Its documented schema policy
  must identify accepted dialects/keywords and refuse unsupported schemas with
  `invalid_params`; this crate does not implement that policy. Draft 2020-12
  below describes the exported tool schemas, not a promise to evaluate every
  caller-supplied schema dialect.
- `max_output_tokens` is required, an integer at least 1.
- `max_cost_usd` follows the common billing rule: a JSON decimal number, not a
  string. Check the estimate against
  this ceiling **before the call**; above it, refuse `cost_exceeded`. Never
  route through a subscription or silently omit a rate/price check.

Result (`complete::Result`):

```text
{text?: string, json?: any JSON value,
 model: {provider, model, variant?}, usage: Usage, cost_usd: number}
```

Exactly **one of `text` and `json`** is present. With `output_schema` it must be
`json`, valid against that schema; otherwise it must be `text`. A JSON null is
a present JSON answer. `model` identifies the model actually used, with
non-empty provider and model strings; `variant` is optional. `cost_usd` is
required, including on replay and when usage is incomplete.

Completion uses these applicable `cortexkit-model-batch` codes unchanged, plus
two new completion-only codes:

| Code | Class | Meaning |
|---|---|---|
| `invalid_params` | permanent | Malformed arguments or an exceeded input bound. |
| `model_unknown` | permanent | A selected model is absent from the catalog. |
| `cost_exceeded` | permanent | The estimate exceeds the supplied ceiling. |
| `account_walled` | transient | The provider account is at its spend limit. |
| `auth_failed` | permanent | Provider credentials were refused. |
| `model_unavailable` | permanent | The provider does not serve the model. |
| `account_limit_exhausted` | transient | The provider account allocation is exhausted until reset. |
| `account_not_entitled` | permanent | The provider account may not use this model. |
| `rate_limited` | transient | Provider rate limiting after retries. |
| `provider_error` | transient | Provider/transport failure after retries. |
| `provider_refused` | permanent | The provider refused the content. |
| `invalid_item` | permanent | The provider rejected the input. |
| `answer_too_large` | permanent | The answer exceeds the serving module's bound. |
| `output_invalid` | permanent | The reply failed `output_schema`. |
| `no_route` | permanent | Routing found no model for the demand. |

Batch identity and role-specific model-kind codes do not apply to completion.

## Shared usage

Every result requires `usage` (possibly `{}`):

```text
{input_tokens?, cached_input_tokens?, cache_write_tokens?,
 output_tokens?, reasoning_tokens?}
```

Each field is an unsigned integer measurement. A field is **absent when the
provider did not report it, never fabricated as zero**. A reported zero remains
zero.

`input_tokens` counts only input neither read from nor written to the prompt
cache; it excludes `cached_input_tokens` and `cache_write_tokens`. Total input
consumed is `input_tokens + cached_input_tokens + cache_write_tokens`, each
where present. An omitted component remains unknown, not zero, so this sum can
be a partial total. A serving module whose provider includes cache reads or
writes in its prompt or input count subtracts those included counts before reporting
`input_tokens`; the same tokens never appear in two fields. If the provider
reports the same cache writes under two names, the module reports them once.
A consumer computing cost prices the three fields separately and never
subtracts one from another.

`reasoning_tokens` is a breakdown of `output_tokens`, never in addition:
when supplied it requires `output_tokens` and cannot exceed it. A module whose
provider reports thinking separately folds it into `output_tokens`.

A consumer enforcing a token cap charges the **token allowance it reserved for
the call** when the result lacks `input_tokens` **or** `output_tokens`. Missing
counts do not imply a free call, zero use, or permission to exceed a cap.
Required cost does not make token measurements known.

## Schemas, validators and vectors

Each module exports `input_schema()` and `result_schema()` as JSON Schema
Draft 2020-12 objects. The workspace role types have no schema derivation, so
these are hand-written and exercised against every valid and invalid vector.
Input schemas have a flat object root for `tool.catalog`. Result schemas
describe successful data, not error envelopes.

Run schema validation **and** the named `validate_request`/`validate_result`
functions. Validators check the completion output XOR, serialized schema byte
bound, costs, demand bounds, a positive `max_output_tokens` and usage breakdown.
They do not compare reported usage to the requested output-token limit. The role
request validators reuse question/field/image/calendar checks. They do not know catalog
limits, scope authority, prices, requested answer IDs/types or JSON Schema
semantics. The serving module must additionally enforce those role admission
and reply checks, real scope/price checks and JSON Schema evaluation.
`complete::validate_result_for_request` also checks the text/JSON branch against
the request; it does not evaluate the caller's arbitrary JSON Schema. The schema
engine is a dev-dependency only, not a runtime dependency of this contract crate.

Enable `test-vectors` for embedded `vectors::FILES`, per-file constants and
`SHA256SUMS`. `test-vectors/llm-tools-v1/` contains invented examples: valid and
invalid inputs per tool, priced results with absent and reported usage, JSON
answers, invalid results, and a refusal per published code. Tests check both
schemas and typed validators, vector inventory, embedded/disk identity, SHA-256
checksums, and refusal classes. No unpriced success is valid.
