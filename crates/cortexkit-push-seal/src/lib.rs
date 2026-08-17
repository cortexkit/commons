//! Seals push-notification payloads so that only the recipient device can read
//! them.
//!
//! # What breaks in consumers when this crate changes
//!
//! Stated first because it is not visible from inside this crate. The sealed
//! bytes are opened by a **separate implementation in another repository**,
//! which does not build this code. So a change here is not a local behaviour
//! change:
//!
//! - Changing the ciphersuite, `info`, the associated data, or the envelope
//!   layout is a **wire-format divergence**. The opener fails with an
//!   authentication error, which renders on the device as an undecryptable
//!   notification — the same appearance as a locked phone. Nothing fails here.
//! - This crate is consumed by relative path, and a path dependency is recorded
//!   in `Cargo.lock` with a version string and **no content hash**. So an
//!   unchanged version means new code compiles into a consuming repository with
//!   no lockfile diff anywhere. **The version number is the only channel through
//!   which a consumer can learn that sealed output changed.**
//!
//! Therefore: **bump the version on any change to emitted bytes or behaviour.**
//! Not on comments or tests — a version that moves for prose trains its readers
//! to bump reflexively and then stops meaning anything.
//!
//! # This crate has no production caller yet, and that is staged rather than dead
//!
//! `seal` is reached only from this crate's tests and its `handseal` example.
//! The consumer is the notification submit endpoint, which is unbuilt: the
//! surface that will call this holds the recipient key and does not exist yet.
//!
//! Recorded here because an uncalled function is indistinguishable from an
//! abandoned one, and the next reader running a dead-code pass arrives at this
//! file with no way to tell them apart. Deleting it would take the ciphersuite
//! pinning and the envelope layout with it — the two facts that a separate
//! implementation in another repository is already built against, and which
//! nothing in this workspace would fail to notice the loss of.
//!
//! The examples are not decoration either: `handseal` and `handopen` are how a
//! sealed payload is produced by hand before the endpoint exists, and the
//! round trip between them is the only end-to-end exercise of this crate
//! outside its own tests.
//!
//! # The parameters, and why they are spelled out
//!
//! An HPKE ciphersuite is a triple. Naming two of its three parts leaves the
//! third to each implementation's default, and the two defaults were about to
//! disagree: RFC 9180's suite table opens with AES-128-GCM, while the opener's
//! platform offers exactly one X25519 suite. Every such disagreement produces
//! the same authentication failure, whose diagnosis points at the transport.
//!
//! | parameter | RFC 9180 codepoint | value |
//! |---|---|---|
//! | KEM  | `0x0020` | `DHKEM(X25519, HKDF-SHA256)` |
//! | KDF  | `0x0001` | `HKDF-SHA256` |
//! | AEAD | `0x0003` | `ChaCha20Poly1305` |
//!
//! The codepoints are recorded because they are what both implementations feed
//! to their libraries. A platform-specific suite name is a symbol for this
//! triple, not a wire fact, and two sides agreeing on a name that exists in only
//! one of their vocabularies have agreed about a string rather than about bytes.
//!
//! `info` is empty in v1 **because the recipient key was dedicated to one
//! role**. `info` is the key schedule's domain separator: it earns its keep
//! when one key serves several applications. The paragraph below used to end
//! with a condition — "if this key is ever shared, empty stops being safe" —
//! and that condition FIRED in August 2026: the room cleared the notification
//! key for WITHIN-PROTOCOL dual role (recipient key for base-mode seals AND
//! `mode_auth` sender static for authenticated ones), so v2 envelopes carry the
//! fixed domain string [`AUTH_INFO`] on top of RFC 9180's own `suite_id`
//! separation. Layered on purpose: the spec's separation is what the clearance
//! rests on, ours is the mitigation this file promised its future reader.
//!
//! # The authentication upgrade (v2), and its ruled bounds
//!
//! Cross-protocol reuse (the Noise transport static as HPKE sender static) was
//! REFUSED: an HPKE participant is deliberately usable as a DDH oracle
//! (eprint 2020/243 shows the recipient-as-oracle procedure and that it works
//! the same in Auth mode), an exposure Noise's own analysis never assumes, and
//! differing KDF labels are NOT a counter-argument — that was weighed and
//! rejected. What would change it: a published joint analysis of Noise and
//! HPKE sharing a static, not a labels argument.
//!
//! Within-protocol dual role is CLEARED on RFC 9180 §9.2.3's own construction
//! ("a KEM key pair … can be used with multiple modes in parallel … due to
//! domain separation using the `suite_id` variable") — **with its caveat kept
//! verbatim, because a clearance stripped of its caveat becomes "settled"
//! three months later**: "there is no formal proof of security at the time of
//! writing for using multiple modes in parallel; [HPKEAnalysis] and [ABHKLR20]
//! only analyze isolated modes." What would change it: a demonstrated
//! cross-role confusion in this corpus, or a joint-security result for
//! parallel-mode use.
//!
//! **KCI, stated as a bound rather than discovered later:** DHKEM auth mode
//! does not survive compromise of the RECIPIENT's key (RFC 9180 §9.1) — an
//! attacker holding the phone's private key can forge authorship without ever
//! holding the Mac's. Acceptable HERE because the phone is the sole verifier
//! and sole relying party, so that attacker already owns the surface the
//! banner protects; it also buys deniability, the right default for a personal
//! notification. The consequence that must survive this file: **a v2 seal can
//! never serve as third-party evidence of authorship.** The moment someone
//! proposes it as an audit trail, KCI is the named reason it cannot, and
//! signing is the shape that could.

