# Changelog

## 0.2.11

- Select unsupported languages, formats, voices, sizes, resolutions and
  qualities using the engine's declared catalog rather than assuming specific
  values are unsupported. Skip only the language probe when no language limit
  is declared or every language candidate is supported; the other fact checks
  still run. Report the case as not applicable when no fact can be probed.
- Keep the media contract pin at `=0.1.11`.

## 0.2.10

- Support fixed catalogs through the existing unsupported `configure_engines`
  hook response. Synthetic-declaration cases become not applicable, with reasons;
  other cases use the provider's declared catalog.
- Do not call `usage.read` for local providers that declare no `usage_feed`, and
  skip direct usage-entry seeding when that group is absent. Declared feeds keep
  their existing record checks.
- Derive transcription input-limit probes and language from describe, using
  uploads for larger probes and reporting limits over the 16 MiB test budget as
  not applicable.
- Report output cases for undeclared engine kinds as not applicable. Check
  unsupported input facts only for declared kinds, and require undeclared kinds
  to refuse on `kind`.
- Keep the media contract pin at `=0.1.11`.

## 0.2.9

- Add the opt-in `Capabilities::with_local_provider(true)` job mode. Job engines
  are local, terminal and replay cases skip billing waits, and optional supplied
  usage records still undergo the media contract's record checks.
- Report remote routing/admission and record-dependent vendor usage/feed cases
  as not applicable in local mode, with reasons. Remote mode remains the default.
- Keep the media contract pin at `=0.1.11`, whose `media.usage.attempt` rule allows
  local work to omit usage records.

## 0.2.8

- Pin media `=0.1.11` and include the principal in attached-call usage record ids
  in the reference provider. Check distinct record ids for the same call under
  different principals.
- Exercise realtime watch tools on voice-front calls rather than dictation
  calls, preserving principal isolation, refusal redaction and end events.
- Reject providers that forward vendor tool calls through a dictation watch.

## 0.2.7

- Pin media 0.1.10 and check attached-call usage per billed model, including
  routed token usage and companion transcription seconds, stable redelivery,
  admission decision attribution and duplicate-model rejection.

## 0.2.6

- Pin media `=0.1.9` with per-frame invalid-data refusals and optional token
  `connect_url`, preserving absent-member compatibility and readers-first rollout.
- Check that oversize and invalidly encoded audio frames refuse on `data`,
  leave the stream and sequence open for a valid retry, never reach the engine
  and do not count toward cumulative limits. Reject faulty providers that
  consume the refused sequence or close the stream.
- Check audio arriving before its open, unknown-id grace expiry and immediate
  refusal for ended ids. Extend the reference provider with principal-scoped
  open waiting, ordered release, the eight-request window and four-id cap.

## 0.2.5

- Pin media 0.1.8 and change stream adapters to open a long-lived request and
  send ordinary `media.stream_audio` requests, observing their own replies.
- Check in-window reordering, out-of-window refusal, principal isolation and
  crossing-frame limit refusals, with independent faulty-provider tests.
- Refresh the streaming rule map, vectors and adapter guidance.
- Check that dictation tokens refuse transcription-only engine declarations,
  then accept the same model when it is also declared with kind `realtime`.

## 0.2.4

- Pin media `=0.1.7` for voice-front availability refusals, optional open watch
  tools and pinned `media.stream_transcribe` / `realtime.watch` operation names.
- Add `configure_voice_front` to set agent-head fork readiness. Its default
  returns a named hook error, so adapters cannot silently skip coverage.
- Check refusal reasons, ordering before routing and after identity/id/member
  checks, no admission or usage writes, no recorded call, same-id retries, and
  dictation independence, with deliberately faulty subjects for each case.
- Validate watch tool shape and presence only on tool_call pushes, with faulty
  subjects for non-string tools and tools on ended pushes.

## 0.2.3

- Pin media `=0.1.6` with the closed measurement source `request_derived`, billed
  only on done records without a vendor-reported billed quantity for that role
  and unit. Failed, cancelled and ended records carry it only as observed.
- Add a request-derived usage-preservation case. A faulty subject that
  rewrites the measurement's source fails that case, and the kit's own tests
  check that it does.
- Readers first: upgrade the usage reader to decode `request_derived` before
  any provider emits it; an older reader exposes undecodable records as `raw`
  in the usage feed.

## 0.2.2

- Pin media `=0.1.5` with the token measurement roles `input_image`,
  `output_image`, `output_video` and `cache_read`.
- Add one usage-preservation case per new role, with independent faulty
  subjects that rewrite the role. Unknown roles remain accepted by the open
  contract decoder.

## 0.2.1

- Pin media `=0.1.4`, whose engine identity is `(kind, provider, model)` and whose
  engine set may change at runtime with describe and the declaration kept equal.
- Compare describe/declaration sets by the full identity. Exercise the same
  kind and model under two providers, declaration order independence, missing
  providers and duplicated provider identities with focused faulty subjects.
- Routers must run `cortexkit-role-routing` 0.7.12 or later before providers
  declare the same `(kind, model)` under two providers; older parsers reject the
  pair and refuse the whole declaration.

## 0.2.0

Breaking: `call_as` now returns the raw RESPONSE body; adapters stop unwrapping.
`StreamObservation::answer` now returns the raw terminal RESPONSE body too.
Both include `{"result": <answer>}`; STREAM_DATA event bodies and referenced
output-object contents stay unwrapped. Refusals still carry the complete ERROR
body through `SubjectError::refused`.

- Pin media `=0.1.3` and check success bodies centrally with `wire::read_reply`.
- Add `reply_envelope` and `stream_reply_envelope` cases with independent bare
  and extra-key faulty providers, mapped to the pinned reply-envelope rule.
- Bump the kit's minor version because existing adapters returning decoded
  answers must change to preserve the provider's RESPONSE body.

## 0.1.2

- Pin media `=0.1.2`, which newly refuses quantities whose coefficient (all
  digits with the decimal point removed) exceeds 9223372036854775807, including
  `9223372036.854775808` and `9223372036854775807.999999999`, even with at most
  9 fractional digits. The refusal remains `invalid_usage_record`, field `quantity`.
- Correct 0.1.1's wrong stated reason: bounding only the whole part and fraction
  length did not ensure exact ledger storage as one signed 64-bit coefficient
  and a scale of at most 9. Use valid exact decimals in the kit fixtures.
- Keep the 10-fractional-digit faulty provider in `usage_quantity_bounds` and
  add `usage_quantity_coefficient_bounds` with an independent coefficient-overflow
  faulty provider. Ship as a patch because no producer emits media usage yet.

## 0.1.1

- Pin media `=0.1.1`, which newly refuses usage quantities with more than
  9 fractional digits or a whole part above 9223372036854775807. Use bounded
  exact decimals in fixtures and check that a provider emitting 10 fractional
  digits fails the named `usage_quantity_bounds` case.
- Ship the stricter validation as a patch because no producer emits media usage
  yet, so no deployed value is affected.

## 0.1.0

- Initial subject interface, generated audio fixtures, and job, object, usage and
  job-admission conformance cases for media-provider/v1.
- Add stream transport hooks and stream, bias/pack, realtime token, call lifecycle
  and shared stream/call admission cases, each checked against focused faulty fakes.
- Complete the rule map and exemption checks for every pinned media rule, with
  wire-extension and realtime-watch cases defended by focused faulty fakes.
