# Changelog

## 0.1.1

- Ship the Claude OAuth wire test vectors inside the package under
  `test-vectors/claude-oauth-wire/`, exposed through the `vectors` module
  behind the new `test-vectors` feature. Each case fixes a host request and
  the exact headers, body, billing header and `cch` a client must send on the
  OAuth route, so every client that sends Claude Code-shaped requests tests
  against the same bytes. No runtime behaviour changes.

## 0.1.0

- Initial public release of Anthropic authentication-header policy, stable beta
  merging and removal, account-limit and out-of-credit classifiers, and the
  unified quota-header allowlist.
