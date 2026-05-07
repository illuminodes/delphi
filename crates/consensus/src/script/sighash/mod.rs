//! Signature hash computation — legacy (pre-segwit) and BIP143 (segwit v0).
//!
//! ## Legacy (`signature_hash`)
//!
//! Produces the 32-byte digest that an ECDSA signature in the legacy regime is
//! computed over. The algorithm:
//!
//! 1. Build a *modified* transaction:
//!    - Replace every input's `script_sig` with the empty script.
//!    - Replace input `n_in`'s `script_sig` with `script_code` (after
//!      `FindAndDelete` of the signature being checked — see
//!      `find_and_delete_sig`).
//!    - Apply the `hash_type` modifications:
//!      - `SIGHASH_NONE`: clear all outputs and zero every other input's sequence.
//!      - `SIGHASH_SINGLE`: keep only output `n_in`; if `n_in >= n_outputs`,
//!        return the constant digest `0x0000…0001` (the **"1" bug**).
//!      - `SIGHASH_ANYONECANPAY`: keep only input `n_in`.
//! 2. Serialise the modified tx in legacy form, append the 4-byte
//!    little-endian `hash_type`, and SHA-256d.
//!
//! All four base modes plus `ANYONECANPAY` are reproduced exactly, including
//! the `SIGHASH_SINGLE` "1" bug — fixing it would be a hard fork.
//!
//! ## BIP143 (`signature_hash_bip143`)
//!
//! Segwit v0 sighash. Commits to amounts and removes quadratic hashing by
//! pre-hashing shared fields (`hashPrevouts`, `hashSequence`, `hashOutputs`)
//! once per transaction via [`Bip143Cache`], then per-input serialises:
//!
//! ```text
//! nVersion || hashPrevouts || hashSequence || outpoint || scriptCode ||
//! amount || nSequence || hashOutputs || nLocktime || nHashType
//! ```
//!
//! `scriptCode` for P2WPKH is a synthetic P2PKH script; for P2WSH it is the
//! witness script with a `VarInt` length prefix (both supplied by the caller
//! from the segwit dispatch logic in `verify_script`).

use mantle::codec::{Encode, VarInt};
use mantle::hash::sha256d;
use mantle::primitives::{Hash256, Satoshi, Transaction, TxIn, TxOut, Witness};

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

/// Per-transaction cache of the three BIP143 hash fields that are shared
/// across all inputs of a single transaction verification.
///
/// Computing these once up front eliminates the quadratic hashing that
/// plagued legacy sighash when many inputs sign the same tx.
///
/// Create with [`Bip143Cache::new`] before calling [`signature_hash_bip143`]
/// for each input; drop it when the transaction is fully verified.
#[derive(Debug, Clone)]
pub struct Bip143Cache {
    /// `SHA256d` of the concatenation of all input outpoints (36 bytes each).
    /// Zeroed when `ANYONECANPAY` is set.
    prevouts: Hash256,
    /// `SHA256d` of the concatenation of all input `nSequence` values (4 LE each).
    /// Zeroed when `ANYONECANPAY`, `SIGHASH_SINGLE`, or `SIGHASH_NONE` is set.
    sequence: Hash256,
    /// `SHA256d` of the concatenation of all outputs serialised as
    /// `value(8 LE) || scriptPubKey(VarInt len + bytes)`.
    /// Zeroed for `SIGHASH_NONE`; per-input for `SIGHASH_SINGLE`.
    outputs: Hash256,
}

impl Bip143Cache {
    /// Compute the shared hash fields from `tx`. Call once per transaction.
    #[must_use]
    pub fn new(tx: &Transaction) -> Self {
        let mut prevouts_buf = Vec::with_capacity(tx.inputs.len() * 36);
        let mut seq_buf = Vec::with_capacity(tx.inputs.len() * 4);
        for input in &tx.inputs {
            // OutPoint is fixed-size; encode into a growing Vec (infallible).
            let _ = input.previous_output.encode(&mut prevouts_buf);
            seq_buf.extend_from_slice(&input.sequence.to_le_bytes());
        }

        let mut outputs_buf: Vec<u8> = Vec::new();
        for output in &tx.outputs {
            let _ = output.encode(&mut outputs_buf);
        }

        Self {
            prevouts: Hash256(sha256d(&prevouts_buf)),
            sequence: Hash256(sha256d(&seq_buf)),
            outputs: Hash256(sha256d(&outputs_buf)),
        }
    }
}

/// Compute the BIP143 (segwit v0) signature hash.
///
/// `script_code` is the scriptCode for this input as defined by BIP143:
/// - P2WPKH: the 25-byte synthetic `OP_DUP OP_HASH160 <20-byte-hash> OP_EQUALVERIFY OP_CHECKSIG`
///   built by the segwit dispatch in `verify_script`.
/// - P2WSH: the witness script bytes (the last witness stack item).
///
/// In both cases this function prepends a `VarInt(len)` length prefix as
/// required by BIP143 §Script Code. The cache must have been built from the
/// same `tx`.
///
/// # Panics
/// Panics if `n_in >= tx.inputs.len()`. The interpreter only reaches this
/// with a valid input index.
#[must_use]
pub fn signature_hash_bip143(
    tx: &Transaction,
    n_in: usize,
    script_code: &[u8],
    amount: Satoshi,
    hash_type: SigHashType,
    cache: &Bip143Cache,
) -> Hash256 {
    let mode = hash_type.mode();
    let anyonecanpay = hash_type.is_anyonecanpay();

    let hash_prevouts = if anyonecanpay { Hash256::ZERO } else { cache.prevouts };

    let hash_sequence =
        if anyonecanpay || mode == SigHashMode::Single || mode == SigHashMode::None {
            Hash256::ZERO
        } else {
            cache.sequence
        };

    let hash_outputs = match mode {
        SigHashMode::All => cache.outputs,
        SigHashMode::None => Hash256::ZERO,
        SigHashMode::Single => {
            if n_in < tx.outputs.len() {
                let mut buf: Vec<u8> = Vec::new();
                let _ = tx.outputs[n_in].encode(&mut buf);
                Hash256(sha256d(&buf))
            } else {
                Hash256::ZERO
            }
        }
    };

    let input = &tx.inputs[n_in];

    let mut buf: Vec<u8> = Vec::with_capacity(200);
    buf.extend_from_slice(&tx.version.to_le_bytes());
    buf.extend_from_slice(&hash_prevouts.0);
    buf.extend_from_slice(&hash_sequence.0);
    let _ = input.previous_output.encode(&mut buf);
    let _ = VarInt(script_code.len() as u64).encode(&mut buf);
    buf.extend_from_slice(script_code);
    buf.extend_from_slice(&u64::from(amount).to_le_bytes());
    buf.extend_from_slice(&input.sequence.to_le_bytes());
    buf.extend_from_slice(&hash_outputs.0);
    buf.extend_from_slice(&tx.locktime.to_le_bytes());
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
            witness: Witness::empty(),
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
                    witness: Witness::empty(),
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
mod tests;
