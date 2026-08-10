// Prints two keys. PK is the recipient key that `handseal` takes; SK is the
// matching secret, needed only by `handopen` to verify a round trip. They are
// the same length in hex, so feeding the wrong one seals to a keypair nobody
// holds and fails silently on the device rather than here.
fn main() {
    use hpke::{Kem, Serializable};
    let (sk, pk) = hpke::kem::X25519HkdfSha256::gen_keypair();
    let h = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    println!("SK {}", h(&sk.to_bytes()));
    println!("PK {}", h(&pk.to_bytes()));
}
