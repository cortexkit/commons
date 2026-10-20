# `agent-inbox/v1`

Normative wire contract, crate version **0.1.0**. **Pinned** means a v1
implementation must obey the rule. **Open** means the implementer chooses and
documents it. This crate supplies serde types, deterministic `neutralize`,
validation helpers and packaged vectors; the sibling
`cortexkit-role-agent-inbox-conformance` runs named checks against a live inbox.
There is no transport, database, scheduler or runtime in either crate.

## 1. Capability and authority (pinned)

An implementer advertises `agent-inbox/v1` (`PROVIDES`). Discovery finds it by
that tag. `no_implementer` means the capability is absent, **not a reply code**.
The operations are `inbox.post`, `inbox.withdraw`, `inbox.list`, `inbox.read`.
The implementer never refuses text for resembling an envelope and never asks
senders to confirm currentness by callback; senders push withdraw or replace
instead. Every item is wrapped by the implementer, even if the sender's text
already resembles an envelope. No inbox content renders as system role.

The carrier authenticates the principal before any receipt lookup. A request
cannot assert its sender, delegated source or authoritative attribution.
The namespace for idempotency, replace and withdraw is
**(attested principal, delegated source, immutable target, key)**. A flow's
delegated source is the `flow_id` on its carrier-stamped scope, never a string
claimed by the flow author. Basal is the flow-execution carrier: different
flows sharing its attested principal, and different target agents, must not
share keys. Withdraw routes bind this same namespace out of
band; selectors do not choose another sender, flow or target.

## 2. Addressing and authorization (pinned)

| Scope | Inbox target |
|---|---|
| Head | stable `agent_id`, surviving sessions, compaction and residence flips |
| Worker | stable task of the `bg_` execution, surviving follow-up runs/revival |
| Flow | `notice.target_agent`, checked against the sink grant |
| Codemode run | no posting authority |

For a **fresh provider post**, ask the daemon's owner-only `scope.served_by`:
`served` admits; `not_served` means `scope_not_yours`; `unknown` means
`scope_unknown`. An ended scope gives `scope_ended`. Scope reference and epoch
are checked together. Flow posts use stamped flow identity, sink grant, owner,
audience and instance-expiry checks, **never** `scope.served_by`.
An owner-only sink given another target returns `sink_target_not_owner`; a
target outside an audience returns `sink_target_not_in_audience`.

The reserved Basal owner-notice path uses attested `reserved:basal`, no scope,
and a top-level `flow_id`. The implementer checks the install exists and
resolves its owner; the post may not choose `target_agent`. Its namespace is
(`reserved:basal`, flow_id, resolved owner, key); Basal's notice_id is its key.
An arbitrary provider cannot obtain this authority by adding `flow_id`.

V1 does not require a restart-specific message authentication code (MAC) for
posting authority. Catalog templates
have principal allowlists; a provider cannot select a core-only template.

## 3. `inbox.post` (pinned)

```json
{
  "scope": {"scope_ref": "scope", "epoch": 1},
  "notice": {
    "idempotency_key": "completion-1",
    "content": {"text": "Finished"},
    "attention": "wake",
    "replace_key": "watch-1",
    "deliver_after_ms": 2000,
    "expires_at_ms": 3000,
    "links": [{"label": "Result", "url": "https://example.test/result"}],
    "data": {"task": "one"}
  }
}
```

`scope` is required except for reserved owner notices (replace `scope` with
`flow_id`). `content` is exactly `{text: string}` or
`{template: {id: string, slots: object}}`. Attention is required and is exactly
`wake`, `ride` or `silent`; target_agent is flow-only. All other notice members
above are optional. There are at most eight links, each with string label and
url. Data is an opaque object; only implementer templates interpret it.
Slots may contain JSON values; every string leaf actually rendered is untrusted.
Unknown request members, wrong types, both content forms, both addressing forms,
and owner notices choosing a target are `invalid_params`. Structural decoding
and `PostRequest::validate` are separate; neither proves authorization.

Times are non-negative integer **absolute Unix milliseconds**, not durations.
`deliver_after_ms` forbids a new admission before that time; `expires_at_ms`
forbids one at or after that time. An absent not-before means immediately
eligible; an absent expiry means no absolute expiry. Expiry keeps running while
pending/held. A previously admitted attempt is not retrospectively retracted.