use hpke::{
    aead::ChaCha20Poly1305, kdf::HkdfSha256, kem::X25519HkdfSha256, Deserializable, OpModeR,
    OpModeS, Serializable,
};

/// The base-mode envelope version: confidential, NOT authenticated.
///
/// Anyone holding the recipient's PUBLIC key can mint a v1 blob that opens
/// cleanly. Openers must treat v1 content as unauthenticated input — on the
/// phone that means the generic banner, never rendered text.
pub const VERSION: u8 = 0x01;

/// The auth-mode envelope version: confidential AND sender-authenticated
/// (HPKE `mode_auth`, sender static = the producer's own notification key in
/// its cleared dual role).
pub const VERSION_AUTH: u8 = 0x02;

/// Fixed `info` domain separator for v2 envelopes.
///
/// v1 keeps `info` empty (deployed openers agreed to empty bytes; changing
/// theirs is a wire divergence). v2 is a new agreement, so it starts with the
/// separator the dual-role condition calls for.
pub const AUTH_INFO: &[u8] = b"cortexkit-push-seal/v2/auth";

/// Length of the truncated key identifier carried by v2 envelopes.
///
/// Identifies WHICH pinned sender key authenticated this seal, so a rotation
/// window (two valid anchors) never forces try-both verification — try-both is
/// not merely 2x cost, it is an ambiguity an attacker chooses. 8 bytes of
/// SHA-256 over the sender's public key bytes: a disambiguator between a
/// handful of operator keys, not a security boundary — the security check is
/// the AEAD verify against the full pinned key.
pub const KEY_ID_LEN: usize = 8;

/// Normative plaintext cap, measured **before** sealing.
///
/// The unit is load-bearing rather than decoration: "2048 bytes" reads as either
/// plaintext or sealed, both fit under the platform's payload limit at this
/// value, and the two readings diverge at a larger one. Plaintext is normative
/// because the composing party is the only one holding plaintext and the only
/// one that can decide what to drop; a sealed-byte cap would require it to model
/// this crate's overhead, which it would get wrong silently.
pub const MAX_PLAINTEXT_BYTES: usize = 2048;

/// Length of the encapsulated key for the pinned KEM.
const ENC_LEN: usize = 32;

/// Failure modes, kept separate because they have different diagnoses.
///
/// Collapsing them makes a field report unactionable: "it did not open" has at
/// least four causes and only one of them is a defect in the bytes.
#[derive(Debug, PartialEq, Eq)]
pub enum SealError {
    /// The plaintext exceeds [`MAX_PLAINTEXT_BYTES`]. Carries both numbers.
    ///
    /// Over-size is refused rather than truncated: the authentication tag covers
    /// the whole ciphertext, so a truncated blob does not decrypt to a fragment,
    /// it fails to decrypt entirely and renders as the generic placeholder —
    /// indistinguishable from a device that has not been unlocked.
    PlaintextTooLarge { limit: usize, observed: usize },
    /// The recipient public key is not a valid X25519 point.
    BadRecipientKey,
    /// The sender keypair is not valid X25519 material (auth mode).
    BadSenderKey,
    /// The HPKE operation itself failed.
    Hpke,
}

