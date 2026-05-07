use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use mantle_core::codec::Encode;
use mantle_core::hash::sha256d;
use mantle_core::primitives::{Block, BlockHeader, Hash256, OutPoint, Transaction};

use crate::chain_view::ChainView;
use crate::error::ValidationError;
use crate::merkle::compute_merkle_root;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum money supply in satoshis (21 million BTC).
pub const MAX_MONEY: u64 = 21_000_000 * 100_000_000;

/// Maximum block weight in weight units (BIP141).
pub const MAX_BLOCK_WEIGHT: u64 = 4_000_000;

/// Base size (non-witness) weight multiplier.
pub const WITNESS_SCALE_FACTOR: u64 = 4;

/// Maximum coinbase scriptSig length in bytes.
pub const MAX_COINBASE_SCRIPTSIG_LEN: usize = 100;
/// Minimum coinbase scriptSig length in bytes.
pub const MIN_COINBASE_SCRIPTSIG_LEN: usize = 2;

/// Clock drift allowance: blocks may not be more than 2 hours in the future.
pub const MAX_FUTURE_BLOCK_TIME: u64 = 2 * 60 * 60;

/// BIP34 activation height on mainnet.
pub const BIP34_HEIGHT: u32 = 227_931;

/// Maximum number of transactions per block (practical consensus limit).
pub const MAX_BLOCK_TRANSACTIONS: usize = 1_000_000;

// ── Null outpoint (coinbase marker) ──────────────────────────────────────────

/// The null outpoint used in coinbase inputs.
const NULL_OUTPOINT: OutPoint = OutPoint {
    txid: Hash256::ZERO,
    vout: u32::MAX,
};

// ── Public API ────────────────────────────────────────────────────────────────

/// Compute the txid (`SHA256d` of the non-witness serialisation) of a transaction.
///
/// # Panics
/// Never — encoding to a `Vec<u8>` cannot fail.
#[must_use]
pub fn txid(tx: &Transaction) -> Hash256 {
    let mut buf = Vec::new();
    tx.encode(&mut buf).expect("Vec<u8> write cannot fail");
    Hash256(sha256d(&buf))
}

/// Stage 1: stateless structural validation.
///
/// Validates the block in isolation — no chain state required. This is the
/// first line of defence against malformed blocks from peers. All checks here
/// are pure functions of the block bytes.
///
/// # Errors
/// Returns the first `ValidationError` encountered.
///
/// # Panics
/// Never — the internal `expect` is guarded by the non-empty check above it.
pub fn validate_block_structure(block: &Block) -> Result<(), ValidationError> {
    // Non-empty tx list.
    if block.transactions.is_empty() {
        return Err(ValidationError::EmptyBlock);
    }

    if block.transactions.len() > MAX_BLOCK_TRANSACTIONS {
        return Err(ValidationError::TooManyTransactions {
            count: block.transactions.len(),
        });
    }

    // First tx must be coinbase; no subsequent tx may be coinbase.
    for (i, tx) in block.transactions.iter().enumerate() {
        let is_first = i == 0;
        validate_transaction_structure(tx, i, is_first)?;
    }

    // Block weight (base serialisation only — we have no witness data in the
    // current Transaction type, so weight = base_size * 4).
    let weight = block_weight(block);
    if weight > MAX_BLOCK_WEIGHT {
        return Err(ValidationError::BlockWeightExceeded { weight });
    }

    // Merkle root.
    let txids: Vec<Hash256> = block.transactions.iter().map(txid).collect();
    let computed_root = compute_merkle_root(&txids).expect("non-empty; checked above");
    if computed_root != block.header.merkle_root {
        return Err(ValidationError::MerkleRootMismatch);
    }

    // PoW: header hash must be ≤ target encoded in nBits.
    validate_proof_of_work(&block.header)?;

    Ok(())
}

