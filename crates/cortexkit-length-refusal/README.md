# cortexkit-length-refusal

> [!WARNING]
> **Alpha.** cortexkit-length-refusal is in early development and is not yet released for general use.
> Breaking changes will happen between versions, including changes to its settings,
> stored data and interfaces. Pin an exact version if you depend on it.

Whether a model provider refused a request because it is too long for the model's context
window, and the token counts the refusal states. Runners share it so every runner reads the
same refusal the same way.

```rust
use cortexkit_length_refusal::{classify, LengthRefusal};

let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 208310 tokens > 200000 maximum"}}"#;
assert_eq!(
    classify("anthropic.messages", Some(400), body),
    Some(LengthRefusal { request_tokens: Some(208_310), limit_tokens: Some(200_000) })
);
```

A refusal counts only when the provider says so in a way it reserves for this case: a
dedicated error code, or a generic error type together with a whole message of a known shape.
Counts the message doesn't state are `None`; nothing is estimated.

`classify_text` reads a refusal from only the words a host kept of it, for a runner that never
sees the wire family or the status.
