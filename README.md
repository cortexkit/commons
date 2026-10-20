# cortexkit/commons

Shared Rust crates for the [CortexKit](https://github.com/cortexkit) fleet: role contracts and conformance suites, mutation testing, test support, leases and private files, bus and store adapters, and model catalog types.

## Crates

| Crate | Purpose | crates.io |
|---|---|---|
| [cortexkit-anthropic-policy](crates/cortexkit-anthropic-policy/README.md) | Anthropic authentication headers, beta tokens, account limits, credits, and quota-header policy. | [0.1.0](https://crates.io/crates/cortexkit-anthropic-policy) |
| [cortexkit-bus-inmemory](crates/cortexkit-bus-inmemory/README.md) | Deterministic, process-local implementation of the bus traits. | [0.3.0](https://crates.io/crates/cortexkit-bus-inmemory) |
| [cortexkit-bus-naming](crates/cortexkit-bus-naming/README.md) | Validation of message-plane names, topology limits, and grants. | [0.2.0](https://crates.io/crates/cortexkit-bus-naming) |
| [cortexkit-bus-nats](crates/cortexkit-bus-nats/README.md) | NATS and JetStream backend for the bus traits. | [0.3.0](https://crates.io/crates/cortexkit-bus-nats) |
| [cortexkit-bus-trait](crates/cortexkit-bus-trait/README.md) | NATS-shaped message, register, stream, and work-queue interfaces. | [0.3.0](https://crates.io/crates/cortexkit-bus-trait) |
| [cortexkit-cache-core](crates/cortexkit-cache-core/README.md) | Prompt-cache stability state machine for freezing and replaying content. | Not published |
| [cortexkit-cow](crates/cortexkit-cow/README.md) | Copy-on-write workspace isolation and change capture. | [0.1.0](https://crates.io/crates/cortexkit-cow) |
| [cortexkit-exec-remote-types](crates/cortexkit-exec-remote-types/README.md) | Caller-facing types and vectors for `exec-remote/v1`. | [0.2.8](https://crates.io/crates/cortexkit-exec-remote-types) |
| [cortexkit-lease](crates/cortexkit-lease/README.md) | Durable single-writer leases and owner-only file/directory helpers. | [0.1.3](https://crates.io/crates/cortexkit-lease) |
| [cortexkit-log](crates/cortexkit-log/README.md) | Logging format, filtering, redaction, rotation, and retention. | [0.3.5](https://crates.io/crates/cortexkit-log) |
| [cortexkit-model-catalog](crates/cortexkit-model-catalog/README.md) | Model catalog wire types and exact integer pricing conversion. | Not published |
| [cortexkit-mutate](crates/cortexkit-mutate/README.md) | The `ckdev-mutate` mutation-testing CLI. | Not published |
| [cortexkit-paths](crates/cortexkit-paths/README.md) | Canonical project-root identities and absolute user-directory paths. | [0.1.2](https://crates.io/crates/cortexkit-paths) |
| [cortexkit-provider-usage](crates/cortexkit-provider-usage/README.md) | Provider usage, quota windows, and reset metadata wire types. | [0.11.0](https://crates.io/crates/cortexkit-provider-usage) |
| [cortexkit-push-seal](crates/cortexkit-push-seal/README.md) | HPKE encryption of push-notification payloads for a recipient device. | Not published |
| [cortexkit-resource-busy](crates/cortexkit-resource-busy/README.md) | Shared refusal detail for exclusive resources held by another caller. | Not published |
| [cortexkit-role-agent-data](crates/cortexkit-role-agent-data/README.md) | Wire contract and vectors for the `agent-data/v1` capability. | [0.2.0](https://crates.io/crates/cortexkit-role-agent-data) |
| [cortexkit-role-agent-inbox](crates/cortexkit-role-agent-inbox/README.md) | Wire contract, neutralization and vectors for `agent-inbox/v1`. | Not published (0.1.0) |
| [cortexkit-role-agent-inbox-conformance](crates/cortexkit-role-agent-inbox-conformance/README.md) | Conformance runner for a live agent inbox implementation. | Not published (0.1.0) |
| [cortexkit-role-classifier](crates/cortexkit-role-classifier/README.md) | Wire contract and vectors for `classifier/v1`. | Not published |
| [cortexkit-role-classifier-conformance](crates/cortexkit-role-classifier-conformance/README.md) | Conformance runner for a live classifier implementation. | Not published |
| [cortexkit-role-compaction-provider](crates/cortexkit-role-compaction-provider/README.md) | Wire contract and vectors for `compaction-provider/v1`. | [0.2.4](https://crates.io/crates/cortexkit-role-compaction-provider) |
| [cortexkit-role-harness](crates/cortexkit-role-harness/README.md) | Spawn, kill, restart, and routing interface for role conformance suites. | [0.1.0](https://crates.io/crates/cortexkit-role-harness) |
| [cortexkit-role-llm-runner](crates/cortexkit-role-llm-runner/README.md) | Wire contract and vectors for `llm-runner/v1`. | [0.2.3](https://crates.io/crates/cortexkit-role-llm-runner) |
| [cortexkit-role-llm-runner-conformance](crates/cortexkit-role-llm-runner-conformance/README.md) | Conformance runner for a live LLM runner with a scripted model. | [0.3.1](https://crates.io/crates/cortexkit-role-llm-runner-conformance) |
| [cortexkit-role-step-transform-provider](crates/cortexkit-role-step-transform-provider/README.md) | Wire contract and vectors for step-transform hooks and operations. | [0.2.3](https://crates.io/crates/cortexkit-role-step-transform-provider) |
| [cortexkit-role-tool-provider](crates/cortexkit-role-tool-provider/README.md) | Wire contract and vectors for tool catalogs, calls, results, and withdrawal. | [0.8.1](https://crates.io/crates/cortexkit-role-tool-provider) |
| [cortexkit-role-tool-provider-conformance](crates/cortexkit-role-tool-provider-conformance/README.md) | Conformance runner for a live tool provider. | [0.8.0](https://crates.io/crates/cortexkit-role-tool-provider-conformance) |
| [cortexkit-store](crates/cortexkit-store/README.md) | SQLite storage opening, migrations, and single-writer ownership. | [0.3.0](https://crates.io/crates/cortexkit-store) |
| [cortexkit-store-postgres](crates/cortexkit-store-postgres/README.md) | PostgreSQL storage opening, migrations, and single-writer leases. | Not published |
| [cortexkit-store-types](crates/cortexkit-store-types/README.md) | Dependency-light storage descriptors and isolation types. | [0.2.2](https://crates.io/crates/cortexkit-store-types) |
| [cortexkit-test-support](crates/cortexkit-test-support/README.md) | Portable scratch storage, executable staging, and daemon fencing. | [0.1.8](https://crates.io/crates/cortexkit-test-support) |

## Use a crate

Add a published crate to your `Cargo.toml`:

```toml
[dependencies]
cortexkit-paths = "0.1.2"
```

Crate READMEs above describe usage; unpublished crates are used from a source checkout. See [role contracts and conformance](docs/role-contracts.md) for the contract/vector/runner pattern, [CONTRIBUTING](CONTRIBUTING.md) for checks and releases, and [SECURITY](SECURITY.md) to report a vulnerability.

## License

[MIT](LICENSE).
