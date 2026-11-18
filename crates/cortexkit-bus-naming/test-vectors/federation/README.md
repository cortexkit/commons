# Federation fixtures

`hub.json` pins the names, leaf-user allow lists, and hub stream descriptors for
the sample box `0123456789abcdef0123456789abcdef`. Both self-hosted and hosted
issuers use the same grant. The residence key-value bucket (agent residence
records) is stored with one replica on a self-hosted hub and three on a hosted
hub, so the fixture lists its descriptor once for each hub kind. The example
module name only shows how a module's deliver prefix is formed
(`ckdel.<box id>.<module>`); it names no real module.
These hub streams must not be provisioned on a machine's local server.

`HUB_ROOMS` is account-wide: `ck.fleet.room.<room_id>` keeps 100,000 messages
per room, evicting that room's oldest message first, with no age limit and a
256 KiB message-size cap. Its leaf read/consumer API permissions match HUB_IDLOG,
with no purge or delete authority. `fleet` is not a canonical box id.

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
KV and object-store backing settings must also agree with the client API's
creation settings, including rollup, direct-get, and delete policies.

Run `cargo test -p cortexkit-bus-naming --all-features federation_golden_matches_bytes`
and `cargo test -p cortexkit-bus-naming --all-features federation_local_golden_matches_bytes`
to check the fixtures against the Rust builders. To regenerate both, run
`UPDATE_FEDERATION_GOLDENS=1 cargo test -p cortexkit-bus-naming --all-features --test federation golden_matches_bytes`.
Intentional policy changes update the fixtures and their `SHA256SUMS` in the same
commit. Verify the checksums from this
directory with `sha256sum -c SHA256SUMS` (or `shasum -a 256 -c SHA256SUMS`).
