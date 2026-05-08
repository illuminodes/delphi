use std::collections::HashSet;

use mantle::codec::{DecodeRef, VarInt};
use mantle::collections::PrehashedSet;
use mantle::hash::sha256d;
use mantle::primitives::{
    Block, BlockHeader, BlockHeight, Hash256, OutPoint, Satoshi, Transaction, TransactionRef,
    TxHash, TxInRef, TxOutRef,
};

use crate::error::ValidationError;
use crate::merkle::compute_merkle_root_reusing;

use super::pow::HeaderValidationExt;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum money supply in satoshis (21 million BTC).
pub const MAX_MONEY: Satoshi = Satoshi::MAX_MONEY;

/// Maximum block weight in weight units (BIP141).
pub const MAX_BLOCK_WEIGHT: u64 = 4_000_000;

/// Base size (non-witness) weight multiplier.
pub const WITNESS_SCALE_FACTOR: u64 = 4;

/// Maximum coinbase scriptSig length in bytes.
pub const MAX_COINBASE_SCRIPTSIG_LEN: usize = 100;
/// Minimum coinbase scriptSig length in bytes.
pub const MIN_COINBASE_SCRIPTSIG_LEN: usize = 2;

/// BIP34 activation height on mainnet.
pub const BIP34_HEIGHT: BlockHeight = BlockHeight(227_931);

/// Maximum number of transactions per block (practical consensus limit).
pub const MAX_BLOCK_TRANSACTIONS: usize = 1_000_000;

// ── Null outpoint (coinbase marker) ──────────────────────────────────────────

/// The null outpoint used in coinbase inputs.
pub(super) const NULL_OUTPOINT: OutPoint = OutPoint {
    txid: Hash256::ZERO,
    vout: u32::MAX,
};

// ── Stage 1 public functions ──────────────────────────────────────────────────

/// Stage 1: stateless structural validation.
///
/// Validates the block in isolation — no chain state required. This is the
/// first line of defence against malformed blocks from peers. All checks here
/// are pure functions of the block bytes.
///
/// Allocates fresh scratch buffers each call. For bulk processing use
/// [`validate_block_structure_reusing`] to amortise those allocations across
/// many blocks.
///
/// # Errors
/// Returns the first `ValidationError` encountered.
///
/// # Panics
/// Never — the internal `expect` is guarded by the non-empty check above it.
pub fn validate_block_structure(block: &Block) -> Result<(), ValidationError> {
    validate_block_structure_reusing(
        block,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut PrehashedSet::default(),
        &mut PrehashedSet::default(),
    )
}

/// Stage 1 with caller-supplied scratch buffers.
///
/// All five scratch containers are cleared before use; the caller just needs to
/// keep them alive across calls to amortise allocations. Capacity is retained
/// between calls so the buffers grow to the high-water mark and stay there.
///
/// - `serial_scratch` — byte buffer reused for tx serialisation (txid hashing).
/// - `txids` — accumulator for per-block txid list fed into the merkle check.
/// - `merkle_buf` — working buffer for the in-place merkle reduction.
/// - `seen_inputs` — dedup set for inputs within a single transaction.
/// - `seen_txids` — dedup set for CVE-2012-2459 check across all txids.
///
/// # Errors
/// Returns the first `ValidationError` encountered.
///
/// # Panics
/// Never.
pub fn validate_block_structure_reusing<S1, S2>(
    block: &Block,
    serial_scratch: &mut Vec<u8>,
    txids: &mut Vec<Hash256>,
    merkle_buf: &mut Vec<Hash256>,
    seen_inputs: &mut HashSet<OutPoint, S1>,
    seen_txids: &mut HashSet<Hash256, S2>,
) -> Result<(), ValidationError>
where
    S1: std::hash::BuildHasher,
    S2: std::hash::BuildHasher,
{
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
    seen_inputs.clear();
    for (i, tx) in block.transactions.iter().enumerate() {
        check_tx_structure(tx, i, i == 0, seen_inputs)?;
    }

    // Block weight.
    let weight = block.weight();
    if weight > MAX_BLOCK_WEIGHT {
        return Err(ValidationError::BlockWeightExceeded { weight });
    }

    // Merkle root — all txids hashed with a single reused serialisation buffer.
    txids.clear();
    txids.reserve(block.transactions.len());
    for tx in &block.transactions {
        tx.txid_into(serial_scratch)?;
        txids.push(Hash256(sha256d(serial_scratch)));
    }

    // CVE-2012-2459: reject any block whose txid set contains duplicates.
    seen_txids.clear();
    seen_txids.reserve(txids.len());
    for &id in txids.iter() {
        if !seen_txids.insert(id) {
            return Err(ValidationError::DuplicateTransaction);
        }
    }

    let computed_root =
        compute_merkle_root_reusing(txids, merkle_buf).expect("non-empty; checked above");
    if computed_root != block.header.merkle_root {
        return Err(ValidationError::MerkleRootMismatch);
    }

    // PoW: header hash must be ≤ target encoded in nBits.
    block.header.check_pow()?;

    Ok(())
}

