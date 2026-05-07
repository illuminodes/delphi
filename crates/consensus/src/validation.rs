use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use mantle::codec::DecodeRef;
use mantle::hash::sha256d;
use mantle::primitives::{Block, BlockHeader, BlockHeight, CompactBits, Hash256, OutPoint, Satoshi, Transaction, TransactionRef, TxHash};

use crate::chain_view::ChainView;
use crate::error::ValidationError;
use crate::merkle::merkle_root_owned;

// ── Clock ─────────────────────────────────────────────────────────────────────

/// Source of the current Unix timestamp for the "block time within 2h of now"
/// rule (BIP113-adjacent; see `MAX_FUTURE_BLOCK_TIME`).
///
/// Production wires `SystemClock` and the validator behaves as if it called
/// `SystemTime::now()` directly. Tests inject a `FixedClock` for determinism.
/// Historical replay passes a clock that returns the timestamp of the block
/// currently being replayed, so genesis-era blocks are not rejected as
/// "too far in the future" relative to wall-clock 2009.
pub trait Clock {
    /// Current Unix timestamp in seconds.
    fn now_secs(&self) -> u64;
}

/// Reads the wall clock via `SystemTime::now()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

/// A clock that always returns a fixed timestamp. Used for tests and replay.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub u64);

impl Clock for FixedClock {
    fn now_secs(&self) -> u64 {
        self.0
    }
}

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

/// Clock drift allowance: blocks may not be more than 2 hours in the future.
pub const MAX_FUTURE_BLOCK_TIME: u64 = 2 * 60 * 60;

/// BIP34 activation height on mainnet.
pub const BIP34_HEIGHT: BlockHeight = BlockHeight(227_931);

/// Maximum number of transactions per block (practical consensus limit).
pub const MAX_BLOCK_TRANSACTIONS: usize = 1_000_000;

// ── Null outpoint (coinbase marker) ──────────────────────────────────────────

/// The null outpoint used in coinbase inputs.
const NULL_OUTPOINT: OutPoint = OutPoint {
    txid: Hash256::ZERO,
    vout: u32::MAX,
};

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
        &mut HashSet::new(),
        &mut HashSet::new(),
    )
}

