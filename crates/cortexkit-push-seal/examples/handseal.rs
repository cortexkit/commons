// Proves the crate can seal a real payload to a real recipient key TODAY.
// Run: cargo run -p cortexkit-push-seal --example handseal -- <recipient_pk_hex> '<json>'
//
// The key is the PK LINE from the `kp` example -- its SECOND line, 64 hex
// characters, no label. Taking the first line hands this the SECRET key
// instead: a 64-char secret seals successfully to a keypair nobody holds, and
// the failure appears only on the device as an undecryptable notification.
// Argument parsing is deliberately unforgiving so that a mispaste panics here
// rather than producing a blob addressed to nothing.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pk = (0..a[1].len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&a[1][i..i + 2], 16).unwrap())
        .collect::<Vec<u8>>();
    let sealed = cortexkit_push_seal::seal(&pk, a[2].as_bytes()).expect("seal");
    println!(
        "{}",
        sealed
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    eprintln!(
        "plaintext {} bytes -> sealed {} bytes",
        a[2].len(),
        sealed.len()
    );
}
