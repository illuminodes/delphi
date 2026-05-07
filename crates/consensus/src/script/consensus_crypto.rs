//! The thin wrapper around `k256` that the rest of the script interpreter
//! calls. Per `principles.md` §3 and `validation.md` §"Cryptographic concerns",
//! this module is the *only* place in Mantle that directly imports `k256`.
//!
//! Its job is to enforce the consensus-level encoding rules that `k256`'s
//! generic API does not enforce on its own:
//!
//! - **Strict-DER signature encoding** (BIP66, `SCRIPT_VERIFY_DERSIG`). The
//!   serialised ECDSA signature must be a tightly canonical DER `SEQUENCE`
//!   of two unsigned integers. We hand-roll the validator here rather than
//!   trust `der`-style parsers — the consensus rule is *behavioural parity
//!   with Core*, not "any valid DER". Re-derived from BIP66 prose; the
//!   differential fuzzer is the test.
//!
//! - **Low-S enforcement** (`SCRIPT_VERIFY_LOW_S`). `k256` does not do this
//!   on its own — we reject `s > n/2` here, gated on the flag.
//!
//! - **Pubkey encoding tolerance** (`SCRIPT_VERIFY_STRICTENC`). 33-byte
//!   compressed (`0x02`/`0x03` lead) and 65-byte uncompressed (`0x04` lead)
//!   are always accepted. Hybrid `0x06`/`0x07`-lead 65-byte encodings are
//!   accepted *without* `STRICTENC` and rejected *with* it.
//!
//! Any signature/pubkey that fails these checks returns `false` from
//! `verify_ecdsa`. The interpreter doesn't distinguish *why* — it only acts
//! on the boolean.

use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::ecdsa::{Signature as EcdsaSig, VerifyingKey};

use super::error::ScriptError;
use super::flags::ScriptFlags;

/// Half the curve order, in big-endian. `s` values strictly above this are
/// rejected under `SCRIPT_VERIFY_LOW_S`. Source: BIP146 (the constant is the
/// secp256k1 group order divided by two, rounded down).
const HALF_CURVE_ORDER_BE: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d,
    0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa0,
];

/// Validate the wire encoding of a signature *under the active flags*. This
/// is what BIP66 / BIP146 / STRICTENC police: shape, not curve correctness.
///
/// `sig` is the full `<DER>||<sighash_byte>` blob as it appeared on the
/// stack. An empty `sig` short-circuits `Ok(())` — the caller (CHECKSIG)
/// treats empty signatures as a curve-check failure with no encoding check
/// (this is what enables NULLFAIL's "must be empty if you wanted to fail").
///
/// # Errors
/// - `SigDer` if DERSIG/STRICTENC is set and the DER part fails strict-DER.
/// - `SigHashtype` if STRICTENC is set and the hashtype byte is unknown.
/// - `SigHighS` if `LOW_S` is set and `s > N/2`.
pub fn check_signature_encoding(sig: &[u8], flags: ScriptFlags) -> Result<(), ScriptError> {
    if sig.is_empty() {
        return Ok(());
    }

    // Split the trailing sighash byte. With non-empty sig, we always have one.
    let (Some(&hashtype), Some(sig_der)) = (sig.last(), sig.split_last().map(|(_, body)| body))
    else {
        return Ok(());
    };

    let strict =
        flags.contains(ScriptFlags::DERSIG) || flags.contains(ScriptFlags::STRICTENC);
    if strict && !is_valid_signature_encoding_der(sig_der) {
        return Err(ScriptError::SigDer);
    }
    if flags.contains(ScriptFlags::LOW_S) && !is_low_s(sig_der) {
        return Err(ScriptError::SigHighS);
    }
    if flags.contains(ScriptFlags::STRICTENC) && !is_defined_hashtype(hashtype) {
        return Err(ScriptError::SigHashtype);
    }

    Ok(())
}

/// Validate the wire encoding of a pubkey under STRICTENC.
///
/// Without STRICTENC, every byte string parses (`k256` is permissive within
/// reason). With it, only 33-byte compressed (`0x02`/`0x03`) and 65-byte
/// uncompressed (`0x04`) leads are accepted.
///
/// # Errors
/// `PubKeyType` if STRICTENC is set and the pubkey doesn't match either form.
pub fn check_pubkey_encoding(pubkey: &[u8], flags: ScriptFlags) -> Result<(), ScriptError> {
    if flags.contains(ScriptFlags::STRICTENC) && !is_compressed_or_uncompressed(pubkey) {
        return Err(ScriptError::PubKeyType);
    }
    Ok(())
}

/// `true` iff `byte` is a recognised hashtype value under STRICTENC.
/// The set is `SIGHASH_ALL | NONE | SINGLE` optionally OR'd with `ANYONECANPAY`.
#[must_use]
fn is_defined_hashtype(byte: u8) -> bool {
    let masked = byte & !0x80; // strip ANYONECANPAY bit
    matches!(masked, 1..=3)
}

