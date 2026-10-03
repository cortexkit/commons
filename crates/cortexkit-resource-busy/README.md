# cortexkit-resource-busy

A small shared wire type for the `resource_busy` refusal. Providers use it
when a caller wants an exclusive per-agent resource that another caller of
the same agent currently holds. It is not tied to a role and can be returned
for plain operations as well as tool calls.

The refusal is **provably unsent**: a provider returns it only before doing
anything for the request. Nothing has been posted, launched, navigated or
dispatched, so a caller may record the call as provably unsent and retry.

A caller waits no longer than `retry_after_ms` before retrying, and never
past its own call deadline. If that deadline passes, the caller reports a
`resource_busy` failure, not a timeout.

The holder is always the same agent as the caller: its head session or one of
its flows. A provider never returns another agent's holder, so the refusal
does not leak information across agents.

The lease belongs to the provider, which must bound its duration. The lease
is released at session or run end, at scope end, or on an idle timeout; it is
never an unbounded lock.

`resource` is intentionally an open string so providers can name additional
resources without waiting for a crate release. Its current wire constraint is
1 to 64 characters from `[a-z0-9_]`.
