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
//! `info` is empty **because the recipient key is dedicated to this purpose**.
//! `info` is the key schedule's domain separator: it earns its keep when one key
//! serves several applications. If this key is ever shared with another protocol
//! — for instance to add sender authentication by reusing a transport static —
//! empty stops being safe and a fixed non-empty domain string becomes the
//! mitigation. The condition is written down rather than the conclusion, because
//! the reader who reuses the key is exactly the reader who cannot see why it
//! mattered.

use hpke::{
    aead::ChaCha20Poly1305, kdf::HkdfSha256, kem::X25519HkdfSha256, Deserializable, OpModeR,
    OpModeS, Serializable,
};

/// The one envelope version this crate emits and accepts.
pub const VERSION: u8 = 0x01;

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
    /// Authentication failed: wrong key, wrong suite, wrong `info`, wrong
    /// associated data, or altered bytes. These are indistinguishable here by
    /// construction — the tag covers all of them.
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
            OpenError::Malformed { .. } | OpenError::BadRecipientKey | OpenError::Aead => {
                "malformed"
            }
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
    // Checked before anything else, and refused rather than skipped.
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
        sealed[0] = 0x02;
        assert_eq!(
            open(&sk, &sealed),
            Err(OpenError::UnknownVersion { observed: 0x02 }),
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
}
