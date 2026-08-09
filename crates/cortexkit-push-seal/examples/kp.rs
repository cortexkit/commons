fn main() {
    use hpke::{Kem, Serializable};
    let (sk, pk) = hpke::kem::X25519HkdfSha256::gen_keypair();
    let h = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    println!("SK {}", h(&sk.to_bytes()));
    println!("PK {}", h(&pk.to_bytes()));
}
