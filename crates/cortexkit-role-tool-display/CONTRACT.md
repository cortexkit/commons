# `tool-display/v1` — writer-authored transcript display

Stability: **alpha**. This document is normative. The Rust crate contains wire
shapes and pure validators, not a server, runner, template interpreter or store.
The shared vectors live in `test-vectors/tool-display-v1/` inside this package,
with `SHA256SUMS` pinning every vector and its README.

## 1. App-server discovery and composition

A module declares `app-server/v1` (`APP_SERVER_CAPABILITY`) when it serves the
apps' transcript reads, sends and watches. Those operations are the app
server's own; this contract fixes only what the capability means to a plan
composer, and says nothing about display storage. A plan composer MUST name the
single declaring module in the plan's `senders` and `readers` lists (the
llm-runner contract's §9.7 and §9.19). If two or more modules declare it, the
composer MUST name neither and log the ambiguity. With no declaration it names
no app server. A plan may still name other senders and readers.

A module independently declares `tool-display/v1` (`TOOL_DISPLAY_CAPABILITY`)
when it serves `display.write` and fetches provider `tool.views`. Providers
discover the **display server** by this capability, never by module name. A
provider writes records only when **exactly one live module** declares it.
Here "live" means listed in the daemon's catalog (`catalog.list`), whether or
not it is ready yet; a display server that is still warming refuses writes with
`module_warming`, and the provider may skip them.
With none or more than one, the provider writes nothing and never fails the
call because of the missing or ambiguous display server. A module may declare
either capability without the other; neither declaration implies the other.

The provider supplies display data after it has produced a call's reply. The display
server stores it; the app server joins it to the call when an app reads the
transcript.
How the two roles share stored data is outside this contract. They may be
served by the same module. Neither the tool
result shown to a model nor model-authored content supplies this display data.
The provider also supplies the row's schema-pinned view, whose record schema
and reads never change for that pin (tool-provider §2.1).

## 2. `display.write`

For tool data, the display server serves the named request
`{name: "display.write", arguments: {runner, call_key, kind, schema_pin, body}}`.
The params are `DisplayWrite`, a non-exhaustive struct with a constructor.

The protocol's opaque-field rule is **1–256 bytes**, each printable non-space
ASCII (`0x21`–`0x7e`); values are compared byte for byte, without interpretation.

- `runner` is subc-protocol's `Principal` JSON form, exactly as the provider saw
  the route's attested principal. For a relayed call it is `origin.carrier`.
- `call_key` is the runner's key, obeying subc-protocol's opaque-field rule.
  For a relayed call it is `origin.call_key`.
- `kind` is `record`, `detail`, `completion` or `message` for these params.
  The same operation accepts `wake` with the separate envelope in §8. Readers
  preserve an unknown kind as a string; the server refuses it as
  `invalid_params {field: "kind"}`.
- `schema_pin` is the pin the provider says it served and obeys the opaque-field
  rule. For a relayed call it is `origin.schema_pin`, defined by subc-protocol
  **0.34.2 and later**. This crate depends on 0.34.1 for `Principal` and opaque
  validation; it does not extract origin fields or provide a compatibility shim.
  A provider reads those fields from the call itself.
- `body` is a JSON object, never null, an array or a scalar.

The display server MUST store every record, detail, completion and message with
the writer as the daemon attested it on that `display.write` route: the route's
**attested opener**, never a writer claimed in the body or params. Only a
daemon-verified module (`Principal::Reserved`) may write. A route opened
`direct` or `unverified` is refused as `display_writer_unverified` before
anything is stored, because no join could ever accept its writes. `message`
admits only the specific reserved writer in §9 (`DisplayWrite::check_writer`).
The app server's join uses that stored attested writer. The display server
stores once per `(attested writer, runner, call_key, kind)`, not per pin. Keying by the
attested writer means a write from another module never blocks the real
provider's write for the same call; the provider-view join (§6) then uses only
a writer that serves the runner's pin. For `message` only, the two attested
writer names in §9 share a key.
It compares the body's RFC 8785 JCS canonical bytes, not received spacing or key
order. `canonical_body` computes those bytes; `idempotency_key` encodes the
tuple unambiguously as JCS. The first write answers `{"result":"stored"}`.
The same bytes again answer `{"result":"already_stored"}`. Different bytes
are refused as `display_write_conflict` (`display_record_conflict` for
`message`); they never replace the first write. The check and insertion must
be atomic. `DisplayWrite::write_outcome` classifies a comparison for the kind
but performs no persistence or concurrency control.

