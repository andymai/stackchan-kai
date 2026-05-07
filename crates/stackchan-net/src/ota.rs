//! Over-the-air firmware-update image format.
//!
//! Defines the byte layout the firmware will accept on a future
//! `POST /firmware/update` route, plus a host-testable parser that
//! splits an image into its header, payload, and signature trailer.
//! The actual Ed25519 verification + partition swap ship in a
//! follow-up that brings in the crypto dep and `esp-hal-ota`.
//!
//! ## Layout
//!
//! ```text
//! +--------+---------+---------+--------------------+----------------+
//! | magic  | version | length  |     payload …      |  signature     |
//! | 4 B    | 4 B BE  | 4 B BE  |   `length` bytes   |   64 B (Ed25519) |
//! +--------+---------+---------+--------------------+----------------+
//! ```
//!
//! - `magic` — fixed `b"SCFW"` ("Stack-chan firmware"). Catches
//!   accidentally-uploaded non-OTA blobs (the `/state` JSON, the
//!   dashboard HTML, etc.) before any signature check runs.
//! - `version` — image-format version. Currently `1`. The verifier
//!   rejects unknown versions so a future format bump can land
//!   without an old firmware silently accepting it.
//! - `length` — payload size in bytes (big-endian u32). Big-endian
//!   matches the existing AW88298 / RGB565 conventions on this
//!   target.
//! - `payload` — the raw `.bin` flashed to the inactive OTA
//!   partition. Length is bounded by `MAX_OTA_PAYLOAD_BYTES`.
//! - `signature` — 64-byte Ed25519 signature of `payload`. The
//!   verifier holds the public key embedded at firmware build time;
//!   the signing key stays on the operator's host.
//!
//! ## Why a custom format
//!
//! ESP-IDF's signed-image format embeds the signature inside the
//! `app_desc` region; cross-checking that needs the bootloader's
//! verification routine, which is a heavy dependency for a Rust
//! firmware. Rolling our own at the application layer keeps the
//! verification path under our control and lets us evolve the
//! format independently of the bootloader.
//!
//! ## Test strategy
//!
//! [`parse_image`] is fully host-testable; signature verification
//! lands as a follow-up that plugs in an Ed25519 verifier (likely
//! `ed25519-compact` or `ed25519-dalek` with `default-features =
//! false`). The parsing tests pin every length / magic / version
//! corner case so the future verifier can layer on top without
//! re-validating the framing.

/// Magic prefix that identifies an `SCFW` (Stack-chan firmware) OTA
/// image. Catches accidentally-uploaded non-OTA blobs before any
/// signature check runs.
pub const OTA_MAGIC: [u8; 4] = *b"SCFW";

/// Current image-format version. Bumped whenever the layout changes;
/// the verifier rejects unknown versions so old firmwares can't
/// silently accept a newer image they wouldn't understand.
pub const OTA_FORMAT_VERSION: u32 = 1;

/// Length of an Ed25519 signature in bytes.
pub const OTA_SIGNATURE_LEN: usize = 64;

/// Length of the fixed image header: magic (4) + version (4) +
/// payload-length (4).
pub const OTA_HEADER_LEN: usize = 12;

/// Hard cap on the OTA payload size.
///
/// Sized for the realistic upper bound of a Stack-chan firmware
/// binary on CoreS3 (8 MB flash; the inactive OTA partition gets
/// ~1.6 MB after factory + bonds + RON + captures). The cap is
/// conservative against a malicious `length` field claiming a multi-
/// gigabyte payload that would otherwise OOM the receive buffer
/// before the signature check could run.
pub const MAX_OTA_PAYLOAD_BYTES: u32 = 4 * 1024 * 1024;

/// Errors returned by [`parse_image`].
///
/// Each variant maps onto a distinct HTTP error response in the
/// future firmware route — `BadMagic` and `BadVersion` are
/// `400 Bad Request`, `TooLarge` is `413 Payload Too Large`,
/// `LengthMismatch` is `400`, and `TooShort` is `400`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OtaError {
    /// Image shorter than `OTA_HEADER_LEN + OTA_SIGNATURE_LEN`.
    TooShort,
    /// First four bytes weren't `OTA_MAGIC`.
    BadMagic,
    /// `version` field didn't equal `OTA_FORMAT_VERSION`.
    BadVersion(u32),
    /// `length` field exceeded `MAX_OTA_PAYLOAD_BYTES`.
    TooLarge(u32),
    /// `length` field declared a payload that doesn't fit the
    /// remaining bytes (after subtracting the header + signature),
    /// or that overshoots the input slice.
    LengthMismatch {
        /// What the header claimed.
        declared: u32,
        /// What the slice actually allows for.
        actual: u32,
    },
}

