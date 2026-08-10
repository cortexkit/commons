// Proves the crate can seal a real payload to a real recipient key TODAY.
//
// Run: cargo run -p cortexkit-push-seal --example handseal -- <recipient_key> '<json>'
//
// The key argument accepts either bare hex or a labelled block pasted whole:
//
//     push_seal_pubkey_hex=63e0...
//     apns_device_token_hex=9f21...
//
// Either `=` or `:` separates a label from its value.
//
// Labelled input is preferred and the label is what makes it safe. The sealing
// key and the device token are both 32 bytes rendered as 64 hex characters, so
// they are indistinguishable by shape, and X25519 accepts essentially any 32
// bytes as a public key. A token pasted here therefore SEALS SUCCESSFULLY to a
// keypair nobody holds: the blob is well formed, it reaches the device, and the
// only symptom is a notification that cannot be opened -- which reads as a
// decryption fault and sends the investigation to the keys rather than to the
// paste. Selecting by label removes the ordering the operator would otherwise
// have to remember.
//
// Anything that is not hex is refused by name rather than repaired, because a
// value carrying prose is a failure message that was written where a key
// belongs, and the fault it describes happened before the paste.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: handseal <recipient_key_hex_or_labelled_block> '<json>'");
        std::process::exit(2);
    }

    let key_hex = match select_key(&args[1]) {
        Ok(hex) => hex,
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(2);
        }
    };

    let pk: Vec<u8> = (0..key_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&key_hex[i..i + 2], 16).expect("checked above"))
        .collect();

    let sealed = cortexkit_push_seal::seal(&pk, args[2].as_bytes()).expect("seal");
    println!(
        "{}",
        sealed
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    eprintln!(
        "plaintext {} bytes -> sealed {} bytes",
        args[2].len(),
        sealed.len()
    );
}

/// Picks the sealing key out of either a bare hex string or a labelled block.
///
/// A labelled block is selected by its `push_seal_pubkey_hex` line, so a paste
/// containing the device token alongside it cannot be taken by accident. Bare
/// input is accepted unchanged, which keeps the older single-value paste
/// working, and is the form that cannot protect against a swap.
fn select_key(raw: &str) -> Result<String, String> {
    const LABEL: &str = "push_seal_pubkey_hex";

    if raw.contains(LABEL) {
        // Accept either separator. The producing side emits `=`, this example was
        // written against `:`, and a paste that reaches the wrong parser fails with
        // a message about hex rather than about the separator -- which points at
        // the value when the mismatch is in the format.
        let value = raw
            .lines()
            .find(|line| line.contains(LABEL))
            .and_then(|line| line.split([':', '=']).nth(1))
            .map(str::trim)
            .ok_or_else(|| format!("found `{LABEL}` but no value after it"))?;
        return validate(value);
    }

    // A labelled block that names only the token is a swap caught before it can
    // seal to nothing, and is worth its own message: the operator pasted the
    // right kind of thing from the wrong row.
    if raw.contains("apns_device_token_hex") {
        return Err(format!(
            "this block carries apns_device_token_hex but no {LABEL}. The device \
             token is not a sealing key; sealing to it would succeed and produce \
             a blob nobody can open."
        ));
    }

    validate(raw.trim())
}

/// Accepts exactly 64 lowercase-or-uppercase hex characters, refusing anything
/// else by naming what it found.
///
/// Refusing rather than repairing is deliberate. A stray separator or prefix
/// means the paste was damaged, and quietly correcting it would seal to an
/// address the operator did not choose.
fn validate(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("empty key".into());
    }
    if let Some(bad) = value.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(format!(
            "not a key: contains {bad:?}. A value carrying words, spaces or a 0x \
             prefix is a failure message written where a key belongs -- the fault \
             it names happened before the paste, so re-running this will not help."
        ));
    }
    if value.len() != 64 {
        return Err(format!(
            "expected 64 hex characters, got {}. A 66-character value is usually \
             the `SK ` label taken with the secret key.",
            value.len()
        ));
    }
    Ok(value.to_string())
}