Refusals use subc-protocol's `ErrorBody` (`code`, `message`, optional `detail`):

- `display_too_large`, detail `{kind, max_bytes}`, before anything is stored;
- `invalid_params`, detail `{field}`, for an invalid parameter;
- `display_write_conflict`, for different bytes at the same key for `record`,
  `detail` or `completion`;
- `display_record_conflict`, for a different `message` body at the same key;
- `display_writer_unverified`, for a write whose route opener isn't a
  daemon-verified module or isn't the authorized writer for `message`;
- `module_warming`, the existing retryable refusal, while the display server is not
  ready. Retrying remains optional for display writers.

`validate_write` checks params, canonical size, detail cut metadata and the
`message` body without fetching a view. Record/completion schema checks are
separate so accepting a write never needs a network read or a previously
fetched view. A decoder's
structural error (including an invalid Principal or non-object body) must be
mapped to `invalid_params` naming the field by the serving implementation.

### 2.1 Pull read: `display.read`

1. **Op.** A provider serves the named request
   `{name: "display.read", arguments: {record_id}}` on its management surface
   (`DISPLAY_READ`, `DisplayRead`). The provider chooses `record_id`, which
   follows the opaque-field rule in §2. The reply is the exact `DisplayWrite`
   params it would have sent to `display.write` for that record: `runner`,
   `call_key`, `kind`, `schema_pin` and `body`, not a new envelope. The same
   validation and size caps apply; `validate_write` must accept the reply.
   `DisplayRead::validate` checks the identifier; structural decode failures
   must be mapped to `invalid_params` naming the field by the provider.
2. **Caller.** A provider answers only the single live module declaring
   `tool-display/v1` under §1's discovery rule, matched by the route's attested
   `Principal::Reserved`. Every other caller is refused `display_read_denied`
   (final). With zero or several declaring modules, it answers nobody.
3. **Refusals.** These use subc-protocol's `ErrorBody`:
   - `display_read_denied` (`DISPLAY_READ_DENIED`), final;
   - `display_record_unknown` (`DISPLAY_RECORD_UNKNOWN`), final: the record was
     never stored or is past retention;
   - `invalid_params`, detail `{field}`;
   - the existing retryable `module_warming`, while the provider is not ready.
4. **Writer on the display server.** The display server stores a pulled record
   with writer `Principal::Reserved` naming the module it called, and only when
   the daemon's catalog lists that module as daemon-verified. The storage key
   and idempotency rules are the same as `display.write` in §2: a record arriving
   by both paths is stored once (`already_stored`), and different canonical body
   bytes at that key are still `display_write_conflict` for provider display data.
   `message` uses the push path in §9.
5. **Wake event.** After a record commits, the provider publishes one bus event
   per record on subject `event.<provider>.display_record_ready.v1`: event name
   `display_record_ready` (`DISPLAY_RECORD_READY_EVENT`), version 1
   (`DISPLAY_RECORD_READY_VERSION`), with no reader suffix. The body is exactly
   `{"record_id":"..."}` (`DisplayRecordReady`), with nothing else, and
   `Nats-Msg-Id` is `record_id`. The event body is strict: readers reject unknown
   fields, so adding a field later is a readers-first change.
   `DisplayRecordReady::validate` checks the same opaque identifier rule.
   The display server acks the event only after it has stored the record or
   received a final refusal.
