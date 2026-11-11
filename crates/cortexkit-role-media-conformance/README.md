# media-provider/v1 conformance kit

Breaking: `call_as` now returns the raw RESPONSE body; adapters stop unwrapping.
`StreamObservation::answer` also returns the raw terminal RESPONSE body, not the
decoded answer. STREAM_DATA event bodies and referenced output-object contents
stay unwrapped.

Version 0.2.2 pins `cortexkit-role-media` exactly to 0.1.5. Implement `MediaSubject`
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

`open_stream` opens the real multi-frame `stream_transcribe` call under its route
principal and returns only after acceptance or first-frame ERROR. Subsequent
`send_stream_frame` calls feed that transport, not unary requests. `poll_stream`
drains events without blocking and reports whether the transport has closed and
its raw terminal RESPONSE body, if any, including the `result` envelope. A restart
closes an accepted stream without an answer. `close_stream` closes the caller's
transport. These are adapter hooks,
not additional wire methods. The stream cases use the same 30-second polling
ceiling as job cases.

`open_watch` subscribes to the real `realtime.watch` transport for a recorded
call; `poll_watch` drains pushes without blocking. The case opens a watch after
attachment, scripts a vendor tool call using `RecordingVendor::push_tool_call`,
and then ends the call. Forward notifications drained by `take_tool_calls` through
the provider's normal vendor-event handling, not a fabricated watch answer.

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
exemptions that differ between the contract and the rule map. The four exemptions
are token-unit bias recounting (`not_observable`), and the hand-off message,
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
call ids, timestamps, and absence of the synthetic token in pushes and admission
refusals. It does not observe the timing of `started`: watch admission requires
an already recorded call, and the contract does not require replay on subscription.
The adapter boundary does not capture provider logs or Debug output.
