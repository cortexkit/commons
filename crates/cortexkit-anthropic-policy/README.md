# cortexkit-anthropic-policy

A transport-independent Rust library for Anthropic authentication-header policy,
beta-header merging, account-exhaustion and out-of-credit classification, and the
unified quota-header allowlist.

It is intended for Rust applications that implement Anthropic OAuth or API-key
clients, gateways, and provider adapters. The library returns policy data without
owning credentials, rendering request bodies, or depending on an HTTP client or
async runtime.

## Included policy

- `header_policy` selects the credential header and OAuth `x-app` value, and
  returns ordered beta additions for explicit feature flags.
- `merge_beta_headers` joins multiple beta fields into a stable, first-seen union;
  `remove_beta_token` removes only an exact token. Both validate the full header
  value and preserve unrelated beta tokens.
- `classify_account_limit_exhausted` recognizes an account wall only for HTTP 429
  with unified status `rejected`, and selects reset information from quota headers
  or a Retry-After hint.
- `credit_balance_exhausted` and `anthropic_credit_balance_too_low` classify the
  Anthropic out-of-credit message without treating unrelated invalid requests as
  billing exhaustion.
- `collect_unified_quota_headers` keeps only the
  `anthropic-ratelimit-unified-*` header family, including future extensions.

## Example

Combine a client beta with OAuth policy additions while retaining stable order:

```rust
use cortexkit_anthropic_policy::{
    header_policy, merge_beta_headers, AuthKind, Features, InvalidBetaHeader,
};

fn main() -> Result<(), InvalidBetaHeader> {
    let policy = header_policy(
        AuthKind::OAuth,
        Features {
            interleaved_thinking: true,
            ..Features::default()
        },
    );
    let beta = merge_beta_headers(["client-beta", policy.beta.as_deref().unwrap_or("")])?;
    assert_eq!(
        beta,
        "client-beta,oauth-2025-04-20,interleaved-thinking-2025-05-14"
    );
    Ok(())
}
```

The returned credential metadata describes where an application should attach its
own credential; secrets remain with the client. See the crate API documentation
for reset-source details and the behavior of each classifier.
