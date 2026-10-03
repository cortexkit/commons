# cortexkit-role-classifier-conformance

The conformance suite for the `classifier/v1` role
(`crates/cortexkit-role-classifier/CONTRACT.md`). A classifier module adds it
as a dev-dependency and runs it in its own CI against its real module over
its real management route. It is never run against a double; the in-process
fake under `tests/fake/` exists only to test the suite itself.

## What a module supplies

- A `ClassifierRoute` adapter: send one `{method, params}` request over the
  real route, with `params` inserted as the exact JSON text the suite gives,
  and return the reply's exact text (or the `ERROR` body).
- A `ClassifierSubject`:
  - the route;
  - the models its catalog serves (`ServedModel`: priced or not, image
    limit, context, egress host), which `role.describe` must match;
  - a catalog id whose row is not a classifier, if it has one;
  - control of a **stand-in provider** every served model routes to. For
    each provider call the stand-in computes `state_key` of the request's
    `state` (its JCS canonical JSON), counts the call under it, and answers
    with what the suite scripted for that key (`Scripted::Answer` with an
    exact body, or `Scripted::Status`); an unscripted key answers 500. It
    can hold the next call for a key until the suite releases it.

## Verdict

A check the module gives the suite no way to ask is **not applicable**,
never passed: the cost and ceiling checks on a module that serves no priced
model, and the concurrent-call check on a stand-in that cannot hold a call.
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
| `null_state_refused_naming_field` | — |
| `unknown_question_type_refused_naming_field` | — |
| `answers_in_request_order` | — |
| `provider_numbers_byte_for_byte` | — |
| `one_failing_item_does_not_fail_batch` | — |
| `auth_failure_stops_the_call_unstored` | — |
| `auth_failure_on_first_call_refused` | — |
| `model_unavailable_stops_the_call_unstored` | — |
| `rate_limit_stops_the_rest_of_the_call` | — |
| `unreported_usage_stays_absent` | — |

`CHECKS` in `src/report.rs` states what each check asserts, and `NARROWINGS`
where it checks less than the contract says; every report prints both.

## The suite's own tests

`tests/suite.rs` runs the suite against the fake: a faithful fake passes
every check, and each deliberate break in `fake::Defects` turns its check
red by name. Each test pins exactly which checks a break turns red.
