//! Legacy (pre-segwit) signature hash computation.
//!
//! `signature_hash` produces the 32-byte digest that an ECDSA signature in
//! the legacy regime is computed over. The algorithm is:
//!
//! 1. Build a *modified* transaction:
//!    - Replace every input's `script_sig` with the empty script.
//!    - Replace input `n_in`'s `script_sig` with `script_code` (after
//!      `FindAndDelete` of the signature being checked, applied by the
//!      caller — see `find_and_delete_sig`).
//!    - Apply the `hash_type` modifications:
//!      - `SIGHASH_NONE`: clear all outputs and zero every other input's
//!        sequence.
//!      - `SIGHASH_SINGLE`: keep only output `n_in`; if `n_in >= n_outputs`,
//!        return the constant digest `0x0000…0001` (the **"1" bug**) and skip
//!        the rest of the algorithm.
//!      - `SIGHASH_ANYONECANPAY`: keep only input `n_in`.
//! 2. Serialise the modified tx in legacy form, append the 4-byte
//!    little-endian `hash_type`, and SHA-256d.
//!
//! All four base modes plus `ANYONECANPAY` are reproduced exactly, including
//! the `SIGHASH_SINGLE` "1" bug — fixing it would be a hard fork.

use mantle::codec::Encode;
use mantle::hash::sha256d;
use mantle::primitives::{Hash256, Transaction, TxIn, TxOut};

/// `hash_type` mode (low 5 bits, masking off `ANYONECANPAY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigHashMode {
    /// Default — sign every input and every output.
    All,
    /// Sign no outputs (anyone can change them).
    None,
    /// Sign only the output at the same index as the input being signed.
    Single,
}

/// The signature hash type byte (the trailing byte of a DER+sighash sig).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigHashType(pub u32);

impl SigHashType {
    /// `SIGHASH_ALL`.
    pub const ALL: Self = Self(0x01);
    /// `SIGHASH_NONE`.
    pub const NONE: Self = Self(0x02);
    /// `SIGHASH_SINGLE`.
    pub const SINGLE: Self = Self(0x03);
    /// `SIGHASH_ANYONECANPAY` flag bit, OR-able with the mode.
    pub const ANYONECANPAY: u32 = 0x80;

    /// Decode the mode from the low 5 bits. `ALL`, `NONE`, `SINGLE` are
    /// recognised; everything else maps to `ALL` (consensus behaviour —
    /// Core treats unknown modes as ALL in the legacy preimage; STRICTENC
    /// rejects them at the encoding layer separately).
    #[must_use]
    pub fn mode(self) -> SigHashMode {
        match self.0 & 0x1f {
            2 => SigHashMode::None,
            3 => SigHashMode::Single,
            _ => SigHashMode::All,
        }
    }

    /// `true` if the `ANYONECANPAY` bit is set.
    #[must_use]
    pub fn is_anyonecanpay(self) -> bool {
        self.0 & Self::ANYONECANPAY != 0
    }
}

/// Compute the legacy signature hash.
///
/// `script_code` is the script being signed — the caller has already applied
/// `OP_CODESEPARATOR` slicing and `FindAndDelete` of the signature.
///
/// Returns the canonical "1" digest if `SIGHASH_SINGLE` is used with `n_in`
/// past the last output.
///
/// # Panics
/// Panics if `n_in >= tx.inputs.len()`. Calling code (the interpreter) only
/// reaches this with a valid input index.
#[must_use]
pub fn signature_hash(
    tx: &Transaction,
    n_in: usize,
    script_code: &[u8],
    hash_type: SigHashType,
) -> Hash256 {
    let mode = hash_type.mode();

    // The SIGHASH_SINGLE "1" bug: if `n_in` is at or past the last output,
    // the digest is the canonical 1, not a hash of anything.
    if mode == SigHashMode::Single && n_in >= tx.outputs.len() {
        let mut h = [0u8; 32];
        h[0] = 0x01;
        return Hash256(h);
    }

    let modified = build_modified_tx(tx, n_in, script_code, hash_type);

    // Serialise + append the 4-byte little-endian hash_type.
    let mut buf = Vec::with_capacity(modified.encoded_len() + 4);
    modified.encode(&mut buf).expect("encoding into Vec cannot fail");
    buf.extend_from_slice(&hash_type.0.to_le_bytes());

    Hash256(sha256d(&buf))
}