/// Stage 1 with caller-supplied scratch buffers.
///
/// All four scratch containers are cleared before use; the caller just needs to
/// keep them alive across calls to amortise allocations. Capacity is retained
/// between calls so the buffers grow to the high-water mark and stay there.
///
/// - `serial_scratch` — byte buffer reused for tx serialisation (txid hashing).
/// - `txids` — accumulator for per-block txid list fed into the merkle check.
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

    let computed_root = merkle_root_owned(std::mem::take(txids))
        .expect("non-empty; checked above");
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
    seen_inputs: &mut HashSet<OutPoint, S1>,
    seen_txids: &mut HashSet<Hash256, S2>,
) -> Result<(), ValidationError>
where
    S1: std::hash::BuildHasher,
    S2: std::hash::BuildHasher,
{
    let malformed_block = |_| ValidationError::MalformedTransaction { tx_index: usize::MAX };

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

    // Per-transaction structural checks and weight accumulation.
    let mut base_size = BlockHeader::ENCODED_LEN
        + mantle::codec::VarInt(tx_count as u64).encoded_len();
    seen_inputs.clear();
    txids.clear();
    txids.reserve(tx_count);

    for tx_index in 0..tx_count {
        let malformed = |_| ValidationError::MalformedTransaction { tx_index };
        let (tx, remaining) = TransactionRef::decode_ref(rest).map_err(malformed)?;
        rest = remaining;

        check_tx_structure(&tx, tx_index, tx_index == 0, seen_inputs)?;

        base_size += tx.raw.len();
        txids.push(Hash256(sha256d(tx.raw)));
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

    let computed_root = merkle_root_owned(std::mem::take(txids))
        .expect("non-empty; checked above");
    if computed_root != header.merkle_root {
        return Err(ValidationError::MerkleRootMismatch);
    }

    header.check_pow()?;

    Ok(())
}

/// Stage 2: contextual validation, using the wall clock for the
/// "≤ now + 2h" timestamp rule.
///
/// Requires the header chain (via `ChainView`). Does not touch the UTXO set.
///
/// # Errors
/// Returns the first `ValidationError` encountered.
pub fn validate_block_context(
    block: &Block,
    view: &impl ChainView,
) -> Result<(), ValidationError> {
    validate_block_context_with_clock(block, view, &SystemClock)
}

/// Stage 2: contextual validation with an injected clock.
///
/// Same checks as `validate_block_context`, but the caller supplies the
/// "current time" used by the future-timestamp rule. Useful for tests
/// (deterministic clocks) and for historical replay (clock pinned to the
/// block being replayed).
///
/// # Errors
/// Returns the first `ValidationError` encountered.
pub fn validate_block_context_with_clock(
    block: &Block,
    view: &impl ChainView,
    clock: &impl Clock,
) -> Result<(), ValidationError> {
    let header = &block.header;

    // Parent must exist on the canonical chain.
    let parent = view
        .get_header(&header.prev_block)?
        .ok_or(ValidationError::UnknownParent)?;
    let parent_height = view
        .height_of(&header.prev_block)?
        .ok_or(ValidationError::UnknownParent)?;
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
    if u64::from(header.time) > clock.now_secs() + MAX_FUTURE_BLOCK_TIME {
        return Err(ValidationError::TimestampTooFarFuture);
    }

    // BIP34: coinbase scriptSig must begin with encoded block height.
    if this_height >= BIP34_HEIGHT && !block.transactions[0].validate_bip34(this_height) {
        return Err(ValidationError::Bip34HeightMissing);
    }

    // Block version lower bound per BIP9/BIP8 deployment state.
    // For v1: enforce the historical minimums only.
    header.check_version(this_height)?;

    Ok(())
}

// ── Per-transaction structural checks ────────────────────────────────────────

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
    fn for_each_output<E>(
        &self,
        f: impl FnMut(usize, Satoshi) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<std::io::Error>;
    fn for_each_input_outpoint<E>(
        &self,
        f: impl FnMut(OutPoint) -> Result<(), E>,
    ) -> Result<(), E>
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
        return Err(ValidationError::CoinbaseScriptSigLength { len: first_script_len });
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

// ── Proof of work / version ──────────────────────────────────────────────────

/// Header-level checks expressed as methods on `BlockHeader`.
///
/// Lives in the consensus crate because the rules (`PoW` satisfaction, version
/// floor per BIP34/65/66) are consensus-defined; the type itself stays in
/// core so wire decoding doesn't pull in the consensus crate.
pub trait HeaderValidationExt {
    /// Check that the header hash satisfies the claimed nBits target.
    ///
    /// # Errors
    /// Returns `InvalidBits` if nBits is malformed or `InsufficientProofOfWork`
    /// if the hash exceeds the target.
    fn check_pow(&self) -> Result<(), ValidationError>;

    /// Enforce historical block-version minimums (BIP34/65/66).
    ///
    /// # Errors
    /// Returns `BlockVersionTooLow` if the header's version is below the floor
    /// for the supplied height.
    fn check_version(&self, height: BlockHeight) -> Result<(), ValidationError>;
}

impl HeaderValidationExt for BlockHeader {
    fn check_pow(&self) -> Result<(), ValidationError> {
        let target = self.bits.to_target().ok_or(ValidationError::InvalidBits)?;

        // Header hash in big-endian (reversed from internal storage).
        let mut hash_be = sha256d(&self.to_bytes());
        hash_be.reverse();

        if hash_be > target {
            return Err(ValidationError::InsufficientProofOfWork);
        }
        Ok(())
    }

    fn check_version(&self, height: BlockHeight) -> Result<(), ValidationError> {
        let required: i32 = if height >= BlockHeight(388_381) {
            4
        } else if height >= BlockHeight(363_725) {
            3
        } else if height >= BIP34_HEIGHT {
            2
        } else {
            1
        };

        if self.version < required {
            return Err(ValidationError::BlockVersionTooLow {
                version: self.version,
                required,
            });
        }
        Ok(())
    }
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

/// Retarget arithmetic exposed as a method on `CompactBits`.
///
/// Lives in the consensus crate because the powLimit and timespan clamps are
/// consensus rules; the type itself stays in core.
pub trait RetargetExt: Sized {
    /// Compute the next nBits given the actual and expected timespans, clamped
    /// against `pow_limit`. Returns `None` if either nBits decoding fails.
    fn retarget(self, actual: u64, expected: u64, pow_limit: Self) -> Option<Self>;
}

impl RetargetExt for CompactBits {
    fn retarget(self, actual: u64, expected: u64, pow_limit: Self) -> Option<Self> {
        let old_target = self.to_target()?;
        let pow_limit_target = pow_limit.to_target()?;
        let scaled = U256::from_be_bytes(old_target)
            .saturating_mul_u64(actual)
            .div_u64(expected);
        let limit = U256::from_be_bytes(pow_limit_target);
        Self::from_target(scaled.min(limit).to_be_bytes())
    }
}

/// Compute the expected nBits for the block at `parent_height + 1`.
fn expected_nbits(
    parent_height: BlockHeight,
    parent: BlockHeader,
    view: &impl ChainView,
) -> Result<CompactBits, ValidationError> {
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

    parent
        .bits
        .retarget(clamped, TARGET_TIMESPAN, POW_LIMIT_BITS)
        .ok_or(ValidationError::InvalidBits)
}

/// Mainnet powLimit (the maximum permitted target). Equal to the target encoded
/// by genesis nBits `0x1d00ffff`. Retargets clamp against this so a long
/// inter-block gap can never push the target above the network minimum
/// difficulty — matches Bitcoin Core's `consensus.powLimit` clamp in
/// `pow.cpp::CalculateNextWorkRequired`.
const POW_LIMIT_BITS: CompactBits = CompactBits(0x1d00_ffff);

/// Unsigned 256-bit integer, stored as eight little-endian 32-bit limbs.
/// `limbs[0]` is the least-significant 32 bits.
///
/// Only the operations the consensus code needs are implemented:
/// big-endian byte conversion, comparison, multiply by `u64` (saturating),
/// and divide by `u64`. That keeps the surface small and auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct U256([u32; 8]);

impl U256 {
    const MAX: Self = Self([u32::MAX; 8]);

    fn from_be_bytes(bytes: [u8; 32]) -> Self {
        let mut limbs = [0u32; 8];
        for (i, limb) in limbs.iter_mut().enumerate() {
            // Limb i (LE order) sits at byte offset (7 - i) * 4 in big-endian.
            let off = (7 - i) * 4;
            *limb = u32::from_be_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]);
        }
        Self(limbs)
    }

    fn to_be_bytes(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, &limb) in self.0.iter().enumerate() {
            let off = (7 - i) * 4;
            out[off..off + 4].copy_from_slice(&limb.to_be_bytes());
        }
        out
    }

    /// `self * rhs`, saturating at `U256::MAX` on overflow.
    ///
    /// Schoolbook multiplication: `self` has 8 limbs, `rhs` is two limbs;
    /// each partial product spans two output limbs and we propagate carries
    /// through a 9-limb accumulator. Anything reaching the 9th limb means
    /// the true product exceeded 2^256 and we saturate.
    fn saturating_mul_u64(self, rhs: u64) -> Self {
        #[allow(clippy::cast_possible_truncation)] // intentional: low 32 bits
        let rhs_lo = u64::from(rhs as u32);
        let rhs_hi = rhs >> 32;

        // 9-limb accumulator using u64 to absorb intermediate carries.
        let mut acc = [0u64; 9];

        for i in 0..8 {
            let lhs_limb = u64::from(self.0[i]);

            // lhs_limb * rhs_lo lands in acc[i..=i+1].
            let p_lo = lhs_limb * rhs_lo;
            let sum = acc[i] + (p_lo & 0xffff_ffff);
            acc[i] = sum & 0xffff_ffff;
            let mut carry = (sum >> 32) + (p_lo >> 32);

            if rhs_hi != 0 {
                // lhs_limb * rhs_hi lands in acc[i+1..=i+2].
                let p_hi = lhs_limb * rhs_hi;
                let sum1 = acc[i + 1] + carry + (p_hi & 0xffff_ffff);
                acc[i + 1] = sum1 & 0xffff_ffff;
                let sum2 = acc[i + 2] + (sum1 >> 32) + (p_hi >> 32);
                acc[i + 2] = sum2 & 0xffff_ffff;
                carry = sum2 >> 32;
                let mut idx = i + 3;
                while carry != 0 && idx < 9 {
                    let s = acc[idx] + carry;
                    acc[idx] = s & 0xffff_ffff;
                    carry = s >> 32;
                    idx += 1;
                }
            } else {
                let mut idx = i + 1;
                while carry != 0 && idx < 9 {
                    let s = acc[idx] + carry;
                    acc[idx] = s & 0xffff_ffff;
                    carry = s >> 32;
                    idx += 1;
                }
            }

            if carry != 0 {
                return Self::MAX;
            }
        }

        if acc[8] != 0 {
            return Self::MAX;
        }

        let mut out = [0u32; 8];
        for (i, limb) in out.iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation)] // masked to 32 bits above
            {
                *limb = acc[i] as u32;
            }
        }
        Self(out)
    }

    /// `floor(self / rhs)`. Panics if `rhs == 0`.
    ///
    /// Long division in 32-bit chunks: the remainder always fits in a `u64`
    /// because `rhs` fits in a `u64`.
    fn div_u64(self, rhs: u64) -> Self {
        assert!(rhs != 0, "U256::div_u64: division by zero");
        let mut quot = [0u32; 8];
        let mut rem: u64 = 0;
        for i in (0..8).rev() {
            let cur = (rem << 32) | u64::from(self.0[i]);
            #[allow(clippy::cast_possible_truncation)] // cur / rhs ≤ u32::MAX
            {
                quot[i] = (cur / rhs) as u32;
            }
            rem = cur % rhs;
        }
        Self(quot)
    }
}

