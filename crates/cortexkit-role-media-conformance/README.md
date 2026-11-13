# media-provider/v1 conformance kit

Breaking: `call_as` now returns the raw RESPONSE body; adapters stop unwrapping.
`StreamObservation::answer` also returns the raw terminal RESPONSE body, not the
decoded answer. STREAM_DATA event bodies and referenced output-object contents
stay unwrapped.

Version 0.2.5 pins `cortexkit-role-media` exactly to 0.1.8. Implement `MediaSubject`
in an adapter and call `run_case(&fresh_subject, case_name)` for each entry of `CASES`.
Each case needs an isolated, empty provider store and a Tokio runtime with time
enabled. `Outcome::Failed` includes the case name; `NotApplicable` names an
unsupported optional capability. Default setup/observation hooks return explicit
errors naming the hook. They never silently skip coverage.

`call_as` sends a real request under the supplied route principal. It returns the
unmodified RESPONSE-body JSON value on success, including `{"result": <answer>}`,
or `SubjectError::refused` with the complete ERROR body on refusal;
transport/setup failures use `SubjectError::new`. The runner uses
`cortexkit_role_media::wire::read_reply` and reports `bare_answer` or `extra_keys`
when the provider omits the envelope or adds top-level keys. Never unwrap or
fabricate an envelope in the adapter. `configure_engines` installs exactly the
supplied facts in describe and the manifest declaration. `engine_inputs` records
each engine attempt, including the actual forwarded bias. A script's failure
ends one attempt, and its remaining steps are available to the next attempt.
`hold_engine` prevents new attempts starting; `Pause` blocks one already started.
`release_engine` unblocks both. Transcripts come from scripts, not recognition
accuracy. Engine output scripts may intentionally be malformed or oversized.

`open_stream` opens the real `media.stream_transcribe` request under its route
principal and returns only after acceptance or an opening ERROR. The request
stays open for STREAM_DATA events and its terminal RESPONSE. Audio and end
frames are ordinary `media.stream_audio` requests sent through `call_as`, with
`stream_id` in every request; their replies acknowledge held frames with `{}`.
The legacy `send_stream_frame` hook is deprecated and no longer used by cases.
`poll_stream`
drains events without blocking and reports whether the transport has closed and
its raw terminal RESPONSE body, if any, including the `result` envelope. A restart
closes an accepted stream without an answer. `close_stream` closes the caller's
open request. These are adapter hooks,
not additional wire methods. The stream cases use the same 30-second polling
ceiling as job cases.

`stream_audio_inputs` observes decoded chunks actually forwarded to the engine
in consumption order, not arrival order. Do not sort or synthesize this
observation in the adapter. The reorder case sends seq 7 through 1 before seq 0,
then verifies that the engine saw their bytes in seq order 0 through 7, regardless
of engine chunk boundaries. The window is independent for
each `(principal, stream_id)`, from next expected seq through next seq plus 7;
buffered frames never advance it across a gap. At most eight audio requests may
remain unanswered per stream. Audio may use a different route from the open
request: preserve principal lookup, not route-local stream state. Unknown,
closed and foreign streams all refuse with `stream_closed`. The crossing frame
gets its own `input_too_large` ERROR and never reaches the engine; the open
request also ends with that failure and the prior finals.

Both stream operations are `mutate` management operations, available only to
local callers on the computer. A phone dictates directly to the vendor with a
token. Dictation token models must also be declared with kind `realtime`:
tokens match only that kind, while duplicate engines key on kind, provider and
model, allowing the same model once per kind.

`open_watch` subscribes to the real `realtime.watch` transport for a recorded
call; `poll_watch` drains pushes without blocking. The case opens a watch after
attachment, scripts a vendor tool call using `RecordingVendor::push_tool_call`,
and then ends the call. Forward notifications drained by `take_tool_calls` through
the provider's normal vendor-event handling, not a fabricated watch answer.