/// Build the modified `Transaction` that the legacy preimage hashes.
///
/// Allocates — this is the legacy path; BIP143 (M2) caches and reuses.
fn build_modified_tx(
    tx: &Transaction,
    n_in: usize,
    script_code: &[u8],
    hash_type: SigHashType,
) -> Transaction {
    let mode = hash_type.mode();
    let anyonecanpay = hash_type.is_anyonecanpay();

    // ── Inputs ──────────────────────────────────────────────────────────────
    let inputs: Vec<TxIn> = if anyonecanpay {
        // Keep only the input being signed.
        vec![TxIn {
            previous_output: tx.inputs[n_in].previous_output,
            script_sig: script_code.to_vec().into(),
            sequence: tx.inputs[n_in].sequence,
        }]
    } else {
        tx.inputs
            .iter()
            .enumerate()
            .map(|(i, txin)| {
                let script_sig = if i == n_in {
                    script_code.to_vec().into()
                } else {
                    Vec::<u8>::new().into()
                };
                // Under SIGHASH_NONE / SIGHASH_SINGLE, every *other* input's
                // sequence is zeroed so those inputs cannot block re-signing.
                let sequence = match mode {
                    SigHashMode::None | SigHashMode::Single if i != n_in => 0,
                    _ => txin.sequence,
                };
                TxIn {
                    previous_output: txin.previous_output,
                    script_sig,
                    sequence,
                }
            })
            .collect()
    };

    // ── Outputs ─────────────────────────────────────────────────────────────
    let outputs: Vec<TxOut> = match mode {
        SigHashMode::All => tx.outputs.clone(),
        SigHashMode::None => Vec::new(),
        SigHashMode::Single => {
            // Keep outputs up to and including `n_in`; everything before
            // `n_in` is replaced with the "null" output (-1 value, empty
            // scriptPubKey). The `n_in >= n_outputs` case has been
            // short-circuited by the "1" bug above.
            let mut out = Vec::with_capacity(n_in + 1);
            for _ in 0..n_in {
                out.push(TxOut {
                    value: u64::MAX.into(), // -1 as i64 / u64
                    script_pubkey: Vec::<u8>::new().into(),
                });
            }
            out.push(tx.outputs[n_in].clone());
            out
        }
    };

    Transaction {
        version: tx.version,
        inputs,
        outputs,
        locktime: tx.locktime,
    }
}

/// Apply BIP-style `FindAndDelete` of `sig` from `script`.
///
/// The legacy sighash subroutine removes every push of the signature being
/// checked from the `scriptCode` before hashing. The match is a *pushdata*
/// match: a push opcode whose payload exactly equals `sig`, found at a
/// script-program boundary. Bytes that happen to contain `sig`'s pattern
/// inside another push body are not removed.
///
/// This matches Core's `FindAndDelete`: walk the script, copy every push or
/// opcode unchanged unless the entire pushed payload byte-for-byte matches
/// `sig`. Non-push opcodes are copied as-is.
///
/// Returns the script with all matching pushes removed.
#[must_use]
pub fn find_and_delete_sig(script: &[u8], sig: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(script.len());
    let mut i = 0;
    while i < script.len() {
        let op = script[i];
        // Determine the (data_start, data_end) span pushed by this opcode, if any.
        let span = pushed_span(script, i);
        if let Some((data_start, data_end, op_end)) = span {
            if &script[data_start..data_end] == sig {
                // Skip this push entirely.
                i = op_end;
                continue;
            }
            // Copy the whole opcode + payload.
            out.extend_from_slice(&script[i..op_end]);
            i = op_end;
        } else {
            // Non-push opcode: copy one byte.
            out.push(op);
            i += 1;
        }
    }
    out
}