6. **Retention.** A provider keeps each record readable by `display.read` for
   as long as it would keep that record to push it with `display.write`, and at
   least 10 minutes after the wake event. Past that window, or after a provider
   restart that drops it, `display.read` answers `display_record_unknown`, and
   the display server drops the record. The pull adds no durability beyond the
   push path. It lets the display server fetch, within the provider's window,
   records announced while it was down. Keeping records in a bounded in-memory
   outbox meets this rule; no provider needs a durable store for it. A provider
   states its window in its own documentation.
7. **Rollout.** Providers keep `display.write` until the display server's reader
   has run on them. Both paths coexist; `display.write` is unchanged.

## 3. Records and completions

A record is a **flat object**. Its standard fields precede provider-specific
fields in the logical schema; JCS sorts keys and gives them no presentation
order. A record requires `durationMs` and `outcome`:

| Field | Rule |
| --- | --- |
| `durationMs` | non-negative i64 JSON integer: provider processing time, excluding queued time and time held waiting for a person |
| `outcome` | `success`, `error`, `partial` or `held` |
| `reason` | optional code matching `^[a-z][a-z0-9_]{0,63}$` |
| `queuedMs`, `heldMs` | optional non-negative i64 JSON integers; `heldMs` is time held waiting for a person, omitted when there was none |
| `location` | optional, only `outside_project` |

Known reasons include `timed_out`, `killed`, `cancelled`, `permission_denied`,
`consent` and `login_handoff`. Reasons are an open vocabulary of valid codes.
If a path would fall outside the project, the provider omits that path field
and sets `location: "outside_project"`; it never sends an absolute path instead.

A call has **at most one completion**, recording the end of its deferred work.
A held call's first record, written after its held reply, carries
`outcome: "held"`. A held reply can mean the call waits for a person or that
its work is already running (tool-provider §4.1). Approval ends a person
wait, not the call's deferred work: if approved work runs in the background,
the completion is written when that work finishes, carrying the person-wait
time in `heldMs`. If approval is refused before the action starts, nothing
runs, and the completion records the refusal with `outcome: "error"` and a
`reason` such as `consent` or `permission_denied`. If the provider stops
deferred work itself and confirms it stopped, the completion records that,
with `reason: "killed"`; this contract sets no deadline for doing so. A
provider that loses track of work without knowing how it ended writes no
completion. The completion is written once under the completion kind's
idempotency key. It requires `outcome`, permits
`reason`, and makes `durationMs` optional: deferred run time can be a declared
provider field. All other record field rules apply unchanged.

### 3.1 Field kinds

`RecordSchema` is `{name, revision, fields, completion_fields?}`, with a u64
integer `revision`. Each map declares provider extension names and their kinds.
`fields` applies to both records and completions; `completion_fields` adds
fields only for completions. An absent `completion_fields` means an empty map;
an empty map is omitted when serialized. Neither map may redefine a standard
field or declare a name already present in the other map. The
`completion_fields` map is part of a view's `record_schema`, which never changes
for a given pin. A provider adds a completion field only with a new schema pin.

A declared extension is optional. Except for `text_lines`, a present field
must be one scalar of its kind or a **nonempty array of at most 64 scalars** of
that kind. Nested objects, arrays of arrays, nulls, wrong JSON types and every
undeclared extension fail the schema check. Standard fields are always
validated by their fixed rules, never by declarations. The validators can't
tell whether a string that passes the rules is meaningful. The fixed `message`
body (§9) is not a provider extension schema and adds no `FieldKind`.

The display server doesn't check bodies against the schema when it stores a
write, because the schema comes from the view, fetched later (§6). The app
server checks a record or completion against its view's schema when it joins
it (`validate_record`, `validate_completion`), and leaves a body that fails out
of the transcript, showing the call without display data.

