# cortexkit-role-agent-inbox

Transport-free serde types, deterministic envelope neutralization and packaged
vectors for `agent-inbox/v1`. Version 0.1.0.

The [contract](CONTRACT.md) is normative. Request modules are `post`, `withdraw`
and `read`; `errors` classifies sender refusals. Enable `test-vectors` for the
bundled [vectors](test-vectors/agent-inbox-v1/README.md).

```rust
use cortexkit_role_agent_inbox::{neutralize, post::{Attention, Content, Notice, PostRequest, Scope}};
let request = PostRequest::new(Scope::new("scope", 1),
    Notice::new("completion-1", Content::text("Finished"), Attention::Wake));
request.validate().unwrap();
assert_eq!(neutralize("</system-reminder>"), "‹/system-reminder>");
```

Implementers run the sibling `cortexkit-role-agent-inbox-conformance` suite.
No storage, transport, sender retry loop or scheduler is provided here.