/// Validated, but not yet signature-verified, OTA image.
///
/// `payload` borrows from the input slice (no allocation for the
/// MB-scale firmware bytes). The 64-byte `signature` is copied into
/// the struct so the caller can return / store it without keeping
/// the input alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedImage<'a> {
    /// The version field from the header. Always equal to
    /// [`OTA_FORMAT_VERSION`] for parsed images (other values are
    /// rejected at parse time).
    pub version: u32,
    /// The raw firmware bytes that get flashed to the inactive OTA
    /// partition. Length is in `0..=MAX_OTA_PAYLOAD_BYTES`.
    pub payload: &'a [u8],
    /// The 64-byte Ed25519 signature trailer. Caller verifies this
    /// against `payload` using the build-embedded public key.
    pub signature: [u8; OTA_SIGNATURE_LEN],
}

/// Parse the image framing and return the payload + signature
/// slices, ready for signature verification.
///
/// Validates magic, version, and length bounds. Does *not* verify
/// the signature — that's the firmware-side step that pulls in the
/// crypto dep.
///
/// # Errors
///
/// See [`OtaError`] variants.
pub fn parse_image(bytes: &[u8]) -> Result<ParsedImage<'_>, OtaError> {
    if bytes.len() < OTA_HEADER_LEN + OTA_SIGNATURE_LEN {
        return Err(OtaError::TooShort);
    }
    let magic = &bytes[0..4];
    if magic != OTA_MAGIC {
        return Err(OtaError::BadMagic);
    }
    // Direct indexing rather than `try_into` on a slice — the
    // `bytes.len() < OTA_HEADER_LEN + …` check above guarantees
    // these reads are in bounds, and four-byte arrays don't need a
    // fallible conversion.
    let version = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != OTA_FORMAT_VERSION {
        return Err(OtaError::BadVersion(version));
    }
    let declared = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if declared > MAX_OTA_PAYLOAD_BYTES {
        return Err(OtaError::TooLarge(declared));
    }
    // Bytes after the header but before the signature trailer.
    let body = &bytes[OTA_HEADER_LEN..];
    let body_payload_len = body
        .len()
        .checked_sub(OTA_SIGNATURE_LEN)
        .ok_or(OtaError::TooShort)?;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "body_payload_len is bounded by bytes.len() <= u32::MAX in practice; \
                  parsing rejects oversize via TooLarge before this point"
    )]
    let body_payload_u32 = body_payload_len as u32;
    if body_payload_u32 != declared {
        return Err(OtaError::LengthMismatch {
            declared,
            actual: body_payload_u32,
        });
    }
    let payload = &body[..body_payload_len];
    let sig_slice = &body[body_payload_len..];
    // 64 bytes is cheap to copy; doing so lets us return an owned
    // fixed-size array without a fallible `try_into` / `expect`.
    let mut signature = [0u8; OTA_SIGNATURE_LEN];
    signature.copy_from_slice(sig_slice);
    Ok(ParsedImage {
        version,
        payload,
        signature,
    })
}

/// Build an OTA image header in-place (caller appends payload +
/// signature). Returns the 12-byte header.
///
/// Useful for host-side tooling that wraps a `.bin` into the SCFW
/// envelope before signing. The firmware never calls this — it only
/// parses.
///
/// # Panics
///
/// Does not panic.
#[must_use]
pub fn build_header(payload_len: u32) -> [u8; OTA_HEADER_LEN] {
    let mut out = [0u8; OTA_HEADER_LEN];
    out[0..4].copy_from_slice(&OTA_MAGIC);
    out[4..8].copy_from_slice(&OTA_FORMAT_VERSION.to_be_bytes());
    out[8..12].copy_from_slice(&payload_len.to_be_bytes());
    out
}

/// Length of an Ed25519 public key in bytes.
pub const OTA_PUBLIC_KEY_LEN: usize = 32;

/// Failure modes for `verify_signature`.
///
/// `MalformedKey` is the only structurally-impossible variant in
/// production (the firmware's build-time public key is checked
/// once at the call site); it surfaces here so host tests can
/// exercise the bad-key path without a custom panic harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VerifyError {
    /// Public key bytes didn't decode as an Ed25519 public key
    /// (e.g. all-zero, or otherwise structurally invalid).
    MalformedKey,
    /// Signature didn't decode as an Ed25519 signature shape.
    MalformedSignature,
    /// Cryptographic check failed: the signature is well-formed but
    /// doesn't match `payload` under `public_key`.
    SignatureMismatch,
}

