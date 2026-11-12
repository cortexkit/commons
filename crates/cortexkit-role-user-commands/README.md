# cortexkit-role-user-commands

`user-commands/v1` wire data and pure validators for commands a person runs
from an app. Shared by declaring modules and a relaying app server; no server
or runtime is included.

Read [CONTRACT.md](CONTRACT.md) for pinned admission, preview, idempotency and
approval duties. The declaration's `subject` is a kind string; request subjects
are identifying objects. Rust types are non-exhaustive and have constructors.

```rust
use cortexkit_role_user_commands::{Command, DescribeReply, SubjectKind};

let status = Command::new(
    "status", "Status", "Shows local status.", SubjectKind::Computer,
    vec![], vec!["read".into()], None,
);
let reply = DescribeReply::new(vec![status]).unwrap();
reply.check().unwrap();
```

Decode preserves unfamiliar effects and fact vocabulary. Producers validate
before sending; readers withhold run for unknown effects and show refused
facts as neutral placeholders rather than failing an entire preview.
Fact validators expose `fact_malformed`, `fact_conflict` and `fact_range`.

Portable samples and their SHA-256 manifest are in
`test-vectors/user-commands-v1/`. Tests exercise every JSON file, valid and
invalid cases, round trips, exact limits and an independently fixed JCS digest.