/// Stage 2: contextual validation.
///
/// Requires the header chain (via `ChainView`). Does not touch the UTXO set.
///
/// # Errors
/// Returns the first `ValidationError` encountered.
pub fn validate_block_context(
    block: &Block,
    view: &impl ChainView,
) -> Result<(), ValidationError> {
    let header = &block.header;

    // Parent must exist in the chain view.
    let parent = view
        .get_header(&header.prev_block)?
        .ok_or(ValidationError::UnknownParent)?;

    let parent_height = {
        // Derive parent height from the chain view tip and header lookups.
        // We find it by looking up the parent's own prev_block iteratively
        // until genesis, but that's O(n). Instead: tip height minus the
        // number of steps from tip to parent. For MemoryChainView the tip
        // height is authoritative; for production the storage thread tracks it.
        // We reconstruct by asking: what height does the view say this header
        // lives at? We do that by scanning get_header_at from the tip down.
        // For correctness and simplicity, we search from tip height downward.
        let tip = view.tip();
        let mut found_height = None;
        let parent_hash = header.prev_block;
        for h in (0..=tip.height).rev() {
            if let Some(hdr) = view.get_header_at(h)? {
                let hdr_hash = Hash256(sha256d(&hdr.to_bytes()));
                if hdr_hash == parent_hash {
                    found_height = Some(h);
                    break;
                }
            }
        }
        found_height.ok_or(ValidationError::UnknownParent)?
    };

    let this_height = parent_height + 1;

    // nBits must match expected difficulty.
    let expected_bits = expected_nbits(parent_height, parent, view)?;
    if header.bits != expected_bits {
        return Err(ValidationError::UnexpectedDifficulty);
    }

    // Timestamp > MTP (BIP113).
    let mtp = view.median_time_past(header)?;
    if header.time <= mtp {
        return Err(ValidationError::TimestampTooEarly);
    }

    // Timestamp ≤ now + 2h.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if u64::from(header.time) > now + MAX_FUTURE_BLOCK_TIME {
        return Err(ValidationError::TimestampTooFarFuture);
    }

    // BIP34: coinbase scriptSig must begin with encoded block height.
    if this_height >= BIP34_HEIGHT {
        validate_bip34_height(&block.transactions[0], this_height)?;
    }

    // Block version lower bound per BIP9/BIP8 deployment state.
    // For v1: enforce the historical minimums only.
    validate_block_version(header, this_height)?;

    Ok(())
}

// ── Per-transaction structural checks ────────────────────────────────────────

fn validate_transaction_structure(
    tx: &Transaction,
    tx_index: usize,
    expect_coinbase: bool,
) -> Result<(), ValidationError> {
    if tx.inputs.is_empty() {
        return Err(ValidationError::EmptyInputs { tx_index });
    }
    if tx.outputs.is_empty() {
        return Err(ValidationError::EmptyOutputs { tx_index });
    }

    let first_is_coinbase = tx.inputs[0].previous_output == NULL_OUTPOINT;

    if expect_coinbase && !first_is_coinbase {
        return Err(ValidationError::FirstTxNotCoinbase);
    }
    if !expect_coinbase && first_is_coinbase {
        return Err(ValidationError::UnexpectedCoinbaseInput { tx_index });
    }

    if expect_coinbase {
        let script_len = tx.inputs[0].script_sig.len();
        if !(MIN_COINBASE_SCRIPTSIG_LEN..=MAX_COINBASE_SCRIPTSIG_LEN).contains(&script_len) {
            return Err(ValidationError::CoinbaseScriptSigLength { len: script_len });
        }
    }

    // Output value bounds.
    let mut total: u64 = 0;
    for (i, out) in tx.outputs.iter().enumerate() {
        if out.value > MAX_MONEY {
            return Err(ValidationError::OutputValueOutOfRange {
                tx_index,
                output_index: i,
            });
        }
        total = total
            .checked_add(out.value)
            .filter(|&s| s <= MAX_MONEY)
            .ok_or(ValidationError::TotalOutputValueOverflow { tx_index })?;
    }

    // Duplicate inputs within a single transaction.
    if !expect_coinbase {
        let mut seen = HashSet::new();
        for input in &tx.inputs {
            if !seen.insert(input.previous_output) {
                return Err(ValidationError::DuplicateInput { tx_index });
            }
        }
    }

    Ok(())
}

// ── Weight ────────────────────────────────────────────────────────────────────

/// Block weight: base serialisation size × 4.
///
/// We don't yet have witness data in our `Transaction` type; once segwit
/// decoding is added this function will account for the witness discount.
fn block_weight(block: &Block) -> u64 {
    let mut buf = Vec::new();
    block.encode(&mut buf).expect("Vec<u8> write cannot fail");
    buf.len() as u64 * WITNESS_SCALE_FACTOR
}

// ── Proof of work ─────────────────────────────────────────────────────────────

