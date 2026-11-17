# `owned-broca` tool output views

These ten synthetic cases are derived from recordings of a runner serving
this profile, taken on 17 November 2026 through its real serve path and
family renderers. The recordings themselves are not published. Their
manifest's SHA-256,
`01d0f53ac1563efc1ecbf863b9c52a50cf10754fa4b801ab267f0969f5b253ca`, lets
whoever holds them confirm these cases came from that set. Case identifiers
and module names are replaced with public examples.

Each case records the `output` envelope and its sibling `is_error` from a
`session.read` tool-result part served with `media: "blocks"`. Expected text
and flags follow the text-only Anthropic request's tool results, checked
against OpenAI Chat, Responses, ChatGPT Responses, Bedrock, Gemini and Code
Assist. JSON text is compact (`{"count":2,"files":["a.rs","b.rs"]}`), not
pretty-printed. Gemini's response wrapper and Bedrock's JSON blocks are
transport shapes, not output text.

For the two media cases the view retains the stored references in order
instead of inserting the text-only request's modality-omission markers.
Only the stored text parts contribute to `text`, joined by a newline.
No notices, part labels, view rewrites or media bytes are included.
