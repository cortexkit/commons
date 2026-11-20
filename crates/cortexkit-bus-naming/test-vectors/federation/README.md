# Federation fixtures

`hub.json` pins the names, hub stream descriptors, and permissions a machine
uses to connect to the hub as a leaf. The sample box is
`0123456789abcdef0123456789abcdef`. The synthetic hub key pin is
`0f1e2d3c4b5a69780f1e2d3c4b5a69780f1e2d3c4b5a69780f1e2d3c4b5a6978`,
so the hub inbox name ends in `_0f1e2d3c4b5a6978`. The leaf grant names that
inbox regardless of whether the hub is self-hosted or hosted. The fixture
lists each KV bucket twice: with one replica on a self-hosted hub and three on
a hosted hub. The example module name only shows how a deliver prefix is formed
(`ckdel.<box id>.<module>`); it names no real module.
These hub streams must not be provisioned on a machine's local server.

| Hub KV bucket | Backing stream | Records | History | Max message | Max bucket size |
| --- | --- | --- | ---: | ---: | ---: |
| `residence` | `KV_residence` | Agent residence | 1 | 16 KiB | 64 MiB |
| `vault-shared-logins` | `KV_vault-shared-logins` | Sealed shared logins, keyed by `{home}.{member}` | 1 | 256 KiB | 64 MiB |
| `vault-manifest-active` | `KV_vault-manifest-active` | Active vault manifests, keyed by `{computer}` | 1 | 16 KiB | 16 MiB |

Each vault key component is a canonical 32-character lowercase hexadecimal box ID.

`HUB_ROOMS` is account-wide: `ck.fleet.room.<room_id>` keeps 100,000 messages
per room, evicting that room's oldest message first, with no age limit and a
256 KiB message-size cap. Its leaf read/consumer API permissions match HUB_IDLOG,
with no purge or delete authority. `fleet` is not a canonical box id.

`HUB_MEMORY` is also account-wide. Its binding and leaf publish grant both use
`ck.fleet.memory.*.*`, with positions for the memory kind and owner. It has no
age or per-subject message limit, caps messages at 64 KiB, and refuses new
publishes when it reaches 1 GiB. Its leaf read and consumer APIs match HUB_ROOMS.

`hub-module-grants.json` pins the six canonical modules' federation-account
hub user grants for the same sample box. Each entry has publish and subscribe
allow lists and explicit denies keyed by operation and subject. During the
rename, `prefrontal-core` gets the same publish allows and denies as
`prefrontal`, but can subscribe only to `ckdel.<box id>.prefrontal-core.>`.
The vault module reads and watches both vault buckets and can put or delete only
records where its own box ID is the `home` or `computer`. Each shared-login list
is sealed to its recipient even though every vault can read the ciphertext.
The connectors module can read only `vault-manifest-active` and cannot write to
either bucket.

`local.json` pins the box-account handoff streams, inbound/outbound subjects,
universal publish deny, both delivery authority ids' outbound grants and module
readers, and the federation account's separate `FEDOUT` stream. Box-account names
use `box_<box id>` while destination and origin tokens are bare box ids. Every
box-account user must receive the explicit deny in addition to its allow list.
Box-account import/export descriptors are owned by the bus module, not this kit.

The JSON is UTF-8, pretty-printed with two spaces, sorted object keys, and one
trailing newline. Array order is significant: publish and subscribe subjects are
fixed-order lists with no duplicates. A consumer in another language can compare
its output byte-for-byte with this fixture, using the same serialization rules.

All stream descriptors require file storage. Stream ages and duplicate windows
are seconds; byte limits are bytes. A zero age
means no TTL, and a zero byte limit means no size cap. `null` means no explicit
limit or replication override: omit that server property, rather than setting
it to zero. `history` and the backing stream's per-subject cap must agree.
KV and object-store backing streams must be provisioned with the settings in
the fixture. The vault buckets disable rollup headers so a writer cannot use a
stream-wide rollup to remove records outside its own keys.

Run `cargo test -p cortexkit-bus-naming --all-features --test federation golden_matches_bytes`
to check the fixtures against the Rust builders. To regenerate them, run
`UPDATE_FEDERATION_GOLDENS=1 cargo test -p cortexkit-bus-naming --all-features --test federation golden_matches_bytes`.
Intentional policy changes update the fixtures and their `SHA256SUMS` in the same
commit. Verify the checksums from this
directory with `sha256sum -c SHA256SUMS` (or `shasum -a 256 -c SHA256SUMS`).
