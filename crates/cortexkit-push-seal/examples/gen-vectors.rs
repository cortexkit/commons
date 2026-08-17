//! Mints the lab-keyed conformance corpus at `test-vectors/push-sealed-payload.json`.
//!
//! PRODUCER-MINTED ON PURPOSE: a consumer's self-written fixture tests its
//! decoder against its own reading of the layout, and both come from one head.
//! This binary is the producer's head; the Swift opener executes THESE bytes.
//!
//! The keypairs are fixed lab scalars, so every identity in the file is
//! re-derivable — but the envelopes are NOT deterministic across runs (HPKE
//! mints a fresh ephemeral per seal). The committed file is therefore the
//! artifact of ONE run; re-running REPLACES the corpus rather than reproducing
//! it, and the vector-set version below must move when the case set changes so
//! consumers comparing case-name sets fail loudly rather than passing quietly.
//!
//! Re-mint against the Mac's real re-enrolled key: pass the sender keypair as
//! `--sender-sk-hex <64> --sender-pk-hex <64>` (the golden re-mint the push
//! room sequenced after ALF's sitting); lab default otherwise.

use cortexkit_push_seal::{
    open, open_auth, seal, seal_auth, sender_key_id, OpenError, AUTH_INFO, KEY_ID_LEN, VERSION,
    VERSION_AUTH,
};
use hpke::{Deserializable, Serializable};

type Kem = hpke::kem::X25519HkdfSha256;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

/// A fixed lab scalar: value-repeated bytes are valid X25519 scalars after the
/// KEM's own clamping, and a recognizable pattern marks these as LAB material
/// in any capture they leak into.
fn lab_keypair(fill: u8) -> (Vec<u8>, Vec<u8>) {
    let sk_bytes = [fill; 32];
    let sk = <Kem as hpke::Kem>::PrivateKey::from_bytes(&sk_bytes).expect("lab scalar");
    let pk = <Kem as hpke::Kem>::sk_to_pk(&sk);
    (sk.to_bytes().to_vec(), pk.to_bytes().to_vec())
}