/// Stage 1 from raw wire bytes — zero heap allocations for the validation
/// path itself.
///
/// Parses the 80-byte header and iterates transactions as [`TransactionRef`]
/// views directly into `wire`. No `Transaction` structs are materialised and
/// txids are computed by hashing the raw wire span directly — no re-encoding.
///
/// - `txids` — accumulator for the merkle check; cleared and filled per call.
/// - `merkle_buf` — working buffer for the in-place merkle reduction.
/// - `seen_inputs` — dedup set for inputs within a single transaction.
/// - `seen_txids` — dedup set for the CVE-2012-2459 check.
///
/// # Errors
/// Returns the first `ValidationError` encountered, or
/// `ValidationError::MalformedTransaction` if the wire bytes are truncated
/// or corrupt.
///
/// # Panics
/// Never — the `.expect` on the merkle root is guarded by the non-empty check
/// at the top of the function.
pub fn validate_block_wire<S1, S2>(
    wire: &[u8],
    txids: &mut Vec<Hash256>,
    merkle_buf: &mut Vec<Hash256>,
    seen_inputs: &mut HashSet<OutPoint, S1>,
    seen_txids: &mut HashSet<Hash256, S2>,
) -> Result<(), ValidationError>
where
    S1: std::hash::BuildHasher,
    S2: std::hash::BuildHasher,
{
    let malformed_block = |_| ValidationError::MalformedTransaction {
        tx_index: usize::MAX,
    };

    let (header, rest) = BlockHeader::decode_ref(wire).map_err(malformed_block)?;

    let (mantle::codec::VarInt(tx_count_raw), mut rest) =
        mantle::codec::VarInt::decode_ref(rest).map_err(malformed_block)?;
    let tx_count = usize::try_from(tx_count_raw)
        .map_err(|_| ValidationError::TooManyTransactions { count: usize::MAX })?;

    if tx_count == 0 {
        return Err(ValidationError::EmptyBlock);
    }
    if tx_count > MAX_BLOCK_TRANSACTIONS {
        return Err(ValidationError::TooManyTransactions { count: tx_count });
    }

    // Single-pass per-transaction scan: parse + validate in one walk.
    let mut base_size = BlockHeader::ENCODED_LEN + VarInt(tx_count as u64).encoded_len();
    seen_inputs.clear();
    txids.clear();
    txids.reserve(tx_count);

    for tx_index in 0..tx_count {
        let (tx_raw_len, txid) = scan_tx_wire(rest, tx_index, seen_inputs)?;
        rest = &rest[tx_raw_len..];
        base_size += tx_raw_len;
        txids.push(txid);
    }

    let weight = base_size as u64 * WITNESS_SCALE_FACTOR;
    if weight > MAX_BLOCK_WEIGHT {
        return Err(ValidationError::BlockWeightExceeded { weight });
    }

    // CVE-2012-2459.
    seen_txids.clear();
    seen_txids.reserve(txids.len());
    for &id in txids.iter() {
        if !seen_txids.insert(id) {
            return Err(ValidationError::DuplicateTransaction);
        }
    }

    let computed_root =
        compute_merkle_root_reusing(txids, merkle_buf).expect("non-empty; checked above");
    if computed_root != header.merkle_root {
        return Err(ValidationError::MerkleRootMismatch);
    }

    header.check_pow()?;

    Ok(())
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Parse and validate one transaction from raw wire bytes in a single pass.
///
/// Returns `(tx_raw_len, txid)` on success. `tx_raw_len` is the number of
/// bytes consumed from `wire`; the caller advances its cursor by that amount.
///
/// Combines what `TransactionRef::decode_ref` + `check_tx_structure` would do
/// in two passes into one: inputs are decoded once and simultaneously checked
/// for coinbase rules and duplicates; outputs are decoded once and checked for
/// value bounds.
fn scan_tx_wire<S>(
    wire: &[u8],
    tx_index: usize,
    seen_inputs: &mut HashSet<OutPoint, S>,
) -> Result<(usize, Hash256), ValidationError>
where
    S: std::hash::BuildHasher,
{
    let malformed = |_: std::io::Error| ValidationError::MalformedTransaction { tx_index };

    let (_, buf) = i32::decode_ref(wire).map_err(malformed)?; // version

    let (VarInt(n), mut buf) = VarInt::decode_ref(buf).map_err(malformed)?;
    let input_count =
        usize::try_from(n).map_err(|_| ValidationError::MalformedTransaction { tx_index })?;
    if input_count == 0 {
        return Err(ValidationError::EmptyInputs { tx_index });
    }

    let expect_coinbase = tx_index == 0;
    let (first_in, buf_after_first) = TxInRef::decode_ref(buf).map_err(malformed)?;
    let first_is_coinbase = first_in.previous_output == NULL_OUTPOINT;

    if expect_coinbase && !first_is_coinbase {
        return Err(ValidationError::FirstTxNotCoinbase);
    }
    if !expect_coinbase && first_is_coinbase {
        return Err(ValidationError::UnexpectedCoinbaseInput { tx_index });
    }
    if expect_coinbase
        && !(MIN_COINBASE_SCRIPTSIG_LEN..=MAX_COINBASE_SCRIPTSIG_LEN)
            .contains(&first_in.script_sig.len())
    {
        return Err(ValidationError::CoinbaseScriptSigLength {
            len: first_in.script_sig.len(),
        });
    }

    buf = buf_after_first;
    if expect_coinbase {
        for _ in 1..input_count {
            let (_, next) = TxInRef::decode_ref(buf).map_err(malformed)?;
            buf = next;
        }
    } else {
        seen_inputs.clear();
        seen_inputs.reserve(input_count);
        seen_inputs.insert(first_in.previous_output);
        for _ in 1..input_count {
            let (inp, next) = TxInRef::decode_ref(buf).map_err(malformed)?;
            buf = next;
            if !seen_inputs.insert(inp.previous_output) {
                return Err(ValidationError::DuplicateInput { tx_index });
            }
        }
    }

    let (VarInt(m), mut buf) = VarInt::decode_ref(buf).map_err(malformed)?;
    let output_count =
        usize::try_from(m).map_err(|_| ValidationError::MalformedTransaction { tx_index })?;
    if output_count == 0 {
        return Err(ValidationError::EmptyOutputs { tx_index });
    }

    let mut total = Satoshi::ZERO;
    for output_index in 0..output_count {
        let (out, next) = TxOutRef::decode_ref(buf).map_err(malformed)?;
        buf = next;
        if out.value > MAX_MONEY {
            return Err(ValidationError::OutputValueOutOfRange {
                tx_index,
                output_index,
            });
        }
        total = total
            .checked_add(out.value)
            .filter(|&s| s <= MAX_MONEY)
            .ok_or(ValidationError::TotalOutputValueOverflow { tx_index })?;
    }

    let (_, rest) = u32::decode_ref(buf).map_err(malformed)?; // locktime
    let tx_raw_len = wire.len() - rest.len();
    Ok((tx_raw_len, Hash256(sha256d(&wire[..tx_raw_len]))))
}

/// Minimal view a transaction must expose to be structurally validated.
///
/// Implemented for both owned `Transaction` and zero-copy `TransactionRef<'_>`
/// so the check is written once and reused on either path. Iterators yield
/// `io::Result<_>` to handle truncated wire bytes uniformly; owned txs simply
/// produce `Ok` items unconditionally.
trait TxStructureView {
    fn input_count(&self) -> usize;
    fn output_count(&self) -> usize;
    fn first_input(&self) -> std::io::Result<(OutPoint, usize)>;
    fn for_each_output<E>(&self, f: impl FnMut(usize, Satoshi) -> Result<(), E>) -> Result<(), E>
    where
        E: From<std::io::Error>;
    fn for_each_input_outpoint<E>(&self, f: impl FnMut(OutPoint) -> Result<(), E>) -> Result<(), E>
    where
        E: From<std::io::Error>;
}

impl TxStructureView for Transaction {
    fn input_count(&self) -> usize {
        self.inputs.len()
    }

    fn output_count(&self) -> usize {
        self.outputs.len()
    }

    fn first_input(&self) -> std::io::Result<(OutPoint, usize)> {
        let i = &self.inputs[0];
        Ok((i.previous_output, i.script_sig.len()))
    }

    fn for_each_output<E>(
        &self,
        mut f: impl FnMut(usize, Satoshi) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<std::io::Error>,
    {
        for (i, o) in self.outputs.iter().enumerate() {
            f(i, o.value)?;
        }
        Ok(())
    }

    fn for_each_input_outpoint<E>(
        &self,
        mut f: impl FnMut(OutPoint) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<std::io::Error>,
    {
        for input in &self.inputs {
            f(input.previous_output)?;
        }
        Ok(())
    }
}

impl TxStructureView for TransactionRef<'_> {
    fn input_count(&self) -> usize {
        TransactionRef::input_count(self)
    }

    fn output_count(&self) -> usize {
        TransactionRef::output_count(self)
    }

    fn first_input(&self) -> std::io::Result<(OutPoint, usize)> {
        let r = self
            .inputs()
            .next()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))??;
        Ok((r.previous_output, r.script_sig.len()))
    }

    fn for_each_output<E>(
        &self,
        mut f: impl FnMut(usize, Satoshi) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<std::io::Error>,
    {
        for (i, r) in self.outputs().enumerate() {
            let o = r.map_err(E::from)?;
            f(i, o.value)?;
        }
        Ok(())
    }

    fn for_each_input_outpoint<E>(
        &self,
        mut f: impl FnMut(OutPoint) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<std::io::Error>,
    {
        for r in self.inputs() {
            let input = r.map_err(E::from)?;
            f(input.previous_output)?;
        }
        Ok(())
    }
}

fn check_tx_structure<T, S>(
    tx: &T,
    tx_index: usize,
    expect_coinbase: bool,
    seen_inputs: &mut HashSet<OutPoint, S>,
) -> Result<(), ValidationError>
where
    T: TxStructureView,
    S: std::hash::BuildHasher,
{
    let malformed = |_| ValidationError::MalformedTransaction { tx_index };

    if tx.input_count() == 0 {
        return Err(ValidationError::EmptyInputs { tx_index });
    }
    if tx.output_count() == 0 {
        return Err(ValidationError::EmptyOutputs { tx_index });
    }

    let (first_outpoint, first_script_len) = tx.first_input().map_err(malformed)?;
    let first_is_coinbase = first_outpoint == NULL_OUTPOINT;

    if expect_coinbase && !first_is_coinbase {
        return Err(ValidationError::FirstTxNotCoinbase);
    }
    if !expect_coinbase && first_is_coinbase {
        return Err(ValidationError::UnexpectedCoinbaseInput { tx_index });
    }

    if expect_coinbase
        && !(MIN_COINBASE_SCRIPTSIG_LEN..=MAX_COINBASE_SCRIPTSIG_LEN).contains(&first_script_len)
    {
        return Err(ValidationError::CoinbaseScriptSigLength {
            len: first_script_len,
        });
    }

    // Output value bounds.
    let mut total = Satoshi::ZERO;
    tx.for_each_output(|i, value| -> Result<(), ValidationError> {
        if value > MAX_MONEY {
            return Err(ValidationError::OutputValueOutOfRange {
                tx_index,
                output_index: i,
            });
        }
        total = total
            .checked_add(value)
            .filter(|&s| s <= MAX_MONEY)
            .ok_or(ValidationError::TotalOutputValueOverflow { tx_index })?;
        Ok(())
    })?;

    // Duplicate inputs within a single transaction.
    if !expect_coinbase {
        seen_inputs.clear();
        seen_inputs.reserve(tx.input_count());
        tx.for_each_input_outpoint(|outpoint| -> Result<(), ValidationError> {
            if !seen_inputs.insert(outpoint) {
                return Err(ValidationError::DuplicateInput { tx_index });
            }
            Ok(())
        })?;
    }

    Ok(())
}