| Kind | Rule |
| --- | --- |
| `code` | string matching `^[a-z][a-z0-9_]{0,63}$` |
| `id` | string following subc-protocol's opaque-field rule |
| `agent_id` | same as `id` |
| `timestamp` | i64 integer milliseconds since Unix epoch, not a string |
| `integer` | i64 JSON integer, not a string or floating-point number |
| `boolean` | JSON boolean, not a string |
| `project_path` | nonempty project-relative string, at most 1,024 UTF-8 bytes; `/` separators; not absolute, a drive path or UNC path; no `..` segment or NUL |
| `web_origin` | lowercase `scheme://host[:port]`, with no userinfo, path (even `/`), query or fragment |
| `app_id` | nonempty reverse-DNS id or executable base name, at most 255 bytes, matching `[A-Za-z0-9._-]+`, with no path separator |
| `text_lines` | only in `completion_fields`: JSON array of zero to three line objects |

A `text_lines` line object requires `text`, a string of 1–200 Unicode scalar
values with no control characters (`char::is_control()`, including line breaks,
tabs and ESC). It may have a `tone` of `"muted"` or `"danger"`; omitting `tone`
uses the default tone. No other members are allowed. An invalid item is refused
as an invalid field, with its index and, when applicable, member name (for
example `tail[1].text`). A declaration outside completion fields is refused as
an invalid schema, naming the field.

Writers strip ANSI escape sequences and cut a long line with a trailing `…`,
which counts toward the 200-scalar limit. Providers are responsible for limiting
the content to text the chat's step log already shows for the same call.
Upgrade display servers to 0.1.12 before any provider declares `text_lines`.
Older readers ignore `completion_fields` and refuse a completion carrying an
undeclared field. The completion is lost, so the row stays without its tail.

Apps render every kind as **plain text, never a link**, with one exception:
an app may pass a value of an `id` field to its own item opener. The opener
handles only the ids the app already opens from links in chat messages: a
question for the person (`ask_`), an artifact (`art_`) or a work item
(`wi_`). Which of those an app can open is the app's own; an id it can't
resolve stays plain text. The opener shows the app's own card for the item and
never treats an id as a URL, so a wrong or forged id opens an item the app
already holds or stays plain text, never a web page. The crate supplies
`FieldKind::validate_scalar` for scalar kinds, `FieldKind::validate` for full
field values, and `validate_record` / `validate_completion` to check required
fields, fixed fields and provider declarations together.

## 4. Details

A detail is any JSON object, with optional standard cut metadata:
`truncated: true`, `omittedItems` and `omittedBytes` (u64 JSON integers).
Other members are free-form, including nested objects. A provider whose detail
would exceed the **served** limit cuts it itself and sets the cut metadata,
rather than sending an oversized write and being refused. `validate_detail`
checks the standard metadata without interpreting provider content.

## 5. Served limits

The display server includes the `tool_display` (`DESCRIBE_GROUP`) group in
`role.describe`:

```json
{"tool_display":{"max_record_bytes":16384,"max_detail_bytes":3145728,"max_completion_bytes":16384,"loading_window_ms":1000}}
```

The four fields are u64 JSON integers (`DisplayLimits`). Byte counts measure
the body's JCS bytes. Pinned ceilings are 16 KiB for a record, completion or
message, and 3 MiB for a detail. Messages use `max_record_bytes`. A server may
serve less (including zero); clients read the served values rather than assuming
the ceilings. `loading_window_ms` is
how long a reader may show a call as loading its display data, counted from
when the runner's transcript records the call's answer. After it, a reader
shows the call without display data until any arrives. Every complete write frame, including envelope and params,
must stay **under** the daemon's 4 MiB limit. Providers must account for framing
and identity overhead as well as the body; a body ceiling is not a frame limit.

## 6. Join and view authorization: app-server obligations

For `record`, `detail` and `completion`, the display server stores a write
first; the app server checks it when joining
a transcript. The app server uses a write only when the **runner's own stored
call pin** has a view that the write's attested writer (§2) served. The runner
pin comes from the runner's stored call, never from the write's `schema_pin`,
so a forged write pin cannot select a view or prove a match. Otherwise the app
server drops the write from the join. Another writer's view cannot authorize
it.