/// Decode nBits to a 256-bit target as a 32-byte big-endian array.
///
/// Returns `None` if nBits is zero, negative, or encodes an overflowed target.
fn nbits_to_target(bits: u32) -> Option<[u8; 32]> {
    let exponent = (bits >> 24) as usize;
    let mantissa = bits & 0x007f_ffff;

    // Negative mantissa or zero: invalid.
    if bits & 0x0080_0000 != 0 {
        return None;
    }
    if mantissa == 0 {
        return None;
    }
    // Exponent must allow the mantissa to fit in 32 bytes.
    if exponent > 34 || (exponent == 34 && mantissa > 0xff) {
        return None;
    }

    let mut target = [0u8; 32];
    // Mantissa occupies 3 bytes; place them at [32 - exponent .. 32 - exponent + 3].
    if exponent >= 3 {
        let start = 32 - exponent;
        target[start] = ((mantissa >> 16) & 0xff) as u8;
        target[start + 1] = ((mantissa >> 8) & 0xff) as u8;
        target[start + 2] = (mantissa & 0xff) as u8;
    } else {
        // exponent < 3: mantissa is right-shifted to fit.
        let shift = (3 - exponent) * 8;
        let m = mantissa >> shift;
        if exponent > 0 {
            target[32 - exponent] = (m & 0xff) as u8;
        }
    }

    Some(target)
}

/// Check that the block header hash satisfies the claimed nBits target.
fn validate_proof_of_work(header: &BlockHeader) -> Result<(), ValidationError> {
    let target = nbits_to_target(header.bits).ok_or(ValidationError::InvalidBits)?;

    // Header hash in big-endian (reversed from internal storage).
    let raw_hash = sha256d(&header.to_bytes());
    let mut hash_be = raw_hash;
    hash_be.reverse();

    if hash_be > target {
        return Err(ValidationError::InsufficientProofOfWork);
    }
    Ok(())
}

// ── Difficulty ────────────────────────────────────────────────────────────────

/// Bitcoin's difficulty retarget interval in blocks.
const RETARGET_INTERVAL: u32 = 2016;
/// Target spacing in seconds (10 minutes).
const TARGET_SPACING: u64 = 600;
/// Target timespan in seconds (two weeks).
const TARGET_TIMESPAN: u64 = RETARGET_INTERVAL as u64 * TARGET_SPACING;
/// Minimum timespan (target timespan / 4, to prevent too-easy mining).
const MIN_TIMESPAN: u64 = TARGET_TIMESPAN / 4;
/// Maximum timespan (target timespan × 4, to prevent too-hard mining).
const MAX_TIMESPAN: u64 = TARGET_TIMESPAN * 4;

/// Compute the expected nBits for the block at `parent_height + 1`.
fn expected_nbits(
    parent_height: u32,
    parent: BlockHeader,
    view: &impl ChainView,
) -> Result<u32, ValidationError> {
    let this_height = parent_height + 1;

    // No retarget — carry parent's bits.
    if this_height % RETARGET_INTERVAL != 0 {
        return Ok(parent.bits);
    }

    // Retarget: find the block RETARGET_INTERVAL blocks ago.
    let retarget_height = parent_height + 1 - RETARGET_INTERVAL;
    let retarget_header = view
        .get_header_at(retarget_height)?
        .ok_or(ValidationError::UnknownParent)?;

    let actual_timespan = u64::from(parent.time).saturating_sub(u64::from(retarget_header.time));
    let clamped = actual_timespan.clamp(MIN_TIMESPAN, MAX_TIMESPAN);

    // Decode parent nBits to a big-integer target, scale, re-encode.
    let old_target = nbits_to_target(parent.bits).ok_or(ValidationError::InvalidBits)?;
    let new_target = scale_target(old_target, clamped, TARGET_TIMESPAN);
    Ok(target_to_nbits(new_target))
}

/// Scale a 256-bit target by `actual / expected`, operating on big-endian bytes.
fn scale_target(target: [u8; 32], actual: u64, expected: u64) -> [u8; 32] {
    // Use u128 arithmetic on the most-significant portion to avoid full
    // big-integer library dependency. Bitcoin targets fit in ~72 bits of
    // mantissa so this is safe for real-world values.
    // We represent the target as a 256-bit value split into high/low u128.
    let hi = u128::from_be_bytes(target[..16].try_into().unwrap());
    let lo = u128::from_be_bytes(target[16..].try_into().unwrap());

    // Scale: new = old * actual / expected.
    // To avoid overflow: scale hi and lo separately then recombine.
    // This is an approximation for very large targets; sufficient for consensus
    // because mainnet targets have at most ~32 significant bits.
    let new_hi = hi.saturating_mul(u128::from(actual)) / u128::from(expected);
    let new_lo = lo.saturating_mul(u128::from(actual)) / u128::from(expected);

    let mut result = [0u8; 32];
    result[..16].copy_from_slice(&new_hi.to_be_bytes());
    result[16..].copy_from_slice(&new_lo.to_be_bytes());
    result
}

