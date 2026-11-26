# cortexkit-role-agent-orchestrator

Wire contract for operations served by the CortexKit `agent-orchestrator/v1`
role: the module that runs agent sessions. Version 0.1.0 holds one operation
family, `awaited-work/v1`, through which a module that runs work for an agent
(a background command, a sidekick) registers that work on the agent's session
and reports its result, so the orchestrator can deliver it and hold a worker
open until it arrives.

`CONTRACT.md` is normative. The crate has serde types, bounds checks and
packaged test vectors (feature `test-vectors`); it has no transport, store or
runtime.
