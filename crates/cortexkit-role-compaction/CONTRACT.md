# The runner and compaction-provider contract

Protocol: `compaction-provider/v2` and `compaction-runner/v1`.

This contract is what a runner and a compaction provider implement. Part A states the behaviour each side owes the other. Part B is the wire reference: every op, every field, every refusal code. Both parts are normative. Where they disagree, that is a defect in this document.

The request, answer and refusal examples in `test-vectors/compaction-v2` round-trip through this crate’s wire types in `wire_vectors_round_trip`; validation examples assert the expected decoding refusal, and checksum examples assert equality after normalization (W1.6). Image-refusal examples pair a request with the runner's required answer; the wire tests round-trip both, not perform edit validation. The unit tests cover part round-trips, compact forms, canonical checksums, and parameter error paths; they do not implement runner behaviour.

**Who owns what.** This crate publishes the wire types in Part B. A separate view library, the `cortexkit-compaction-view` crate, provides applying a view, the provider-rule check, the render plan (W6.3), the reply cleaning (W4.6), the model-to-rules defaults (W3.3), their WebAssembly build for runners written in JavaScript, and JSON test vectors. The library is published on crates.io with a versioned API and depends on this contract crate.

# Part A: behaviour

## A1. Roles, capabilities and routes

| Id | Rule |
|---|---|
| R1 | The runner holds the transcript, renders every request, and knows nothing about how history should be edited. The provider decides every edit. |
| R1a | An op's prefix names the side that serves it: `compaction.*` is served by the compaction provider; `session.*` and `run.*` by the runner. |
| R2 | Capabilities pair by side. The provider declares `compaction-provider/v2` and serves `compaction.setup`, `compaction.notice`, `compaction.ride`. The runner declares `compaction-runner/v1` and serves `session.entries`, `session.apply_view` and `session.applied_view`. A capability's version counts per name, so the new name starts at v1. |
| R3 | The provider also uses llm-runner's `session.send` and `run.result`, as published in `llm-runner/v1`. This contract does not change llm-runner. |
| R4 | All of these are calls over routes. A bus is not used. |
| R5 | A session's preset is a name only. The provider decides what each preset enables; the runner never sends feature flags. |
| R6 | A session's provider also serves its tools (`tool-provider/v1`: `tool.catalog`, `tool.call`) and the `pre_user` hook (`step-transform-provider/v2`, defined by the `cortexkit-role-step-transform-provider` crate from 0.5.0: `transform.declare`, `transform.hook`). This contract does not redefine them. |
| R7 | Every compaction call and every `transform.hook` is about the session its route is bound to. Each side opens one route per session and per role to the other (W2); step-transform discovery and declaration may also use a route without a session (W2.1). No request body names a runner, a session or a project. |
| R8 | Identity comes only from the daemon: the caller is the module the daemon attests for the route, the session is the route's bind identity, and the project is the daemon's scope stamp on the route. Refusals: W2.4; who may call the runner: W2.5. |


## A2. The neutral part model

| Id | Rule |
|---|---|
| P1 | An entry is every record of the host's transcript, on the active path, that the model sees or that holds part of what it sees, in host order. Marker records are never entries. One rule for every host; which storage records it means per host is W4.2. An entry may have no parts, and then renders nothing. |
| P2 | The part model covers every content type of the AI SDK's provider prompt (LanguageModelV4), so every model provider's history maps to it (W4.2 to W4.5, W4.10). |
| P3 | A tool part is one item at the call's position, holding both call and result even when the host stores them apart. Dropping it removes both. |
| P4 | Compact detail keeps what the provider plans with and leaves out the bulk (W4.8). Checksums use the full-detail projections defined in W7, minus the metadata W7 lists. |
| P5 | A stored entry changes only when a tool call in it gets its result (even when the host stores that result in a later message) or a send is delivered with one of its results (W4.9). It may also gain a previously absent `completed_at`, which is not a transcript change (W4.9). Count metadata updates alone (W4.11) are not entry changes; an update that also includes a notice trigger still sends the notice normally. Any other change is a transcript change. |
| P6 | Within a session, a message id is never given to a different message, even after a revert or branch switch; the same message keeps its id if it returns to the active path. A part's id is unique within its message, so within the session's state a provider identifies a part by `(mid, part id)` and keys durable part state on both. |

## A3. Tag numbers

| Id | Rule |
|---|---|
| T1 | Only the runner counts. A part's number is the checkpoint of the session's latest compaction marker on the active path, plus the part's position (counting from 1) among the numbered parts from the marker's first kept entry on. Hosts append markers at the tail and point at the first kept entry, so the count starts there, not where the marker sits. Which parts are numbered: W4.7. |
| T2 | The checkpoint a runner writes on a marker is the highest number before the marker's first kept entry. The first numbered part from there then gets checkpoint + 1, which is the number it already had, so a cut never changes a number. With no marker on the active path the checkpoint is 0; for a session moved in, it is the number handed over at the move (A13). |
| T3 | The runner stores the checkpoint durably together with its marker: in OpenCode 2's compaction row metadata, in OpenCode 1's marker compaction part (or the runner's own store keyed by the marker's message id, written before the marker is reported), in Pi's compaction entry under `details.cortexkit_checkpoint`, in a host’s own marker record. If a revert removes the latest marker, the previous one applies again, and recounting gives the same numbers as before. |
| T4 | Numbers are contiguous on the active path. After a revert or a branch switch, counting continues from what remains, so a removed number is given to the next new part. |
| T5 | The provider never computes a number. It keys its state by part identity, and refuses a context-management tool call (`ctx_*`) whose own message is no longer in the transcript, so a reused number never reaches another part. |
| T6 | The runner renders the prefix (W4.7) in front of every numbered part's text. Neutral text never carries it. Bytes from a `replace` edit are final and get no prefix. |
| T7 | A model reply's text is cleaned of tag notation (W4.6) when the runner maps it to neutral text and when it renders it. Whether the runner also cleans what it stores is its own choice. If the model's leading prefix named exactly the number the runner assigns and the reply holds no other tag notation, and cleaning changes nothing else, the reply renders byte for byte as the model wrote it. |
| T8 | The numbering rules of this contract apply to a session from a marker on. A session that ran under other rules switches only at a new marker whose checkpoint is the highest number its model has seen, which the move (A13) writes. The marker records only the checkpoint; ordinals need nothing from it (MV4). |

## A4. Setup

| Id | Rule |
|---|---|
| S1 | The runner calls `compaction.setup` (W5.1) when it starts serving a session whose preset has a compaction provider, when an applied plan names a different compaction provider (to the new provider, W2.5), after its own restart, before the next send after a Setup that failed with no view applied (W1.3a), and when a notice answers `setup_required` or a ride is refused `unknown_session`. |
| S2 | Setup either answers `ready` or fails with a refusal (W1.3) naming why. It never carries content, and the provider never reads the transcript inside it. |
| S3 | A second Setup for a session the provider already holds keeps the provider's state; both sides restart the notice count (W5.2 `seq`). |
| S4 | If the applied view differs from the one the provider holds (version or checksum), the provider adopts the runner's with `session.applied_view`. |
| S5 | The provider sets no ceiling or percentage on the runner. |

## A5. Notices

| Id | Rule |
|---|---|
| N1 | After every durable change to the transcript (W5.2 "When"), the runner calls `compaction.notice` with the new and changed entries in compact form. An entry only gaining `completed_at` does not require a notice; neither do count metadata updates alone (W4.9, W4.11). An update that also includes a notice trigger still sends the notice normally. |
| N2 | The provider answers at once and does the work afterwards. The runner never waits on that work. |
| N3 | The runner sends a session's notices one at a time: the next only after the previous one was answered. Changes made while a notice is unanswered go into the next notice, so a slow provider never builds a queue. Before a ride, the runner sends any change not yet noticed. |
| N4 | A notice is a convenience, not the source of truth. The provider keeps its own position, and reads any gap with `session.entries`. |
| N5 | A notice whose `seq` is not greater than the last one handled since the latest Setup is ignored. |

## A6. Reads

| Id | Rule |
|---|---|
| D1 | `session.entries` (W5.4) returns entries by ordinal, message id or tag number, in compact or full detail, paged by size. It reaches the whole active path, including entries a cut leaves out. |
| D2 | Full detail is for readers of exact content (the summarizer, `ctx_expand`); compact is for catching up. |
| D3 | The runner does not have to serve reads before Setup completes. |

## A7. Views