/// Encode a 256-bit big-endian target back to nBits compact format.
fn target_to_nbits(target: [u8; 32]) -> u32 {
    // Find the most-significant non-zero byte.
    let Some(first_nonzero) = target.iter().position(|&b| b != 0) else {
        return 0;
    };

    // Exponent = number of bytes from the most-significant byte to the end.
    // first_nonzero ≤ 31 so 32 - first_nonzero ≤ 32, always fits in u32.
    let exponent = u32::try_from(32 - first_nonzero).expect("≤ 32, always fits");

    // Mantissa = 3 bytes starting at first_nonzero.
    let mut mantissa = u32::from(target[first_nonzero]) << 16;
    if first_nonzero + 1 < 32 {
        mantissa |= u32::from(target[first_nonzero + 1]) << 8;
    }
    if first_nonzero + 2 < 32 {
        mantissa |= u32::from(target[first_nonzero + 2]);
    }

    // If the sign bit would be set, shift right by one byte and increment exponent.
    if mantissa & 0x0080_0000 != 0 {
        mantissa >>= 8;
        (exponent + 1) << 24 | mantissa
    } else {
        exponent << 24 | mantissa
    }
}

// ── BIP34 ─────────────────────────────────────────────────────────────────────

/// BIP34: coinbase scriptSig must begin with a minimally-encoded block height.
fn validate_bip34_height(coinbase_tx: &Transaction, height: u32) -> Result<(), ValidationError> {
    let script = &coinbase_tx.inputs[0].script_sig;
    let encoded = encode_bip34_height(height);
    if script.len() < encoded.len() || script[..encoded.len()] != encoded {
        return Err(ValidationError::Bip34HeightMissing);
    }
    Ok(())
}

/// Minimal Script push encoding of a block height as used by BIP34.
fn encode_bip34_height(height: u32) -> Vec<u8> {
    if height == 0 {
        return vec![0x00]; // OP_0
    }
    // Encode as little-endian bytes, minimal length, with sign bit.
    let mut le = height.to_le_bytes().to_vec();
    // Trim trailing zeros.
    while le.last() == Some(&0) {
        le.pop();
    }
    // Add sign byte if the high bit of the last byte is set.
    if le.last().is_some_and(|&b| b & 0x80 != 0) {
        le.push(0x00);
    }
    // BIP34 height script push: length byte fits in u8 (max 4 bytes for u32).
    let len = u8::try_from(le.len()).expect("BIP34 height encoding ≤ 5 bytes");
    let mut out = vec![len]; // OP_PUSHDATA length byte
    out.extend_from_slice(&le);
    out
}

// ── Block version ─────────────────────────────────────────────────────────────