/// If `script[i]` is a push opcode, return `(data_start, data_end, op_end)`
/// — the absolute byte indices of the pushed payload and one past the
/// last consumed byte. Returns `None` for non-push opcodes or truncated
/// pushes (truncated pushes are passed through as a single byte in
/// `find_and_delete_sig`, mirroring Core's tolerant parser there).
fn pushed_span(script: &[u8], i: usize) -> Option<(usize, usize, usize)> {
    let op = script[i];
    // OP_0 (0x00) falls through to the wildcard `None` — `FindAndDelete` only
    // matches non-empty data pushes, so OP_0 is treated as a regular opcode.
    match op {
        // 0x01..=0x4b: direct push of N bytes.
        n @ 0x01..=0x4b => {
            let len = usize::from(n);
            let data_start = i + 1;
            let data_end = data_start + len;
            if data_end > script.len() {
                None
            } else {
                Some((data_start, data_end, data_end))
            }
        }
        // OP_PUSHDATA1
        0x4c => {
            if i + 1 >= script.len() {
                return None;
            }
            let len = usize::from(script[i + 1]);
            let data_start = i + 2;
            let data_end = data_start + len;
            if data_end > script.len() {
                None
            } else {
                Some((data_start, data_end, data_end))
            }
        }
        // OP_PUSHDATA2
        0x4d => {
            if i + 2 >= script.len() {
                return None;
            }
            let len = usize::from(u16::from_le_bytes([script[i + 1], script[i + 2]]));
            let data_start = i + 3;
            let data_end = data_start + len;
            if data_end > script.len() {
                None
            } else {
                Some((data_start, data_end, data_end))
            }
        }
        // OP_PUSHDATA4
        0x4e => {
            if i + 4 >= script.len() {
                return None;
            }
            let len = u32::from_le_bytes([
                script[i + 1],
                script[i + 2],
                script[i + 3],
                script[i + 4],
            ]) as usize;
            let data_start = i + 5;
            let data_end = data_start + len;
            if data_end > script.len() {
                None
            } else {
                Some((data_start, data_end, data_end))
            }
        }
        _ => None,
    }
}

/// Slice a script at the byte offset of the last `OP_CODESEPARATOR`.
///
/// `code_separator_pos` is the byte index of the *next* byte after the
/// `OP_CODESEPARATOR` (or 0 if none has executed yet — the whole script is
/// the scriptCode). Returns the suffix of `script` from that position.
#[must_use]
pub fn script_code_after_codeseparator(script: &[u8], code_separator_pos: usize) -> &[u8] {
    if code_separator_pos > script.len() {
        return &[];
    }
    &script[code_separator_pos..]
}

// ── codec helpers ──────────────────────────────────────────────────────────
// `encoded_len` is on `Transaction` already; we route through it to avoid
// a second size pass. `Vec<u8>` conversion of script_sig/script_pubkey is
// needed because `ScriptSig` / `ScriptPubKey` round-trip via `Vec<u8>`.

#[cfg(test)]
mod tests {
    use super::*;
    use mantle::primitives::{OutPoint, Satoshi};

    fn make_tx() -> Transaction {
        Transaction {
            version: 1,
            inputs: vec![
                TxIn {
                    previous_output: OutPoint {
                        txid: Hash256([0x11; 32]),
                        vout: 0,
                    },
                    script_sig: vec![0xaa].into(),
                    sequence: 0xffff_fffe,
                },
                TxIn {
                    previous_output: OutPoint {
                        txid: Hash256([0x22; 32]),
                        vout: 1,
                    },
                    script_sig: vec![0xbb].into(),
                    sequence: 0xffff_fffd,
                },
            ],
            outputs: vec![
                TxOut {
                    value: Satoshi::new(1_000),
                    script_pubkey: vec![0xcc].into(),
                },
                TxOut {
                    value: Satoshi::new(2_000),
                    script_pubkey: vec![0xdd].into(),
                },
            ],
            locktime: 0,
        }
    }