| Id | Rule |
|---|---|
| V1 | A view (W6) is a version, a checksum, and edits over the transcript addressed by message and part id. Everything not named is rendered unchanged. |
| V2 | Views are cumulative: each one carries every edit of the previous view, unchanged or changed in place (a changed edit takes the new version as its `since`, W6.2), plus new ones. Never both an old and a new replace of the same field. An edit leaves only when a revert removes the message it names. |
| V3 | Edits: drop, replace, insert, cut (W6.2). |
| V4 | The provider hands over a view with `session.apply_view` (W5.5), or inline in a ride's answer. The runner checks it (W6.5), answers in the same call, stores an accepted view durably, and applies it from the next step boundary, never in the middle of a step. How the runner stores views is its own choice; storing each one as the edits new since the previous one is fine. |
| V5 | A view is always sent whole, in one call. A provider refuses by name (`view_too_large`, logged on its side) to send a view over 3 MiB of JSON. There is no fetch-by-version op. |
| V6 | A refused view changes nothing: the runner keeps applying its previous view. The provider does not send that view again. |
| V7 | After a revert, the runner renders with the edits whose messages still exist (W6.6) until the provider sends a view for the new transcript. |
| V8 | Between two views, while the transcript only grows, the runner sends exactly what it sent before plus what is new, so every such request extends the previous one byte for byte; a revert, branch switch or host edit can break that before a newer view arrives (W6.6). Whether the model provider's cache still holds it depends on the cache's lifetime and on the rest of the request staying the same (A9). |
| V9 | Runners render from the render plan (W6.3) that the view library computes, applied to their own host messages, so kept parts go out byte-identical (signatures, encrypted reasoning, item ids, cache markers). The provider uses the same library to know what the model sees, so the two cannot drift. |

## A8. Provider rules

| Id | Rule |
|---|---|
| PR1 | The runner declares the session's rules (W3.3) on Setup and in every notice and ride. Only it knows the route and the account. Defaults by model family come from the view library, so runners keep no table of their own. |
| PR2 | Under `signed_parts: bound`, a kept signed part is valid only while everything rendered before it, apart from signed parts left out as allowed here, renders as it did when the part was generated. Among the session's signed parts, those left out may form only an oldest run, a newest run, or both; any other edit before a kept signed part invalidates it. The provider writes the companion drops this requires into the view; the runner refuses a view that breaks it. |
| PR3 | A tool call is never separated from its result. |
| PR4 | `history_edit_cost: full_resend` changes nothing in the runner. It tells the provider that any edit to history already sent costs a full resend. A runner that cannot tell whether requests are chained (for example where an auth plugin chains) declares `suffix`. |
| PR5 | A sealed part (signed or encrypted) can be dropped but never replaced; neither can a provider-executed tool part (the host replays it by reference). |
| PR6 | A view never removes the turn the request it renders ends on. That turn is the transcript's newest entry that has parts; entries with no parts (a step the runner has opened but not filled, or a message whose tool results ride inside the calls) render nothing and never count. When that entry holds a tool call, its last tool call stays, though its output may be replaced, because a result whose call is dropped does not render; otherwise at least one of its parts stays; and no cut reaches it. Without this the request can end on the model's own reply, which providers refuse. The provider builds views that keep it. The runner checks it wherever it checks a view (`session.apply_view`, a ride's answer, and each step boundary) and refuses a view that breaks it with the `rule` refusal `final_turn_removed` (W6.5), keeping the view already in force; at a step boundary it reports `view_refused`. |

## A9. Rides

| Id | Rule |
|---|---|
| RD1 | Before a model request that rewrites the model provider's cache anyway, or that would not fit, the runner calls `compaction.ride` (W5.3) with every cause that applies, and waits up to its deadline. |
| RD2 | The answer is a view or nothing. A late or missing answer means the request goes out with the view the runner has. |
| RD3 | Cache expiry is the runner's knowledge (W5.3), and `cache_expired` means proven expiry. The runner sends it only when all of these hold: it knows the cache lifetime it asked the model provider for on its last request for the session (for example a prompt-cache TTL); it has that request's time, recorded on the same clock it reads now and kept durably if it is to survive a restart; and that clock shows the lifetime has passed. Keep-warm requests count as requests. A runner missing any of these, including after its own restart without such a record, or behind a host that keeps the cache warm without telling it the lifetime, sends no `cache_expired`: the provider's cache may still be warm, and a provider treats `cache_expired` as a rebuild that costs nothing extra. |
| RD4 | The runner's only limit is its own usable window (W3.4). When the next request would exceed it, the runner rides with `over_limit`. If the request still does not fit, it refuses the turn with `context_ceiling`. It never sends over its limit. |
| RD5 | Setup lists the causes the runner can detect (`detects`). A cause the runner cannot detect is unknown to the provider, never "did not happen". A runner lists `cache_expired` only for routes where it can meet RD3; a route that cannot leaves it out. If `cache_expired` is absent from `detects`, the provider treats expiry as unknown. If it is listed but absent from a ride's causes, expiry was not proven for that ride; that does not certify the cache is warm. |
| RD6 | A one-step fallback is not a model change: a resend to another model after a refusal, or a model provider's own server-side fallback. The session's model stays as it was in the conditions, and no ride happens. |

## A10. A person's message (`pre_user`)

| Id | Rule |
|---|---|
| U1 | Before storing a message a person wrote, the runner runs the `pre_user` hook of `step-transform-provider/v2` and stores the message with the provider's prepend and append operations. |
| U2 | The provider declares the hook for presets that use it with a budget of 1,500 ms and `on_unavailable: pass`: on a timeout or an error the message is stored unchanged. |
| U3 | For a time marker, the provider takes the previous message's `completed_at`, or its `written_at` when it has none (W4.1), and compares it with the new message's time: the `pre_user` hook's optional `at_ms`, taken on the runner's machine (`step-transform-provider/v2`), or the provider's own clock when `at_ms` is absent. Times from the same machine carry no clock skew. Gaps against moved or imported history, or between different clocks, are estimates. |

## A11. Messages and runs the provider asks for

| Id | Rule |
|---|---|
| SR1 | The provider queues text for the session's model with llm-runner's `session.send` (W5.7), as a template send with a delivery order: `["tool_result"]` for routine text, `["steer"]` only for text the model must act on. |
| SR2 | `tool_result` text is rendered with the next tool result, the first time that result is sent, and never starts a model step. The runner presents it in the tool part's `delivered` list (W4.4). |
| SR3 | The provider runs its summarizer as a run in a child session it owns (`session.send` with run settings, then `run.result`). Nothing in it reaches the parent's transcript. |

## A12. Host marker and forks

| Id | Rule |
|---|---|
| M1 | Once a view with a cut has applied, the runner writes its host's own compaction marker at the cut (OpenCode's compaction message, Pi's compaction entry, a host’s marker record), with the checkpoint of T2, and reports it in its next notice. This applies to any compaction provider. |
| M2 | The marker names the session it was written for. A marker naming another session means the session is a fork, and the runner refuses to enroll it. |

## A13. Moving a standalone session in

| Id | Rule |
|---|---|
| MV1 | The provider's move (offline) refuses by name a session already moved or not fully summarized, builds the first view (history head and a cut through the newest entry), records it as its own, prints it with the standalone plugin's highest tag number, and writes the plugin's fence last. |
| MV2 | The runner’s move installs that view as its applied view, with the same version and checksum, and writes a marker whose checkpoint is that number. Setup then finds the view applied. A runner that cannot adopt a standalone session refuses the move. |
| MV3 | A session already using an earlier version of this protocol moves the same way, from its standalone host store. Its prior-protocol provider state is discarded first: the old provider store, the summaries and notes keyed by its former internal session keys, and the old move record and fence. No live ordinal conversion of that discarded state is needed; standalone data is converted by MV4. Turns that exist only in the runner’s store from the earlier move are not carried. |
| MV4 | The move converts every ordinal the provider stored for the session from the standalone plugin's counting to this contract's (W4.1, W4.2), so a session using this protocol has one counting rule for its whole life. It builds a map per message id: the message's standalone ordinal (from that host's standalone reader) and its entry ordinals under W4.2. Through that map it rewrites, in one transaction: summary ranges (their headings are what the model passes to the context-expansion tool `ctx_expand` to show summarized messages and parts again), the events and fact anchors tied to summaries, user-memory source ranges, and the raw-message search index. A range maps by coverage: its start goes to the first entry of its first message, its end to the last entry of its last message, so a Pi tool-result run folded into one standalone ordinal stays fully covered. If any stored ordinal cannot be mapped, the move refuses by name (`ordinal_unmapped`) and changes nothing. |
| MV5 | Claude Code sessions are not moved into this protocol. |
| MV6 | The same conversion moves a session between any two hosts or runners (for example OpenCode to another host). Two things come on top of MV4: the importer that copies the transcript hands the move a map from each source message to the target entries it became, since message ids change; and the session’s provider data (summaries, notes, search index) is re-keyed from the source `(harness, session)` to the target’s in the same transaction. Numbers continue from a checkpoint equal to the highest number the model has seen (T8). The target runner must be able to adopt the session. |

