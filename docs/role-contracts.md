# Role contracts and conformance

A role crate defines the wire types and validation shared by an implementation and its callers. Its `CONTRACT.md` is the normative text; this page is a map, not a replacement specification.

## Contracts, vectors, and runners

| Role or capability | Normative contract | JSON vectors | Conformance runner |
|---|---|---|---|
| `tool-provider/v1` | [Contract](../crates/cortexkit-role-tool-provider/CONTRACT.md) | [Vectors](../crates/cortexkit-role-tool-provider/test-vectors/tool-provider-v1/) | [Tool-provider suite](../crates/cortexkit-role-tool-provider-conformance/README.md) |
| `llm-runner/v1` | [Contract](../crates/cortexkit-role-llm-runner/CONTRACT.md) | [Vectors](../test-vectors/llm-runner-v1/) | [LLM-runner suite](../crates/cortexkit-role-llm-runner-conformance/README.md) |
| `classifier/v1` | [Contract](../crates/cortexkit-role-classifier/CONTRACT.md) | [Crate-local vectors](../crates/cortexkit-role-classifier/tests/vectors/) | [Classifier suite](../crates/cortexkit-role-classifier-conformance/README.md) |
| `compaction-provider/v1` | [Contract](../crates/cortexkit-role-compaction-provider/CONTRACT.md) | [Vectors](../test-vectors/compaction-provider-v1/) | No separate runner crate yet |
| `step-transform-provider/v1` | [Contract](../crates/cortexkit-role-step-transform-provider/CONTRACT.md) | [Vectors](../test-vectors/step-transform-provider-v1/) | No separate runner crate yet |
| `agent-data/v1` | [Contract](../crates/cortexkit-role-agent-data/CONTRACT.md) | [Crate-local vectors](../crates/cortexkit-role-agent-data/test-vectors/agent-data-v1/README.md) | No separate runner crate yet |

Vectors provide concrete wire examples that tests decode, re-encode, or validate. They do not replace the contract, and their presence alone does not mean a live implementation has passed conformance. Follow each contract's pinned/open or draft status rather than assuming every documented case is implemented.

## Testing an implementation

Keep the role crate in production dependencies and its separate `-conformance` crate in test dependencies. The split keeps the suite out of a module's production build.

The [shared harness](../crates/cortexkit-role-harness/README.md) gives suites a way to spawn a module on a fresh state root, stop it at a named durable point, restart it on the same root, and reach it over its real route. Its crash driver records the kill mechanism and requires a real process kill during a crash run. See the [harness source](../crates/cortexkit-role-harness/src/lib.rs) for the interface and lifecycle requirements.

Use the runner README linked above for its subject/route adapters, setup, and suite entry point. Running the commons workspace tests checks the shared crates and the suites' own tests; it does not certify a separate module implementation.

## Packaging vectors

Vector locations currently vary, as the table shows. A published crate cannot rely on files outside its package directory. If it reads or embeds vectors, include those bytes inside the crate's package. The tool-provider wire crate packages its shared vectors and exposes them through the `test-vectors` feature; the conformance crate enables that feature and reads the same bytes instead of keeping private copies. See [contribution guidance](../CONTRIBUTING.md#versions-and-packaged-test-vectors) for the package check and versioning rule.
