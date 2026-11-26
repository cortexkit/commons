# `agent-orchestrator` role: `awaited-work/v1`

Normative wire contract, crate version **0.2.0**. **Pinned** means a v1
implementation must obey the rule. **Open** means the implementer chooses and
documents it. This crate supplies serde types, bounds checks and packaged
vectors. There is no transport, store or runtime in it.

The `agent-orchestrator/v1` role is served by the module that owns agent
sessions: it starts heads and workers, holds their scopes, and delivers their
inbox. Today that module is prefrontal. This crate holds one operation family
of that role so far, `awaited-work/v1`.

## 1. What it is for (informative)

An agent often starts work that finishes later, outside its own turn: a shell
command running in the background, a sidekick answering a question. The module
running that work (the *provider module*) knows when it ends; the orchestrator
knows the agent. Awaited work joins the two. The provider module registers each
piece of work as an *item* on the agent's session when it starts, and finishes
the item when the work ends. The orchestrator then:

- delivers the result to the agent through its inbox, once;
- for a worker agent, holds the worker's completion and its stall reminders
  while any of its items is open, so a worker can't end its task while work it
  is waiting on is still running;
- ends items whose provider module restarted without them, or was removed,
  and tells the agent the work was lost.

No item ever ends because of a timer.

## 2. Capability and authority (pinned)

The implementer lists `awaited-work/v1` (`PROVIDES`) in its `role.describe`
for `agent-orchestrator/v1`. A provider module finds it by that tag; when it
is absent, the provider module uses its own older path (section 8).

The operations are `awaited_work.open`, `awaited_work.finish`,
`awaited_work.progress`, `awaited_work.cancel`, `awaited_work.sync` and
`awaited_work.list`. Each is one request on a route to the implementer. The
request body is `{"method": <operation>, "params": <params>}`. A success is a
Response whose body is that operation's reply object. A refusal is an Error
whose body is `{code, message, detail?}` (section 7).

Only modules call these operations. The caller is the module the daemon stamps
as the route's `principal`, which must be `reserved:<module_id>` (a module the
daemon launched and verified). A route with any other principal, such as an
app, a direct client or an agent's own tool route, is refused
`caller_not_module`. No request names its caller.

Items are namespaced by **(caller module, key)**. One module never sees,
finishes or cancels another module's items.

## 3. Instances and `sync` (pinned)

`instance` is an id the provider module mints at random each time its process
starts: 1 to 64 ASCII letters, digits, `-` or `_`. The daemon stamps no
instance identity on routes, so a new `instance` is how the implementer
learns that the module restarted. The verified `principal` already proves
which module is calling; a module that misreports its own instance only harms
its own items.

`awaited_work.sync` must be the first awaited-work call of each new instance.
The implementer records the most recent instance that synced as the module's
**current instance**. `open`, `finish`, `progress` and `cancel` carry
`instance`; when it is not the current instance, the call is refused
`sync_required` and changes nothing. A module that refuses or skips `sync`
therefore gets no awaited work and keeps its older path (section 8).

A module whose old and new process run side by side during a swap must have
the new instance's `sync` list the work it took over; the old instance's other
items end as `lost`.

## 4. Operations

### 4.1 `awaited_work.open` (pinned)

Params: `{instance, key, label}`.

- `key`: the provider module's own id for the work, 1 to 128 ASCII printable
  characters without spaces (for example a background task id).
- `label`: one line that the agent and the person see, 1 to 200 characters,
  with no line breaks or other control characters.

`open` is sent on a route **scoped to the agent's session**. The provider module
opens that route under the scope of the tool call it is serving, naming that
call's `scope_epoch`, as a listed carrier of the scope (section 6). The session
is read from the daemon's bind stamp on that route, never from the params. An
unscoped route, a scope the implementer does not own, or a scope that is not a
head or worker session it runs is refused `not_orchestrator_session`. A session
that has ended is refused `session_ended`.

Opening an item that is already open for the same session with the same key is
idempotent: it answers the existing item and does not change its label. Opening
a key the module already used for a different session, or a key whose item
has ended, is refused `key_reused`. Each key names one piece of work, once.

At most **32** items may be open per (session, module). One more is refused
`too_many_open`. The cap bounds a module that loops.

