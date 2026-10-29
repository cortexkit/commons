# cortexkit/commons

Shared Rust crates for the [CortexKit](https://github.com/cortexkit) fleet: role contracts and conformance suites, mutation testing, test support, leases and private files, bus and store adapters, and model catalog types.

## Crates

| Crate | Purpose | crates.io |
|---|---|---|
| [cortexkit-agent-identity](crates/cortexkit-agent-identity/README.md) | Typed agent identity, session, fleet and context reads for the agent registry (0.1.1). | Not published |
| [cortexkit-anthropic-policy](crates/cortexkit-anthropic-policy/README.md) | Anthropic authentication headers, beta tokens, account limits, credits, and quota-header policy. | [Published](https://crates.io/crates/cortexkit-anthropic-policy) |
| [cortexkit-bus-inmemory](crates/cortexkit-bus-inmemory/README.md) | Deterministic, process-local implementation of the bus traits. | [Published](https://crates.io/crates/cortexkit-bus-inmemory) |
| [cortexkit-bus-naming](crates/cortexkit-bus-naming/README.md) | Validation of message-plane names, topology limits, and grants. | [Published](https://crates.io/crates/cortexkit-bus-naming) |
| [cortexkit-bus-nats](crates/cortexkit-bus-nats/README.md) | NATS and JetStream backend for the bus traits. | [Published](https://crates.io/crates/cortexkit-bus-nats) |
| [cortexkit-bus-trait](crates/cortexkit-bus-trait/README.md) | NATS-shaped message, register, stream, and work-queue interfaces. | [Published](https://crates.io/crates/cortexkit-bus-trait) |
| [cortexkit-cache-core](crates/cortexkit-cache-core/README.md) | Prompt-cache stability state machine for freezing and replaying content. | Not published |
| [cortexkit-chatgpt-policy](crates/cortexkit-chatgpt-policy/README.md) | ChatGPT and Codex subscription account ids, refusal classification, and quota-header policy. | [Published](https://crates.io/crates/cortexkit-chatgpt-policy) |
| [cortexkit-cow](crates/cortexkit-cow/README.md) | Copy-on-write workspace isolation and change capture. | [Published](https://crates.io/crates/cortexkit-cow) |
| [cortexkit-exec-remote-types](crates/cortexkit-exec-remote-types/README.md) | Caller-facing types and vectors for `exec-remote/v1`. | [Published](https://crates.io/crates/cortexkit-exec-remote-types) |
| [cortexkit-lease](crates/cortexkit-lease/README.md) | Durable single-writer leases and owner-only file/directory helpers. | [Published](https://crates.io/crates/cortexkit-lease) |
| [cortexkit-llm-tools](crates/cortexkit-llm-tools/README.md) | Single-item LLM tool contracts, JSON schemas, validation and vectors for codemode and flows. | [Published](https://crates.io/crates/cortexkit-llm-tools) |
| [cortexkit-log](crates/cortexkit-log/README.md) | Logging format, filtering, redaction, rotation, and retention. | [Published](https://crates.io/crates/cortexkit-log) |
| [cortexkit-model-batch](crates/cortexkit-model-batch/README.md) | Batch rules shared by the model-batch roles: errors, batch identity, images, and the cost estimate. | [Published](https://crates.io/crates/cortexkit-model-batch) |
| [cortexkit-model-catalog](crates/cortexkit-model-catalog/README.md) | Model catalog wire types and exact integer pricing conversion. | Not published |
| [cortexkit-mutate](crates/cortexkit-mutate/README.md) | The `ckdev-mutate` mutation-testing CLI. | Not published |
| [cortexkit-paths](crates/cortexkit-paths/README.md) | Canonical project-root identities and absolute user-directory paths. | [Published](https://crates.io/crates/cortexkit-paths) |
| [cortexkit-persona](crates/cortexkit-persona/README.md) | Load, merge and render a module's role personas: tiers with trust classes, project approvals, sealed MiniJinja templates and digests. | [Published](https://crates.io/crates/cortexkit-persona) |
| [cortexkit-project-identity](crates/cortexkit-project-identity/README.md) | Typed project requests, replies, and forward-compatible decoding for the agent registry. | Not published |
| [cortexkit-provider-usage](crates/cortexkit-provider-usage/README.md) | Provider usage, quota windows, and reset metadata wire types. | [Published](https://crates.io/crates/cortexkit-provider-usage) |
| [cortexkit-push-seal](crates/cortexkit-push-seal/README.md) | HPKE encryption of push-notification payloads for a recipient device. | Not published |
| [cortexkit-resource-busy](crates/cortexkit-resource-busy/README.md) | Shared refusal detail for exclusive resources held by another caller. | Not published |
| [cortexkit-rich-ui-syntax](crates/cortexkit-rich-ui-syntax/README.md) | Bounded rich UI parser and reference scanner for `$$tc(N)` tokens. | [Published](https://crates.io/crates/cortexkit-rich-ui-syntax) |
| [cortexkit-role-agent-data](crates/cortexkit-role-agent-data/README.md) | Wire contract and vectors for the `agent-data/v1` capability. | [Published](https://crates.io/crates/cortexkit-role-agent-data) |
| [cortexkit-role-agent-inbox](crates/cortexkit-role-agent-inbox/README.md) | Wire contract, neutralization and vectors for `agent-inbox/v1`. | [Published](https://crates.io/crates/cortexkit-role-agent-inbox) |
| [cortexkit-role-agent-inbox-conformance](crates/cortexkit-role-agent-inbox-conformance/README.md) | Conformance runner for a live agent inbox implementation. | [Published](https://crates.io/crates/cortexkit-role-agent-inbox-conformance) |
| [cortexkit-role-classifier](crates/cortexkit-role-classifier/README.md) | Wire contract and vectors for `classifier/v1`. | [Published](https://crates.io/crates/cortexkit-role-classifier) |
| [cortexkit-role-classifier-conformance](crates/cortexkit-role-classifier-conformance/README.md) | Conformance runner for a live classifier implementation. | [Published](https://crates.io/crates/cortexkit-role-classifier-conformance) |
| [cortexkit-role-compaction-provider](crates/cortexkit-role-compaction-provider/README.md) | Wire contract and vectors for `compaction-provider/v1`. | [Published](https://crates.io/crates/cortexkit-role-compaction-provider) |
| [cortexkit-role-extractor](crates/cortexkit-role-extractor/README.md) | Wire contract and vectors for `extractor/v1`. | [Published](https://crates.io/crates/cortexkit-role-extractor) |
| [cortexkit-role-extractor-conformance](crates/cortexkit-role-extractor-conformance/README.md) | Conformance runner for a live extractor implementation. | [Published](https://crates.io/crates/cortexkit-role-extractor-conformance) |
| [cortexkit-role-harness](crates/cortexkit-role-harness/README.md) | Spawn, kill, restart, and routing interface for role conformance suites. | [Published](https://crates.io/crates/cortexkit-role-harness) |
| [cortexkit-role-llm-runner](crates/cortexkit-role-llm-runner/README.md) | Wire contract and vectors for `llm-runner/v1`. | [Published](https://crates.io/crates/cortexkit-role-llm-runner) |
| [cortexkit-role-llm-runner-conformance](crates/cortexkit-role-llm-runner-conformance/README.md) | Conformance runner for a live LLM runner with a scripted model. | [Published](https://crates.io/crates/cortexkit-role-llm-runner-conformance) |
| [cortexkit-role-routing](crates/cortexkit-role-routing/README.md) | Wire contract, runner program declarations and gated harness selection for `routing/v1` (0.4.1). | [Published](https://crates.io/crates/cortexkit-role-routing) |
| [cortexkit-role-routing-conformance](crates/cortexkit-role-routing-conformance/README.md) | Conformance kit for a live routing provider, including hosted programs and non-starting runners (0.6.0). | [Published](https://crates.io/crates/cortexkit-role-routing-conformance) |
| [cortexkit-role-step-transform-provider](crates/cortexkit-role-step-transform-provider/README.md) | Wire contract and vectors for step-transform hooks and operations. | [Published](https://crates.io/crates/cortexkit-role-step-transform-provider) |
| [cortexkit-role-tool-provider](crates/cortexkit-role-tool-provider/README.md) | Wire contract and vectors for tool catalogs, calls, results, and withdrawal. | [Published](https://crates.io/crates/cortexkit-role-tool-provider) |
| [cortexkit-role-tool-provider-conformance](crates/cortexkit-role-tool-provider-conformance/README.md) | Conformance runner for a live tool provider. | [Published](https://crates.io/crates/cortexkit-role-tool-provider-conformance) |
| [cortexkit-store](crates/cortexkit-store/README.md) | SQLite storage opening, migrations, and single-writer ownership. | [Published](https://crates.io/crates/cortexkit-store) |
| [cortexkit-store-postgres](crates/cortexkit-store-postgres/README.md) | PostgreSQL storage opening, migrations, and single-writer leases. | Not published |
| [cortexkit-store-types](crates/cortexkit-store-types/README.md) | Dependency-light storage descriptors and isolation types. | [Published](https://crates.io/crates/cortexkit-store-types) |
| [cortexkit-test-support](crates/cortexkit-test-support/README.md) | Portable scratch storage, executable staging, and daemon fencing. | [Published](https://crates.io/crates/cortexkit-test-support) |
| [cortexkit-usage-steps](crates/cortexkit-usage-steps/README.md) | Wire contract, vectors and a last-row-wins helper for the live usage step feed. | [Published](https://crates.io/crates/cortexkit-usage-steps) |

## Use a crate

Add a published crate to your `Cargo.toml`:

```toml
[dependencies]
cortexkit-paths = "0.1.2"
```

Crate READMEs above describe usage; unpublished crates are used from a source checkout. See [role contracts and conformance](docs/role-contracts.md) for the contract/vector/runner pattern, [CONTRIBUTING](CONTRIBUTING.md) for checks and releases, and [SECURITY](SECURITY.md) to report a vulnerability.

Run `scripts/gate.sh fast` and `scripts/gate.sh lint` before opening a pull request, and the slower `scripts/gate.sh test` on a Linux machine with many cores.
See `scripts/gate.sh --help` for prerequisites and CI-only coverage.

## License

[MIT](LICENSE).