The display server MUST ask `tool.views` for every pin it needs, including a
pin outside the writer's current `tool.catalog`, **before judging catalog
membership**. It decides from the answer:

- If the view carries `tool`, the server computes its `schema_digest` from
  `tool.input_schema` and recomputes the tool-provider contract's `tp1` pin
  from `tool.name`, that digest and `tool.semantics`. It accepts the view only
  when that pin equals the requested pin, and takes the input schema from
  `tool.input_schema`; no current catalog membership is needed.
- A view without `tool` is accepted only if the requested pin belongs to a
  tool in the writer's current `tool.catalog`: the `tp1` pin computed from the
  catalog tool's `name`, `schema_digest` and `semantics` must equal it. The
  input schema then comes from that catalog entry, as for older providers.
- Anything else is `unknown` and cannot authorize a join: no view, `"unknown"`,
  a `tool` that cannot recompute to the requested pin, or no `tool` and no
  matching current catalog entry. A mismatched `tool` never falls back to the
  catalog check.

The server keeps the accepted result and input schema with the view, through
later template refreshes, so a call recorded before the provider changed its
catalog still joins. A matching pin fixes the tool's name, structural input
schema and `semantics`; it doesn't fix `description` text or say which
provider owns the tool. A server that cached a pin as unusable only because
the pin was missing from the writer's current catalog, without asking
`tool.views`, MUST ask for that pin again once and judge the answer under
these rules. A provider's own `"unknown"` answer keeps its normal retry rules.

For one call (`runner`, `call_key`), if stored writes from more than one
writer each have an accepted view for the runner's stored call pin, the app
server uses none of those writes for that call. That covers another provider
listing an identical tool, since the matching pin doesn't say which provider
owns it.