### Durable receipts and retries

Successful reply: `{"state":"accepted","item_id":"item-1"}` with optional
`"replayed":true` on a retry. **Accepted means durable**: payload custody has
transferred and the sender may drop its copy. Authenticate, then look up the
receipt for (namespace, idempotency_key) **before fresh authorization**,
validation, size limits, quotas or template checks. An identical parsed request
(JSON object ordering is irrelevant) returns its immutable first acceptance,
apart from the optional replay annotation. A different request under the same
key is `idempotency_conflict`, even after scope end or authorization loss.
Replacement, withdrawal, expiry, delivery and daemon restart do not erase the
receipt. Keep receipts at least as long as the sender's retry horizon; seven
days is proposed, not a universal finite bound.

### Replacement and attention

Strength: `silent < ride < wake`. `wake` may steer/start a turn subject to policy;
`ride` waits for a safe next turn/result; `silent` waits for a digest. DND holds
all three inbox attentions. Notices sent directly to a runner solely to ride
along with existing turns are outside this role and its DND gate.

A fresh post with replace_key replaces only **pending, live, undelivered**
editions in that namespace. Retire expired or withdrawn editions first.
Take the strongest attention of the live editions and clamp it to the current
sender/sink grant. Take **the newest post's** not-before and expiry values,
including their absence; never the minimum old not-before. In-flight, landed
and unconfirmed editions are immutable; a new post creates another pending
edition rather than mutating an owned attempt. Old idempotency receipts remain.

### Size and pressure

The cap is **16,384 bytes of the whole serialized notice**, not 16 KiB of text.
It includes keys, content, slots, links, data, timing and attention, but excludes
the outer scope/flow addressing object. Measure the compact serde JSON UTF-8
encoding of the decoded Notice (`serde_json::to_vec`); input indentation does
not count. At the cap is allowed; above it is `content_too_large`.
`check_notice_len` checks this; `validate` also checks addressing and link count.
The implementer enforces a per-sender pending cap per target with `inbox_full`.
The numeric pending cap is open. Neither size nor quota checks supersede an
accepted receipt lookup. Neutralization never refuses content.

## 4. `inbox.withdraw` (pinned)

Params are exactly `{"idempotency_key":"k"}` **or** `{"replace_key":"k"}`.
Both or neither and unknown members are `invalid_params`. Use the same
authenticated namespace as the post. Replies have one `state`:

| State | Meaning |
|---|---|
| `withdrawn` | The selected pending item is retired; it cannot later be admitted. |
| `already_delivered` | The selected item has already left withdrawable pending custody. This is not proof of a runner landing. |
| `not_found` | No selectable item exists, including an already withdrawn/expired one. |

Withdraw is cheap and idempotent; not_found and already_delivered are normal.
It never retracts in-flight sends or deletes acceptance receipts. Replace-key
selection refers to the current pending edition, otherwise an existing owned
edition. Retired editions do not lend attention to future replacements.

## 5. Agent pull tools and states (pinned)

These are tools scoped to the agent's **own** inbox by its authenticated route.
They cannot select another target. `inbox.list {}` returns
`{"items":[Item,...]}` including unread, held/pending and unconfirmed bodies.
`inbox.read {"item_id":"..."}` returns `{"item":Item}` or `{"item":null}`
when no item is visible to that inbox. Item is
`{item_id, state, body, data?}`: body is the implementer's wrapped, neutralized
rendering with attested attribution; data, if exposed, is opaque JSON.
Pulling does not claim a landing or silently remove an uncertain obligation.
Pagination, ordering and read/unread bookkeeping are open; v1's minimal list
shape returns every visible item. Retention of landed/retired bodies is open.

Store lifecycle: `pending` → `in_flight` → `landed` or `unconfirmed`.
Only pending can be withdrawn, expired or made obsolete and `retired`.
A claim atomically checks DND, lifecycle and residence generation, assigns
attempt_id/send_id, and persists exact rendered bytes/hash before I/O. Two
schedulers cannot claim the same edition. Definite not-sent/retracted returns
the same row to pending, not a newly minted item. In-flight stays pinned to its
original residence until resolved; only pending migrates. A pending/paused or
joined-run runner reply is not a landing. Resolve with status or the same
send_id and bytes. Unconfirmed is terminal for push, remains pull-readable,
is excluded from later digests, and is named once in the next delivery.
The guarantee is durable acceptance and no automatic duplicate resend under
uncertainty, **not exactly-once landing**.