/// Verify a parsed OTA image's Ed25519 signature against
/// `public_key`. Returns `Ok(())` if the signature is valid; the
/// caller may then commit the payload to the inactive flash slot.
///
/// The public key is the 32-byte Ed25519 verify key the operator
/// embeds in the firmware at build time. The matching signing key
/// stays on the operator's host and never enters the firmware.
///
/// `pub(crate)` — production callers use [`parse_and_verify`]
/// instead; the crate-internal split keeps the test surface
/// granular without leaking a parse-then-verify-yourself shape.
///
/// # Errors
///
/// See [`VerifyError`].
pub(crate) fn verify_signature(
    image: &ParsedImage<'_>,
    public_key: &[u8; OTA_PUBLIC_KEY_LEN],
) -> Result<(), VerifyError> {
    let pk = ed25519_compact::PublicKey::from_slice(public_key)
        .map_err(|_| VerifyError::MalformedKey)?;
    let sig = ed25519_compact::Signature::from_slice(&image.signature)
        .map_err(|_| VerifyError::MalformedSignature)?;
    pk.verify(image.payload, &sig)
        .map_err(|_| VerifyError::SignatureMismatch)
}

/// Parse + verify in one shot. Convenience for HTTP handlers that
/// don't care about intermediate `ParsedImage` borrowing — most of
/// the time you just want `Ok(payload_slice)` or an error.
///
/// # Errors
///
/// [`OtaImageError`] — either a framing problem from
/// [`parse_image`] or a signature problem from the crate-internal
/// `verify_signature`.
pub fn parse_and_verify<'a>(
    bytes: &'a [u8],
    public_key: &[u8; OTA_PUBLIC_KEY_LEN],
) -> Result<&'a [u8], OtaImageError> {
    let image = parse_image(bytes)?;
    verify_signature(&image, public_key)?;
    Ok(image.payload)
}

/// Top-level OTA failure surface.
///
/// Either a framing error from [`parse_image`] or a verification
/// error from `verify_signature`. Used by [`parse_and_verify`] so
/// handlers have a single error type to map onto HTTP status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtaImageError {
    /// Image-framing problem (magic / version / length).
    Framing(OtaError),
    /// Cryptographic problem (key / signature).
    Verify(VerifyError),
}

impl From<OtaError> for OtaImageError {
    fn from(e: OtaError) -> Self {
        Self::Framing(e)
    }
}

impl From<VerifyError> for OtaImageError {
    fn from(e: VerifyError) -> Self {
        Self::Verify(e)
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::missing_docs_in_private_items,
    reason = "test-only: fixtures we just built are well-formed by construction"
)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// Build a complete SCFW image: header + payload + signature.
    fn build_image(payload: &[u8], signature: [u8; 64]) -> Vec<u8> {
        let mut out = Vec::new();
        #[allow(clippy::cast_possible_truncation)]
        let len = payload.len() as u32;
        out.extend_from_slice(&build_header(len));
        out.extend_from_slice(payload);
        out.extend_from_slice(&signature);
        out
    }

    #[test]
    fn parse_round_trips_header_payload_signature() {
        let payload = b"hello firmware";
        let signature = [0xABu8; 64];
        let image = build_image(payload, signature);
        let parsed = parse_image(&image).expect("valid image should parse");
        assert_eq!(parsed.version, OTA_FORMAT_VERSION);
        assert_eq!(parsed.payload, payload);
        assert_eq!(parsed.signature, signature);
    }

    #[test]
    fn rejects_too_short_image() {
        // Anything below header + signature can't carry a payload.
        let too_short = [0u8; OTA_HEADER_LEN + OTA_SIGNATURE_LEN - 1];
        assert_eq!(parse_image(&too_short), Err(OtaError::TooShort));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut image = build_image(b"x", [0u8; 64]);
        image[0] = b'X';
        assert_eq!(parse_image(&image), Err(OtaError::BadMagic));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut image = build_image(b"x", [0u8; 64]);
        // Version field at bytes 4..8.
        image[4..8].copy_from_slice(&999u32.to_be_bytes());
        assert_eq!(parse_image(&image), Err(OtaError::BadVersion(999)));
    }

    #[test]
    fn rejects_oversize_payload() {
        // Build a header that *claims* an over-cap payload. We don't
        // actually allocate the bytes — the parser should reject on
        // the length field alone.
        let mut header = build_header(MAX_OTA_PAYLOAD_BYTES + 1);
        let _ = &mut header;
        let mut image = Vec::new();
        image.extend_from_slice(&header);
        // Pad out to header + signature so we trip the length check
        // (rather than the too-short check) first.
        image.resize(OTA_HEADER_LEN + OTA_SIGNATURE_LEN, 0);
        assert_eq!(
            parse_image(&image),
            Err(OtaError::TooLarge(MAX_OTA_PAYLOAD_BYTES + 1))
        );
    }

    #[test]
    fn rejects_length_mismatch() {
        // Header claims 100 bytes of payload but we only ship 5.
        let mut image = Vec::new();
        image.extend_from_slice(&build_header(100));
        image.extend_from_slice(b"short"); // 5-byte "payload"
        image.extend_from_slice(&[0u8; 64]); // signature
        let err = parse_image(&image).expect_err("should reject mismatch");
        assert!(matches!(err, OtaError::LengthMismatch { .. }));
        if let OtaError::LengthMismatch { declared, actual } = err {
            assert_eq!(declared, 100);
            assert_eq!(actual, 5);
        }
    }

