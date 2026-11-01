# `tool-display/v1` — provider-authored transcript display

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

The display server serves the named request
`{name: "display.write", arguments: {runner, call_key, kind, schema_pin, body}}`.
The params are `DisplayWrite`, a non-exhaustive struct with a constructor.

The protocol's opaque-field rule is **1–256 bytes**, each printable non-space
ASCII (`0x21`–`0x7e`); values are compared byte for byte, without interpretation.

- `runner` is subc-protocol's `Principal` JSON form, exactly as the provider saw
  the route's attested principal. For a relayed call it is `origin.carrier`.
- `call_key` is the runner's key, obeying subc-protocol's opaque-field rule.
  For a relayed call it is `origin.call_key`.
- `kind` is `record`, `detail` or `completion`. Readers preserve an unknown
  kind as a string; the server refuses it as `invalid_params {field: "kind"}`.
- `schema_pin` is the pin the provider says it served and obeys the opaque-field
  rule. For a relayed call it is `origin.schema_pin`, defined by subc-protocol
  **0.34.2 and later**. This crate depends on 0.34.1 for `Principal` and opaque
  validation; it does not extract origin fields or provide a compatibility shim.
  A provider reads those fields from the call itself.
- `body` is a JSON object, never null, an array or a scalar.

The display server MUST store every record, detail and completion with the
writer as the daemon attested it on that `display.write` route: the route's
**attested opener**, never a writer claimed in the body or params. Only a
daemon-verified module (`Principal::Reserved`) may write. A route opened
`direct` or `unverified` is refused as `display_writer_unverified` before
anything is stored, because no join could ever accept its writes
(`check_writer`). The app
server's join uses that stored attested writer. The display server stores once
per `(attested writer, runner, call_key, kind)`, not per pin. Keying by the
attested writer means a write from another module never blocks the real
provider's write for the same call; the join (§6) then uses only a writer that
serves the runner's pin.
It compares the body's RFC 8785 JCS canonical bytes, not received spacing or key
order. `canonical_body` computes those bytes; `idempotency_key` encodes the
tuple unambiguously as JCS. The first write answers `{"result":"stored"}`.
The same bytes again answer `{"result":"already_stored"}`. Different bytes
are refused as `display_write_conflict`; they never replace the first write.
The check and insertion must be atomic. `write_outcome` classifies a comparison
but performs no persistence or concurrency control.

Refusals use subc-protocol's `ErrorBody` (`code`, `message`, optional `detail`):

- `display_too_large`, detail `{kind, max_bytes}`, before anything is stored;
- `invalid_params`, detail `{field}`, for an invalid parameter;
- `display_write_conflict`, for different bytes at the same key;
- `display_writer_unverified`, for a write whose route opener isn't a
  daemon-verified module;
- `module_warming`, the existing retryable refusal, while the display server is not
  ready. Retrying remains optional for display writers.

`validate_write` checks params, canonical size and detail cut metadata without
fetching a view. Record/completion schema checks are separate so accepting a
write never needs a network read or a previously fetched view. A decoder's
structural error (including an invalid Principal or non-object body) must be
mapped to `invalid_params` naming the field by the serving implementation.

## 3. Records and completions

A record is a **flat object**. Its standard fields precede provider-specific
fields in the logical schema; JCS sorts keys and gives them no presentation
order. A record requires `durationMs` and `outcome`:

| Field | Rule |
| --- | --- |
| `durationMs` | non-negative i64 JSON integer: provider processing time, excluding queued time and time held waiting for a person |
| `outcome` | `success`, `error`, `partial` or `held` |
| `reason` | optional code matching `^[a-z][a-z0-9_]{0,63}$` |
| `queuedMs`, `heldMs` | optional non-negative i64 JSON integers |
| `location` | optional, only `outside_project` |

Known reasons include `timed_out`, `killed`, `cancelled`, `permission_denied`,
`consent` and `login_handoff`. Reasons are an open vocabulary of valid codes.
If a path would fall outside the project, the provider omits that path field
and sets `location: "outside_project"`; it never sends an absolute path instead.

A **completion** records call-time facts of when deferred work ended, such as
a background command's exit or a held call settling later. A held call's
first record, written after its held reply, carries `outcome: "held"`; its
later result goes in the completion. It is written once,
under the completion kind's idempotency key. It requires `outcome`, permits
`reason`, and makes `durationMs` optional: deferred run time can be a declared
provider field. All other record field rules apply unchanged.