/// Failure modes when opening. Separate from [`SealError`] deliberately.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The envelope is shorter than a version byte plus an encapsulated key.
    Malformed { observed: usize },
    /// The version byte is not one this build understands.
    ///
    /// Distinct from [`OpenError::Aead`] on purpose. The byte exists so that a
    /// format change is loud; folding it into a generic failure would put a
    /// format change into the same bucket as a corrupt payload, which is the
    /// bucket that already has three other causes.
    UnknownVersion { observed: u8 },
    /// The recipient private key is not a valid X25519 scalar.
    BadRecipientKey,
    /// The named sender public key is not a valid X25519 point (auth mode).
    BadSenderKey,
    /// The envelope's version is real but belongs to the OTHER open call:
    /// a v1 (base) envelope handed to [`open_auth`], or a v2 (auth) envelope
    /// handed to [`open`].
    ///
    /// Distinct from [`OpenError::UnknownVersion`] deliberately: the version is
    /// understood, the CALL is wrong for it, and the two have different
    /// remedies. For the phone's mode-gated rendering this is the downgrade
    /// arm's signal — a v1 envelope where authentication was expected renders
    /// the GENERIC banner, never rich, and never a refused notification.
    ModeMismatch { observed: u8 },
    /// The v2 envelope's key-id does not match the sender key the opener was
    /// told to verify against.
    ///
    /// Split from [`OpenError::Aead`] because the remedies differ: a key-id
    /// mismatch during a rotation window means "verify against your OTHER
    /// pinned key", while an AEAD failure on a MATCHING key-id is a forgery or
    /// corruption — retrying other keys on it would be try-both verification,
    /// the ambiguity the key-id exists to remove.
    KeyIdMismatch { observed: [u8; KEY_ID_LEN] },
    /// Authentication failed: wrong key, wrong suite, wrong `info`, wrong
    /// associated data, or altered bytes. These are indistinguishable here by
    /// construction — the tag covers all of them. In auth mode this is ALSO
    /// the arm a wrong or forged SENDER lands in: `mode_auth` folds sender
    /// verification into the same tag.
    Aead,
}

/// Seals `plaintext` to `recipient_public_key`.
///
/// Returns `version || enc || ciphertext`, where the version byte is also the
/// associated data, so it is authenticated. Left cleartext and unbound it would
/// not be covered by the tag, and flipping it would silently select a different
/// parse rather than failing.
pub fn seal(recipient_public_key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, SealError> {
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(SealError::PlaintextTooLarge {
            limit: MAX_PLAINTEXT_BYTES,
            observed: plaintext.len(),
        });
    }

    let pk = <X25519HkdfSha256 as hpke::Kem>::PublicKey::from_bytes(recipient_public_key)
        .map_err(|_| SealError::BadRecipientKey)?;

    let aad = [VERSION];
    let (enc, ciphertext) =
        hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
            &OpModeS::Base,
            &pk,
            &[],
            plaintext,
            &aad,
        )
        .map_err(|_| SealError::Hpke)?;

    let enc = enc.to_bytes();
    let mut out = Vec::with_capacity(1 + enc.len() + ciphertext.len());
    out.push(VERSION);
    out.extend_from_slice(&enc);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

impl OpenError {
    /// The wire vocabulary a conformance vector reports.
    ///
    /// Lives here rather than in the corpus generator so the four-to-three
    /// collapse has ONE home. A generator that restated it would be a second
    /// independent statement of the same fact, free to drift from this one.
    ///
    /// **Three of the four variants collapse to `malformed`, and that is
    /// deliberate rather than lossy.** `Aead` already covers a wrong key, a
    /// wrong ciphersuite, a wrong `info`, a wrong associated data and altered
    /// bytes — the authentication tag cannot separate them, and an opener that
    /// could would be telling an attacker which part of the envelope was wrong.
    /// `Malformed` and `BadRecipientKey` join it because they are equally
    /// "this envelope is not usable", and splitting them on the wire would
    /// imply a distinction the opener cannot honour.
    ///
    /// The non-obvious consequence, worth stating because it looks like a bug:
    /// an envelope carrying a valid version and encapsulated key with an EMPTY
    /// ciphertext clears the length gate and fails as `Aead`, yet still reports
    /// `malformed`. The vector expecting `malformed` passes for a reason its
    /// author did not choose.
    pub fn wire_code(&self) -> &'static str {
        match self {
            OpenError::UnknownVersion { .. } => "unsupported_version",
            // Honourable distinctions: both are readable from cleartext bytes
            // BEFORE any crypto, so the opener can honour them and the
            // downgrade/rotation contracts branch on them.
            OpenError::ModeMismatch { .. } => "mode_mismatch",
            OpenError::KeyIdMismatch { .. } => "key_id_mismatch",
            OpenError::Malformed { .. }
            | OpenError::BadRecipientKey
            | OpenError::BadSenderKey
            | OpenError::Aead => "malformed",
        }
    }
}

