# cortexkit-role-tool-display

Wire types, RFC 8785 JSON canonicalization and idempotency helpers, and
validators for `tool-display/v1`.
Tool providers asynchronously supply transcript rows to the display server after
answering calls. Provider views describe how apps draw those rows and are keyed
by the runner's recorded tool-schema pin; a pin never changes meaning.

`APP_SERVER_CAPABILITY` (`app-server/v1`) says the module serves the apps'
transcript reads and sends, and lets a plan composer name it in the plan's
senders and readers;
it says nothing about display storage.
`TOOL_DISPLAY_CAPABILITY` (`tool-display/v1`) independently promises
`display.write` and provider-view fetching. Providers discover the display
server by that capability, never by module name, and write nothing unless
exactly one live module declares it. A module may declare either capability
without the other. The display server stores the writer identity authenticated
by the daemon on the incoming write route (the logical channel carrying the
request); the app server uses that identity when joining a transcript.

```toml
[dependencies]
cortexkit-role-tool-display = "0.1.0"
```

The [contract](CONTRACT.md) is normative. `DisplayWrite`, `DisplayKind`,
`DisplayLimits` and `WriteReply` cover writes; `RecordSchema`, `FieldKind` and
the `schema` validators check flat records and completions against fixed
field kinds, none of which is free text.
`canonical_body`, `idempotency_key`, `validate_write` and `write_outcome` are
pure helpers. They do not route, persist, fetch views or authorize joins.

`test-vectors/tool-display-v1/` ships with the package and is pinned by
`SHA256SUMS`. Tests read every JSON vector. The optional `tool.views` operation
and its manifest types live in
[`cortexkit-role-tool-provider`](../cortexkit-role-tool-provider/README.md).

Enable the optional `test-vectors` feature to access `vectors::FILES`,
`vectors::VIEWS_JSON` and `vectors::SHA256SUMS` from another crate's tests.
