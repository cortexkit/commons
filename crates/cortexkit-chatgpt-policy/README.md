# cortexkit-chatgpt-policy

A transport-independent Rust library for the policy a ChatGPT-subscription (Codex) client
needs: resolving the ChatGPT account id, classifying the backend's refusals, and selecting the
`x-codex-*` quota headers.

It is intended for Rust applications that implement Codex clients, gateways, and provider
adapters on ChatGPT subscription accounts. The library returns policy data without owning
credentials, building request headers, rendering request bodies, or depending on an HTTP
client or async runtime.

## Included policy

- `account_id` returns a non-blank vault-served account id unchanged, else the access token's
  `chatgpt_account_id` claim (top level, then nested under `https://api.openai.com/auth`,
  then the first organization id). Its `MissingAccountId` error never contains the token.
- `classify_response` and `classify_response_with_retry_after` classify an HTTP refusal as
  an exhausted plan allowance with its reset time, a plan without the requested use, a
  credential failure, the backend's own rejected `sk-` key (transient, and never a reason to
  refresh or report the client's credential), a rate limit, an overloaded backend, or other.
- `classify_stream_failure` applies the same classes to an in-band `response.failed` or
  `error` stream event.
- `quota_headers` keeps only the `x-codex-*` rate-limit headers the Codex CLI parses, as a
  typed `QuotaHeader` allowlist. Free-text promotion headers are excluded.
- `CODEX_CLI_ORIGINATOR` is the Codex CLI's `originator` header value. Clients that already
  send their own identity headers, such as the Codex CLI itself, keep them.

## Example

Decide whether a refusal should end the request until the account resets:

```rust
use cortexkit_chatgpt_policy::{classify_response, Classification};
use std::time::{Duration, UNIX_EPOCH};

let body = r#"{"error":{"type":"usage_limit_reached","resets_at":1790021043}}"#;
let now = UNIX_EPOCH + Duration::from_secs(1_789_579_322);
let class = classify_response(429, [("retry-after", "30")], body, now);
assert!(matches!(
    class,
    Classification::AccountLimitExhausted { resets_at_ms: Some(1_790_021_043_000), .. }
));
```

## Shared test vectors

`test-vectors/` holds JSON vectors of real and reconstructed backend answers with the class
each must receive, plus account-id resolution cases. Each body is marked CAPTURED or
RECONSTRUCTED. The crate's own tests run every vector, and other implementations can run the
same files.

## Third-party notices

The usage-limit classification follows the open-source OpenAI Codex client
(https://github.com/openai/codex, Apache-2.0, Copyright 2025 OpenAI). The account-claim
extraction follows OpenCode's `extractAccountIdFromClaims` (MIT, Copyright (c) 2025
opencode).
