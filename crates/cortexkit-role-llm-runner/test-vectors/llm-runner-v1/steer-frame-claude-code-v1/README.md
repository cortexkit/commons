# Claude Code mid-turn host frames

These six examples were captured with Claude Code 2.1.331 and synthetic messages.
Each message was sent during the same running turn. The read envelopes name
`chat-app` as the fixture sender; each `stored-message.json` is the verbatim
native message, whose role is `system`. The read envelope's role is `user`.
The captures predate the `host_frame` fact.

`head.txt` includes the opening line's newline. `tail.txt` starts with two
newlines and has no trailing newline. The examples cover a single line,
multiple lines with a blank line, non-ASCII words, a verbatim copy of the
opening frame, a verbatim copy of the closing frame, and slash-prefixed words.
