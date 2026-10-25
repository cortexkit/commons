# Live usage step feed — contract v1

Apps follow prompt-cache diagnostics live through two operations served by the
usage feed provider: a subscription that only says new steps exist, and a paged
read that returns them. Apps talk only to the usage feed provider. It combines
the steps it reads from harness stores with the `step.finished.v1` events
runners publish on the bus. A step a runner announces is in the feed within
seconds; billing never comes from this feed.

This crate holds the shapes (Rust types), the vectors in
`test-vectors/usage-steps-v1/`, and the public `LatestRows` type, the
last-row-wins helper.

## Transport

A request body is `{"method": <operation>, "params": <object>}`. A success is
a Response frame whose body is `{"result": <reply>}`. A refusal is an Error
frame whose body is `{"code": <code>, "message": <text>}`, plus `detail` where
a code below names one. `code` is a closed string from this document;
`message` is for people and is not a stable interface. A request object is
closed: an unknown member is refused `malformed`. An optional request member
also accepts `null`.

## Who may read

The provider decides from the principal the local daemon attested when the
route was bound. It admits exactly these:

- `Principal::Direct`: a client of this computer's daemon running as the same
  user.
- A route whose attested principal is the reserved principal of the
  federation module (the module that relays paired devices) and whose device
  label begins `fed-phone:`.

Every other caller is refused `not_permitted`. That includes the federation
module's routes whose device label begins `fed-sim:` (simulators) or
`fed-computer:` (other computers), or carries any other label.

## Cursors

Cursors are opaque strings. They are ordered by the provider's commits, are
stable across restarts and are never reused. Apps pass them back as `after`
and never parse them or compare their order.

## Order and reconnect rules

1. Subscribe first, and hold the notices that arrive.
2. Read history with `usage.steps.read`: from your last cursor if you hold
   one, otherwise without `after`. Follow `next` until it is null, then read
   again from the reply's `through` whenever a notice arrives.
3. Drop rows already seen: apply rows in the order reads return them and keep
   the last row per identity, so a row seen twice replaces itself.
4. After any reconnect, read once more a few seconds later: a step that has
   just finished can take a few seconds to be committed.

## `usage.steps.subscribe`

Request: `{}`. Any member is refused `malformed`.

Each notice is one StreamData frame:

```json
{"through": "c0000110"}
```

`through` is the newest committed cursor. Notices are coalesced: at most four
a second, and a notice may cover many steps or none that the caller may see.
A notice carries no rows.

| How it ends | Meaning |
|---|---|
| StreamEnd | The provider is stopping or the route closed. Subscribe again. |
| Error `lagged`, detail `{"through": "<last notified cursor>"}` | The subscriber fell behind its queue bound. Subscribe again and read from the last cursor it holds. |
| Caller's Cancel | Ended by the caller. The provider may still send a last Error `cancelled`, which the daemon may drop. A reader treats `cancelled` after its own Cancel as the normal end, never as a failure. |

Opening refusals: `not_permitted`, `subscription_capacity` (more than 16 open
subscriptions), `draining` (the provider is stopping), `malformed`.

## `usage.steps.read`

Request: `{"after": "<cursor>", "limit": 200}`. Both members are optional.
Without `after`, the read starts at the oldest row the feed still holds. The
feed holds the rows committed in the last 24 hours. `limit` defaults to 200,
with a maximum of 1,000.

Reply:

```json
{"rows": [<row>, ...], "next": "<cursor>" | null, "through": "<cursor>"}
```

- `rows` are oldest first.
- `next` is the cursor to pass as `after` for the following page, or null
  when the reply reached `through`.
- `through` is the newest committed cursor.

Refusals:

| Code | Condition |
|---|---|
| `malformed` | The request does not decode, or it carries an unknown member. |
| `invalid_request` | `limit` is 0, or `after` is not a cursor the provider issued. |
| `range_too_large` | `limit` is above 1,000. |
| `cursor_expired` | `after` is older than the feed holds. Detail `{"through": "<newest cursor>"}`. Reload per-session history with `usage.cache.sessions` and `usage.cache.steps`, then read from `through`. |
| `not_permitted` | The caller is not admitted (see [Who may read](#who-may-read)). |

## The row

A row's identity is `harness` plus `step_ref`. When any field of a step
changes (a runner-announced step later recorded from its log, a finish reason
that arrives late, or a verdict recomputed because an earlier step of its
session arrived late), the whole row is sent again at a new cursor. **The last
row per identity wins; a reader replaces, never appends.**

Rows carry no content: never prompt text, tool names or error text.

Required members:

| Member | Type | Meaning |
|---|---|---|
| `cursor` | string (cursor) | This row's feed position. |
| `harness` | string | The producer: `claude-code`, `codex`, `gemini`, `kimi`, `omp`, `opencode`, `pi`, or a runner's module id. |
| `source` | string | `file` for a step read from a harness store or a runner's durable log; `runner` for a step known so far only from a runner's bus event. |
| `step_ref` | string | The producer's own id for the step. For a step a runner announced, see [Runner steps](#runner-steps). |
| `session` | string | The producer's session. |
| `occurred_at_ms` | integer, Unix ms | When the step happened. Steps without a time are not in the feed. |
| `prompt` | integer | Input plus cache read plus cache write. |
| `verdict` | string | See [Verdict](#verdict). |
| `prompt_dropped` | boolean | See [Verdict](#verdict). |

Optional members, omitted when unknown and never zero for unknown:

| Member | Type | Meaning |
|---|---|---|
| `run_id` | string | Runner steps only. |
| `provider` | string | |
| `model` | string | |
| `input` | integer | Uncached input tokens. |
| `cache_read` | integer | Cached tokens read. |
| `cache_write` | integer | Cache tokens written, the sum of every cache lifetime. |
| `cache_write_5m` | integer | Present only when the source split cache writes by lifetime. |
| `cache_write_1h` | integer | Present only when the source split cache writes by lifetime. |
| `output` | integer | Output tokens, reasoning included. |
| `reasoning` | integer | The part of `output` the provider reports as reasoning; present only when the source reports it. |
| `finish` | string | The producer's finish or stop reason. |
| `context_limit` | integer | The model's context limit, where the producer reports it. |
| `request_body_len` | integer | Request body length, where the producer reports it. |
| `retention` | number | Non-negative; never rounded or clamped. See [Verdict](#verdict). |
| `request_size` | string | `smaller` or `not_smaller`; see [Verdict](#verdict). |

`reasoning` is a breakdown of `output`, never an addition to it: it counts
output the provider reports as reasoning, and that output is already inside
`output`. So `reasoning <= output` whenever both are present. A reader
computing cost charges `output` once and never adds `reasoning` on top. A
missing `reasoning` means not reported, not zero. This is the same rule the
runner contract pins for `reasoning_tokens` and `output_tokens`.

`cache_write` is the sum of every cache lifetime.

Readers still decode a row whose `reasoning` exceeds its `output`; such a row
breaks this rule on the provider's side. `StepRow::check_counts` names it, for
the provider's own tests.

### Runner steps

A step a runner announces on the bus (`step.finished.v1`, runner contract
§14.3) is served live before any durable read records it. Its row must carry
the identity of the row the provider later records from a durable read, or
the last-row-wins rule could not replace it. The identity comes from the
event's §14.3 members:

- **The event carries `harness_step_ref`.** The runner declared
  `step_events.history_key: "harness"`, and the host's own store is the
  durable source. `harness` is the event's `harness` (the hosting program,
  such as `opencode` or `claude-code`) and `step_ref` is the event's
  `harness_step_ref` (the host's own record id). The provider's later read of
  the host store replaces the live row.
- **The event carries no `harness_step_ref`.** `harness` is the event's
  `runner` (the runner's module id) and `step_ref` is the event's `run_id`,
  `step_id` and `attempt` joined by `#`: `<run_id>#<step_id>#<attempt>`, with
  `attempt` in decimal and 1-based, as the runner contract pins it. The
  provider records one row per attempt actually sent, because a billed retry
  is spend and its cache behaviour differs. For a runner that is its own
  durable source (`history_key: "step_ref"`), that is the identity the
  provider gives the attempt when it reads the runner's step log, so the
  durable row replaces the live one. For a runner keyed by its host (`history_key:
  "harness"`) whose step has no host record, the row stays a separate
  live-only row with `source: "runner"` and is never joined to a host row.

`live_row_identity` in this crate computes the pair from those members.

### Readers and new members

The provider may add optional row members within v1. Any member it adds
carries no content: identifiers, counts, model names, times or flags only,
never prompt text, tool names or error text. Readers ignore members they don't
know, on a row, a reply or a notice. Removing or retyping a member, or
changing a threshold, is a new contract version.

In this crate, `StepRow`, `ReadReply` and `Notice` ignore unknown members when
decoding, and `StepRow::check_closed`, `ReadReply::check_closed` and
`Notice::check_closed` name any member outside the published lists, for the
provider's own tests. Requests stay strict.

## Verdict

`prompt`, `retention`, `verdict`, `prompt_dropped` and `request_size` are
computed over the step's whole session in time order, so a row's verdict does
not depend on the page it arrives in.

Verdict values: `cold_start`, `stable`, `warning`, `bust`, `full_bust`,
`unknown`. `unknown` is a real verdict: the cache's health cannot be judged for
this step. A value a reader does not recognize is a newer verdict; the reader
shows it as received rather than treating it as `unknown` (in this crate,
`Verdict::Other`).

Rules, in order:

- A step without a stored cache read is `unknown`. It is also never the
  baseline the next step is compared with.
- In a session where no step ever read a positive number of cached tokens,
  every step is `unknown`. Codex is the exception.
- The session's first step is `cold_start`.
- Otherwise, the step is compared with the last earlier step that reported its
  cache. When there is none, or that step read zero, the verdict is `stable`
  with no `retention`. Otherwise, `expected` is that step's cache read plus its
  growth, which is its cache write when positive and otherwise its uncached
  input. `retention` is this step's cache read divided by `expected`.
- `retention` is a non-negative number, never rounded or clamped. Because it
  is this step's cache read divided by the expected cache, it can exceed 1
  when a step reads more than expected.
- Codex reports no cache writes, so its `retention` is the cache read divided
  by the cache read plus the uncached input. When that sum is zero, the
  verdict is `unknown`.
- A cache read of zero is `full_bust`. Otherwise, a `retention` of at least
  **0.95** is `stable`, at least **0.80** is `warning`, and anything lower is
  `bust`.
- `prompt_dropped` is true when `prompt` is at least **15,000** tokens smaller
  than that earlier step's prompt.
- On a prompt drop, `request_size` is present when both adjacent steps report
  body lengths: `smaller` when the current body is shorter, otherwise
  `not_smaller`.

The thresholds 0.95 and 0.80 and the 15,000-token prompt drop are part of this
contract's version: changing one is a new contract version.

## Vectors

`test-vectors/usage-steps-v1/` holds the normative bytes, indexed by
`cases.json` and pinned by `SHA256SUMS`; its README describes each case.