/// Verify an ECDSA signature against a 32-byte sighash digest. *Curve-check
/// only* — encoding rules are checked separately by
/// [`check_signature_encoding`] and [`check_pubkey_encoding`], which the
/// opcode handler must call before this one. An empty signature here always
/// returns `false` (no DER body to parse).
#[must_use]
pub fn verify_ecdsa(sig_der: &[u8], pubkey_bytes: &[u8], sighash: &[u8; 32]) -> bool {
    let Ok(sig) = EcdsaSig::from_der(sig_der) else {
        return false;
    };
    let Ok(vk) = VerifyingKey::from_sec1_bytes(pubkey_bytes) else {
        return false;
    };
    vk.verify_prehash(sighash, &sig).is_ok()
}

/// Hand-rolled BIP66 strict-DER validator.
///
/// Behavioural mirror of Core's `IsValidSignatureEncoding` — the consensus
/// rule is "what Core accepts", not "what is valid DER". Re-derived from BIP66
/// prose. Layout:
///
/// ```text
/// 0x30  total_len
/// 0x02  r_len  r_bytes...
/// 0x02  s_len  s_bytes...
/// ```
///
/// Length range is `8..=72` total bytes (excluding sighash byte). `r` and `s`
/// are positive integers: leading zero only when next byte's high bit is set;
/// no extra leading zeros otherwise; non-zero magnitude.
#[must_use]
pub fn is_valid_signature_encoding_der(sig: &[u8]) -> bool {
    // Minimum: 0x30 len(1) 0x02 r_len(1) r(1) 0x02 s_len(1) s(1) = 8 bytes.
    // Maximum: 72 (signature alone, no sighash byte).
    if sig.len() < 8 || sig.len() > 72 {
        return false;
    }
    // Outer SEQUENCE tag.
    if sig[0] != 0x30 {
        return false;
    }
    // Total length excludes the leading two header bytes (tag + length).
    if usize::from(sig[1]) != sig.len() - 2 {
        return false;
    }

    // r: tag 0x02, then length, then magnitude.
    if sig[2] != 0x02 {
        return false;
    }
    let r_len = usize::from(sig[3]);
    // r occupies sig[4..4+r_len]. Need at least r_len + 2 (for s tag + s_len)
    // + 1 (for at least one s byte) bytes after sig[3].
    if r_len == 0 || 4 + r_len + 3 > sig.len() {
        return false;
    }
    let r = &sig[4..4 + r_len];

    // s: tag 0x02, then length, then magnitude.
    if sig[4 + r_len] != 0x02 {
        return false;
    }
    let s_len = usize::from(sig[5 + r_len]);
    if s_len == 0 || 6 + r_len + s_len != sig.len() {
        return false;
    }
    let s = &sig[6 + r_len..6 + r_len + s_len];

    // r and s rules: positive, minimal-encoded.
    is_valid_der_integer(r) && is_valid_der_integer(s)
}

/// Validate a single DER `INTEGER` body under BIP66.
///
/// - Top bit of first byte must be 0 (i.e. positive).
/// - No unnecessary leading 0x00 byte: only allowed when the next byte's
///   high bit is set (otherwise the leading zero is redundant).
fn is_valid_der_integer(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes[0] & 0x80 != 0 {
        return false;
    }
    if bytes.len() > 1 && bytes[0] == 0x00 && (bytes[1] & 0x80) == 0 {
        return false;
    }
    true
}

/// `s` value of the signature is below or equal to half the curve order.
///
/// Returns `false` if the signature is not parseable as a strict-DER
/// signature (already strict-checked at this point in normal flow). This
/// helper does the BIP66-style walk again to extract `s` without going
/// through `k256`'s parser — that lets us reject high-S without first
/// successfully constructing an `EcdsaSig` (which would normalise it).
#[must_use]
fn is_low_s(sig_der: &[u8]) -> bool {
    // Extract s. Mirrors the validator above but only as far as needed.
    if sig_der.len() < 8 || sig_der[0] != 0x30 {
        return false;
    }
    if sig_der[2] != 0x02 {
        return false;
    }
    let r_len = usize::from(sig_der[3]);
    if 4 + r_len + 2 > sig_der.len() {
        return false;
    }
    if sig_der[4 + r_len] != 0x02 {
        return false;
    }
    let s_len = usize::from(sig_der[5 + r_len]);
    let s_start = 6 + r_len;
    if s_start + s_len > sig_der.len() {
        return false;
    }
    let s = &sig_der[s_start..s_start + s_len];

    // Strip the leading 0x00 sign-padding, if any. The BIP66 validator
    // forbids a redundant 0x00, but a 0x00 *is* required when the magnitude's
    // high bit is set, and we want to compare magnitude to N/2.
    let s_mag: &[u8] = if !s.is_empty() && s[0] == 0x00 { &s[1..] } else { s };

    // Right-align into a 32-byte buffer for big-endian comparison. Anything
    // longer than 32 bytes can't be ≤ N/2 (which is 256 bits).
    if s_mag.len() > 32 {
        return false;
    }
    let mut s_be = [0u8; 32];
    s_be[32 - s_mag.len()..].copy_from_slice(s_mag);
    s_be <= HALF_CURVE_ORDER_BE
}