## 6. Wrapping and `neutralize` (pinned)

`ENVELOPE_TAG` is **`system-reminder`**. This is the only envelope tag an inbox
implementer may use. Each item has its own `<system-reminder>` /
`</system-reminder>` wrapper and an attribution line derived from attested
metadata, never from the request. No inbox content is rendered as system role.
Neutralize every sender-supplied rendered string: text, template slots, link
labels, displayed URLs and strings read from data. Neutralizing the complete
rendered untrusted body immediately before wrapping is valid. A multi-item
digest cannot use one shared wrapper instead of these per-item boundaries.

`neutralize(text) -> String` scans for this **prefix** grammar:

1. ASCII `<`;
2. zero or more of ASCII space, tab, CR or LF;
3. optional `/`;
4. zero or more of those same four whitespace bytes;
5. `system-reminder`, matched ASCII case-insensitively.

Replace **only** the matching `<` with `‹` (U+2039); preserve every other byte.
No closing `>` or name boundary is needed, so attributes, incomplete tags,
suffixes and nested markers are neutralized too. `<\nsystem-reminder` and
`</ system-reminder>` match. `<system-\nreminder>` does not: the name cannot be
split. Unicode look-alikes, Unicode whitespace, form feed and vertical tab do
not match. `‹` is already neutral, never a marker. There is no Unicode
normalization, general HTML escaping or Markdown rewriting. The function is
idempotent. Content resembling an envelope is accepted, not refused.

## 7. Refusals and sender classification (pinned)

Refusal body: `{code: string, message: string, detail?: JSON}`. The carrier
distinguishes refusal frames from success; no transport framing is pinned here.

| Code | Finality | Meaning |
|---|---|---|
| `scope_not_yours` | terminal | daemon says not_served |
| `scope_unknown` | terminal | daemon lacks served history |
| `scope_ended` | terminal | fresh post to an ended scope |
| `idempotency_conflict` | terminal | same namespaced key, different request |
| `content_too_large` | terminal | serialized notice exceeds 16 KiB |
| `inbox_full` | retryable | pending quota reached |
| `template_unknown` | terminal | no such catalog template |
| `template_not_allowed` | terminal | principal cannot select template |
| `sink_target_not_owner` | terminal | owner-only sink targeting someone else |
| `sink_target_not_in_audience` | terminal | sink target outside audience |
| `invalid_params` | terminal | malformed, unknown or inconsistent request fields |

`errors::is_terminal` / `is_retryable` classify every code, including unknown
codes (terminal). Consumers preserve unknown codes as `Code::Other(String)`.
Successful reply states and item states likewise preserve unknown strings;
unknown states are **not** interpreted as acceptance or landing. New reply
fields are ignored; public reply types are non_exhaustive with constructors
and builders. Request enums stay closed to prevent silent question changes.

Sender recipe: accepted transfers custody. After a terminal refusal, do not
retry the unchanged post; any fallback is sender-specific and must not bypass
DND or authorization. Retry inbox_full, timeouts and transport failures with
the **same key, body and implementer**. Never switch to self-injection during
uncertainty. For example, AFT (the tool-execution provider) keeps authorization-
refused completions readable through bash_status and appends them to the
session's next tool result, rather than directly injecting a new notification
past DND. Capability absence is a discovery branch, not a terminal refusal
received from the inbox.

## 8. Vectors and conformance

`test-vectors/agent-inbox-v1/` ships in the package. Enable `test-vectors` for
the public `vectors` module: FILES, SHA256SUMS, README and file(name). Tests
check the complete disk inventory, checksums, typed shape round trips, refusal
finality, strict request rejection and exact neutralize pairs/idempotence.
The conformance crate publishes CASES, a Subject adapter and a report with
explicit narrowings. Its fake defect tests validate the kit, not a production
inbox. Receipt durability, retry-before-auth, replacement, clocks, withdraw,
namespace isolation, daemon mapping, wrapping, cap and pull are live cases.