/// Opens an envelope produced by [`seal`].
///
/// Present for tests and for generating the cross-language corpus. Production
/// opening happens in the recipient's own implementation.
///
/// **No size cap here, deliberately.** `seal` enforces one because it is the
/// only party holding plaintext; the bound on the opening side is the
/// transport's, which caps bytes before they reach this code. A second cap here
/// would duplicate a limit owned elsewhere and could disagree with it — the
/// same two-numbers-one-fact hazard the plaintext cap exists to avoid.
pub fn open(recipient_private_key: &[u8], envelope: &[u8]) -> Result<Vec<u8>, OpenError> {
    if envelope.len() < 1 + ENC_LEN {
        return Err(OpenError::Malformed {
            observed: envelope.len(),
        });
    }
    // Checked before anything else, and refused rather than skipped. A v2
    // envelope is a MODE mismatch, not an unknown version: this call opens
    // base-mode envelopes only, and silently opening an authenticated envelope
    // without verifying its sender would be a downgrade the caller never chose.
    if envelope[0] == VERSION_AUTH {
        return Err(OpenError::ModeMismatch {
            observed: VERSION_AUTH,
        });
    }
    if envelope[0] != VERSION {
        return Err(OpenError::UnknownVersion {
            observed: envelope[0],
        });
    }

    let sk = <X25519HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(recipient_private_key)
        .map_err(|_| OpenError::BadRecipientKey)?;
    let enc = <X25519HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(&envelope[1..1 + ENC_LEN])
        .map_err(|_| OpenError::Aead)?;

    let aad = [VERSION];
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeR::Base,
        &sk,
        &enc,
        &[],
        &envelope[1 + ENC_LEN..],
        &aad,
    )
    .map_err(|_| OpenError::Aead)
}

/// Truncated identifier of a sender public key, as carried by v2 envelopes.
///
/// Defined here so producer, opener, and roster tooling compute ONE derivation;
/// a second implementation of "the first 8 bytes of the hash" is a drift site
/// wearing a helper's clothes.
pub fn sender_key_id(sender_public_key: &[u8]) -> [u8; KEY_ID_LEN] {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(sender_public_key);
    let mut id = [0u8; KEY_ID_LEN];
    id.copy_from_slice(&digest[..KEY_ID_LEN]);
    id
}

/// Seals `plaintext` to `recipient_public_key`, authenticated as authored by
/// the holder of `sender_private_key` (HPKE `mode_auth`).
///
/// Returns `VERSION_AUTH || key_id || enc || ciphertext`, with
/// `VERSION_AUTH || key_id` as the associated data so both are covered by the
/// tag: a flipped version byte cannot re-parse the envelope, and a swapped
/// key-id cannot re-route verification to a different pinned key.
pub fn seal_auth(
    sender_private_key: &[u8],
    sender_public_key: &[u8],
    recipient_public_key: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, SealError> {
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(SealError::PlaintextTooLarge {
            limit: MAX_PLAINTEXT_BYTES,
            observed: plaintext.len(),
        });
    }

    let sk = <X25519HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(sender_private_key)
        .map_err(|_| SealError::BadSenderKey)?;
    let spk = <X25519HkdfSha256 as hpke::Kem>::PublicKey::from_bytes(sender_public_key)
        .map_err(|_| SealError::BadSenderKey)?;
    let pk = <X25519HkdfSha256 as hpke::Kem>::PublicKey::from_bytes(recipient_public_key)
        .map_err(|_| SealError::BadRecipientKey)?;

    let key_id = sender_key_id(sender_public_key);
    let mut aad = Vec::with_capacity(1 + KEY_ID_LEN);
    aad.push(VERSION_AUTH);
    aad.extend_from_slice(&key_id);

    let (enc, ciphertext) =
        hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
            &OpModeS::Auth((sk, spk)),
            &pk,
            AUTH_INFO,
            plaintext,
            &aad,
        )
        .map_err(|_| SealError::Hpke)?;

    let enc = enc.to_bytes();
    let mut out = Vec::with_capacity(1 + KEY_ID_LEN + enc.len() + ciphertext.len());
    out.push(VERSION_AUTH);
    out.extend_from_slice(&key_id);
    out.extend_from_slice(&enc);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Opens a v2 envelope, verifying it was authored by the holder of
