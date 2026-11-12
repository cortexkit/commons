# user-commands/v1

This is the wire contract for commands a person runs directly from an app,
without asking a model. A declaring module owns the operation, its preview,
approval and audit. The app draws shared display parts and asks the person;
the relay checks the incoming caller and forwards verified facts.

Rules marked **[pinned]** are interoperability and safety requirements. Rust
types are non-exhaustive and have constructors. Validators are pure: they do
not implement a server, persistence, approvals, timing or auditing.

## Discovery and framing

**[pinned]** A module lists `user-commands/v1` in its manifest's capability
`provides` when it serves all three module operations on its **management
surface**. The command list is never in the manifest. Discovery uses the
daemon catalog; a relay may cache descriptions per catalog build.

Each operation is a request whose method is the operation name, with the
following params object. This is method-plus-params framing, not a tool call
with `{name, arguments}`. Replies are the bodies below, not another envelope.

| Surface | Method | Params | Reply |
| --- | --- | --- | --- |
| Module management | `commands.describe` | `{}` | `{commands: [Command], digest}` |
| Module management | `commands.preview` | `{id, subject, args, caller}` | `Preview` |
| Module management | `commands.run` | `{id, subject, args, preview_version, request_id, caller}` | `RunReply` |
| Relaying app server | `commands.list` | `{subject}` | `{commands: [{module, command: Command}]}` |
| Relaying app server | `commands.preview` | `{module, id, subject, args}` | `Preview` |
| Relaying app server | `commands.run` | `{module, id, subject, args, preview_version, request_id}` | `RunReply` |

**[pinned]** `digest` is 64 lowercase hexadecimal characters: SHA-256 of the
RFC 8785 (JCS) bytes of the **commands array alone**, in its served order.
`commands_digest` uses `serde_jcs`, without enabling serde_json's
`float_roundtrip` or `arbitrary_precision`. `DescribeReply::check` verifies the
digest as well as the declarations. A merged list contains commands applying
to the requested subject, qualified by module; ids are unique per module,
not globally.

Typed objects refuse unknown members, except the daemon-owned refusal body,
and facts, which retain extra members solely to diagnose them as malformed.
Optional members are omitted on write; readers accept `table: null` as absent.
Effects, fact units/states/tones, outcome codes and caller facts retain unknown
strings on decode. Decoding alone is not producer validation.

## Admission and audit

**[pinned]** A module admits **all three** management operations only from the
daemon-attested principal `reserved:occipital` (`ADMITTED_CALLER`). Once the
daemon's first-party mark is live, it additionally requires a first-party
caller. Admission happens before inspecting params or state. `check_admission`
takes the daemon facts and a deployment switch; neither comes from the body.

**[pinned]** The relay checks the incoming caller itself before forwarding
preview or run. Module-side `caller` is `{app, admission, device?, peer_build?}`:
known apps are `desktop | phone`, known admissions are
`operator_app | development | phone`. All are open string facts. `device` is
the phone fingerprint in hexadecimal, never a hard-coded identity.
`peer_build` is reserved for a later daemon fact; readers ignore it when
absent. Its presence in a body grants no authority.

**[pinned]** The module records `caller` in its own audit beside the attested
principal and never uses these forwarded claims as authority for anything.
Runs are person actions carried by an app/device, never agent actions. They
do not enter chat history, a transcript, or model input. An agent learns of a
`context_change` from its next request or one module-sent data notice.

## Commands and arguments

`Command` is `{id, title, summary, subject, args, effects, run?}`. The
declaration's **`subject` is a kind string** (`session | agent | computer`);
a request's **`subject` is an object identifying the thing** (§ Subjects).

**[pinned]** There are at most 64 commands per module, with unique ids. An id
is 1–64 ASCII characters `[a-z0-9_]`. A title is 1–40 characters and summary
1–240. Character limits count Unicode scalar values unless specified as bytes.

There are at most 8 args, each `{key, kind, label, default?, options?}`. Keys
follow the command-id rule and are unique within the command. Arg labels and
option labels are 1–40 characters. `options` is required only for `choice`,
with 1–32 `{value, label}` items; values are unique 1–64 character strings.
It is absent for every other kind. A default must satisfy the same value rule
as a request. No coercions are performed.

| Kind | Request/default value |
| --- | --- |
| `text` | JSON string, at most 2,000 characters (empty is allowed) |
| `number` | finite JSON number |
| `choice` | string equal to one declared option value |
| `toggle` | JSON boolean |
| `agent` | nonempty agent-id string, at most 128 characters |
| `session` | `{runner, harness, session}`, each 1–256 characters |