    #[test]
    fn sighash_type_mode_decoding() {
        assert_eq!(SigHashType(0x01).mode(), SigHashMode::All);
        assert_eq!(SigHashType(0x02).mode(), SigHashMode::None);
        assert_eq!(SigHashType(0x03).mode(), SigHashMode::Single);
        // ANYONECANPAY bit doesn't change the mode.
        assert_eq!(SigHashType(0x81).mode(), SigHashMode::All);
        assert_eq!(SigHashType(0x82).mode(), SigHashMode::None);
        assert_eq!(SigHashType(0x83).mode(), SigHashMode::Single);
        // Unknown modes fall through to ALL.
        assert_eq!(SigHashType(0x05).mode(), SigHashMode::All);
    }

    #[test]
    fn anyonecanpay_bit() {
        assert!(!SigHashType(0x01).is_anyonecanpay());
        assert!(SigHashType(0x81).is_anyonecanpay());
        assert!(SigHashType(0x83).is_anyonecanpay());
    }

    #[test]
    fn sighash_single_one_bug() {
        // Two inputs, two outputs. n_in = 5 → past the last output → "1" digest.
        let tx = make_tx();
        let h = signature_hash(&tx, 5, &[0xab], SigHashType::SINGLE);
        let mut expected = [0u8; 32];
        expected[0] = 0x01;
        assert_eq!(h.0, expected);
    }

    #[test]
    fn sighash_single_at_boundary_is_one_bug() {
        // n_in = 2 (== n_outputs) is also the "1" digest.
        let tx = make_tx();
        let h = signature_hash(&tx, 2, &[0xab], SigHashType::SINGLE);
        let mut expected = [0u8; 32];
        expected[0] = 0x01;
        assert_eq!(h.0, expected);
    }

    #[test]
    fn sighash_all_differs_from_none_and_single() {
        let tx = make_tx();
        let all = signature_hash(&tx, 0, &[0xab], SigHashType::ALL);
        let none = signature_hash(&tx, 0, &[0xab], SigHashType::NONE);
        let single = signature_hash(&tx, 0, &[0xab], SigHashType::SINGLE);
        assert_ne!(all, none);
        assert_ne!(all, single);
        assert_ne!(none, single);
    }

    #[test]
    fn sighash_anyonecanpay_changes_digest() {
        let tx = make_tx();
        let standard = signature_hash(&tx, 0, &[0xab], SigHashType::ALL);
        let anyone = signature_hash(&tx, 0, &[0xab], SigHashType(0x81));
        assert_ne!(standard, anyone);
    }

    #[test]
    fn sighash_changes_with_input_index() {
        let tx = make_tx();
        let h0 = signature_hash(&tx, 0, &[0xab], SigHashType::ALL);
        let h1 = signature_hash(&tx, 1, &[0xab], SigHashType::ALL);
        assert_ne!(h0, h1);
    }

    #[test]
    fn modified_tx_clears_other_input_scripts_under_sighash_all() {
        let tx = make_tx();
        let m = build_modified_tx(&tx, 0, &[0x99], SigHashType::ALL);
        assert_eq!(m.inputs[0].script_sig.as_slice(), &[0x99]);
        assert!(m.inputs[1].script_sig.is_empty());
        // SIGHASH_ALL preserves sequence on every input.
        assert_eq!(m.inputs[1].sequence, tx.inputs[1].sequence);
        // Outputs unchanged.
        assert_eq!(m.outputs.len(), 2);
    }

