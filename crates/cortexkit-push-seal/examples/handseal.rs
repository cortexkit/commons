// Proves the crate can seal a real payload to a real recipient key TODAY.
// Run: cargo run -p cortexkit-push-seal --example handseal -- <recipient_pk_hex> '<json>'
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