The display server fetches `tool.views {pins: [...]}` on the **writer's own route**,
for unseen pins and again when the writer's `module_version` or, where its
catalog entry carries one, declared build changes, and **never on the write
path**. It may also fetch again under its own cache-age or template-validation
policy (tool-provider §2.1). The optional operation
and `tool-views/v1` capability are defined by the
[`tool-provider/v1` contract](../cortexkit-role-tool-provider/CONTRACT.md#21-optional-display-views).
The reply is `{views: {<pin>: <ViewManifest> | "unknown"}}`. A manifest has a
record schema, declared reads, a template and an optional pinned `tool`
definition (tool-provider §2.1). The template is opaque JSON
that only the app server interprets; this contract defines no template format,
so which templates an app server understands is its own choice. An `unknown`
view cannot authorize a join.
A pin never changes meaning, so the display server may keep a view's record
schema and reads forever; it replaces the template only with a higher
`template_revision` (tool-provider §2.1).

## 7. Tool-provider obligations

For `record`, `detail` and `completion`, a provider writes only **after its
handler has produced the call's reply** (a held reply included), asynchronously,
off the reply path. It never waits on
the write, never delays the reply for it, and never fails a call because of
it. The rule doesn't require proof that the reply frame was flushed first:
starting the write once the handler has returned the reply is enough. Records
are keyed by the call and joined when an app reads, so the order of the write
and the reply frame doesn't change which call a record joins; at most an app
sees a record a moment before the runner records the call's answer. It takes the runner from the route's
attested principal, and the call key and served pin from the call's own
`call_key` and `schema_pin` members, or `origin.carrier`, `origin.call_key` and
`origin.schema_pin` for a relayed call; never from tool arguments. Only a call
that carries both a key and a pin qualifies. A call without either gets no
display write.
It discovers the display server by `tool-display/v1`, never by module name,
and writes nothing unless exactly one live module declares that capability.
Retries are allowed, never required. A retry resends the **first canonical
body bytes exactly**, not freshly measured durations or recomputed facts.
Here a scope carrier is the route opener carrying a daemon-stamped scope on
behalf of that scope's owner; the scope is not taken from request arguments.
On a route that the app server opened as a scope carrier, the provider admits
only the reads its views declare. A declared read is a provider operation that
is not offered to the model as a tool, which the app server calls to page
through a call's details; its request and reply shapes are the provider's own. A view declaration grants no broader route
authority and does not turn a read into a model tool.

## 8. Wake records

Wake records describe the bundle that woke an agent, not a tool call. The writer
declares `wake-record-writer/v1` (`WAKE_RECORD_WRITER_CAPABILITY`); the display
server declares `wake-record-display/v1` (`WAKE_RECORD_DISPLAY_CAPABILITY`).
Neither writes nor serves wake records unless the other has declared its token.
As with tool display discovery, the writer writes only to exactly one live
declaring display server, and skips writes when discovery is missing or ambiguous.
The tokens are independent of `tool-display/v1` and `app-server/v1`.

### 8.1 Writes, identity and lookup

The named request is `{name: "display.write", arguments:
{kind: "wake", runner, session, send_id, body}}` (`WakeWrite`). `runner` is the
destination runner's `Principal`; `session` and `send_id` obey the opaque-field
rule in §2. `send_id` is the writer's own send id. When the runner delivers the
send riding a tool result instead of as its own message (llm-runner §9.7), the
notice it records for that delivery names the same `send_id`, so the record
matches either delivery. A writer keys every send it writes a record
for, because a runner-assigned `submission_id` arrives only after the send.
An unkeyed send gets no wake record and its row draws the plain card.

The sender is exclusively the route's daemon-attested writer, verified by
`check_writer` exactly as in §2. There is no claimed `sender` member in either
envelope or body; a payload attempting to supply one is malformed
(`invalid_params`). A direct or unverified writer is refused
`display_writer_unverified`. Only the sending module may write for its own
sends: another module cannot claim its identity, and its writes cannot join
rows whose attested `sender` differs.

The immutable storage key is `(session, attested sender, send_id)`.
`wake_idempotency_key` encodes it using the RFC 8785 JSON Canonicalization
Scheme (JCS). The server stores canonical body bytes
atomically; identical retries answer `already_stored`, different bytes at that
key are `display_write_conflict`, and the first write answers `stored`, as in
§2. The destination runner is provenance, not an extra key component. Stored
records never change and the display server retains them for **180 days**.

Apps fetch records beside the chat page, **by message id** for the rows they
draw, and never parse wake prompt text. The server reads `(sender, send_id)`
from the runner's row in the session requested by the app, never from prompt
text or a claimed writer. A record with that pair in the same session matches
immediately. For a row moved to another session (including an imported head),
it matches only when exactly one stored record has that pair across sessions.
Two or more are ambiguous: return no record and draw the row's plain card.
Missing or unknown display facts likewise mean no match. The app operation
`session.wake_records` and the watch hint belong to the display server's app
protocol, **not this crate**; that protocol reuses `WakeRecord` defined here.

Writers write before sending, with their own short deadline. A missed write
deadline never holds back or fails the send: the row then draws the existing
plain card. Retries retain the first canonical bytes, never refreshed facts.

### 8.2 Record and item shapes

`WakeRecord` is `{items: [WakeItem, ...], omittedItems?: n}`. An item is
`{kind, refs, from?, to?, at, excerpt}`. Field spelling is snake_case for refs
and envelope fields; `omittedItems` deliberately reuses the existing display
cut marker. `at` is an i64 JSON integer in milliseconds since Unix epoch.
`from` and `to` are `{agent_id, display_name?}` (`WakeAgent`), reusing the
registry fleet's agent identity and display-name concepts with snake_case
spelling. A registry id is `agent_` followed by 8 or 16 lowercase hex digits.
The optional name is a fallback for an agent not listed in the fleet; these
identities are not module principals.

`excerpt` is **plain text, not Markdown**, at most **200 Unicode scalar values**.
The writer cuts it from the text the model actually received, never authors a
separate summary. When cut it ends with `…`, included in the 200-character
limit. Validators can check length but cannot prove provenance or detect
whether arbitrary text was intended as Markdown; those remain writer duties.

Writers use the following closed set. On the wire `kind` is an open string.
Readers preserve unknown kinds, including their refs, and the app draws a
plain card instead of failing the record. `WakeRecord::check` accepts unknown
kinds; `check_writer` on the record refuses them. Known refs are checked even
by readers; required strings obey the opaque-field rule unless stated below.

| Kind | `refs` | Additional rules |
| --- | --- | --- |
| `peer_message` | `{id}` | `id` is the delivered message's id under the opaque-field rule, not only `pm_…` (core also delivers room invites and removals as peer messages); `from` and `to` required |
| `room_post` | `{room, first_seq, last_seq}` | u64 integer sequences, `first_seq <= last_seq`; one item covers a fold of posts from one room |
| `worker_question` | `{ask, task}` | both required |
| `work_result` | `{task, run?, item?}` | optional refs are validated when present |
| `sidekick_answer` | `{id}` | required |
| `approval_card` | `{id}` | starts `acc_` or `el_` |
| `ask_answer` | `{ask, question, answer, by, round?}` | `ask` is an opaque ask id starting with `ask_`; other refs are described below |
| `notice` | `{family, title}` | `family` is an opaque identifier; `title` is nonempty plain text |

For `ask_answer`, `question` is nonempty plain text of at most 200 Unicode
scalar values. For a form it is the form's title. `answer` is an object with
exactly one member:

- `option`: the selected option's label, nonempty plain text of at most 80
  Unicode scalar values.
- `text`: the person's words as plain text, at most 200 Unicode scalar values.
- `form`: an object with only `answered` and `total`, both integers, with
  `1 <= total <= 6` and `0 <= answered <= total`.

When `question` or `answer.text` is cut, it ends with `…`, counted in the limit
as for `excerpt`. `by` is `"person"` or `"default"`; `"default"` means the ask's
default applied because nobody answered. `round`, when present, is an integer
of at least 2 indicating a form's follow-up round.

Display servers must adopt 0.1.11 or later before any writer emits `ask_answer`:
older servers refuse the new kind on write. Older readers still preserve the
record and draw a plain card for the unfamiliar kind.

`notice` covers every bundle member outside the other kinds, including consult
results, flow digests, stall alerts, worker notices and module notices, so a
stack never silently loses part of its bundle. Ask answers use `ask_answer`
once writers switch; `notice` remains valid for ask answers in the meantime.

### 8.3 Limits and refusals

A record's canonical JCS body is at most **16 KiB** and it contains at most
**64 items**. These wake ceilings are fixed, not additions to `DisplayLimits`:
§5's served tool limits remain unchanged. A writer exceeding either keeps the
first items that fit both limits and sets `omittedItems` to the number omitted,
including the marker itself in the encoded byte budget. The server refuses an
oversized record as `display_too_large` with `{kind: "wake", max_bytes: 16384}`
and never cuts it itself. Malformed known kinds, missing peer agents and
overlong excerpts are `invalid_params` naming the field. `WakeWrite::check`
checks the envelope, verified writer, record and closed writer vocabulary;
structural decode failures must also be mapped to `invalid_params`.

## 9. Message records

- [pinned] For `say`, the writer sends one `display.write` with `kind: "message"`
  before answering the call. `runner`, `call_key` and `schema_pin` come from the
  call as in §2. `body` is a `MessageRecord`; it never carries the message text.
  The app server takes that text from the runner's stored `say` input and uses
  the record as message metadata only when the stored call succeeded and has
  string `text`. This path does not use provider views (§6).
- [pinned] In this version, only `reserved:prefrontal` and its compatibility
  alias `reserved:prefrontal-core` may write a message. The alias lets the same
  writer retry under its old or new name during rollout. The display
  server checks the daemon-attested route opener (`DisplayWrite::check_writer`)
  and refuses any other writer, even a different reserved module, as
  `display_writer_unverified`.
- [pinned] The server keeps the actual attested writer. For `message` only,
  `idempotency_key` encodes both permitted names as `reserved:prefrontal` in §2's
  tuple, so a retry under either name addresses the same immutable record.
  Identical canonical body bytes answer `already_stored`; different bytes at
  that key are refused as `display_record_conflict`. When an agent moves to
  another computer, its chats' message records move with the rest of its
  display records, under the same key, so the chat still draws them there.
- [pinned] The body and every nested object are closed. All listed members
  are required unless marked `?`; optional members are omitted, not `null`.
  All counts, sizes, dimensions, durations and pull request numbers are u64
  JSON integers (non-negative, not strings or floats). The body is:

  | Member | Rule |
  | --- | --- |
  | `v` | Integer `1`. |
  | `reply_to?` | String: the runner's message id of the person's message being answered. |
  | `quote?` | String, at most 280 Unicode scalar values. Requires `reply_to`. The writer includes it only if it exactly matches the person's words in that message. |
  | `media` | Array of at most 8 media objects; empty when there are none. |
  | `level` | `chat`, `info`, `attention` or `urgent`. |
  | `key?` | String naming a standing condition, at most 64 Unicode scalar values. |
  | `done?` | Boolean; requires `key`. If true, this message clears that condition. |
  | `deliver?` | `desktop`, `phone` or `none`. Required unless `level` is `chat`. |

- [pinned] Each media object has a `kind` tag and exactly the members for that
  kind. `artifact` and `mime` are strings; artifacts identify resolved files.

  | `kind` | Other members |
  | --- | --- |
  | `image` | `artifact`, `mime`, `byteCount`, `width`, `height`, `title?`. |
  | `video` | `artifact`, `mime`, `byteCount`, `width`, `height`, `durationMs?`, `poster?` (`{artifact, width, height}`), `title?`. |
  | `file` | `artifact`, `mime`, `byteCount`, `name` (display name). |
  | `link` | `url` (absolute `http://` or `https://` URL with a host), `title?`, `description?`. Apps do not fetch it; without a title they display the host. |
  | `diff` | `artifact` (the `text/x-diff` unified diff), `repo` (`{host?, owner?, name}`), exactly one of `commit` (SHA), `range` (`{base, head}`, both SHAs) or `pr` (`{number, head?}`, optional SHA), `files`, `added`, `removed`. A local checkout can use `{name}` alone and draws no repo link. |

  A SHA is a full 40- or 64-digit hexadecimal Git object id. `files` is an array
  of at most 200 entries, each `{path, added, removed, from?}` or
  `{path, binary: true, from?}`. `path` and `from` are strings; `from` is the
  previous path of a renamed file. Apps draw the diff card from these members,
  reading the artifact only when the person opens it.
- [pinned] The verified quote and media titles are bounded plain-text strings
  written by the privileged writer, not a free-text kind under §3.1. The quote
  has its own character limit; titles, file names, link descriptions and repo
  members are strings bounded by the body's byte cap. The link target is a
  separate, validated `url` member; display strings are not link targets.
- [pinned] The canonical JCS body is subject to `max_record_bytes`, at most
  16 KiB (§5). The server refuses excess bytes as `display_too_large` with
  `{kind: "message", max_bytes}` and does not truncate. Malformed bodies are
  `invalid_params` with `{field}` naming the member, for example
  `body.media[0].poster.width`. A missing or conflicting diff selector names
  its media item. `validate_message` decodes the closed body and validates its
  member rules; `validate_write` also checks the envelope and canonical size.
  The writer is responsible for verifying the quote and resolving artifacts;
  structural envelope decode failures must also be mapped to `invalid_params`.