    #[test]
    fn modified_tx_under_sighash_none_drops_outputs_and_seq() {
        let tx = make_tx();
        let m = build_modified_tx(&tx, 0, &[0x99], SigHashType::NONE);
        assert!(m.outputs.is_empty());
        assert_eq!(m.inputs[0].sequence, tx.inputs[0].sequence);
        assert_eq!(m.inputs[1].sequence, 0); // zeroed
    }

    #[test]
    fn modified_tx_under_anyonecanpay_keeps_only_self_input() {
        let tx = make_tx();
        let m = build_modified_tx(&tx, 1, &[0x99], SigHashType(0x81));
        assert_eq!(m.inputs.len(), 1);
        assert_eq!(m.inputs[0].previous_output, tx.inputs[1].previous_output);
        assert_eq!(m.inputs[0].script_sig.as_slice(), &[0x99]);
    }

    #[test]
    fn modified_tx_under_sighash_single_truncates_outputs() {
        let tx = make_tx();
        let m = build_modified_tx(&tx, 1, &[0x99], SigHashType::SINGLE);
        // Outputs has length n_in + 1 = 2. Output[0] is the null sentinel,
        // output[1] is the real output at index n_in.
        assert_eq!(m.outputs.len(), 2);
        assert_eq!(u64::from(m.outputs[0].value), u64::MAX);
        assert!(m.outputs[0].script_pubkey.is_empty());
        assert_eq!(m.outputs[1], tx.outputs[1]);
    }

    // ── FindAndDelete ───────────────────────────────────────────────────────

    #[test]
    fn find_and_delete_removes_exact_push() {
        // Script: <push 3 [0xaa,0xbb,0xcc]> OP_DUP <push 2 [0xaa,0xbb,0xcc]> OP_DROP
        let mut script = Vec::new();
        script.push(0x03);
        script.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
        script.push(0x76); // OP_DUP
        script.push(0x03);
        script.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
        script.push(0x75); // OP_DROP
        let out = find_and_delete_sig(&script, &[0xaa, 0xbb, 0xcc]);
        assert_eq!(out, vec![0x76, 0x75]);
    }

    #[test]
    fn find_and_delete_doesnt_match_partial() {
        // Pushed payload contains the sig as a substring but is not equal to it.
        let mut script = vec![0x04];
        script.extend_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]);
        let out = find_and_delete_sig(&script, &[0xaa, 0xbb, 0xcc]);
        assert_eq!(out, script);
    }

    #[test]
    fn find_and_delete_empty_sig_doesnt_match_op0() {
        // OP_0 is byte 0x00 — not a push of empty under FindAndDelete (Core's
        // FindAndDelete only matches actual data pushes).
        let script = vec![0x00, 0x76];
        let out = find_and_delete_sig(&script, &[]);
        assert_eq!(out, script);
    }

    #[test]
    fn find_and_delete_pushdata1_match() {
        // OP_PUSHDATA1 [0x03] [0xaa,0xbb,0xcc] then OP_DUP
        let script = vec![0x4c, 0x03, 0xaa, 0xbb, 0xcc, 0x76];
        let out = find_and_delete_sig(&script, &[0xaa, 0xbb, 0xcc]);
        assert_eq!(out, vec![0x76]);
    }

    // ── script_code_after_codeseparator ─────────────────────────────────────

    #[test]
    fn codesep_slice_returns_suffix() {
        let script = [0x01, 0xaa, 0xab, 0x02, 0xbb, 0xcc];
        assert_eq!(script_code_after_codeseparator(&script, 0), &script);
        assert_eq!(script_code_after_codeseparator(&script, 3), &script[3..]);
        assert_eq!(script_code_after_codeseparator(&script, 6), &[] as &[u8]);
    }

    #[test]
    fn codesep_slice_out_of_bounds_is_empty() {
        let script = [0x01, 0xaa];
        assert_eq!(script_code_after_codeseparator(&script, 99), &[] as &[u8]);
    }
}