## A14. Failures and restarts

| Id | Rule |
|---|---|
| F1 | Provider unreachable: the runner keeps applying its last view. An `over_limit` ride with no answer ends in `context_ceiling`. |
| F2 | Provider restart: notices answer `setup_required`, rides are refused `unknown_session`, the runner calls Setup again, and the provider rebuilds from its store and one read from its last cut. |
| F3 | Runner restart: it reloads its stored applied view and transcript and keeps applying them, and calls Setup again. It rides with `cache_expired` only when its durable record of its last request proves that the requested cache lifetime has passed (RD3). |
| F4 | One session's slow or silent runner never delays another session. |

# Part B: wire reference

## W1. Transport

**W1.1 Requests.** Every op is one request on a route. The body is the JSON object `{"method": "<op name>", "params": <request object>}`. A successful answer's body is the op's answer object.

**W1.2 Values.** The contract's own integer fields (ordinals, counts, sizes, versions, `seq`, and the like) are JSON numbers in the range 0..2^53−1 (0..9007199254740991), so JavaScript `JSON.parse` can round-trip them exactly. Decoding refuses a larger number as `invalid_params` naming the field. This ceiling does not apply inside opaque JSON payloads, such as tool `input` and replace `value`, or to the payload kept for an unknown tagged value (W1.5); those keep their numbers unchanged (W4.4, W7). Unknown fields on known objects are ignored (W1.4), not kept. Times are milliseconds since the Unix epoch, UTC. Sizes are bytes. Token counts the model provider reports (`usage`, `last_prompt_tokens`) are in the model provider's own tokens; runner-sent W4.11 counts use the response's `token_family` counter, which may be a stand-in (`token_family_exact`). Strings are UTF-8. An optional field that is absent is left out of the object, never sent as `null`.

**W1.3 Refusals.** An op that cannot be answered fails with the transport's error frame `{code, message, detail?}`. `code` is a stable snake_case name; `message` is text for people that no program parses; machine-readable fields go in `detail`. A refusal changes nothing on the side that refused.

A frame the daemon itself produced (`frame_too_large`, a closed route, and the like) carries the daemon-origin flag; a receiver never mistakes it for the peer's refusal.

Codes and what the caller does next:

| Code | `detail` | Meaning | Caller's next step |
|---|---|---|---|
| `invalid_params` | `field` | `params` does not decode into the op's request. | A bug in the caller: log it. On `compaction.setup` the runner applies W1.3a; on other methods it does not retry. |
| `unknown_method` | `method` | The route's role does not serve this method. | A bug in the caller: log it, do not retry. On `compaction.setup` (for example from a provider that predates this contract), the runner handles it as a refused Setup (W1.3a). |
| `unknown_preset` | `preset` | Setup names a preset the provider does not know. | The runner logs it and handles it as a refused Setup (W1.3a). |
| `unknown_session` | | A ride for a session the provider holds no Setup for. | The runner treats it as `nothing`, then calls Setup before its next notice. |
| `compaction_not_permitted` | `reason` | The runner refuses a `compaction-runner/v1` route or call from a caller that is not the session's selected provider (W2.5). | The provider suspends work for that session until a new Setup answers `ready` (W2.5). |

**W1.3a A refused or unanswered Setup.** When `compaction.setup` is refused, with any code (`invalid_params`, `unknown_method` and `unknown_preset` included), or gets no answer within the runner's deadline, the runner gets no new views from that Setup:

- **A view already applied:** the runner keeps applying it (F1) and calls Setup again at its next S1 trigger.
- **No view applied:** the current run ends `error` with `provider_code: compaction_unavailable` before any model call, and the runner calls Setup again before the next send. After that Setup failure the runner makes no model call for the current turn.

`setup_required` is an answer of `compaction.notice`, not a refusal (W5.2). A view the runner refuses is an answer of `session.apply_view`, not a refusal (W5.5).

**W1.4 Unknown fields.** A receiver ignores fields it does not know. A sender sends a field whose meaning the receiver must understand only when the receiver declared the capability that defines it. The same holds for a value added to one of the open sets in W1.5: a sender uses it only when the receiver declared the capability that defines it.

**W1.5 Unknown values.** The sets below may grow in later versions. A receiver decodes a value it does not know and keeps it, so one unknown value never fails the whole request, then acts on it on the safe side. It logs each unknown value it meets. A fallback never waives another check: PR2 and the running-tool rule still apply.

| Value | Received by | Treated as |
|---|---|---|
| Setup answer | runner | A Setup refusal, handled as W1.3a: a view already applied stays applied, and with no view the turn ends `compaction_unavailable` before any model call. |
| Notice answer | runner | `setup_required`: the runner calls Setup before its next notice. |
| Ride answer | runner | `nothing`. |
| `session.apply_view` answer | provider | Outcome unconfirmed: the provider doesn't send that view again, and reads the runner's applied view (`session.applied_view`) before relying on either outcome. |
| View refusal code or rule break | provider | A refusal it cannot act on: it does not send that view again (V6). |
| Ride cause | provider | A cause after which the request rewrites the model provider's cache anyway. |
| Notice event | provider | A change whose reach it doesn't know: it reads the whole active path again before relying on its state. |
| `rules.signed_parts` | provider | `bound`. |
| `rules.history_edit_cost` | provider | `full_resend`. |
| Part kind | provider | An opaque part (W4.5): it may be dropped, never replaced, and its remaining payload is carried unchanged. `tokens` remains reserved Part envelope metadata (W4.11). |
| Entry role | provider | Every part of that entry is handled as an opaque part. |
| `seal` | provider and runner | A sealed part: it may be dropped, never replaced (PR5). |
| Tool `status` | provider and runner | `running`: never dropped, replaced or cut. |
| Output format | provider and runner | `output` is never replaced; the whole part may still be dropped. |
| `delivered.forms` item | provider and runner | Skip the unknown value; if no known form remains, use `tool_result`. |
| Attachment item or file transport | provider and runner | Never interpreted or rewritten. An `attachments` edit keeps it by naming its id, or removes it by leaving the id out, as for a drop. |
| Edit `op`, insert `at` or inserted message `role` | runner | The view is refused `unsupported_edit` (W6.5). |
| `compaction_not_permitted` `detail.reason` | provider | The same as every defined reason: the provider suspends work for that session until a new Setup (W2.5). |
| `session.entries` `select` | runner | Refused `invalid_params` naming `select`. |

