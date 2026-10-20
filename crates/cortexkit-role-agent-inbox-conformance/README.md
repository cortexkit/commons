# cortexkit-role-agent-inbox-conformance

A live-implementer kit for the sibling role's [contract](../cortexkit-role-agent-inbox/CONTRACT.md).
Version 0.1.0. Implement `AgentInboxSubject`, then call `run(&mut subject)` and
print `report.render()`. Only `SuiteVerdict::Passed` is a full suite pass.

The subject opens real authenticated routes for two providers, two flows
sharing one carrier, and a second target for the first flow. Requests use
`scope_ref: "scope"`, epoch 1. The adapter may map this fixture identity to
native scope names, preserving the same mapping through restart. Flow routes
must carry stamped flow identities, not merely add JSON fields. Each case gets
fresh state, a frozen dispatcher, a controlled millisecond clock, wake grant
and an independent receiver. `call` must forward real operation bodies and
replies, not synthesize them. `deliver` observes actual received payloads,
attention and role, not inbox.list or submitted params. Hooks install daemon
served-history answers, a rendered echo template, quota, restart, and one
ambiguous landing resolved as unconfirmed. These test hooks must not be
available to ordinary production callers.

The trait is synchronous to avoid a runtime/async-trait dependency; adapters
can bridge their existing runtime at its boundary. Every hook is required;
errors are reported by case name, never skipped or counted as passes. The
report always prints NARROWINGS: this suite does not prove crash atomicity,
arbitrary runner/DND scheduling, or every flow policy refusal. See CASES for
the exact live claims. `tests/fake` injects one named defect per case to test
the kit itself; green fake tests are not production inbox conformance.

## Sender side

Use `errors::is_terminal(code)` / `is_retryable(code)` from the role crate.
Only inbox_full is retryable; unknown codes are terminal. Retry transport
failures with the unchanged key/body against the same implementer. An accepted
reply transfers durable custody; an unknown state does not. Missing capability
is discovery's no_implementer branch, not a refusal body. Safe fallbacks must
not inject past DND after scope authorization failures or ambiguous acceptance.