/// Enforce historical block version minimums.
///
/// BIP34 (height ≥ 227,931): version ≥ 2
/// BIP66 (height ≥ 363,725): version ≥ 3
/// BIP65 (height ≥ 388,381): version ≥ 4
fn validate_block_version(header: &BlockHeader, height: u32) -> Result<(), ValidationError> {
    let required: i32 = if height >= 388_381 {
        4
    } else if height >= 363_725 {
        3
    } else if height >= BIP34_HEIGHT {
        2
    } else {
        1
    };

    if header.version < required {
        return Err(ValidationError::BlockVersionTooLow {
            version: header.version,
            required,
        });
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain_view::MemoryChainView;
    use mantle_core::primitives::{TxIn, TxOut};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn coinbase_tx(height: u32, value: u64) -> Transaction {
        Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: NULL_OUTPOINT,
                script_sig: encode_bip34_height(height),
                sequence: u32::MAX,
            }],
            outputs: vec![TxOut {
                value,
                script_pubkey: vec![0x51], // OP_1
            }],
            locktime: 0,
        }
    }

    fn regular_tx(prev_txid: Hash256, vout: u32, value: u64) -> Transaction {
        Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: OutPoint { txid: prev_txid, vout },
                script_sig: vec![0x00],
                sequence: u32::MAX,
            }],
            outputs: vec![TxOut {
                value,
                script_pubkey: vec![0x51],
            }],
            locktime: 0,
        }
    }

    fn make_block(txs: Vec<Transaction>, bits: u32) -> Block {
        let txids: Vec<Hash256> = txs.iter().map(txid).collect();
        let merkle_root = compute_merkle_root(&txids).unwrap();
        Block {
            header: BlockHeader {
                version: 1,
                prev_block: Hash256::ZERO,
                merkle_root,
                time: 1_231_006_505,
                bits,
                nonce: 0,
            },
            transactions: txs,
        }
    }

    /// Build a block with a valid PoW nonce (brute-force search).
    /// Only used for tests that exercise PoW validation.
    fn mine_block(mut block: Block) -> Block {
        let target = nbits_to_target(block.header.bits).unwrap();
        for nonce in 0..=u32::MAX {
            block.header.nonce = nonce;
            let raw = sha256d(&block.header.to_bytes());
            let mut be = raw;
            be.reverse();
            if be <= target {
                return block;
            }
        }
        panic!("could not find a valid nonce — use a very easy nBits in tests");
    }

    // ── Stage 1 tests ─────────────────────────────────────────────────────────

    #[test]
    fn empty_block_rejected() {
        let block = Block {
            header: BlockHeader {
                version: 1,
                prev_block: Hash256::ZERO,
                merkle_root: Hash256::ZERO,
                time: 0,
                bits: 0x207f_ffff,
                nonce: 0,
            },
            transactions: vec![],
        };
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::EmptyBlock)
        );
    }

    #[test]
    fn first_tx_not_coinbase_rejected() {
        let tx = regular_tx(Hash256([1u8; 32]), 0, 1000);
        let mut block = make_block(vec![tx], 0x207f_ffff);
        // Fix merkle root so we reach the coinbase check.
        // (make_block already does this, but the block won't have PoW — that's fine,
        // we're testing structure checks which run before PoW.)
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::FirstTxNotCoinbase)
        );
        let _ = &mut block; // suppress unused warning
    }

    #[test]
    fn coinbase_script_too_short() {
        let mut tx = coinbase_tx(1, 5_000_000_000);
        tx.inputs[0].script_sig = vec![0x01]; // 1 byte < MIN (2)
        let block = make_block(vec![tx], 0x207f_ffff);
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::CoinbaseScriptSigLength { len: 1 })
        );
    }

    #[test]
    fn coinbase_script_too_long() {
        let mut tx = coinbase_tx(1, 5_000_000_000);
        tx.inputs[0].script_sig = vec![0x00; 101]; // 101 > MAX (100)
        let block = make_block(vec![tx], 0x207f_ffff);
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::CoinbaseScriptSigLength { len: 101 })
        );
    }

    #[test]
    fn output_value_out_of_range() {
        let mut tx = coinbase_tx(1, MAX_MONEY + 1);
        tx.outputs[0].value = MAX_MONEY + 1;
        // Re-encode the block with the bad tx — merkle root will be recomputed.
        let block = make_block(vec![tx], 0x207f_ffff);
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::OutputValueOutOfRange {
                tx_index: 0,
                output_index: 0
            })
        );
    }

    #[test]
    fn duplicate_inputs_rejected() {
        let cb = coinbase_tx(1, 5_000_000_000);
        let mut tx2 = regular_tx(Hash256([2u8; 32]), 0, 1000);
        // Add a duplicate input.
        tx2.inputs.push(tx2.inputs[0].clone());
        let block = make_block(vec![cb, tx2], 0x207f_ffff);
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::DuplicateInput { tx_index: 1 })
        );
    }

    #[test]
    fn merkle_root_mismatch() {
        let tx = coinbase_tx(1, 5_000_000_000);
        let mut block = make_block(vec![tx], 0x207f_ffff);
        block.header.merkle_root = Hash256([0xde; 32]); // corrupt root
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::MerkleRootMismatch)
        );
    }

    #[test]
    fn pow_invalid() {
        let tx = coinbase_tx(1, 5_000_000_000);
        // Use a very hard nBits — nonce=0 will almost certainly fail PoW.
        let block = make_block(vec![tx], 0x1d00_ffff);
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::InsufficientProofOfWork)
        );
    }

    #[test]
    fn valid_block_easy_pow() {
        let tx = coinbase_tx(1, 5_000_000_000);
        // 0x207fffff is the easiest possible nBits (used in regtest).
        let block = mine_block(make_block(vec![tx], 0x207f_ffff));
        assert!(validate_block_structure(&block).is_ok());
    }

    // ── nbits helpers ─────────────────────────────────────────────────────────

    #[test]
    fn nbits_roundtrip_genesis() {
        // Genesis nBits = 0x1d00ffff.
        let bits = 0x1d00_ffff_u32;
        let target = nbits_to_target(bits).unwrap();
        let back = target_to_nbits(target);
        assert_eq!(back, bits);
    }

    #[test]
    fn nbits_zero_mantissa_is_invalid() {
        assert!(nbits_to_target(0x0300_0000).is_none());
    }

    #[test]
    fn nbits_negative_is_invalid() {
        // High bit of mantissa set → negative.
        assert!(nbits_to_target(0x0380_0000).is_none());
    }

    // ── BIP34 height encoding ─────────────────────────────────────────────────

    #[test]
    fn bip34_height_encoding() {
        // Height 1: [0x01, 0x01] — push 1 byte: 0x01.
        assert_eq!(encode_bip34_height(1), vec![0x01, 0x01]);
        // Height 256: [0x02, 0x00, 0x01] — push 2 bytes: little-endian 256.
        assert_eq!(encode_bip34_height(256), vec![0x02, 0x00, 0x01]);
        // Height 0x80 (128): needs sign byte → [0x02, 0x80, 0x00].
        assert_eq!(encode_bip34_height(0x80), vec![0x02, 0x80, 0x00]);
    }

    // ── Stage 2 tests ─────────────────────────────────────────────────────────

    fn build_view_with_chain(len: u32) -> MemoryChainView {
        let mut view = MemoryChainView::new();
        let mut prev = Hash256::ZERO;
        for i in 0..len {
            let h = BlockHeader {
                version: 4,
                prev_block: prev,
                merkle_root: Hash256::ZERO,
                time: 1_296_688_602 + i * 600, // regtest-style spacing
                bits: 0x207f_ffff,
                nonce: 0,
            };
            prev = Hash256(sha256d(&h.to_bytes()));
            view.push_header(h);
        }
        view
    }

    #[test]
    fn unknown_parent_rejected() {
        let view = build_view_with_chain(1);
        let tx = coinbase_tx(1, 5_000_000_000);
        let block = Block {
            header: BlockHeader {
                version: 4,
                prev_block: Hash256([0xff; 32]), // unknown
                merkle_root: compute_merkle_root(&[txid(&tx)]).unwrap(),
                time: 1_296_688_602 + 600,
                bits: 0x207f_ffff,
                nonce: 0,
            },
            transactions: vec![tx],
        };
        assert_eq!(
            validate_block_context(&block, &view),
            Err(ValidationError::UnknownParent)
        );
    }

    #[test]
    fn timestamp_too_early_rejected() {
        let view = build_view_with_chain(12);
        let tip = view.tip();
        let tip_header = view.get_header_at(tip.height).unwrap().unwrap();
        let tx = coinbase_tx(tip.height + 1, 5_000_000_000);
        let mtp = view.median_time_past(&BlockHeader {
            version: 4,
            prev_block: tip.hash,
            merkle_root: Hash256::ZERO,
            time: 0,
            bits: 0x207f_ffff,
            nonce: 0,
        }).unwrap();
        let block = Block {
            header: BlockHeader {
                version: 4,
                prev_block: tip.hash,
                merkle_root: compute_merkle_root(&[txid(&tx)]).unwrap(),
                time: mtp, // exactly MTP — must be > MTP
                bits: 0x207f_ffff,
                nonce: 0,
            },
            transactions: vec![tx],
        };
        let _ = tip_header; // used for clarity
        assert_eq!(
            validate_block_context(&block, &view),
            Err(ValidationError::TimestampTooEarly)
        );
    }

    #[test]
    fn bip34_missing_rejected() {
        let view = build_view_with_chain(BIP34_HEIGHT);
        let tip = view.tip();
        let mut tx = coinbase_tx(tip.height + 1, 5_000_000_000);
        // Replace BIP34 scriptSig with garbage that doesn't encode the height.
        tx.inputs[0].script_sig = vec![0x02, 0x00, 0x00];
        let block = Block {
            header: BlockHeader {
                version: 4,
                prev_block: tip.hash,
                merkle_root: compute_merkle_root(&[txid(&tx)]).unwrap(),
                time: 1_296_688_602 + BIP34_HEIGHT * 600 + 600,
                bits: 0x207f_ffff,
                nonce: 0,
            },
            transactions: vec![tx],
        };
        assert_eq!(
            validate_block_context(&block, &view),
            Err(ValidationError::Bip34HeightMissing)
        );
    }
}