`configure_voice_front(agent_id, unavailable_reason)` sets up head fork readiness:
`None` makes it ready, `fork_unsupported` means its runner lacks fork answers,
and `fork_refused` means preparing the fork was refused. Configure the real head
fixture, not a fabricated token refusal. The cases check token refusal order,
reason preservation, no admission/usage/call, same-id retry and dictation
independence. The default hook fails by name until an adapter implements it.
`open_stream` uses `media.stream_transcribe`; `open_watch` opens a long-lived push
watch, not a serial request.

`refuse_next_stream` injects one pre-acceptance `realtime_lane_full` ERROR and
rejects any other code with an error naming the hook. The first-frame case checks
only refusal semantics, never a provider's lane capacity; routing refusals come
from the routing peer instead. Stream `Partial`/`Final` scripts contain the
event members without `type`. `Output` can supply engine-level terminal metadata,
including a deliberately contradictory `{segments, no_speech: true}` answer.
The speech-check verdict is honored only when `has_speech_check` is true.
`engine_inputs` returns each forwarded kind's input object, including the actual
`bias` object; phrase bias forwards retained terms and omits `prompt` entirely.

Forward decoded `route.account_for` and `route.account_observe` requests to the
provided `RecordingRoutingPeer::call`, then wrap its decoded answer in the
routing wire envelope in a transport adapter. The peer answers synthetic account
ids, never credentials. Each renewal carries the account pin's id and generation
from the preceding admission or renewal answer.
`advance_clock` advances the shared peer clock and fires every due provider timer
before returning. Status polling uses a separate wall clock (30 seconds, every
50 ms), never advancing provider time to make work finish. The vendor peer maps
synthetic session ids to the token references opening them and scripted usage.

## Current coverage

This kit covers job, object, usage/feed, stream, bias/pack, realtime token/call/watch
and reply-envelope/wire-extension cases. Unary and terminal-stream envelope cases
reject bare answers and extra top-level keys by name. The rule map lists every
`[pinned]` rule id from the
[media contract](../cortexkit-role-media/CONTRACT.md). Its completeness test rejects
missing or extra ids, unknown cases, pinned rules lacking marker-position ids and
exemptions that differ between the contract and the rule map. The five exemptions
include recounting bias with the engine's tokenizer after combining terms and
prompt (`not_observable`), and local-only stream management classification
(`not_observable`: the adapter has no management-op classification and no
remote-computer caller). The other exemptions are the hand-off message,
hand-off send-id and pack privacy helpers (`caller_side`, checked by the contract
crate's helper tests). Shared admission cases cover
jobs, queued/running streams, unattached tokens and attached calls, including
renewals, principal-scoped keys, lapse and exactly-once release.

The meta-tests run every mapped case against a conforming stateful fake and a
faulty fake and require the faulty result to name that case. Empty transcripts,
empty media, corrupt chunk reads, decimal rounding, excess fractional digits
and coefficient overflow have independent focused faults. Synthetic WAV fixtures,
exact durations and regeneration instructions
live under `test-vectors/media-v1/`. No recorded speech ships.

### Narrowings

The wire-extension case drives unknown engine kind, latency class and bias unit
through configured engine facts and the provider's describe answer, and rejects
an unknown member in a status request. It does not inject unknown describe groups
or job statuses into a provider that does not produce them. Groups, job status,
refusal class, charge basis, authentication method, measurement unit/role and
watch type share the contract crate's `open_enum!` decoder; its exhaustive
unknown-value round-trip tests cover those other enum paths, not this provider
case.

The watch case checks principal admission, tool-call then ended push order,
call ids, timestamps, optional string tools only on tool_call pushes, and that
the vendor token the case first obtains from `realtime.token` never appears in a
push or an admission refusal. It does not observe the timing of `started`: watch admission requires
an already recorded call, and the contract does not require replay on subscription.
The adapter boundary does not capture provider logs or Debug output.