### 3.1 No free-text kinds

`RecordSchema` is `{name, revision, fields: {<field>: <kind>}}`, with a u64
integer `revision`. `fields` declares **every provider extension field**, not
just strings. It cannot redefine a standard field. A declared extension is
optional, but when present must be one scalar of its kind or a **nonempty array
of at most 64 scalars** of that kind. Nested objects, arrays of arrays, nulls,
wrong JSON types and every undeclared extension fail the schema check. The same
schema describes records and completions; standard fields are always validated
by their fixed rules, never by declarations. There is no free-text kind: each
kind is a code, an identifier, a number, a boolean, a project path, a web
origin or an app id, checked by its lexical rule. The validators can't tell
whether a string that passes those rules is meaningful.

The display server doesn't check bodies against the schema when it stores a
write, because the schema comes from the view, fetched later (§6). The app
server checks a record or completion against its view's schema when it joins
it (`validate_record`, `validate_completion`), and leaves a body that fails out
of the transcript, showing the call without display data.

| Kind | Scalar rule |
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

Apps render every kind as **plain text, never a link**, with one exception:
an app may pass a value of an `id` field to its own item opener. The opener
handles only the ids the app already opens from links in chat messages: a
question for the person (`ask_`), an artifact (`art_`) or a work item
(`wi_`). Which of those an app can open is the app's own; an id it can't
resolve stays plain text. The opener shows the app's own card for the item and
never treats an id as a URL, so a wrong or forged id opens an item the app
already holds or stays plain text, never a web page. The crate supplies one
validator per kind, `FieldKind::validate_scalar`, the scalar-or-array
`FieldKind::validate`, and `validate_record` / `validate_completion` to check
required fields, fixed fields and provider declarations together.

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
the body's JCS bytes. Pinned ceilings are 16 KiB for a record or completion,
and 3 MiB for a detail. A server may serve less (including zero); clients read
the served values rather than assuming the ceilings. `loading_window_ms` is
how long a reader may show a call as loading its display data, counted from
when the runner's transcript records the call's answer. After it, a reader
shows the call without display data until any arrives. Every complete write frame, including envelope and params,
must stay **under** the daemon's 4 MiB limit. Providers must account for framing
and identity overhead as well as the body; a body ceiling is not a frame limit.

## 6. Join and view authorization: app-server obligations

The display server stores a write first; the app server checks it when joining
a transcript. The app server uses a write only when the **runner's own stored
call pin** has a view that the write's attested writer (§2) served. The runner
pin comes from the runner's stored call, never from the write's `schema_pin`,
so a forged write pin cannot select a view or prove a match. Otherwise the app
server drops the write from the join. Another writer's view cannot authorize
it.

When the display server first fetches a view for a pin, it also checks that the
pin belongs to a tool in that writer's `tool.catalog`: the pin computed from
the tool's `name`, `schema_digest` and `semantics` (the tool-provider
contract's `tp1` pin) must equal it. It keeps that result with the view, since
a pin never changes meaning, so a call recorded before the provider changed its
catalog still joins.

The display server fetches `tool.views {pins: [...]}` on the **writer's own route**,
for unseen pins and again when the writer's `module_version` or, where its
catalog entry carries one, declared build changes, and **never on the write
path**. It may also fetch again under its own cache-age or template-validation
policy (tool-provider §2.1). The optional operation
and `tool-views/v1` capability are defined by the
[`tool-provider/v1` contract](../cortexkit-role-tool-provider/CONTRACT.md#21-optional-display-views).
The reply is `{views: {<pin>: <ViewManifest> | "unknown"}}`. A manifest has a
record schema, declared reads and a template. The template is opaque JSON
that only the app server interprets; this contract defines no template format,
so which templates an app server understands is its own choice. An `unknown`
view cannot authorize a join.
A pin never changes meaning, so the display server may keep a view's record
schema and reads forever; it replaces the template only with a higher
`template_revision` (tool-provider §2.1).

## 7. Tool-provider obligations

A provider writes only **after its handler has produced the call's reply**
(a held reply included), asynchronously, off the reply path. It never waits on
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
