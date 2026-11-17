# Federation hub fixture

`hub.json` pins the names, leaf-user allow lists, and hub stream descriptors for
the sample box `0123456789abcdef0123456789abcdef`. Both self-hosted and hosted
issuers use the same grant. Residence replication is shown separately for each
hub kind. The example module name only demonstrates the deliver-prefix shape.
These hub streams must not be provisioned on a machine's local server.

The JSON is UTF-8, pretty-printed with two spaces, sorted object keys, and one
trailing newline. Array order is significant: publish and subscribe subjects are
fixed-order lists with no duplicates. A consumer in another language can compare
its output byte-for-byte with this fixture, using the same serialization rules.

Stream ages and duplicate windows are seconds; byte limits are bytes. A zero age
means no TTL, and a zero byte limit means no size cap. `null` means no explicit
limit or replication override: omit that server property, rather than setting
it to zero. `history` and the backing stream's per-subject cap must agree.
KV and object-store backing settings must also agree with the client API's
creation settings, including rollup, direct-get, and delete policies.

Run `cargo test -p cortexkit-bus-naming --all-features federation_golden_matches_bytes`
to check the fixture against the Rust builders. Intentional policy changes update
the fixture and its `SHA256SUMS` in the same commit. Verify the checksum from this
directory with `sha256sum -c SHA256SUMS` (or `shasum -a 256 -c SHA256SUMS`).
