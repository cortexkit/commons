# Provider service-tier vectors

`cases.json` is the shared sample set. Its digest is pinned in `SHA256SUMS`.
`cortexkit-role-llm-runner`, `cortexkit-role-compaction-provider`,
`cortexkit-role-routing` and `cortexkit-usage-steps` carry byte-for-byte copies so
their published tests do not depend on a repository checkout. Update the samples
and all copies together.

The set covers accepted and refused describe maps, each send level and absence,
the unsupported and fixed-run refusal details, applied-only and provider-stated
served usage (including an unmapped value), a tier-change ride, and notice
conditions with and without the digest-covered tier header. Send encoders omit
only absence and preserve explicit standard. Standard notice conditions omit
the tier header;
a ride returning to standard still carries its required new level.