    #[test]
    fn empty_payload_is_valid() {
        // A zero-length payload is a degenerate but well-formed image.
        // The signature still has to verify (downstream); the parser
        // accepts it.
        let image = build_image(&[], [0u8; 64]);
        let parsed = parse_image(&image).expect("zero-length payload OK");
        assert_eq!(parsed.payload.len(), 0);
    }

    #[test]
    fn build_header_round_trips() {
        let h = build_header(42);
        assert_eq!(&h[0..4], &OTA_MAGIC);
        assert_eq!(
            u32::from_be_bytes(h[4..8].try_into().expect("4 bytes")),
            OTA_FORMAT_VERSION
        );
        assert_eq!(
            u32::from_be_bytes(h[8..12].try_into().expect("4 bytes")),
            42
        );
    }

    /// Build a fresh keypair from an in-test seed. ed25519-compact
    /// derives both keys deterministically, so this gives reproducible
    /// public-key bytes across machines and CI runs without
    /// shipping pre-computed bytes that drift if the dep changes.
    fn fixed_test_keypair() -> ed25519_compact::KeyPair {
        let seed = ed25519_compact::Seed::from_slice(&[7u8; 32]).expect("seed length is 32");
        ed25519_compact::KeyPair::from_seed(seed)
    }

    /// Wrap a payload into a signed SCFW image using the test keypair.
    fn sign_image(payload: &[u8]) -> (Vec<u8>, [u8; OTA_PUBLIC_KEY_LEN]) {
        let kp = fixed_test_keypair();
        let sig = kp.sk.sign(payload, None);
        let mut sig_bytes = [0u8; OTA_SIGNATURE_LEN];
        sig_bytes.copy_from_slice(sig.as_ref());
        let image = build_image(payload, sig_bytes);
        let mut pk_bytes = [0u8; OTA_PUBLIC_KEY_LEN];
        pk_bytes.copy_from_slice(kp.pk.as_ref());
        (image, pk_bytes)
    }

    #[test]
    fn verify_accepts_signature_from_matching_key() {
        let (image, pk) = sign_image(b"firmware bytes here");
        let payload = parse_and_verify(&image, &pk).expect("valid image + key");
        assert_eq!(payload, b"firmware bytes here");
    }

    #[test]
    fn verify_rejects_signature_under_wrong_key() {
        let (image, _correct_pk) = sign_image(b"firmware");
        let other_kp = ed25519_compact::KeyPair::from_seed(
            ed25519_compact::Seed::from_slice(&[42u8; 32]).expect("seed length is 32"),
        );
        let mut wrong_pk = [0u8; OTA_PUBLIC_KEY_LEN];
        wrong_pk.copy_from_slice(other_kp.pk.as_ref());
        let err = parse_and_verify(&image, &wrong_pk).expect_err("wrong key");
        assert_eq!(err, OtaImageError::Verify(VerifyError::SignatureMismatch));
    }

    #[test]
    fn verify_rejects_tampered_payload() {
        let (mut image, pk) = sign_image(b"firmware bytes here");
        // Flip a byte inside the payload (between header at 0..12
        // and signature at len-64..).
        let payload_offset = OTA_HEADER_LEN + 4;
        image[payload_offset] ^= 0xFF;
        let err = parse_and_verify(&image, &pk).expect_err("tampered payload");
        assert_eq!(err, OtaImageError::Verify(VerifyError::SignatureMismatch));
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let (mut image, pk) = sign_image(b"firmware");
        let len = image.len();
        // Flip a byte in the signature trailer (last 64 bytes).
        image[len - 1] ^= 0xFF;
        let err = parse_and_verify(&image, &pk).expect_err("tampered signature");
        // ed25519-compact may surface either a malformed-signature
        // shape error or a signature-mismatch depending on which bit
        // flipped — both are acceptable rejection paths.
        assert!(matches!(
            err,
            OtaImageError::Verify(VerifyError::SignatureMismatch | VerifyError::MalformedSignature)
        ));
    }

    #[test]
    fn verify_rejects_all_zero_public_key() {
        let (image, _pk) = sign_image(b"firmware");
        let zero_pk = [0u8; OTA_PUBLIC_KEY_LEN];
        let err = parse_and_verify(&image, &zero_pk).expect_err("zero key");
        // ed25519-compact accepts the all-zero point as structurally
        // valid but the signature won't verify under it. Either
        // rejection path is acceptable.
        assert!(matches!(
            err,
            OtaImageError::Verify(VerifyError::MalformedKey | VerifyError::SignatureMismatch)
        ));
    }
}