`open` is one short store write with no waiting on other work; its target is
under 100 ms. The provider module opens the item **before** the agent learns the
work exists, for example before it returns a task id to the agent. Otherwise a
worker could end its turn before the item exists.

Reply: `{item}` (section 5).

### 4.2 `awaited_work.finish` (pinned)

Params: `{instance, key, outcome, summary, more?, notice_record?}`, on any route
of the caller module.

- `outcome`: `done` or `failed`.
- `summary`: what the agent needs to act on, 0 to 2,000 characters. Line
  breaks are allowed.
- `more`: an optional hint of 1 to 200 characters, one line, on where the full
  result is, for example `full output: bash_status({ taskId: "bash-…" })`.
  Long output never travels through the inbox.
- `notice_record`: an optional `{writer, id}` reference to a display record
  stored by the caller. `writer` is the caller's bare module id: 1 to 64
  lowercase ASCII letters, digits or `-`, starting and ending with a letter or
  digit. `id` is the record's writer-assigned opaque id: 1 to 256 printable
  non-space ASCII bytes.
- `notice_record` is display-only. The implementer may store it on its result
  notice so the apps show the writer's card, and falls back to its own generic
  card when the reference is absent or the record cannot be found. Nothing
  about admission, delivery or settlement depends on it.
- `writer` must be the module that calls `finish` (the route's caller). The
  implementer refuses a mismatch with `invalid_params`, naming
  `notice_record.writer`. A module cannot point at another module's records.
- `notice_record` is part of the request for any idempotency or replay compare
  of `finish`: a retry sends the same value.

The item ends with state `done` or `failed`, and the implementer delivers the
result (section 6). `finish` on an item that has already ended changes nothing
and answers that item's current state. A key the module never opened is
refused `unknown_key`.

Reply: `{item}`.

### 4.3 `awaited_work.progress` (pinned)

Params: `{instance, key, text, notice_record?}`.

- `text`: 1 to 2,000 characters.
- `notice_record`: an optional `{writer, id}` reference to a display record
  stored by the caller. `writer` is the caller's bare module id: 1 to 64
  lowercase ASCII letters, digits or `-`, starting and ending with a letter or
  digit. `id` is the record's writer-assigned opaque id: 1 to 256 printable
  non-space ASCII bytes.
- `notice_record` is display-only. The implementer may store it on its progress
  notice so the apps show the writer's card, and falls back to its own generic
  card when the reference is absent or the record cannot be found. Nothing
  about admission, delivery or settlement depends on it.
- `writer` must be the module that calls `progress` (the route's caller). The
  implementer refuses a mismatch with `invalid_params`, naming
  `notice_record.writer`. A module cannot point at another module's records.
- `notice_record` is part of the request for any idempotency or replay compare
  of `progress`: a retry sends the same value.

Something the agent asked to hear about while the work runs, such as a pattern
it is watching for appearing in a command's output. The item stays open. At
most one undelivered progress notice exists per item: a newer one replaces it
before delivery. A provider module never sends periodic "still running"
progress. On an ended item `progress` delivers nothing and answers the item's
current state. A key the module never opened is refused `unknown_key`.

Reply: `{item}`.

### 4.4 `awaited_work.cancel` (pinned)

Params: `{instance, key}`.

The item ends with state `cancelled` and nothing is delivered. Use it when the
work was withdrawn, or when the agent already has the result another way (it
read the output itself, or stopped the work and got the answer in that call's
reply). For a worker, the hold ends at once. On an ended item `cancel` changes
nothing and answers the item's current state. A key the module never opened is
refused `unknown_key`.

Reply: `{item}`.

### 4.5 `awaited_work.sync` (pinned)

Params: `{instance, open: [key, ...]}`, at most 4,096 keys, no duplicates.

Sent after the module starts, once it has reattached to the work it still runs.
`open` names every item of this module that is still running. The implementer
makes `instance` the module's current instance and ends each other open item of
this module as `lost`, with a notice to its agent (section 6). A later `sync`
from the current instance does the same again.

Reply: `{lost: [key, ...], unknown: [key, ...]}`. `lost` names the items this
call ended. `unknown` names keys in `open` that have no open item of this
module (never opened, or already ended); they are reported, not refused, so one
stale key never fails a sync.

### 4.6 `awaited_work.list` (pinned)

Params: `{}`. Needs no instance and may be called before `sync`.

Reply: `{items: [item, ...]}`, the caller module's **open** items only, for
reconciling after a restart.

## 5. The item (pinned)

```json
{"item": {"key": "bash-7f3a", "label": "cargo test -p example", "state": "open", "opened_at_ms": 1795700000000}}
```

`item` members: `key`, `label`, `state`, `opened_at_ms`, and `ended_at_ms`
once the item has ended. Times are the implementer's wall clock in Unix
milliseconds.

`state` is an open string. v1 values:

| State | Meaning |
|---|---|
| `open` | registered and running |
| `done` | finished by the module, `outcome: done` |
| `failed` | finished by the module, `outcome: failed` |
| `cancelled` | cancelled by the module; nothing delivered |
| `lost` | the module's `sync` left it out, or the module was removed |
| `session_ended` | the agent's session ended first; nothing delivered |

Readers keep unknown states and treat them as ended. The item never says
which session it belongs to: the provider module already knows, and the
reply must not reveal anything about a session to a route that may not be
scoped to it.

## 6. What the implementer does (pinned)

**Scope carriers.** For each head or worker session, the implementer lists every
tool provider module in that session's plan as a carrier on the session's scope
record, targeted to the implementer: `{"principal": "reserved:<module_id>",
"targets": ["<implementer module id>"]}`. One entry per principal; when the
module is already a targeted carrier for other modules, the implementer's id
joins that entry's `targets`. A bare carrier entry already reaches every module
and stays as it is.

**Holds.** While any item of a worker's session is open, the worker's turn end
does not complete its task and its stall reminders are held. After a result is
delivered, completion waits until the worker has taken a turn after that
delivery. A head has no completion and nothing is held; its results arrive as a
wake.

**Delivery.** Results and progress reach the agent through the implementer's
inbox as one notice family, `awaited_work_result`, exempt from do-not-disturb.
Each is delivered once across restarts: its idempotency key is the module, the
key and the kind (the result, or a progress notice with its number). The notice
names the module by its plain name and quotes the label, outcome and summary;
the implementer neutralizes module text before putting it in a notice. `failed`
and `lost` results are delivered like `done` ones. `cancelled` and
`session_ended` deliver nothing.

**Ending.** An item ends only for one of these reasons:
1. the module finishes or cancels it;
2. the module's `sync` leaves it out (`lost`);
3. the session ends: the worker is settled, cancelled or rejected, or the head's
   session is retired (`session_ended`; nothing is delivered);
4. the module is gone (`lost`): when the daemon's module list changes, the
   implementer asks for a route to a module with open items and reads a refusal
   with `subc_protocol::error_codes::is_retryable_route_open_refusal`. False
   (`unknown_module`, or `target_unavailable` with `detail.expected_back:
   false`) means the module was removed, stopped, disabled or failed past its
   crash budget. A module that is only restarting or reloading keeps its items
   until its next `sync`.

**Visibility.** The agent sees its own open items in its status line; a
worker's owner sees them in the worker's status (module, label, age).

## 7. Refusals (pinned)

| Code | Operations | Meaning |
|---|---|---|
| `caller_not_module` | all | the route's principal is not a launched module |
| `not_orchestrator_session` | `open` | the route's scope is not a head or worker session the implementer owns |
| `session_ended` | `open` | the session has ended |
| `too_many_open` | `open` | 32 items are already open for this session and module |
| `key_reused` | `open` | the key belongs to another session or to an item that has ended |
| `sync_required` | `open`, `finish`, `progress`, `cancel` | `instance` is not the module's current instance |
| `unknown_key` | `finish`, `progress`, `cancel` | the module never opened this key |
| `invalid_params` | all | malformed, unknown or out-of-bounds params |

Every code is terminal: an unchanged retry has no remedy, and the provider
module falls back (section 8). A transport failure (the route could not be
opened, or closed before the reply) is not a refusal. Readers keep unknown
codes and treat them as terminal.

Requests are closed: an unknown params member is `invalid_params`.
An optional member may be omitted or `null`; both mean absent.
Replies are open: readers ignore unknown members.

## 8. Fallback (pinned)

When `awaited-work/v1` is absent, `open` is refused, or the implementer can't be
reached, the provider module does what it did without awaited work, and it
must not tell the agent the result will arrive through the inbox. Awaited work
replaces a module's own result path only for items it opened successfully.
