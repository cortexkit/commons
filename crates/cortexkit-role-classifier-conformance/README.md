# cortexkit-role-classifier-conformance

The conformance suite for the `classifier/v1` role
(`crates/cortexkit-role-classifier/CONTRACT.md`). A classifier module adds it
as a dev-dependency and runs it in its own CI against its real module over
its real management route. It is never run against a double; the in-process
fake under `tests/fake/` exists only to test the suite itself.

## What a module supplies

- A `ClassifierRoute` adapter: open its route with `role_versions` naming
  `classifier/v1` (`{"classifier": "v1"}`); the kit itself does not open
  routes. Send one `{method, params}` request over the real route, with
  `params` inserted as the exact JSON text the suite gives, and return the
  reply's exact text (or the `ERROR` body).
- A `ClassifierSubject`:
  - the route;
  - the models its catalog serves (`ServedModel`: priced or not, image
    limit, context, egress host), which `role.describe` must match;
  - a catalog id whose row is not a classifier, if it has one;
  - a catalog id whose row states no `kind`, if the catalog contains such a row;
  - control of a **stand-in provider** every served model routes to. For
    each provider call the stand-in computes `state_key` of the request's
    `state` (its JCS canonical JSON), counts the call under it, and answers
    with what the suite scripted for that key (`Scripted::Answer` with an
    exact body, or `Scripted::Status`); an unscripted key answers 500. It
    can hold the next call for a key until the suite releases it.
  - optionally, the **image-reference hook**
    (`ClassifierSubject::issue_image_reference` and
    `set_image_source_down`), for a module that declares `image_reference`:
    hand bytes to the image source the module resolves handles through and
    return the handle, and take that source down or bring it back. Without
    it, the checks that need a resolvable handle are not applicable.

## Verdict

For `routed_models`, optionally implement `script_routing`,
`routing_observations` and `finish_routing`. The hook scripts ordered
`RoutingOutcome`s, counts actual selections, and records the model, account
and decision id used for each actual provider request. The last scripted
outcome repeats. Observations must come from the stand-ins, not be inferred
from the classifier's reply. The suite restores routing after each case.
Without the group or script hook these cases report not applicable, with a
reason, rather than passing.

Two additional optional hooks exercise answered refusals with detail, without
changing the existing `RoutingOutcome` enum. Build a non-exhaustive
`RoutingRefusalFixture` with `new(code, message)` and `with_detail(value)`:

- `script_routing_admission_refusal(fixture)` resets observations and makes
  routing refuse admission with that body until `finish_routing`.
- `script_routing_renewal(fixture)` runs after `script_routing` has installed
  the initial selection. Force a lease renewal after the first answered item,
  before any later item dispatch, and refuse it with the fixture. Renewal is
  not a new selection. Keep recording actual provider requests and selections.

Both hooks default to unsupported (`false`); their cases then report named
`NotApplicable` outcomes, never failures or passes. `finish_routing` must
also clear these scripts. Timed fixtures use absolute Unix milliseconds;
the checks bound `retry_after_ms` by the actual call's start and end times
and require zero for an elapsed timestamp.

A check the module gives the suite no way to ask is **not applicable**,
never passed: the cost and ceiling checks on a module that serves no priced
model; the concurrent-call check on a stand-in that cannot hold a call; and
the kindless-row check when the catalog has no row without a kind; and the
image-reference checks that need a resolvable handle when the module does
not declare `image_reference` or the subject has no image-reference hook.
A run with no failure and some not-applicable checks is "passed for what the
module does", naming them; only a run where every check ran and passed is
"passed".

## Checks

| Check | Not applicable when |
|---|---|
| `describe_complete` | — |
| `concurrent_call_refused_batch_in_progress` | the stand-in cannot hold a call |
| `retry_calls_only_unanswered_items` | — |
| `stored_permanent_error_replayed_transient_retried` | — |
| `cost_usd_stable_on_replay` | no served model is priced |
| `looser_max_cost_does_not_raise_ceiling` | no served model is priced |
| `tighter_max_cost_stops_crossing_items` | no served model is priced |
| `tightened_ceiling_recorded_across_resends` | no served model is priced |
| `reuse_refused_naming_field` | — |
| `reordered_object_state_is_replay` | — |
| `validation_refusals_reach_no_provider` | — |
| `kindless_catalog_row_refused_not_classifier` | the catalog has no row that states no `kind` |
| `null_state_refused_naming_field` | — |
| `unknown_question_type_refused_naming_field` | — |
| `answers_in_request_order` | — |
| `provider_numbers_byte_for_byte` | — |
| `one_failing_item_does_not_fail_batch` | — |
| `auth_failure_stops_the_call_unstored` | provider declares `routed_models`; see its replacement below |
| `auth_failure_on_first_call_refused` | provider declares `routed_models`; see its replacement below |
| `model_unavailable_stops_the_call_unstored` | — |
| `rate_limit_stops_the_rest_of_the_call` | — |
| `unreported_usage_stays_absent` | — |
| `image_reference_follows_declaration` | no served model takes images |
| `image_reference_resolved_and_answered` | no served model takes images, the module does not declare `image_reference`, or the subject has no image-reference hook |
| `image_reference_oversized_refused_like_inline` | as above |
| `image_source_down_refused_transient` | as above |
| `image_reference_identity_is_its_handle` | as above |
| `routed_replay_keeps_decision` | no `routed_models` group or routing hook |
| `routed_partial_failure_keeps_model` | as above |
| `routed_reply_has_decision_id` | as above |
| `routed_pin_never_substituted` | as above |
| `routed_admission_refusals` | as above |
| `routed_transient_admission_refusals` | no `routed_models` group or detail-aware admission hook |
| `routed_permanent_admission_refusal` | as above |
| `routed_renewal_refusals` | no `routed_models` group, routing script hook, or renewal hook |
| `routed_initial_account_failure_bounded` | as above |

The two legacy auth checks are not applicable to `routed_models` providers:
`auth_failure_stops_the_call_unstored` is replaced by
`routed_partial_failure_keeps_model`, and `auth_failure_on_first_call_refused`
by `routed_initial_account_failure_bounded`. Non-account transient retries
remain applicable and must use the frozen model and account.

`CHECKS` in `src/report.rs` states what each check asserts, and `NARROWINGS`
where it checks less than the contract says; every report prints both.

## The suite's own tests

`tests/suite.rs` runs the suite against legacy and routed fake providers:
each passes every applicable check and names its not-applicable cases.
Each deliberate break in `fake::Defects` turns its check red by name. Each
test pins exactly which checks a break turns red.