fn main() {
    // --sender-sk-hex/--sender-pk-hex switch the corpus onto a REAL sender key
    // (the golden re-mint); everything else stays lab-keyed.
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let (mac_sk, mac_pk, provenance) = match (flag("--sender-sk-hex"), flag("--sender-pk-hex")) {
        (Some(sk), Some(pk)) => (unhex(&sk), unhex(&pk), "production-sender"),
        (None, None) => {
            let (sk, pk) = lab_keypair(0x11);
            (sk, pk, "lab")
        }
        _ => panic!("--sender-sk-hex and --sender-pk-hex travel together"),
    };
    let (phone_sk, phone_pk) = lab_keypair(0x22);
    let (imposter_sk, imposter_pk) = lab_keypair(0x33);

    let plaintext: &[u8] = br#"{"kind":"ask","asker":"ALF","title":"A decision needs you"}"#;

    let mut cases: Vec<serde_json::Value> = Vec::new();
    let mut push = |name: &str,
                    envelope: Vec<u8>,
                    expect: &str,
                    open_as: &str,
                    expected_sender: Option<&[u8]>,
                    note: &str| {
        cases.push(serde_json::json!({
            "name": name,
            "envelope_hex": hex(&envelope),
            "open_as": open_as,
            "expected_sender_pubkey_hex": expected_sender.map(hex),
            "expect": expect,
            "note": note,
        }));
    };

    // ep-valid-auth: the rich-banner arm.
    let auth = seal_auth(&mac_sk, &mac_pk, &phone_pk, plaintext).expect("seal_auth");
    assert_eq!(
        open_auth(&phone_sk, &mac_pk, &auth).expect("self-check"),
        plaintext
    );
    push(
        "ep-valid-auth-mode",
        auth.clone(),
        "opens",
        "auth",
        Some(&mac_pk),
        "verifies against the pinned sender key and yields the plaintext; rich rendering is licensed by THIS outcome only",
    );

    // ep-valid-base: the downgrade arm — must open as v1, and its rendering
    // contract is GENERIC, never rich.
    let base = seal(&phone_pk, plaintext).expect("seal");
    assert_eq!(open(&phone_sk, &base).expect("self-check"), plaintext);
    push(
        "ep-valid-base-mode-downgrade",
        base.clone(),
        "opens",
        "base",
        None,
        "opens as v1; rendering contract: GENERIC banner (unauthenticated input), never rich, never a refused notification",
    );

    // ep-reject-cross-role: CKCRED's REQUIRED arm. A base seal TO the phone,
    // re-framed as v2 claiming the MAC (and separately the PHONE) authored it.
    for (label, claimed_pk) in [("mac", &mac_pk), ("phone-self", &phone_pk)] {
        let mut forged = Vec::with_capacity(base.len() + KEY_ID_LEN);
        forged.push(VERSION_AUTH);
        forged.extend_from_slice(&sender_key_id(claimed_pk));
        forged.extend_from_slice(&base[1..]);
        assert_eq!(
            open_auth(&phone_sk, claimed_pk, &forged),
            Err(OpenError::Aead),
            "cross-role self-check"
        );
        push(
            &format!("ep-reject-cross-role-{label}"),
            forged,
            "malformed",
            "auth",
            Some(claimed_pk),
            "a blob sealed TO a key must not verify as authored BY it (RFC 9180 mode separation, OUR binding pinned)",
        );
    }

    // ep-reject-wrong-sender: imposter authorship against the pinned key —
    // key-id forged to the Mac's so it reaches the tag, which refuses.
    let imposter = seal_auth(&imposter_sk, &imposter_pk, &phone_pk, plaintext).expect("seal_auth");
    let mut id_forged = imposter.clone();
    id_forged[1..1 + KEY_ID_LEN].copy_from_slice(&sender_key_id(&mac_pk));
    assert_eq!(
        open_auth(&phone_sk, &mac_pk, &id_forged),
        Err(OpenError::Aead),
        "wrong-sender self-check"
    );
    push(
        "ep-reject-wrong-sender-static",
        id_forged,
        "malformed",
        "auth",
        Some(&mac_pk),
        "an imposter forging the key-id still fails the tag: the id is routing, the tag is the boundary",
    );

    // ep-reject-key-id-mismatch: honest seal, opener pins a different key —
    // the rotation-window routing signal, distinct from forgery.
    assert_eq!(
        open_auth(&phone_sk, &imposter_pk, &auth),
        Err(OpenError::KeyIdMismatch {
            observed: sender_key_id(&mac_pk)
        }),
        "key-id self-check"
    );
    push(
        "ep-reject-key-id-mismatch",
        auth.clone(),
        "key_id_mismatch",
        "auth",
        Some(&imposter_pk),
        "routing refusal: try the OTHER pinned key; never retried against forgery (Aead on a MATCHING id)",
    );

    // ep-reject-mode-confusion, both directions.
    assert_eq!(
        open_auth(&phone_sk, &mac_pk, &base),
        Err(OpenError::ModeMismatch { observed: VERSION }),
        "mode self-check v1->auth"
    );
    push(
        "ep-reject-mode-confusion-v1-as-auth",
        base.clone(),
        "mode_mismatch",
        "auth",
        Some(&mac_pk),
        "the downgrade signal: distinct from corruption so the render layer maps it to the GENERIC banner",
    );
    assert_eq!(
        open(&phone_sk, &auth),
        Err(OpenError::ModeMismatch {
            observed: VERSION_AUTH
        }),
        "mode self-check auth->v1"
    );
    push(
        "ep-reject-mode-confusion-auth-as-v1",
        auth.clone(),
        "mode_mismatch",
        "base",
        None,
        "an authenticated envelope must not be silently opened unauthenticated: the downgrade decision belongs to the render layer",
    );

    // ep-reject-tampered-ciphertext: last byte flipped on a valid auth seal.
    let mut tampered = auth.clone();
    *tampered.last_mut().expect("nonempty") ^= 0x01;
    assert_eq!(
        open_auth(&phone_sk, &mac_pk, &tampered),
        Err(OpenError::Aead),
        "tamper self-check"
    );
    push(
        "ep-reject-tampered-ciphertext",
        tampered,
        "malformed",
        "auth",
        Some(&mac_pk),
        "altered bytes fail the tag",
    );

    let corpus = serde_json::json!({
        "corpus_meta": {
            "vector_set_version": 2,
            "spec": "subconscious/docs/specs/push-sealed-payload.md",
            "producer": "cortexkit-push-seal examples/gen-vectors.rs",
            "sender_key_provenance": provenance,
            "suite": { "kem_id": 0x0020, "kdf_id": 0x0001, "aead_id": 0x0003 },
            "auth_info_utf8": String::from_utf8_lossy(AUTH_INFO),
            "versions": { "base": VERSION, "auth": VERSION_AUTH },
            "key_id_len": KEY_ID_LEN,
            "key_id_derivation": "first 8 bytes of SHA-256 over the sender public key bytes",
        },
        "keys": {
            "mac_sender": { "sk_hex": if provenance == "lab" { hex(&mac_sk) } else { "withheld-production-key".to_string() }, "pk_hex": hex(&mac_pk), "key_id_hex": hex(&sender_key_id(&mac_pk)) },
            "phone_recipient": { "sk_hex": hex(&phone_sk), "pk_hex": hex(&phone_pk) },
            "imposter": { "sk_hex": hex(&imposter_sk), "pk_hex": hex(&imposter_pk), "key_id_hex": hex(&sender_key_id(&imposter_pk)) },
        },
        "plaintext_utf8": String::from_utf8_lossy(plaintext),
        "cases": cases,
    });

    println!("{}", serde_json::to_string_pretty(&corpus).expect("json"));
}