/// `expected_sender_public_key`'s private half.
///
/// The caller supplies the PINNED key it trusts — never a key read from the
/// envelope. The key-id is a routing hint for WHICH pinned key to supply
/// (rotation windows hold two); the verification is the AEAD open against the
/// full key. A v1 envelope here refuses [`OpenError::ModeMismatch`] — the
/// downgrade arm — rather than falling back to unauthenticated opening: the
/// FALLBACK DECISION BELONGS TO THE RENDER LAYER, which knows that
/// unauthenticated means generic, and must see the downgrade to make it.
pub fn open_auth(
    recipient_private_key: &[u8],
    expected_sender_public_key: &[u8],
    envelope: &[u8],
) -> Result<Vec<u8>, OpenError> {
    if envelope.len() < 1 + KEY_ID_LEN + ENC_LEN {
        // Length-gate BEFORE version interpretation, mirroring `open`: a
        // 1-byte envelope reading as "mode mismatch" would claim knowledge of
        // a version field it never checked against a real layout.
        if envelope.first() == Some(&VERSION) {
            return Err(OpenError::ModeMismatch { observed: VERSION });
        }
        return Err(OpenError::Malformed {
            observed: envelope.len(),
        });
    }
    match envelope[0] {
        VERSION_AUTH => {}
        VERSION => return Err(OpenError::ModeMismatch { observed: VERSION }),
        other => return Err(OpenError::UnknownVersion { observed: other }),
    }

    let expected_id = sender_key_id(expected_sender_public_key);
    let mut observed = [0u8; KEY_ID_LEN];
    observed.copy_from_slice(&envelope[1..1 + KEY_ID_LEN]);
    if observed != expected_id {
        return Err(OpenError::KeyIdMismatch { observed });
    }

    let sk = <X25519HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(recipient_private_key)
        .map_err(|_| OpenError::BadRecipientKey)?;
    let spk = <X25519HkdfSha256 as hpke::Kem>::PublicKey::from_bytes(expected_sender_public_key)
        .map_err(|_| OpenError::BadSenderKey)?;
    let enc = <X25519HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(
        &envelope[1 + KEY_ID_LEN..1 + KEY_ID_LEN + ENC_LEN],
    )
    .map_err(|_| OpenError::Aead)?;

    let mut aad = Vec::with_capacity(1 + KEY_ID_LEN);
    aad.push(VERSION_AUTH);
    aad.extend_from_slice(&observed);
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeR::Auth(spk),
        &sk,
        &enc,
        AUTH_INFO,
        &envelope[1 + KEY_ID_LEN + ENC_LEN..],
        &aad,
    )
    .map_err(|_| OpenError::Aead)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hpke::{aead::Aead, kdf::Kdf, Kem as KemTrait};

    fn keypair() -> (Vec<u8>, Vec<u8>) {
        let (sk, pk) = X25519HkdfSha256::gen_keypair();
        (sk.to_bytes().to_vec(), pk.to_bytes().to_vec())
    }

    /// The suite is pinned by CODEPOINT, not by the type names above.
    ///
    /// Naming the types in the signatures makes a feature-flag change a compile
    /// error, which is necessary and not sufficient: a library could rename or
    /// re-point a type and still compile. The codepoints are what the opener's
    /// implementation is agreeing to, so they are what this asserts.
    #[test]
    fn the_pinned_suite_is_the_one_the_opener_agreed_to() {
        assert_eq!(X25519HkdfSha256::KEM_ID, 0x0020, "KEM codepoint");
        assert_eq!(HkdfSha256::KDF_ID, 0x0001, "KDF codepoint");
        assert_eq!(ChaCha20Poly1305::AEAD_ID, 0x0003, "AEAD codepoint");
    }

    #[test]
    fn a_sealed_payload_opens_to_the_same_plaintext() {
        let (sk, pk) = keypair();
        let sealed = seal(&pk, b"a question").expect("seal");
        assert_eq!(open(&sk, &sealed).expect("open"), b"a question");
    }

    #[test]
    fn the_envelope_is_version_then_enc_then_ciphertext() {
        let (_, pk) = keypair();
        let sealed = seal(&pk, b"x").expect("seal");
        assert_eq!(sealed[0], VERSION, "version byte leads");
        // 1 + 32 + ciphertext, and the ciphertext carries a 16-byte tag.
        assert_eq!(sealed.len(), 1 + ENC_LEN + 1 + 16);
    }

    /// Two seals of identical plaintext to one recipient must differ.
    ///
    /// This lives here rather than in the shared corpus because the corpus
    /// cannot see it: every vector opens correctly under a sealer that reuses
    /// one ephemeral key forever, since each vector is examined alone. Base-mode
    /// confidentiality rests on a fresh ephemeral per message, so without this
    /// there is no place the defect would surface.
    #[test]
    fn each_seal_uses_a_fresh_ephemeral() {
        let (_, pk) = keypair();
        let a = seal(&pk, b"same").expect("seal");
        let b = seal(&pk, b"same").expect("seal");
        assert_ne!(
            a[1..1 + ENC_LEN],
            b[1..1 + ENC_LEN],
            "encapsulated key must not repeat across messages"
        );
    }

    #[test]
    fn an_oversized_plaintext_is_refused_with_both_numbers() {
        let (_, pk) = keypair();
        let too_big = vec![0u8; MAX_PLAINTEXT_BYTES + 1];
        assert_eq!(
            seal(&pk, &too_big),
            Err(SealError::PlaintextTooLarge {
                limit: MAX_PLAINTEXT_BYTES,
                observed: MAX_PLAINTEXT_BYTES + 1
            })
        );
        // Positive control: the boundary itself succeeds, so the refusal above
        // is not satisfied by an implementation that refuses everything.
        let at_limit = vec![0u8; MAX_PLAINTEXT_BYTES];
        assert!(seal(&pk, &at_limit).is_ok(), "the cap itself must seal");
    }

    #[test]
    fn an_unknown_version_is_refused_as_a_version_rather_than_as_corruption() {
        let (sk, pk) = keypair();
        let mut sealed = seal(&pk, b"q").expect("seal");
        // 0x7f is nobody's version. 0x02 stopped being a valid specimen the day
        // it became VERSION_AUTH -- handing it to `open` now refuses as a MODE
        // mismatch, which is a different (and correct) refusal than unknown.
        sealed[0] = 0x7f;
        assert_eq!(
            open(&sk, &sealed),
            Err(OpenError::UnknownVersion { observed: 0x7f }),
            "a format change must be distinguishable from a corrupt payload"
        );
    }

    #[test]
    fn a_truncated_envelope_is_malformed_rather_than_an_aead_failure() {
        let (sk, pk) = keypair();
        let sealed = seal(&pk, b"q").expect("seal");
        let short = &sealed[..ENC_LEN];
        assert_eq!(
            open(&sk, short),
            Err(OpenError::Malformed { observed: ENC_LEN })
        );
    }

    /// The wire mapping is total and the collapse is exercised.
    ///
    /// Includes the empty-ciphertext case explicitly, because it reaches
    /// `malformed` by a route nobody would predict: 33 bytes clears the length
    /// gate, so it fails authentication rather than shape, and still reports
    /// `malformed`. Without this case a vector expecting `malformed` passes
    /// while the reasoning behind it goes unrecorded.
    #[test]
    fn every_open_failure_maps_to_the_wire_vocabulary() {
        let (sk, pk) = keypair();
        let sealed = seal(&pk, b"q").expect("seal");

        let mut wrong_version = sealed.clone();
        wrong_version[0] = 0x7f;
        assert_eq!(
            open(&sk, &wrong_version).unwrap_err().wire_code(),
            "unsupported_version"
        );

        assert_eq!(
            open(&sk, &sealed[..ENC_LEN]).unwrap_err().wire_code(),
            "malformed",
            "too short to split"
        );

        let empty_ct = &sealed[..1 + ENC_LEN];
        assert_eq!(open(&sk, empty_ct).unwrap_err(), OpenError::Aead);
        assert_eq!(open(&sk, empty_ct).unwrap_err().wire_code(), "malformed");

        let (other_sk, _) = keypair();
        assert_eq!(
            open(&other_sk, &sealed).unwrap_err().wire_code(),
            "malformed",
            "a wrong key must not be distinguishable from other failures"
        );

        // Positive control: a valid envelope produces no failure to map.
        assert!(open(&sk, &sealed).is_ok());
    }

    #[test]
    fn the_wrong_recipient_cannot_open() {
        let (_, pk) = keypair();
        let (other_sk, _) = keypair();
        let sealed = seal(&pk, b"q").expect("seal");
        assert_eq!(open(&other_sk, &sealed), Err(OpenError::Aead));
    }

    /// The associated data is load-bearing, and the no-AAD call compiles.
    ///
    /// An implementation that forgets to authenticate the version byte gets an
    /// authentication failure — which lands in the same bucket as a wrong suite
    /// and a wrong key. This proves the binding exists rather than trusting it.
    #[test]
    fn the_version_byte_is_authenticated_not_merely_present() {
        let (sk, pk) = keypair();
        let sealed = seal(&pk, b"q").expect("seal");

        let recipient = <X25519HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(&sk).unwrap();
        let enc = <X25519HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(&sealed[1..1 + ENC_LEN])
            .unwrap();

        // Opening with NO associated data must fail, which is what proves the
        // sealer bound it.
        let without_aad = hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
            &OpModeR::Base,
            &recipient,
            &enc,
            &[],
            &sealed[1 + ENC_LEN..],
            &[],
        );
        assert!(
            without_aad.is_err(),
            "the version byte must be bound as AAD"
        );

        // Positive control in the same test: with the correct AAD it opens, so
        // the failure above is about the AAD rather than about the envelope.
        assert_eq!(open(&sk, &sealed).expect("open"), b"q");
    }

    // ---- v2 (auth mode) ------------------------------------------------

    /// Dual-role keypairs, named for the production roles they model: the
    /// sender (Mac) authenticates with the same kind of key the recipient
    /// (phone) receives with.
    fn auth_parties() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let (mac_sk, mac_pk) = keypair();
        let (phone_sk, phone_pk) = keypair();
        (mac_sk, mac_pk, phone_sk, phone_pk)
    }

    /// The authenticated round trip, with layout pinned.
    #[test]
    fn an_auth_sealed_envelope_opens_against_the_pinned_sender_key() {
        let (mac_sk, mac_pk, phone_sk, phone_pk) = auth_parties();
        let sealed = seal_auth(&mac_sk, &mac_pk, &phone_pk, b"rich banner").expect("seal_auth");
        assert_eq!(sealed[0], VERSION_AUTH);
        assert_eq!(&sealed[1..1 + KEY_ID_LEN], sender_key_id(&mac_pk));
        assert_eq!(
            open_auth(&phone_sk, &mac_pk, &sealed).expect("open_auth"),
            b"rich banner"
        );
    }

    /// THE RULING-REQUIRED ARM (CKCRED, push room [#224]): dual role must not
    /// allow cross-role confusion. A blob sealed TO a key (the key in its
    /// RECIPIENT role) must not verify as authored BY that key (its SENDER
    /// role), even when an attacker re-wraps the base envelope in v2 clothing.
    ///
    /// RFC 9180's mode separation should make this unconstructable; this pins
    /// OUR binding of it rather than the spec's intent.
    #[test]
    fn a_blob_sealed_to_a_key_never_verifies_as_authored_by_it() {
        let (_mac_sk, mac_pk, phone_sk, phone_pk) = auth_parties();
        // Attacker: holds only PUBLIC keys. Seals base-mode to the phone (which
        // anyone can do) ...
        let base = seal(&phone_pk, b"forged rich content").expect("seal");
        // ... then re-frames the same enc/ciphertext as a v2 envelope claiming
        // the Mac authored it.
        let mut forged = Vec::new();
        forged.push(VERSION_AUTH);
        forged.extend_from_slice(&sender_key_id(&mac_pk));
        forged.extend_from_slice(&base[1..]); // enc || ciphertext from the base seal
        assert_eq!(
            open_auth(&phone_sk, &mac_pk, &forged),
            Err(OpenError::Aead),
            "a re-wrapped base seal must fail sender verification"
        );
        // And the same bytes must not have been openable as authored by the
        // PHONE's own key either (recipient key in claimed sender role).
        let mut self_forged = Vec::new();
        self_forged.push(VERSION_AUTH);
        self_forged.extend_from_slice(&sender_key_id(&phone_pk));
        self_forged.extend_from_slice(&base[1..]);
        assert_eq!(
            open_auth(&phone_sk, &phone_pk, &self_forged),
            Err(OpenError::Aead),
            "a key's recipient-role blob must not verify in its sender role"
        );
    }

    /// Wrong sender static: seals authored by an IMPOSTER keypair fail against
    /// the pinned key — the property the whole upgrade exists to add.
    #[test]
    fn an_imposter_sender_fails_verification_against_the_pinned_key() {
        let (_mac_sk, mac_pk, phone_sk, phone_pk) = auth_parties();
        let (imposter_sk, imposter_pk) = keypair();
        let sealed = seal_auth(&imposter_sk, &imposter_pk, &phone_pk, b"spoof").expect("seal_auth");
        // The imposter cannot even reach AEAD verification with the Mac's id:
        // their key-id differs (the routing hint refuses first) ...
        assert!(matches!(
            open_auth(&phone_sk, &mac_pk, &sealed),
            Err(OpenError::KeyIdMismatch { .. })
        ));
        // ... and an imposter FORGING the Mac's key-id still fails the tag,
        // which is the actual security boundary (the id is routing, not proof).
        let mut id_forged = sealed.clone();
        id_forged[1..1 + KEY_ID_LEN].copy_from_slice(&sender_key_id(&mac_pk));
        assert_eq!(
            open_auth(&phone_sk, &mac_pk, &id_forged),
            Err(OpenError::Aead)
        );
    }

    /// Mode confusion, both directions, by name — the downgrade arm's crate
    /// half. The render layer's generic-vs-rich branch rides on these being
    /// DISTINCT from Aead: a v1 blob where auth was expected is a DOWNGRADE
    /// (render generic), not a corruption.
    #[test]
    fn mode_confusion_refuses_by_name_in_both_directions() {
        let (mac_sk, mac_pk, phone_sk, phone_pk) = auth_parties();
        let base = seal(&phone_pk, b"plain").expect("seal");
        let auth = seal_auth(&mac_sk, &mac_pk, &phone_pk, b"rich").expect("seal_auth");
        assert_eq!(
            open_auth(&phone_sk, &mac_pk, &base),
            Err(OpenError::ModeMismatch { observed: VERSION })
        );
        assert_eq!(
            open(&phone_sk, &auth),
            Err(OpenError::ModeMismatch {
                observed: VERSION_AUTH
            })
        );
        // Wire vocabulary: the render layer branches on this string.
        assert_eq!(
            OpenError::ModeMismatch { observed: VERSION }.wire_code(),
            "mode_mismatch"
        );
    }

    /// Rotation-window routing: the key-id refusal names the OBSERVED id so the
    /// opener can pick its other pinned key, and its wire code is distinct from
    /// forgery.
    #[test]
    fn a_key_id_mismatch_names_the_observed_id_and_is_not_a_forgery_code() {
        let (mac_sk, mac_pk, phone_sk, phone_pk) = auth_parties();
        let (_, old_pk) = keypair(); // the OTHER pinned anchor in a rotation window
        let sealed = seal_auth(&mac_sk, &mac_pk, &phone_pk, b"q").expect("seal_auth");
        let err = open_auth(&phone_sk, &old_pk, &sealed).expect_err("must refuse");
        assert_eq!(
            err,
            OpenError::KeyIdMismatch {
                observed: sender_key_id(&mac_pk)
            }
        );
        assert_eq!(err.wire_code(), "key_id_mismatch");
    }

    /// The COMMITTED corpus stays executable against this crate: every case in
    /// test-vectors/push-sealed-payload.json re-runs through the real open
    /// paths with its recorded expectation. Envelopes are one-run artifacts
    /// (fresh HPKE ephemerals per seal), so this opens the committed BYTES
    /// rather than comparing regenerated ones — which is also the consumer's
    /// exact contract: the opener opens the generator's bytes.
    #[test]
    fn the_committed_corpus_executes_against_this_crate() {
        let raw = match std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test-vectors/push-sealed-payload.json"
        )) {
            Ok(raw) => raw,
            Err(err) => panic!("corpus missing — regenerate with the gen-vectors example: {err}"),
        };
        let corpus: serde_json::Value = serde_json::from_slice(&raw).expect("corpus json");
        let unhex = |s: &str| -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        };
        let phone_sk = unhex(
            corpus["keys"]["phone_recipient"]["sk_hex"]
                .as_str()
                .unwrap(),
        );
        let plaintext = corpus["plaintext_utf8"]
            .as_str()
            .unwrap()
            .as_bytes()
            .to_vec();
        let cases = corpus["cases"].as_array().expect("cases");
        assert!(cases.len() >= 9, "corpus shrank: {} cases", cases.len());
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let envelope = unhex(case["envelope_hex"].as_str().unwrap());
            let expect = case["expect"].as_str().unwrap();
            let outcome: Result<Vec<u8>, OpenError> = match case["open_as"].as_str().unwrap() {
                "auth" => {
                    let sender = unhex(case["expected_sender_pubkey_hex"].as_str().unwrap());
                    open_auth(&phone_sk, &sender, &envelope)
                }
                "base" => open(&phone_sk, &envelope),
                other => panic!("{name}: unknown open_as {other}"),
            };
            match expect {
                "opens" => assert_eq!(outcome.as_deref(), Ok(plaintext.as_slice()), "{name}"),
                code => {
                    let err = outcome.expect_err(name);
                    assert_eq!(err.wire_code(), code, "{name}: {err:?}");
                }
            }
        }
    }

    /// v2's version byte and key-id are AUTHENTICATED, not merely present: a
    /// tampered key-id fails even when the opener is tricked into expecting the
    /// tampered value's key.
    #[test]
    fn the_v2_header_is_bound_by_the_tag() {
        let (mac_sk, mac_pk, phone_sk, phone_pk) = auth_parties();
        let (_, other_pk) = keypair();
        let mut sealed = seal_auth(&mac_sk, &mac_pk, &phone_pk, b"q").expect("seal_auth");
        // Rewrite the key-id to another real key's id; open expecting THAT key:
        // key-id gate passes, sender verification and AAD both refuse.
        sealed[1..1 + KEY_ID_LEN].copy_from_slice(&sender_key_id(&other_pk));
        assert_eq!(
            open_auth(&phone_sk, &other_pk, &sealed),
            Err(OpenError::Aead)
        );
    }
}