/// Pubkey is one of the two strict encodings: 33-byte compressed (`0x02`
/// or `0x03` lead) or 65-byte uncompressed (`0x04` lead). Hybrid 65-byte
/// encodings (`0x06`/`0x07` lead) are excluded under `STRICTENC`.
#[must_use]
fn is_compressed_or_uncompressed(pubkey: &[u8]) -> bool {
    match pubkey.first().copied() {
        Some(0x02 | 0x03) => pubkey.len() == 33,
        Some(0x04) => pubkey.len() == 65,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Strict-DER ──────────────────────────────────────────────────────────

    #[test]
    fn der_minimum_valid() {
        // SEQUENCE { INTEGER 1, INTEGER 1 }
        let sig = [0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01];
        assert!(is_valid_signature_encoding_der(&sig));
    }

    #[test]
    fn der_too_short() {
        assert!(!is_valid_signature_encoding_der(&[0x30, 0x05]));
    }

    #[test]
    fn der_too_long() {
        let sig = [0u8; 73];
        assert!(!is_valid_signature_encoding_der(&sig));
    }

    #[test]
    fn der_wrong_outer_tag() {
        let sig = [0x31, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01];
        assert!(!is_valid_signature_encoding_der(&sig));
    }

    #[test]
    fn der_wrong_total_length() {
        // total length byte claims 5 but actual body is 6
        let sig = [0x30, 0x05, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01];
        assert!(!is_valid_signature_encoding_der(&sig));
    }

    #[test]
    fn der_r_with_leading_zero_not_needed() {
        // r = 00 01 — leading zero is redundant since 0x01's high bit is clear.
        let sig = [0x30, 0x07, 0x02, 0x02, 0x00, 0x01, 0x02, 0x01, 0x01];
        assert!(!is_valid_signature_encoding_der(&sig));
    }

    #[test]
    fn der_r_with_required_leading_zero() {
        // r = 00 80 — high bit set, leading zero required to keep r positive.
        let sig = [0x30, 0x07, 0x02, 0x02, 0x00, 0x80, 0x02, 0x01, 0x01];
        assert!(is_valid_signature_encoding_der(&sig));
    }

    #[test]
    fn der_negative_r() {
        // r high bit set without leading zero — would parse as negative.
        let sig = [0x30, 0x06, 0x02, 0x01, 0x80, 0x02, 0x01, 0x01];
        assert!(!is_valid_signature_encoding_der(&sig));
    }

    #[test]
    fn der_zero_r_length() {
        let sig = [0x30, 0x05, 0x02, 0x00, 0x02, 0x01, 0x01];
        assert!(!is_valid_signature_encoding_der(&sig));
    }

    // ── Low-S ───────────────────────────────────────────────────────────────

    #[test]
    fn low_s_accepts_small_s() {
        // r = 1, s = 1 — well under N/2.
        let sig = [0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01];
        assert!(is_low_s(&sig));
    }

    #[test]
    fn low_s_rejects_above_half_order() {
        // s = N/2 + 1 (high bit set, requires leading zero pad).
        let mut s_be = HALF_CURVE_ORDER_BE;
        s_be[31] += 1;
        let mut sig = vec![0x30, 0x00, 0x02, 0x01, 0x01, 0x02, 33, 0x00];
        sig.extend_from_slice(&s_be);
        sig[1] = u8::try_from(sig.len() - 2).expect("len fits");
        assert!(!is_low_s(&sig));
    }

    #[test]
    fn low_s_accepts_exactly_half_order() {
        // Exactly N/2 — high bit set, requires leading zero pad.
        let mut sig = vec![0x30, 0x00, 0x02, 0x01, 0x01, 0x02, 33, 0x00];
        sig.extend_from_slice(&HALF_CURVE_ORDER_BE);
        sig[1] = u8::try_from(sig.len() - 2).expect("len fits");
        assert!(is_low_s(&sig));
    }

    // ── Pubkey encoding ─────────────────────────────────────────────────────

    #[test]
    fn pubkey_compressed_accepted() {
        let mut pk = [0u8; 33];
        pk[0] = 0x02;
        assert!(is_compressed_or_uncompressed(&pk));
        pk[0] = 0x03;
        assert!(is_compressed_or_uncompressed(&pk));
    }

    #[test]
    fn pubkey_uncompressed_accepted() {
        let mut pk = [0u8; 65];
        pk[0] = 0x04;
        assert!(is_compressed_or_uncompressed(&pk));
    }

    #[test]
    fn pubkey_hybrid_rejected_under_strictenc() {
        let mut pk = [0u8; 65];
        pk[0] = 0x06;
        assert!(!is_compressed_or_uncompressed(&pk));
        pk[0] = 0x07;
        assert!(!is_compressed_or_uncompressed(&pk));
    }

    #[test]
    fn pubkey_wrong_length_rejected() {
        let pk = [0x02u8; 32]; // compressed lead, wrong length
        assert!(!is_compressed_or_uncompressed(&pk));
        let pk = [0x04u8; 64]; // uncompressed lead, wrong length
        assert!(!is_compressed_or_uncompressed(&pk));
    }
}