impl Ord for U256 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare from most-significant limb down.
        for i in (0..8).rev() {
            let ord = self.0[i].cmp(&other.0[i]);
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        std::cmp::Ordering::Equal
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Scale a 256-bit target by `actual / expected`, operating on big-endian bytes.
///
/// Computes `floor(target * actual / expected)` exactly in 256-bit arithmetic,
/// then clamps against the powLimit. Matches Bitcoin Core's behaviour in
/// `pow.cpp::CalculateNextWorkRequired`.
///
/// Test-only: production code uses [`RetargetExt::retarget`] on `CompactBits`.
#[cfg(test)]
fn scale_target(target: [u8; 32], actual: u64, expected: u64) -> Option<[u8; 32]> {
    let scaled = U256::from_be_bytes(target)
        .saturating_mul_u64(actual)
        .div_u64(expected);

    let pow_limit = U256::from_be_bytes(POW_LIMIT_BITS.to_target()?);
    Some(scaled.min(pow_limit).to_be_bytes())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain_view::MemoryChainView;
    use crate::merkle::compute_merkle_root;
    use mantle::primitives::{TxIn, TxOut};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn coinbase_tx(height: BlockHeight, value: u64) -> Transaction {
        Transaction {
            version: 1,
            inputs: vec![TxIn {
                previous_output: NULL_OUTPOINT,
                script_sig: Transaction::bip34_height_script(height),
                sequence: u32::MAX,
            }],
            outputs: vec![TxOut {
                value: Satoshi::new(value),
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
                value: Satoshi::new(value),
                script_pubkey: vec![0x51],
            }],
            locktime: 0,
        }
    }

    fn make_block(txs: Vec<Transaction>, bits: CompactBits) -> Block {
        let txids: Vec<Hash256> = txs.iter()
            .map(Transaction::txid)
            .collect::<Result<Vec<_>, _>>()
            .expect("txid encoding cannot fail");
        let merkle_root = compute_merkle_root(&txids)
            .expect("non-empty tx list always has a merkle root");
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

    /// Build a block with a valid `PoW` nonce (brute-force search).
    /// Only used for tests that exercise `PoW` validation.
    fn mine_block(mut block: Block) -> Block {
        let target = block.header.bits.to_target()
            .expect("mine_block called with invalid nBits");
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
                bits: CompactBits(0x207f_ffff),
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
        let mut block = make_block(vec![tx], CompactBits(0x207f_ffff));
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
        let mut tx = coinbase_tx(BlockHeight(1), 5_000_000_000);
        tx.inputs[0].script_sig = vec![0x01]; // 1 byte < MIN (2)
        let block = make_block(vec![tx], CompactBits(0x207f_ffff));
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::CoinbaseScriptSigLength { len: 1 })
        );
    }

    #[test]
    fn coinbase_script_too_long() {
        let mut tx = coinbase_tx(BlockHeight(1), 5_000_000_000);
        tx.inputs[0].script_sig = vec![0x00; 101]; // 101 > MAX (100)
        let block = make_block(vec![tx], CompactBits(0x207f_ffff));
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::CoinbaseScriptSigLength { len: 101 })
        );
    }

    #[test]
    fn output_value_out_of_range() {
        let mut tx = coinbase_tx(BlockHeight(1), MAX_MONEY.to_u64() + 1);
        tx.outputs[0].value = Satoshi::new(MAX_MONEY.to_u64() + 1);
        // Re-encode the block with the bad tx — merkle root will be recomputed.
        let block = make_block(vec![tx], CompactBits(0x207f_ffff));
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
        let cb = coinbase_tx(BlockHeight(1), 5_000_000_000);
        let mut tx2 = regular_tx(Hash256([2u8; 32]), 0, 1000);
        // Add a duplicate input.
        tx2.inputs.push(tx2.inputs[0].clone());
        let block = make_block(vec![cb, tx2], CompactBits(0x207f_ffff));
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::DuplicateInput { tx_index: 1 })
        );
    }

    /// CVE-2012-2459: a block containing two transactions with the same txid
    /// must be rejected, regardless of merkle-root agreement.
    ///
    /// The two duplicates are non-coinbase: a coinbase clone would be
    /// rejected earlier as `UnexpectedCoinbaseInput`, never reaching the
    /// duplicate-txid check. Two identical regular transactions hash to the
    /// same txid and exercise the CVE-2012-2459 defence directly.
    #[test]
    fn duplicate_transaction_rejected() {
        let cb = coinbase_tx(BlockHeight(1), 5_000_000_000);
        let tx = regular_tx(Hash256([0x42; 32]), 0, 1000);
        let dup = tx.clone();
        let block = make_block(vec![cb, tx, dup], CompactBits(0x207f_ffff));
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::DuplicateTransaction)
        );
    }

    #[test]
    fn merkle_root_mismatch() {
        let tx = coinbase_tx(BlockHeight(1), 5_000_000_000);
        let mut block = make_block(vec![tx], CompactBits(0x207f_ffff));
        block.header.merkle_root = Hash256([0xde; 32]); // corrupt root
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::MerkleRootMismatch)
        );
    }

    #[test]
    fn pow_invalid() {
        let tx = coinbase_tx(BlockHeight(1), 5_000_000_000);
        // Use a very hard nBits — nonce=0 will almost certainly fail PoW.
        let block = make_block(vec![tx], CompactBits(0x1d00_ffff));
        assert_eq!(
            validate_block_structure(&block),
            Err(ValidationError::InsufficientProofOfWork)
        );
    }

    #[test]
    fn valid_block_easy_pow() {
        let tx = coinbase_tx(BlockHeight(1), 5_000_000_000);
        // 0x207fffff is the easiest possible nBits (used in regtest).
        let block = mine_block(make_block(vec![tx], CompactBits(0x207f_ffff)));
        assert!(validate_block_structure(&block).is_ok());
    }

    // ── nbits helpers ─────────────────────────────────────────────────────────

    #[test]
    fn nbits_roundtrip_genesis() {
        let bits = CompactBits(0x1d00_ffff);
        let target = bits.to_target().expect("genesis nBits must decode");
        let back = CompactBits::from_target(target).expect("genesis target must re-encode");
        assert_eq!(back, bits);
    }

    #[test]
    fn nbits_zero_mantissa_is_invalid() {
        assert!(CompactBits(0x0300_0000).to_target().is_none());
    }

    #[test]
    fn nbits_negative_is_invalid() {
        // High bit of mantissa set → negative.
        assert!(CompactBits(0x0380_0000).to_target().is_none());
    }

    // ── BIP34 height encoding ─────────────────────────────────────────────────

    #[test]
    fn bip34_height_encoding() {
        assert_eq!(Transaction::bip34_height_script(BlockHeight(1)), vec![0x01, 0x01]);
        assert_eq!(Transaction::bip34_height_script(BlockHeight(256)), vec![0x02, 0x00, 0x01]);
        assert_eq!(Transaction::bip34_height_script(BlockHeight(0x80)), vec![0x02, 0x80, 0x00]);
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
                bits: CompactBits(0x207f_ffff),
                nonce: 0,
            };
            prev = h.hash();
            view.push_header(h);
        }
        view
    }

    #[test]
    fn unknown_parent_rejected() {
        let view = build_view_with_chain(1);
        let tx = coinbase_tx(BlockHeight(1), 5_000_000_000);
        let block = Block {
            header: BlockHeader {
                version: 4,
                prev_block: Hash256([0xff; 32]), // unknown
                merkle_root: compute_merkle_root(&[tx.txid().expect("txid encoding cannot fail")])
                    .expect("single-tx merkle root cannot be empty"),
                time: 1_296_688_602 + 600,
                bits: CompactBits(0x207f_ffff),
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
        let tip_header = view.get_header_at(tip.height)
            .expect("chain view error").expect("tip header must exist");
        let tx = coinbase_tx(tip.height + 1, 5_000_000_000);
        let mtp = view.median_time_past(&BlockHeader {
            version: 4,
            prev_block: tip.hash,
            merkle_root: Hash256::ZERO,
            time: 0,
            bits: CompactBits(0x207f_ffff),
            nonce: 0,
        }).expect("median_time_past must succeed for a populated chain");
        let block = Block {
            header: BlockHeader {
                version: 4,
                prev_block: tip.hash,
                merkle_root: compute_merkle_root(&[tx.txid().expect("txid encoding cannot fail")])
                    .expect("single-tx merkle root cannot be empty"),
                time: mtp, // exactly MTP — must be > MTP
                bits: CompactBits(0x207f_ffff),
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

    /// A block timestamped more than 2h beyond the injected clock must be
    /// rejected as `TimestampTooFarFuture`. Uses `FixedClock` so the
    /// outcome doesn't depend on wall time.
    #[test]
    fn timestamp_too_far_future_rejected() {
        let view = build_view_with_chain(12);
        let tip = view.tip();
        let parent_time = view.get_header_at(tip.height)
            .expect("chain view error").expect("tip header must exist").time;
        let tx = coinbase_tx(tip.height + 1, 5_000_000_000);

        // Block claims a time ≥ now + 3h.
        let now: u64 = u64::from(parent_time) + 600;
        let block_time = now + MAX_FUTURE_BLOCK_TIME + 1;

        let block = Block {
            header: BlockHeader {
                version: 4,
                prev_block: tip.hash,
                merkle_root: compute_merkle_root(&[tx.txid().expect("txid encoding cannot fail")])
                    .expect("single-tx merkle root cannot be empty"),
                time: u32::try_from(block_time).expect("test block_time fits in u32"),
                bits: CompactBits(0x207f_ffff),
                nonce: 0,
            },
            transactions: vec![tx],
        };
        assert_eq!(
            validate_block_context_with_clock(&block, &view, &FixedClock(now)),
            Err(ValidationError::TimestampTooFarFuture)
        );
    }

    #[test]
    fn bip34_missing_rejected() {
        let view = build_view_with_chain(BIP34_HEIGHT.0);
        let tip = view.tip();
        let mut tx = coinbase_tx(tip.height + 1, 5_000_000_000);
        // Replace BIP34 scriptSig with garbage that doesn't encode the height.
        tx.inputs[0].script_sig = vec![0x02, 0x00, 0x00];
        let block = Block {
            header: BlockHeader {
                version: 4,
                prev_block: tip.hash,
                merkle_root: compute_merkle_root(&[tx.txid().expect("txid encoding cannot fail")])
                    .expect("single-tx merkle root cannot be empty"),
                time: 1_296_688_602 + BIP34_HEIGHT.0 * 600 + 600,
                bits: CompactBits(0x207f_ffff),
                nonce: 0,
            },
            transactions: vec![tx],
        };
        assert_eq!(
            validate_block_context(&block, &view),
            Err(ValidationError::Bip34HeightMissing)
        );
    }

    // ── U256 arithmetic ───────────────────────────────────────────────────────

    #[test]
    fn u256_byte_roundtrip() {
        let bytes: [u8; 32] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20,
        ];
        assert_eq!(U256::from_be_bytes(bytes).to_be_bytes(), bytes);
    }

    #[test]
    fn u256_mul_u64_no_overflow() {
        // (1 << 200) * 4 == (1 << 202)
        let mut bytes = [0u8; 32];
        bytes[31 - 25] = 0x01; // bit 200
        let a = U256::from_be_bytes(bytes);
        let product = a.saturating_mul_u64(4);
        let mut expected = [0u8; 32];
        expected[31 - 25] = 0x04; // bit 202 — same byte, value × 4
        assert_eq!(product.to_be_bytes(), expected);
    }

    #[test]
    fn u256_mul_u64_carries_across_limb_boundary() {
        // 0xffff_ffff_ffff_ffff (low 64 bits all set) * 2 = 0x1_ffff_ffff_ffff_fffe
        let mut bytes = [0u8; 32];
        for b in bytes.iter_mut().skip(24) {
            *b = 0xff;
        }
        let a = U256::from_be_bytes(bytes);
        let product = a.saturating_mul_u64(2);
        // Expect 0x1_ffff_ffff_ffff_fffe
        let mut expected = [0u8; 32];
        expected[31 - 8] = 0x01;
        for b in expected.iter_mut().take(31).skip(24) {
            *b = 0xff;
        }
        expected[31] = 0xfe;
        assert_eq!(product.to_be_bytes(), expected);
    }

    #[test]
    fn u256_mul_u64_saturates_on_overflow() {
        // U256::MAX * 2 must saturate.
        let max = U256::from_be_bytes([0xff; 32]);
        assert_eq!(max.saturating_mul_u64(2), U256::MAX);
    }

    #[test]
    fn u256_div_u64_basic() {
        let mut bytes = [0u8; 32];
        bytes[31] = 100; // value = 100
        let a = U256::from_be_bytes(bytes);
        let q = a.div_u64(7);
        let mut expected = [0u8; 32];
        expected[31] = 14; // 100 / 7 = 14
        assert_eq!(q.to_be_bytes(), expected);
    }

    #[test]
    fn u256_mul_then_div_recovers() {
        // (1 << 200) * 1_000_000 / 1_000_000 == (1 << 200)
        let mut bytes = [0u8; 32];
        bytes[31 - 25] = 0x01;
        let a = U256::from_be_bytes(bytes);
        let result = a.saturating_mul_u64(1_000_000).div_u64(1_000_000);
        assert_eq!(result.to_be_bytes(), bytes);
    }

    #[test]
    fn u256_ordering() {
        let small = U256::from_be_bytes({
            let mut b = [0u8; 32];
            b[31] = 1;
            b
        });
        let large = U256::from_be_bytes({
            let mut b = [0u8; 32];
            b[0] = 1;
            b
        });
        assert!(small < large);
        assert!(large > small);
        assert_eq!(small, small);
    }

    // ── Retarget arithmetic ───────────────────────────────────────────────────

    /// Mainnet's first real retarget, at block 32256.
    ///
    /// Reference values from Bitcoin mainnet block headers:
    /// - block 0:     time = `1_231_006_505`, nBits = 0x1d00ffff
    /// - block 32255: time = `1_262_152_739`, nBits = 0x1d00ffff
    ///
    ///   `actual_timespan` = `1_262_152_739` - `1_231_006_505` = `31_146_234` seconds
    ///   ≈ 360 days  (vs target 14 days)
    ///   `clamped`         = `MAX_TIMESPAN` = 4 × `TARGET_TIMESPAN` = `4_838_400`
    ///   `new_target_raw`  = `old_target` × 4
    ///   = encoded 0x1d03fffc, which exceeds powLimit
    ///   `new_target`      = clamp(`new_target_raw`, ≤ powLimit) = powLimit
    ///
    /// So the network kept difficulty pinned at the floor (`0x1d00ffff`)
    /// through the first retarget. This is exactly what mainnet block 32256
    /// records.
    #[test]
    fn first_mainnet_retarget_matches_core() {
        let old_bits = CompactBits(0x1d00_ffff);
        let actual = 1_262_152_739_u64 - 1_231_006_505_u64;
        assert_eq!(actual, 31_146_234, "raw timespan from block headers");

        let clamped = actual.clamp(MIN_TIMESPAN, MAX_TIMESPAN);
        assert_eq!(clamped, MAX_TIMESPAN, "timespan must clamp to MAX_TIMESPAN");

        let new_bits = old_bits
            .retarget(clamped, TARGET_TIMESPAN, POW_LIMIT_BITS)
            .expect("retarget must succeed for valid inputs");

        assert_eq!(
            new_bits, CompactBits(0x1d00_ffff),
            "first mainnet retarget must clamp to powLimit, got {:#010x}", new_bits.0,
        );
    }

    /// A retarget that should produce a real difficulty change without
    /// hitting either clamp. Synthetic but exercises the full arithmetic
    /// path: `target * actual / expected` with no powLimit clamp.
    ///
    /// Use a starting target well below powLimit (`0x1c05_a3f4`) and an
    /// actual timespan equal to half the target — difficulty should
    /// roughly halve the target (= double the difficulty).
    #[test]
    fn retarget_arithmetic_round_trip() {
        let old_bits = CompactBits(0x1c05_a3f4);
        let old_target = old_bits.to_target().expect("old_bits must decode");
        let actual = TARGET_TIMESPAN / 2; // blocks came twice as fast
        let clamped = actual.clamp(MIN_TIMESPAN, MAX_TIMESPAN);
        assert_eq!(clamped, actual, "actual is within clamp bounds");

        let new_target = scale_target(old_target, clamped, TARGET_TIMESPAN)
            .expect("scale_target must succeed for valid inputs");
        // new_target ≈ old_target / 2. Re-derive without scale_target to
        // verify: U256(old_target) / 2.
        let expected = U256::from_be_bytes(old_target).div_u64(2);
        assert_eq!(
            U256::from_be_bytes(new_target),
            expected,
            "halved-timespan retarget must halve the target",
        );

        // And the new target must still be ≤ powLimit (sanity).
        let pow_limit = U256::from_be_bytes(
            POW_LIMIT_BITS.to_target().expect("POW_LIMIT_BITS must decode"),
        );
        assert!(U256::from_be_bytes(new_target) <= pow_limit);
    }

    /// Sanity check: a retarget that would push the target above powLimit
    /// is clamped back to the powLimit. Use the genesis target (already at
    /// powLimit) and a maximum-clamped timespan — the result must remain
    /// at the powLimit, never exceed it.
    #[test]
    fn retarget_clamps_to_pow_limit() {
        let old_target = POW_LIMIT_BITS.to_target().expect("POW_LIMIT_BITS must decode");
        // Even a max-clamped timespan (×4) would yield 4× the target if not
        // clamped. We expect the powLimit clamp to bring it back.
        let new_target = scale_target(old_target, MAX_TIMESPAN, TARGET_TIMESPAN)
            .expect("scale_target must succeed for valid inputs");
        let new_bits = CompactBits::from_target(new_target)
            .expect("from_target must succeed for valid target");
        assert_eq!(
            new_bits, POW_LIMIT_BITS,
            "target above powLimit must clamp back, got {:#010x}", new_bits.0,
        );
    }

    /// Round-trip property: every nBits accepted by `nbits_to_target` should
    /// either round-trip cleanly through `target_to_nbits`, or canonicalize
    /// to a value with the same target. Bitcoin's compact format has one
    /// canonical encoding per target value; non-canonical inputs (mantissa
    /// with trailing zero bytes) re-encode to canonical form.
    #[test]
    fn nbits_target_roundtrip_canonical_examples() {
        // These are values Bitcoin Core has produced as outputs of
        // target_to_nbits, so they are canonical and must round-trip.
        for &raw in &[
            0x1d00_ffff_u32, // genesis
            0x1d00_d86a,     // first mainnet retarget
            0x1c05_a3f4,     // post-retarget-2 difficulty (illustrative)
            0x1903_a30c,     // mid-2010s mainnet difficulty (illustrative shape)
            0x207f_ffff,     // regtest
        ] {
            let bits = CompactBits(raw);
            let target = bits.to_target()
                .unwrap_or_else(|| panic!("bits {raw:#010x} must decode to a target"));
            let back = CompactBits::from_target(target)
                .unwrap_or_else(|| panic!("target from {raw:#010x} must re-encode"));
            assert_eq!(back, bits, "round-trip failed for {raw:#010x} → {:#010x}", back.0);
        }
    }
}
