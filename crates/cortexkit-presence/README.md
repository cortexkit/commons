# cortexkit-presence

Pure presence summaries and notification routing, with no clock, storage or
I/O. Callers pass Unix epoch milliseconds and devices whose identity a
trusted service has already established.

- `summarize(devices, now_ms)` turns one person's device reports, the latest
  per device, into a `Summary`: where the person is (`phone`, `computer`,
  `away` or `unknown`) and which agents' chats active apps are showing, with
  the item kinds (caller-defined strings, such as `ask`) each draws inline.
- `route(&summary, kind, agent_id, now_ms)` answers where an out-of-chat ask
  or notice goes: `none` when a chat seen in the last 60 seconds already draws
  that kind inline, `desktop` when the person is at the computer, and `phone`
  otherwise, including away and unknown.

A presence service and every consumer that routes notices use this crate, so
they reach the same decision from the same summary, kind, agent and time.
`Summary::same_attention` ignores report times, which `route` reads, so route
from a fresh summary rather than one kept from an earlier update.

```toml
[dependencies]
cortexkit-presence = "0.1"
```