**[pinned]** Request `args` is an object keyed by declared keys. Unknown keys
are `invalid_params` naming the key. The **module applies defaults**: the app
may omit an arg the person did not change. An omitted arg with no default is
required and is refused. `Command::resolve_args` validates and fills defaults.

**[pinned]** Effects are a nonempty set from the table below, without duplicates.
`read` stands alone and has no `run`; its preview is the answer. Every non-read
command has `run: {}`. Neither arbitrary run metadata nor jobs are defined.
A reader encountering an unknown effect offers **no run, only the preview**;
`offers_run` enforces that fallback. Producers send only the listed effects.

| Effect | Meaning |
| --- | --- |
| `read` | Nothing changes. |
| `reversible` | A change the module can undo. |
| `ends` | Stops running work. |
| `narrow` | Removes reach or authority. |
| `writes_files` | Changes the person's files. The module backs up what it overwrites and says so in the preview. |
| `destroys` | Deletes for good. |
| `widen` | Grants reach or authority. |
| `context_change` | Changes an agent's prompt, history, or files/state it relied on. The change appears in its next request, or the module sends one notice worded as data after the run. |
| `spend` | Buys something the session would not otherwise pay for: an outside service or a provider call made only for the command. The preview includes the amount as facts. An unavoidable session cost brought forward is a fact, not `spend`. |

## Subjects

Requests name exactly one of:

```json
{"kind":"session","session":{"runner":"example-runner","harness":"example-host","session":"example-session"}}
{"kind":"agent","agent_id":"example-agent"}
{"kind":"computer"}
```

The session triple mirrors the compaction role's `SessionRef` without depending
on that crate. Each part is 1–256 characters, used without normalization.
An agent id is 1–128 characters. The app supplies the subject from its current
page; nothing infers it. Checking shape does not prove the subject exists.

**[pinned]** In a session subject, `runner` is the module id of the runner that
hosts the session, never the name of a host process or agent program, and
`harness` is that runner's harness name, exactly as the runner's compaction
`SessionRef` carries them. Apps take `{runner, harness, session}` verbatim from the app server's
session details and never assemble it. A session that no runner module hosts,
such as one inside an agent program running outside every runner module, has no
session subject, so no module offers it session commands.

## Previews and display data

`Preview` is `{version, available, reason?, effect?, facts?, figure?, table?}`.
`version` is opaque, 1–128 printable ASCII **bytes** (`0x20`–`0x7e`), compared
byte for byte. `available: false` requires a reason code. Reasons are
1–64 characters `[a-z0-9_]`; modules may add codes. Apps word known codes and
use a generic line for unknown ones. `effect` is one line, 1–120 characters.

**[pinned]** A preview is a local, bounded read. It never calls an outside
service and never spends. Its version covers every state change that would
invalidate what the person sees. Producer duties such as locality cannot be
established by a body validator.

`facts` contains at most 16 facts; apps show four in a row. A `figure` is
`{label, value, unit, was?}` with a 1–64 character label, finite numeric value
and optional finite prior value, and a listed unit. A `table` has 1–6 column
labels (1–40 characters each) and up to 32 rows, each `{label, cells}` with a
1–64 character row label. The number of cells equals the number of columns.
Cells are `FactValue`, the fact shape without `label`.

### Facts [pinned]

`Fact` is `{label, value?, unit?, noun?, approx?, state?, progress?, tone?}`.
`label` is 1–64 characters. Exactly one of numeric `value` or `state` exists:
there are no free-text values. A value is finite. `unit` is optional, defaults
to `count`, and is one of `count | tokens | bytes | duration | ratio`.
Duration is milliseconds; ratio treats 1 as the whole and **may exceed 1**.
`time` is not a v1 unit. `noun` is 1–24 characters and goes only with `count`.
`approx` is a boolean on number facts only; true is printed as "about".

`state` is `ready | building | paused | stale | unavailable | off`.
`progress` is a finite number in 0–1, only on `building`. State tones are fixed.
Number facts may set `tone` to `neutral | info | success | warning | danger`,
overriding neutral. No state may carry unit, noun, approx or tone, even false
`approx`. Unknown units, states and tones survive typed decoding, but fail
validation; producers send only the listed values.

Fact validators use the app library's diagnostic classes:

| Class | Cause |
| --- | --- |
| `fact_malformed` | Unknown member, unit, state or tone; a mistyped field (including text value); missing label. |
| `fact_conflict` | Both/neither value and state; unit/noun/approx/tone on a state; progress except on building; noun except with count. |
| `fact_range` | Label empty or over 64; noun empty or over 24; progress outside 0–1; non-finite number. |