**W1.6 Empty arrays.** An array marked "may be left out when empty" may be omitted and reads as empty. Today these fields are tool `attachments` and `delivered`, notice `events` and `entries`, and inserted message `images`. Checksums are computed over the form that leaves these empty arrays out, even when the received object explicitly includes them, so a known object is normalized before its JSON Canonicalization Scheme (JCS) serialization (this crate's typed serializers do it). Hashing raw JSON as received does not normalize it. Empty arrays inside opaque payloads (tool `input`, replace `value`) are never removed.

## W2. Routes and identity

**W2.1 Routes the runner opens to the provider,** one per session and per role, each bound to the session's identity (`harness`, `session`) under the session's scope. The one exception is step-transform discovery and declaration, in its row below:

| Role (`role_versions`) | Ops on it |
|---|---|
| `compaction-provider` `v2` | `compaction.setup`, `compaction.notice`, `compaction.ride` |
| `tool-provider` `v1` | `tool.catalog`, `tool.call` (published) |
| `step-transform-provider` `v2` | `role.describe` and `transform.declare` are answered on any route of this role, including module-level routes and scoped routes opened before a session exists, so a plan composer can declare before hook routes exist. `transform.hook` requires a route bound to a session; otherwise that call is refused `route_unbound` (W2.4). |

**W2.2 Routes the provider opens to the runner,** one per session and per role, bound the same way. Only the session's own compaction provider may open a `compaction-runner/v1` route (W2.5):

| Role | Ops on it |
|---|---|
| `compaction-runner` `v1` | `session.entries`, `session.apply_view`, `session.applied_view` |
| `llm-runner` `v1` | `session.send`, `run.result` (published) |

**W2.3 Identity.** A session is keyed by three values, none from a request body: `runner`, the runner's module id (for example `example-runner`), which is the daemon-attested caller on routes the runner opens (W2.1) and the receiving module on routes the provider opens (W2.2); `harness`, the host the session lives in (for example `opencode`, `pi`); and `session`, the host’s own session id. `harness` and `session` stay the same when a session moves from a standalone host to a runner. The project is the daemon’s scope stamp on the route: `workdir` and, when present, `project_id`.

**W2.4 Route refusals** are normally given when a route is opened (bind), not per op. Two exceptions:

- `compaction_not_permitted` may also be given on a single call. After the caller loses its authority, W2.5 requires the runner either to close its routes or to refuse every later call on them with this code.
- `step-transform-provider/v2` answers `role.describe` and `transform.declare` on any route of that role, including module-level routes and scoped routes opened before a session exists, and refuses a `transform.hook` on an unbound route with `route_unbound`.

Compaction routes require a bound session at bind.

| Code | Meaning |
|---|---|
| `unattested_runner` | The daemon attests no module for the caller. |
| `unknown_route_role` | The route declares none of the roles its receiver serves, or more than one. |
| `route_unbound` | The route carries no bind identity. Refused at bind except for `step-transform-provider/v2`, where it refuses a `transform.hook` call, not `role.describe` or `transform.declare`. |
| `compaction_not_permitted` | The runner cannot authorize the caller for the bound session under W2.5. `detail.reason` says why; its defined values are in W2.5. |

**W2.5 Who may call the runner.** A runner admits a `compaction-runner/v1` route only when the module the daemon attests for the caller is the session's selected provider: `compaction_item.provider` of the plan currently applied to the session. That is the effective plan under llm-runner's next-run rule (its §10.2), not an accepted refresh that is still pending. This covers every op on that role, reads included: `session.entries` returns the transcript and `session.apply_view` changes what the model sees. Otherwise the runner refuses `compaction_not_permitted` at bind. `detail.reason` is an open string; the values defined today are:

| `reason` | When |
|---|---|
| `provider_mismatch` | The applied plan's compaction item names a different provider. |
| `no_compaction_item` | The applied plan has no compaction item. |
| `session_not_held` | The route carries a bind identity, but the runner holds no session under this contract for it. A route with no bind identity is `route_unbound` (W2.4). |

A runner may also check each call and refuse it with the same code. When a change takes effect that leaves a caller without this authority, the runner closes that caller's existing `compaction-runner/v1` routes for the session, or refuses every later call on them with this code. Such changes are: an applied plan that names a different provider, an applied plan with no compaction item, and the runner ceasing to hold the session. A runner may instead refuse a `session.refresh` that would change the compaction item's provider, with `invalid_params {field: "plan.compaction_item"}`; that conforms too.

On this code, whatever the reason, and when the runner closes such a route, the provider suspends its work for that (`runner`, `harness`, `session`). It logs the reason, keeps its state (S3) and opens no further routes for that session. It resumes only after a later `compaction.setup` from that same attested runner for the same session answers `ready`. Setup does not bypass this rule, and the provider still reads the transcript only after Setup (S2).

On routes the runner opens, the provider keys session state by (`runner`, `harness`, `session`) with `runner` the attested caller (W2.3). Another caller therefore cannot reach an existing runner's state by reusing its bind values, and the provider needs no matching plan check for that isolation.

## W3. Shared values

**W3.1 `ModelRef`**

| Field | Type | Meaning |
|---|---|---|
| `provider` | string | The model provider, for example `anthropic`, `openai`, `google`. |
| `model` | string | The model id as the provider names it. |
| `variant` | string, optional | The reasoning variant or effort, when the runner sets one. |
| `service_tier` | string, optional | The service tier (for example OpenAI's `priority`) when it is not the provider's default. |

**W3.2 `AppliedView`:** names a view. `version` (integer) and `checksum` (string, W7). A version alone is not enough: a provider restored from an older backup could send different content under a version the runner already applied.

**W3.3 `ProviderRules`**

| Field | Values | Meaning |
|---|---|---|
| `signed_parts` | `none`, `bound` | `bound`: PR2 holds for parts with `seal: signed` (W4.3). Anthropic thinking on the current families (Opus 5.5, Sonnet 5.5, Fable 5.1, Haiku 5.5). `none`: PR2's binding doesn't apply; PR5 still does. |
| `history_edit_cost` | `suffix`, `full_resend` | `suffix`: an edit costs a re-read from the first changed byte. `full_resend`: any edit to history already sent forces the whole prompt to be resent (chained OpenAI requests). |

The view library (`cortexkit-compaction-view`) gives the default values for known model families, from the route's provider and model; a runner overrides them only with what it knows better (an account it knows is not bound, a chained route).

**W3.4 Windows.** `context_window` is the tokens the model takes for prompt and reply together: the runner's configured limit, or the model's maximum from the model catalog. `output_reserve` is the tokens the runner keeps free for the reply. The usable window is `context_window − output_reserve`; the runner never sends a prompt larger than that.

**W3.5 `Conditions`:** what the session runs under now. Notices and rides both carry it, so a change reaches the provider with the next one.

| Field | Type | Meaning |
|---|---|---|
| `model` | `ModelRef` | The session's model. A one-step fallback does not change it (RD6). |
| `accepts_images` | boolean, optional | Whether the session's model reads images. The runner sends it only when known; absent means unknown. A provider inserts images only when it is `true`. |
| `context_window` | integer | W3.4. |
| `output_reserve` | integer | W3.4. |
| `rules` | `ProviderRules` | For the current model and account. |
| `last_prompt_tokens` | integer, optional | The prompt size the model provider reported for the session's newest model request: uncached input plus cache reads plus cache writes. Absent before the first request, or when the provider reported none. Requests of child sessions do not count. |
| `system_tokens` | `{count, exact}`, optional | Count of the runner's own system text for the next request under these conditions, excluding system entries; in notices only, never rides (W4.11). |
| `tools_tokens` | `{count, exact}`, optional | Count of the tool definitions for the next request under these conditions; in notices only, never rides (W4.11). |

**W3.6 `Cursor`:** where an entry sits. `ordinal` (W4.1), `mid` (the message id) and `digest` (the entry checksum, W7). A provider holding an entry with the same three values is caught up to it.

**W3.7 `StepUsage`:** the token counts the model provider reported for one model request, attached to the assistant entry it produced. Every count is optional: absent means the provider did not report it, never zero. A runner whose host stores 0 for "not reported" leaves the count out unless it knows the 0 was reported; a `prompt_tokens` of 0 is always left out.

| Field | Type | Meaning |
|---|---|---|
| `prompt_tokens` | integer, optional | The whole prompt: uncached input plus cache reads plus cache writes. |
| `cache_read_tokens` | integer, optional | Prompt tokens read from the cache. |
| `cache_write_tokens` | integer, optional | Prompt tokens written to the cache. |
| `output_tokens` | integer, optional | Reply tokens, reasoning included. |
| `reasoning_tokens` | integer, optional | Reasoning tokens within `output_tokens`. The reasoning budget depends on it: encrypted reasoning has no other measure. |

## W4. The neutral part model

**W4.1 Entry:** one record of the host's transcript, as stored. Views never change entries; they change what the runner renders from them.

An entry is every record on the active path that the model sees or that holds part of what it sees, in host order (W4.2 per host). Marker records are never entries. A record whose content belongs to another entry (a Pi `toolResult`, a Claude Code user message holding only tool results, a host’s tool-role message holding a batch's results) is an entry with no parts; its results sit in the call's tool part (P3). A step that died before any content (no parts, not finished, no error) is an entry with no parts. An entry with no parts renders nothing.

| Field | Type | Meaning |
|---|---|---|
| `ordinal` | integer | The entry's position on the active path, counting from 1 at the session's first entry. Every entry counts, including the ones a cut leaves out. After a revert or a branch switch, a new entry takes the ordinal of the removed one. For a session that ran under the standalone plugin, the move converted every ordinal the provider stored to this counting (MV4). |
| `mid` | string | The host's message id. Stable for the life of the message and never reused within the session, even after a revert or branch switch. A part's id is unique within its message, so `(mid, part id)` is never reused either; providers key durable part state on both ids (P6). Nothing may derive it from the ordinal or the reverse (a host’s ids might be `m0`, `m1`, … while its ordinals start at 1). |
| `role` | `user`, `assistant`, `system` | `system` is a turn-scoped system entry (llm-runner's `turn_system` delivery); it holds text parts only. |
| `notice` | `NoticeOrigin`, optional | Set when a module or the runner delivered a user-role entry (a reminder, a wake, a notice) rather than a person typing it. `sender`: the module id. `template`: the plan template it was rendered from, when it was. |
| `written_at` | integer | When the host first stored the message. It does not change when the entry is updated. For a message the runner did not store itself (history from before the runner, a moved session), the host's own write time. |
| `completed_at` | integer, optional | When the host marked the message finished, from the host's own stamp, in epoch milliseconds under the same integer rules as `written_at` (W1.2). Absent while the entry is unfinished, when the host never marks that kind of message finished (for example a user message), or when imported history has no such stamp. Readers that need a time then use `written_at`. |
| `digest` | string | Checksum of the full-detail projection of `role`, `notice` and `parts` defined in W7, minus the metadata W7 lists. |
| `parts` | array of `Part` | In host order. May be empty. |
| `usage` | `StepUsage`, optional | Assistant entries: what the request that produced it reported. |
| `reasoning_tokens` | `{count, measured}`, optional | Assistant entries: the step's reasoning count (W4.11), separate from reported `usage`. |
| `generated_under` | `AppliedView`, optional | Assistant entries: the view applied to the request that produced it; absent when none was. PR2 checks against it. |

Allowed kinds by role: `user` (and notices): `text`, `file`. `assistant`: `text`, `reasoning`, `tool`, `file`, `opaque`. `system`: `text`.

When an entry is finished and its host records a completion stamp, the runner returns that stamp as `completed_at`. Host mappings:

- OpenCode: the message's `time.completed`.
- Pi: the journal entry's timestamp.
- A runner that keeps its own log: the assistant step's end time, and for a tool-result entry, the latest answer time among that step's calls.

The field is absent while the entry is unfinished, including while a tool part in it is `running`. A later read may add it (W4.9).

**W4.2 Which records are entries, per host.** One rule (W4.1); the table says which storage records it means on each host. It differs from the standalone plugin's counting on several hosts, and the move converts stored ordinals (MV4).

| Host | Entries, in host order | Marker records (never entries) | Not entries |
|---|---|---|---|
| OpenCode 1 | every `message` row of the session, by `(time_created, id)`. A dead step (no parts, not completed, no error) is an entry with no parts. Of a compaction pair whose summary never completed, the opener is an entry with one text part (OpenCode renders its `compaction` part as user text `What did we do so far?`) and the summary is an ordinary assistant entry. An assistant row with an `error` is an entry with no parts, unless the error is an abort and it holds a part other than step-start or reasoning. Parts OpenCode never renders are not parts: user text with `ignored: true` or empty text, and `text/plain` or `application/x-directory` file parts. A `subtask` part is a text part `The following tool was executed by the user`. | a completed compaction pair: the user message holding the `compaction` part and its summary assistant message with `summary: true`, a `finish` (any value) and no `error`, the only pair OpenCode treats as a boundary (`filterCompacted` in OpenCode 1.18.35's `message-v2.ts`) | none |
| OpenCode 2 | `session_message` rows, by `seq`, that OpenCode's model-message conversion (`to-llm-message.ts`, 2.0.26 and 2.2.3) renders or may render: `user`, `synthetic`, `assistant`, `skill`, `system`; `location-switched` and a foreground `shell` (each one text part); `model-switched` (an entry with no parts, since whether it renders depends on the request's model). A `user` row with no text and no files, and a dead step, are entries with no parts. | `compaction` rows | `idle`, `agent-switched`, and a `shell` with `metadata.background: true`: the conversion never renders them |
| Pi and Oh My Pi | active-branch journal entries of type `message` (every role, each `toolResult` its own entry), `custom_message`, `branch_summary` | `compaction` | `model_change`, `context_edit` and other control entries |
| A host with a native message log | every message in its log, in log order | its marker records | none |
| Claude Code (new sessions only, MV5) | the messages of the request's `messages` array, as Claude Code merges them from its journal | none | summary requests |

**W4.3 Part:** fields every part has, plus the fields of its kind.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Stable for the life of its message, unique within it. Views address parts by it. |
| `kind` | `text`, `reasoning`, `tool`, `file`, `opaque` | W4.4 to W4.6. |
| `tag` | integer, optional | The part's number (A3), when it is numbered (W4.7). |
| `seal` | `none`, `signed`, `encrypted` | Below. Absent means `none`. |
| `tokens` | `{count}` or `{input, output}`, optional | Reserved Part envelope member for every kind; W4.11 defines which known kinds carry counts. W1.5 preserves the remaining payload of an unknown kind. |
| `digest` | string | Checksum of the part in full form without `id`, `tag`, `digest` and `tokens` (W7): its kind, content fields and `seal`. |

- `text`: `text` (string), the text without any tag prefix, cleaned per W4.6 for assistant replies; `tokens` (`{count}`, optional, W4.11). A text block the host stores empty is not a part: it carries nothing and takes no number, and the runner keeps it with its row (OpenCode 1 keeps one as a separator between signed thinking blocks and sends it as `" "`). Text that becomes empty only through W4.6 cleaning stays a part.
- `reasoning`: `text` (string, optional), the visible reasoning text or summary when the model provider returned any; `text_bytes` (integer, optional), its size.
- `seal` says what the model provider checks when the part is sent back:
  - `signed`: a provider signature binds it to what came before it. Anthropic thinking and redacted thinking; Gemini thought signatures on text, tool calls and files (open point 5).
  - `encrypted`: opaque provider content, not bound to earlier history. OpenAI Responses reasoning with `encrypted_content`.
  - `none`: nothing to check.
- A sealed part can be dropped, never replaced (PR5).

**W4.4 `tool`:** a tool call and its result as one item, at the call's position.

| Field | Type | Meaning |
|---|---|---|
| `call_id` | string | The call id. Never changes. |
| `name` | string | The tool name. Never changes. |
| `input` | any JSON | The call's arguments, as the model wrote them. |
| `provider_executed` | boolean | The model provider ran the tool itself (hosted web search, code execution, image generation); its result is part of the assistant message. Absent means `false`. |
| `status` | `running`, `completed`, `error`, `denied` | `running` also covers a call waiting for approval. `denied`: the call was not executed. |
| `output` | string, optional | The result's text: the output when completed, the error text on `error`, the reason on `denied`. Absent while running, and in compact form. |
| `output_format` | `text`, `json` | `json`: `output` is the JCS text (W7) of a JSON result. Absent means `text`. |
| `output_bytes` | integer, optional | The size of `output` in bytes, in both forms. Absent while running. |
| `attachments` | array of `Attachment` | Files and opaque items the result carries besides its text; may be left out when empty. |
| `approval` | object, optional | A provider-executed call's approval answer: `approved` (boolean) and `reason` (string, optional). |
| `delivered` | array of `Delivered` | Rides delivered with `tool_result`, in delivery order; may be left out when empty. Each: `sender` (module id), `template` (string, optional), `text` (string; absent in compact form), `text_bytes` (integer), `send_id` (string, optional), `forms` (array of `tool_result` and `turn_system`, optional), `tokens` (`{count}`, optional; W4.11). |
| `tokens` | `{input, output}`, optional | Present only once the result exists; running tools omit it. Separate counts let the provider estimate the saving from replacing the tool's output, keeping the call/result structure (W4.11). |

An `Attachment` is `id` (stable within its tool part) plus the fields of a `file` or an `opaque` item (W4.5), told apart by `kind`.

**W4.5 `file` and `opaque`**

- `file`: any media, images included.

  | Field | Type | Meaning |
  |---|---|---|
  | `mime` | string | The media type, for example `image/png`, `video/mp4`, `application/pdf`. |
  | `name` | string, optional | The file name. |
  | `transport` | `inline`, `url`, `reference`, `text` | How the host sends it: bytes, a URL, a provider file id, or text. |
  | `size` | integer, optional | Size in bytes, when the runner knows it. |
  | `width`, `height` | integer, optional | Pixels, for images and video, when known. |
  | `duration_ms` | integer, optional | For audio and video, when known. |
  | `reasoning` | boolean | The model produced it as part of its reasoning (AI SDK `reasoning-file`). Absent means `false`. |

  A file part may also carry `tokens` (`{count}`, optional), counted by the reading model's image rule. Other media (audio, video, PDF) omit it unless the runner has a counting rule (W4.11).

  A transcript `file` part carries metadata only: `session.entries` never returns its bytes. Inserted images are a separate case and carry their bytes as base64 (W6.2).
- `opaque`: provider-specific content the runner sends back as it is (AI SDK `custom`, Anthropic's server-side fallback). `provider_kind` (string, `provider.type`, for example `anthropic.fallback`) and `size` (integer, optional). It can only be dropped.

**W4.6 Cleaning tag notation from replies.** A model reply's text is cleaned exactly, in this order: the leading prefixes are split off (their first number is the model's own tag, T7); every remaining well-formed `§N§` is removed; then the malformed hybrids `§N">`, `§N">§` and `§N">§M§`; then a dangling `§N` with at most one improvised non-word closer (never followed by a digit or by `.` and a digit, so `§5.1` stays); then any stray `§`; then surrounding whitespace is trimmed. If cleaning leaves the text empty, it becomes empty only on a route that accepts an empty text part; elsewhere the original text stays (standalone: Anthropic accepts empty text, other providers keep the original). Only the model's own replies are cleaned; user, notice and tool text is never changed. Standalone cleans because models, after heavy drops, start writing numbers as delimiters, including broken forms. The view library ships this function with standalone's test cases.

**W4.7 Which parts are numbered.** Every part the model sees as text, other than what a view inserts:
- text parts of user, notice and assistant entries;
- tool parts, for their result, whatever its format: text, JSON, error or denial text, or only attachments.

Never numbered: reasoning, files, opaque parts, the text of `system` entries, and provider-executed tool parts, which the host sends back by reference or as signed blocks, so no prefix can be attached.

The prefix is the number between two section signs followed by one space: `§12§ `. The runner renders it in front of a text part's text and a tool result's `output`. A tool result with no text renders the prefix followed by `[image]` when every attachment is an image, or `[file]` otherwise.

**W4.8 Compact form.** A part in compact form leaves out a tool part's `output`, each `delivered` text (their sizes stay), and a reasoning part's `text`. Everything else, including the full tool `input` and every text part, stays. `digest` is always the checksum of W7's full-detail projection minus its listed metadata, so an edit computed from compact detail names the exact part.

**W4.9 What may change.** A stored entry changes only when a tool part in it goes from `running` to a final status with its result (also when the host stores that result in a later message), or when a send is delivered with one of its results. The runner then sends the changed entry again in its next notice. Any other change to a stored entry is a transcript change (W5.2 `transcript_changed`).

For rides delivered after adopting 0.2.6, the runner sets `send_id` and
`forms`: the send's `delivery` list restricted to `tool_result` and
`turn_system`, in list order. Both are omitted when absent, preserving older
bytes and checksums. The runner never adds them to a delivered item it has
already served: that would be a transcript change. Absent `forms` means
`[tool_result]`. Each ride takes the first form its model route supports at
that position, under llm-runner §9.7's recorded per-session/model route
decision and existing template-authority and provider-position rules.
Unsupported system form falls back to append; old rides always append.
Rendering uses llm-runner's shared `rides` module, also re-exported by this
crate. A ride's optional `tokens` follows W4.11's per-part count and family
rules, including later arrival without a transcript change, and enters no
checksum (W7).

**Discarded rides: narrow content exception.** Only for tool parts in entries
behind the session's applied cut, once that cut has passed the carrying part
for at least 24 hours, and only when the applied view has no edit naming that
part, the runner may discard ride records (W6.2). On later reads
it omits the discarded `Delivered` items and recomputes the part's and its
entry's digests. `history_version` does not move and no notice is sent. The
part's `rides` display fact goes away too. A reader MUST NOT treat a digest
change confined to such parts, in entries behind the applied cut, as
`transcript_changed`. This is the only exception for removing served content
under W4.9; every other content change remains a transcript change except
the result/delivery updates already allowed above. Count and timing metadata
keep their existing rules below.

A stored entry may also gain a previously absent `completed_at`. That alone is not a transcript change: `completed_at` enters none of the W7 checksums, and adding it alone needs no notice. When the same update also changes a tool result or delivers a send, the rules above still apply and the notice carries the entry with its `completed_at`. A `completed_at` once served does not change.

Count metadata updates alone (W4.11) are not entry changes, never divergence, and require no notice; they enter none of the W7 checksums. A changed count under the same family is an update, not a transcript change. If the same update includes a notice trigger, the runner sends the notice normally.

**W4.10 Mapping from the AI SDK provider prompt (LanguageModelV4),** for runners and as the completeness check of this model:

| AI SDK | Neutral model |
|---|---|
| `system` message (string) | runner's system prompt, not an entry; a turn-scoped system message is a `system` entry |
| `user` `text` / `file` | `text` / `file` |
| assistant `text` | `text` |
| assistant `reasoning` (Anthropic signature or redacted data) | `reasoning`, `seal: signed` |
| assistant `reasoning` (OpenAI encrypted content) | `reasoning`, `seal: encrypted` |
| assistant `file` / `reasoning-file` | `file` / `file` with `reasoning: true` |
| assistant `custom` | `opaque` |
| `tool-call` + its `tool-result` (tool or assistant message) | one `tool` part |
| `tool-call` with `providerExecuted` | `tool` with `provider_executed: true` |
| result `text` / `json` / `error-text` / `error-json` / `execution-denied` | `output` with `output_format` and `status` |
| result `content` (text, file, custom) | text joined into `output`; files and custom items as `attachments` |
| `tool-approval-response` | the tool part's `approval` |
| Gemini thought signatures (`providerOptions.google.thoughtSignature`) | `seal: signed` on that part |

Provider options a host keeps on a part (cache control, citations) are the runner's: it renders them with the part as it is, and decides which stay valid for a replaced field.

**W4.11 Optional token counts.** These additive fields require no new capability. A runner may send part `tokens`, each `delivered` item's `tokens`, and assistant-entry `reasoning_tokens` in `session.entries` pages and notices, including compact parts. Reasoning parts have no per-part count. An absent count means the provider keeps its own estimate, never zero.

**Coverage.** Part counts cover full content as the runner renders it to the model provider, without message framing, using that family's counter. They describe full content, not a view's rendering.

| Part / field | Counted content |
|---|---|
| Text `tokens.count` | Text including its tag prefix when the runner renders one. |
| Tool `tokens.input` | Tool name plus arguments as rendered. |
| Tool `tokens.output` | Everything in the result as rendered: text (including any tag prefix), delivered text and images by the reading model's image rule. |
| Delivered `tokens.count` | The ride's `text`, using the same family and stability rules as per-part counts below. |
| File `tokens.count` | The reading model's image rule; other media (audio, video, PDF) omit `tokens` unless the runner has a rule. |

**Families.** Every page or notice carrying any of these counts MUST carry `token_family`. A session has one family at a time. The reader adopts the family named by each response in receipt order and MUST discard held counts of any other family. All counts added by this section in that response use its family. The family is a runner-assigned id for counting rules, not a model-name format (for example `claude-v5/1`); compare the whole string by equality only. It changes whenever the same input would count differently; the runner then recounts everything, for example on a model switch.

Alongside `token_family`, a page or notice may carry `token_family_exact` (boolean). `true` means the family uses the model's own tokenizer; `false` means a stand-in, for example o200k for an unknown model. Absent means unknown and readers treat it as `false`.

**Part stability.** The runner counts each part once for unchanged content within one family. Once sent, that count MUST NOT change while the content and family are unchanged. A part not yet counted omits `tokens`. A tool part gets `tokens` only once its result exists; a running tool omits it. A tool result arriving is a W4.9 change, and its new count accompanies the changed entry.

**Reasoning estimate.** Assistant-entry `reasoning_tokens` is `{count, measured}`. With `measured: false`, the count is the model provider's separately reported reasoning tokens when available. Only when no reasoning figure is reported is it the step's output tokens minus every counted non-reasoning output, files included; omit it when that accounting is unavailable. The estimate may change on a later page.

**Measured reasoning.** With `measured: true`, the runner MUST check from its own serialization that request N's rendered prompt is a byte prefix of request N+1's: system, tools and messages in the provider's wire format as the provider receives them, excluding sampling settings. Checking only the message list handed to a host is insufficient and always means `measured: false`. Request N is the request that produced the assistant entry being measured. The value is input(N+1) − input(N) − A − F. Here input(N) is request N's total prompt tokens as defined for `last_prompt_tokens`: uncached input plus cache reads plus cache writes. A is the count of everything actually appended to the checked prompt between the two requests, as rendered there (not the full-content counts where an edit makes them differ), except the step's reasoning being inferred. F is the family's message-frame cost per appended message, not already in A. Both request prompt totals, the step's reported output tokens (reasoning included, i.e. what the step generated), and every operand of A and F must be available; otherwise no measured value is sent. Keep the value only if 0 ≤ value ≤ that output total.

**Reasoning stability.** Once sent with `measured: true`, the runner MUST freeze the value within its entry and family: no later page in that family may change it; providers may rely on that. On a family change, the runner sends a new value only if it can measure again, otherwise it omits it.

**Surface counts.** Notice conditions may carry `system_tokens` and `tools_tokens`, each `{count, exact}`, for the request the runner sends next under these conditions. They never go in rides. `system_tokens` covers the runner's own system text, not system entries (those are parts); `tools_tokens` covers tool definitions. `exact: true` means the counted bytes are the bytes the model provider receives; `exact: false` means the runner counted what it handed the host, which rendered the final format. Providers may use inexact counts for reporting but SHOULD NOT use them for drop decisions. All `count`, `input` and `output` values follow W1.2; `measured` and `exact` are booleans.

**Notice budget.** The W5.2 budget is on the serialized compact `entries` array. Drop entry counts (`tokens`, `reasoning_tokens`) first, then the entries if still over budget. Condition counts stay, and `token_family` stays whenever any count remains. Providers may read omitted entries and their counts with `session.entries`; condition counts are not returned there. Counts, `token_family` and `token_family_exact` sit outside every W7 checksum.

## W5. Ops

### W5.1 `compaction.setup` (provider)

Request:

| Field | Type | Meaning |
|---|---|---|
| `preset` | string | The session's preset, for example `head`, `worker`, `reader`. |
| `model` | `ModelRef` | W3.1. |
| `accepts_images` | boolean, optional | Whether the session's model reads images. The runner sends it only when known; absent means unknown. A provider inserts images only when it is `true`. |
| `context_window` | integer | W3.4. |
| `output_reserve` | integer | W3.4. |
| `rules` | `ProviderRules` | W3.3. |
| `detects` | array of cause names | The ride causes (W5.3) the runner can detect (RD5). |
| `applied_view` | `AppliedView`, optional | The view the runner applies now: after its restart, the one it reloaded; for a session moved in, the installed first view. |
| `newest` | `Cursor`, optional | The newest entry; absent for an empty transcript. |
| `now` | integer | The runner's clock. |

Answer: `{"answer": "ready"}`. Refusals: `unknown_preset`, `invalid_params`.

### W5.2 `compaction.notice` (provider)

When the runner sends one: after a message is stored (a user, notice or system entry; an assistant entry at the end of its model step, never while it streams); after an entry changes (W4.9; an entry only gaining `completed_at`, or count metadata updates alone, does not count); after a revert or branch switch; after it refused a view at a step boundary; after it wrote a host marker. If an update also includes a notice trigger, send the notice normally. Changes made while a notice is unanswered go into the next one (N3).

Request:

| Field | Type | Meaning |
|---|---|---|
| `seq` | integer | 1 for the session's first notice after each Setup, then one more for each notice. The runner keeps the counter in memory and restarts it whenever it calls Setup. |
| `newest` | `Cursor`, optional | The newest entry after the change; absent only when the transcript is empty, including after a revert to an empty transcript. |
| `applied_view` | `AppliedView`, optional | The view the runner applies now: not an accepted view still waiting for the next step boundary. Absent when it applies none. |
| `conditions` | `Conditions` | W3.5. |
| `token_family` | string, optional | Required when the notice carries W4.11 counts; applies to all of them. |
| `token_family_exact` | boolean, optional | Alongside `token_family`: the model's own tokenizer (`true`) or a stand-in (`false`); absent is unknown and treated as `false` (W4.11). |
| `events` | array of `Event` | Below. Empty when nothing but entries changed; may be left out when empty. |
| `entries` | array of `Entry` | In compact form, oldest first: every entry stored since the previous notice, plus every changed entry again. The serialized compact array's budget is 1,048,576 bytes. Drop entry counts first, then omit the entries if still over budget; condition counts stay, and `token_family` stays whenever any count remains (W4.11). The provider may read omitted entries with `session.entries`, which does not return condition counts. May be left out when empty. |

Events, each an object with an `event` field:

| `event` | Fields | Meaning |
|---|---|---|
| `transcript_changed` | `from_ordinal` | Entries from this ordinal on may differ from what earlier notices and reads said: a revert, a branch switch, or a host edit of stored messages. Sent in the first notice after the change. A runner whose host never reverts, switches branch, or changes stored messages beyond what W4.9 allows never sends it. |
| `view_refused` | `version`, `refusal` (W6.5) | A view the runner refused when it came to apply it: a ride's answer, or an accepted view that no longer fits the transcript at the next step boundary. |
| `marker_written` | `version` | The runner wrote its host's compaction marker for this view's cut. |

Answer: `{"answer": "received"}`, or `{"answer": "setup_required"}` when the provider holds no session for the route (it restarted); the runner then calls Setup before its next notice. Refusals: `invalid_params`.

### W5.3 `compaction.ride` (provider)

Request:

| Field | Type | Meaning |
|---|---|---|
| `causes` | array of cause names | Every cause that applies, at least one, none twice. |
| `deadline_ms` | integer | How long the runner waits for the answer. Its choice; a ride that may make the provider read the transcript from scratch, for example after either side restarts, can get a longer one. This never implies `cache_expired`, which RD3 alone governs. |
| `conditions` | `Conditions` | W3.5. |
| `applied_view` | `AppliedView`, optional | As in a notice. |
| `request_tokens` | integer, optional | With `over_limit`: the runner's estimate of the prompt it is about to send. |

Causes:

| Cause | When the runner rides with it |
|---|---|
| `cache_expired` | Its own clock shows that the time since its last request for the session (keep-warm requests count) is longer than the cache lifetime it requested. Never sent when it cannot tell (RD3). |
| `chain_reset` | It is about to start a new request chain on a chained route (connection age or idle limit, a run starting without a kept connection). |
| `model_change` | The next request uses a different model than the previous one, other than a one-step fallback (RD6). |
| `tier_change` | A different service tier. |
| `account_change` | A different account. |
| `surface_change` | A different system prompt or tool definitions. |
| `over_limit` | Its estimate of the next request exceeds the usable window (W3.4). |

Answer: `{"answer": "view", "view": View}` or `{"answer": "nothing"}`. The runner checks a returned view like one handed over with `session.apply_view` (W6.5). If accepted, it applies the view to this very request and stores it as its latest; if refused, it keeps its view and reports `view_refused` in its next notice. With `over_limit`, it then estimates again, and refuses the turn with `context_ceiling` if the request still does not fit.

Refusals: `unknown_session`, `invalid_params`.

### W5.4 `session.entries` (runner, `compaction-runner/v1`)

Request:

| Field | Type | Meaning |
|---|---|---|
| `select` | object, with `by` | `{"by": "after", "after_ordinal"?: integer, "limit"?: integer}`: entries with an ordinal greater than `after_ordinal` (from the first when absent), oldest first, at most `limit`, which must be at least 1 when present. `{"by": "mids", "mids": [string]}`: those messages in transcript order; ids not on the active path are left out. `{"by": "tags", "tags": [integer]}`: the messages holding parts with these numbers, in transcript order. |
| `detail` | `compact`, `full` | W4.8. |
| `max_bytes` | integer, optional | Stop adding entries once the page's entries reach this many bytes of JSON. A page holds at least one entry when any matches. |

Answer:

| Field | Type | Meaning |
|---|---|---|
| `entries` | array of `Entry` | |
| `token_family` | string, optional | Required when the page carries W4.11 counts; applies to all of them. |
| `token_family_exact` | boolean, optional | Alongside `token_family`: the model's own tokenizer (`true`) or a stand-in (`false`); absent is unknown and treated as `false` (W4.11). |
| `next_after` | integer, optional | For `by: after`: the ordinal to continue after when more entries match than fit. Absent when the page reaches the end. |
| `head` | object | `newest` (`Cursor`, optional) and `applied_view` (`AppliedView`, optional) at the moment of the read. |

Refusals: `invalid_params`. A read reaches the whole active path, including entries a cut leaves out.

### W5.5 `session.apply_view` (runner, `compaction-runner/v1`)

Request: `{"view": View}`.

Answer: `{"answer": "accepted"}`: stored durably; it applies from the next step boundary. Or `{"answer": "refused", "refusal": ViewRefusal}` (W6.5): not stored; the runner keeps its view.

Refusals: `invalid_params`.

### W5.6 `session.applied_view` (runner, `compaction-runner/v1`)

Request: `{}`. Answer: `{"view"?: View}`, the whole view the runner applies now; absent when it applies none.

### W5.7 `session.send` and `run.result` (runner, `llm-runner/v1`)

Used as llm-runner publishes them; this contract adds nothing. What a provider sends:

- Reminders: a template send (`template`, no `prompt`), `send_id` derived from what it delivers, and `delivery` as an ordered list: `["tool_result"]`, or `["steer"]` for text the model must act on.
- Summarizer runs: a send that starts a run in its own child session, with its system prompt, model and generation settings; then `run.result` until the run finishes.

## W6. Views

**W6.1 `View`**

| Field | Type | Meaning |
|---|---|---|
| `version` | integer | At least 1. Greater than every view the provider built for the session before. |
| `checksum` | string | Checksum of `{"version", "edits"}` (W7). |
| `edits` | array of `Edit` | Applied in order. |

**W6.2 `Edit`:** `since` (integer: the version of the first view that held this edit unchanged; a new or changed edit gets the view's own version) plus `op` and the op's fields:

| `op` | Fields | Effect |
|---|---|---|
| `drop` | `mid`, `part` (optional) | Leave out one part, or the whole entry when `part` is absent. Dropping a tool part removes call and result. |
| `replace` | `mid`, `part`, `field`, `value`, `expect` | Render one field of the part with new bytes. `expect` is the part's `digest` when the edit was computed. Fields: `text` (a text part, or a file part, which the runner then renders as a text block in the file's place; `value` a string); `output` (tool part; a string; the runner maps it to the host's result field, such as the error field of a failed call); `input.<key>` (tool part whose `input` is an object with a string at `key`; a string); `attachments` (tool part; an array of the attachment ids to keep); `delivered` (tool part; an array of the indexes of `delivered` items to keep). |
| `insert` | `at`, `message` | Render a message the transcript does not hold, such as the history head. `at`: `"start"` (before the first rendered message) or `{"before": mid}` (immediately before that entry, even if it is dropped). `message`: `id` (unique in the view, kept the same while the message is unchanged), `role`, `texts` (array of strings), `images` (array of images; may be left out when empty). Each image has `media_type` (string, currently only `image/png`) and `data` (string, standard padded base64 of the bytes). Only a `user`-role inserted message may carry images, and a provider inserts them only when `accepts_images` is `true`. Inserted messages are never numbered. |
| `cut` | `through` (mid) | Leave out every entry up to and including `through`: the entries the history head replaces. At most one per view. |

Rides follow their carrying output. Replacing a tool part's `output` without
an edit naming its `delivered` list suppresses all that part's rides; dropping
the output suppresses them too. If the view names `delivered`, its choice
stands. A ride otherwise renders only while its carrying output renders.

The runner may discard a part's ride records only once the session's applied
compaction cut has passed the carrying part for at least 24 hours and the
applied view has no edit naming that part. Any surviving replace, including
an output replace, retains its `expect` digest and therefore prevents
discarding that part's rides. A session without
compaction never discards ride records. A discarded ride is no longer
rendered or served; providers can rely on records being retained until both
conditions hold.

**W6.3 The render plan.** The view library turns a view and the transcript into a render plan, and every runner renders from it (V9): for each entry, keep or leave out; for each kept part, keep, leave out, or replace a field with given bytes; the inserted messages and where each goes; and each kept part's number, with whether its prefix is rendered. Rules: inserts at the start come first, in edit order; each entry follows in transcript order, preceded by the inserts anchored before it; a dropped or cut entry is left out; dropped parts are left out; an entry left with no parts is not rendered; numbered text gets its prefix (W4.7), except text a `replace` wrote, which is final. The runner applies the plan to its host messages, so everything the plan keeps goes out byte-identical. The plan is available from the Rust library, from its WebAssembly build, and as JSON test vectors.

**W6.4 Numbers and views.** No edit changes a number. The runner counts from the transcript (A3), not from what it renders: inserted messages are not entries and are never counted, a dropped part keeps its number unused (a gap, unless a `replace` writes a placeholder showing it instead), a replaced part keeps its number, and a cut only hides the oldest numbers. What the model sees may therefore have gaps and start above 1. If a view leaves out the newest number (for example by dropping the newest result to fit the usable window) and the model then writes a lower number, the runner replaces it with the assigned number (T7).

What could mislead the model is a number in bytes the provider writes, such as a summary quoting an old prefix. So the runner checks every view's bytes for tag prefixes (a section sign, one or more ASCII digits, a section sign, anywhere in the text):

- An inserted message's texts may contain none (`tagged_insert`). The provider strips them before it renders the history head.
- A replace value may contain the part's own number (a placeholder needs it) and any number the field's original text already contains, and no other (`foreign_tag`). This needs the original text, so a runner reads each replaced message in full when it checks a view.

**W6.5 `ViewRefusal`:** why a runner refuses a view. An object with `code` and the listed fields.

| `code` | Fields | When |
|---|---|---|
| `bad_checksum` | | The checksum does not match the content. |
| `stale_version` | `version`, `latest` | The version is not greater than the latest view the runner accepted. |
| `bad_since` | | An edit's `since` is 0, greater than the view's version, or claims a version the runner applied while the edit is not in that view unchanged. |
| `unknown_message` | `mid` | An edit names an entry not on the active path. |
| `unknown_part` | `mid`, `part` | |
| `stale_part` | `mid`, `part` | The part's `digest` differs from the edit's `expect`. |
| `bad_field` | `mid`, `part`, `field` | The field does not exist for that kind, the value has the wrong shape, an attachment or delivered index does not exist or repeats, or the part is sealed or provider-executed (PR5). |
| `conflict` | `mid`, `part` | Two edits cannot both hold: a drop and a replace of one part, or two replaces of one field. |
| `running_tool` | `mid`, `part` | A tool still running cannot be dropped, replaced or cut. |
| `unknown_anchor` | `mid` | An insert anchors before an entry not on the active path. |
| `duplicate_insert` | `id` | Two inserts share an id. |
| `duplicate_cut` | | More than one cut. |
| `tagged_insert` | `id` | An inserted message's text contains a tag prefix (W6.4). |
| `foreign_tag` | `mid`, `part`, `field`, `tag` | A replace value contains a tag prefix the part neither owns nor already contained (W6.4). |
| `rule` | `rule`, and its fields | The view breaks a provider rule: `{"rule": "signed_part_invalid", "mid", "part"}` (PR2), or `{"rule": "final_turn_removed", "index"}` (PR6), where `index` is the zero-based position in `edits` of the edit after which nothing that must stay of the final turn is left. |
| `unsupported_edit` | `index` | The edit at this zero-based position of `edits` has an `op`, an insert `at` or an inserted message `role` the runner does not know (W1.5), or carries images the runner cannot apply (below). |

An insert with images is refused as `unsupported_edit` when its role is not `user`, any image's `media_type` is not `image/png`, or any image's `data` is not valid standard padded base64. Each case carries the same detail: `index` names the insert's zero-based position in `edits`; no image-specific detail field is added. The runner checks the standard base64 alphabet, length, padding placement and zero padding bits; inspecting PNG contents is not required by this contract. A conforming runner performs these checks before applying the view. This wire crate preserves the strings without decoding or validating them; image checks and image entries in the render plan are the view library's (W6.3), which adds them in a release after this one.

**W6.6 After a revert.** Until a newer view arrives, the runner renders with the edits whose messages still exist (the view library's `view::edits_for_transcript`): drops, replaces and inserts naming a removed entry are skipped, a cut through a removed entry is skipped, and inserts at the start stay.

**W6.7 Rendering and size.** The runner renders each inserted image as an image part after the message's texts, in array order and in its host's own form. Like inserted texts, images get no tag number. A provider sends no view over 3 MiB (3,145,728 bytes) of JSON (V5); image fields, including their base64 data, count toward that limit.

## W7. Checksums

Every checksum is the lowercase hex SHA-256 of the value's RFC 8785 (JCS) serialization, computed with llm-runner's shared `content_digest` function, never a separate implementation. A runner written in JavaScript computes it through the view library's WebAssembly build, which runs that same function. Absent optional fields and the empty arrays named in W1.6 are left out, so they never enter a checksum. Floats occur (a tool's `input` is any JSON), and JCS fixes their form.

| Checksum | Over |
|---|---|
| Part `digest` | The part's object in full form without `id`, `tag`, `digest` and `tokens`. |
| Entry `digest` | `{"role", "notice", "parts"}`, with each part in full form including its `id`, `tag` and `digest`, but without its `tokens`. |
| View `checksum` | `{"version", "edits"}`, including inserted images and their base64 data. Empty `images` arrays hash as absent (W1.6). |

Entry timing metadata (`written_at` and `completed_at`) enters none of these checksums.
Count metadata (`tokens`, including each `delivered` item's count, assistant-entry `reasoning_tokens`, condition `system_tokens` and `tools_tokens`), `token_family` and `token_family_exact` also enter none of these checksums. `tokens` is reserved in the Part envelope even for unknown kinds; the remaining unknown-kind payload is preserved under W1.5. Opaque payload fields with the same names are content, not count metadata, and remain in checksums.

# Open points

3. MV4 needs, per host, the standalone reader’s counting as the map’s old side: OpenCode 1 counts raw session messages; OpenCode 2 counts raw message types by `seq`; Pi folds tool-result runs into one ordinal, while extension messages, branch summaries and compactions take none. The move reuses those readers’ rules exactly.
5. Gemini’s rule for thought signatures (which parts keep them, within which turn, and what an edit before them does) is unmeasured. Until a live check, a Gemini runner declares `signed_parts: none`, and the provider never replaces a signed part (PR5).
7. Whether models have stopped writing tag numbers as delimiters is unmeasured; W4.6 keeps the cleaning either way.
8. OpenCode 2’s row types: a runner must confirm against `to-llm-message.ts` on 2.0.26 and 2.2.3 which of `agent-switched`, `model-switched`, `idle` and background-shell rows the model sees; each one it renders is an entry.
9. MV6 needs the importer to return a source-to-target message map when the target runner adopts this contract.