`validate_fact` and `validate_fact_value` classify raw JSON, including structural
mistakes; typed `check` methods validate decoded data and in-memory non-finite
numbers. An app shows a refused fact as its label with a neutral placeholder;
one refused fact never fails the whole preview. Producer `Preview::check`
is intentionally stricter: all its facts must validate. A fact diagnostic is
mapped to `invalid_params {field}` if used to refuse a request on wire.

Informative state wording (not pinned):

| State | Wording | Tone |
| --- | --- | --- |
| `ready` | Ready | success |
| `building` | Building, or "Building, 40%" with progress 0.4 | info |
| `paused` | Paused | warning |
| `stale` | Out of date | warning |
| `unavailable` | Unavailable | danger |
| `off` | Off | neutral |

## Runs and replies

`RunReply` is `{outcome, facts?, open?}`. `open` is `{kind, id}`, kind one of
`ask | artifact | work_item`, with a nonempty id. An id is not a URL. Outcome
codes are 1–64 characters `[a-z0-9_]`; modules may add codes. Apps word known
codes and fall back to a generic line for unknown ones.

| Common outcome [pinned] | Meaning |
| --- | --- |
| `applied` | The change is already in effect. |
| `scheduled` | Accepted; takes effect at the session's next request. |
| `preview_stale` | State moved since `preview_version`; nothing ran. The app shows a fresh preview and the person presses again. No confirmation carries over. |
| `device_proof_required` | A phone-started spend, widen or destroys lacks device proof or approval. |
| `approval_declined` | The required approval was declined; nothing ran. |
| `approval_expired` | The approval wait expired; nothing ran. |
| `request_conflict` | The same request id was used with a different body; the conflicting run does not execute. |

**[pinned]** `request_id` is 1–64 ASCII characters `[A-Za-z0-9_-]`. The module
stores each answer for **at least 24 hours**. A repeat with the same id and
body returns the stored answer and never runs twice. Comparing a repeat uses
the full module-side body, including subject, args, version and caller facts,
not just the command id. Checking and insertion are atomic module duties.

**[pinned]** A `request_id` is scoped to its subject. The module keys stored
answers by subject and request id, so the same id on another subject is a
different run, never a repeat and never `request_conflict`. The conflict and
repeat rules above apply within one subject. Apps make a fresh id for every run.

**[pinned]** A run needing the module's own approval keeps the call open while
waiting. The module bounds the wait at **no more than five minutes** and then
answers. The app shows waiting for approval, not a timed-out spinner. From a
phone, `spend`, `widen` and `destroys` require device proof or independent
approval; forwarded caller claims alone do not suffice. `widen` and `destroys`
require the module's own approval (operator confirmation at the computer or an
approval card), not merely the relay's word.

## Refusals [pinned]

Refusals are errors, not outcomes: the daemon ERROR body
`{code, message, detail?}` (`subc_protocol::ErrorBody`). Message is short
operator text and never parsed. The body decodes leniently. Known codes:

- `invalid_params`, detail `{field}`: malformed shape or invalid field. A
  serving implementation maps structural decode errors to this code too.
- `unknown_command`: no such command in the selected module.
- `unknown_subject`: the module does not hold the named subject.
- `caller_not_admitted`: route caller or required first-party mark fails.
- `unavailable`: the module could not take the run now, for example because it
  could not record its answer. Nothing ran, so repeating the same request,
  with the same `request_id`, is safe.

The latter four need no detail. Field validators operate after routing has
selected a command; selecting unknown commands/subjects and applying admission
before validation remain serving duties.

## Confirmation by effect (informative, not pinned)

Apps confirm by the strictest effect in the set.

| Effect | Desktop | Phone |
| --- | --- | --- |
| `read` | no run | no run |
| `reversible` | one press | a tap |
| `ends` | 3-second hold | 3-second hold |
| `narrow` | 3-second hold | 3-second hold |
| `writes_files` | 3-second hold | 3-second hold |
| `destroys` | hold, then a second dialog repeating facts with its own press, then the module's own approval | hold plus the phone's biometric check, then the module's own approval |
| `widen` | hold, then the module's own approval | the phone's biometric check, then the module's own approval |
| `context_change` | 3-second hold | 3-second hold |
| `spend` | 3-second hold | hold plus the phone's biometric check |

## Not in v1

Jobs and long-running command handles/progress are not defined. Neither are
layouts, arbitrary text results, transcript insertion, daemon proof of an
original caller across a relay, or using `peer_build` as device proof. The
crate does not implement a relay, command server, app renderer, or approval
provider.
