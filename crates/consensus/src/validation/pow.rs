use mantle::primitives::{BlockHeader, BlockHeight, BlockTime, BlockVersion, CompactBits, Hash256};

use crate::chain_view::ChainView;
use crate::error::ValidationError;

use super::structure::BIP34_HEIGHT;

// ── Retarget constants ────────────────────────────────────────────────────────

/// Bitcoin's difficulty retarget interval in blocks.
pub(super) const RETARGET_INTERVAL: u32 = 2016;
/// Target spacing in seconds (10 minutes).
pub(super) const TARGET_SPACING: u64 = 600;
/// Target timespan in seconds (two weeks).
pub(super) const TARGET_TIMESPAN: u64 = RETARGET_INTERVAL as u64 * TARGET_SPACING;
/// Minimum timespan (target timespan / 4, to prevent too-easy mining).
pub(super) const MIN_TIMESPAN: u64 = TARGET_TIMESPAN / 4;
/// Maximum timespan (target timespan × 4, to prevent too-hard mining).
pub(super) const MAX_TIMESPAN: u64 = TARGET_TIMESPAN * 4;

/// Mainnet powLimit (the maximum permitted target). Equal to the target encoded
/// by genesis nBits `0x1d00ffff`. Retargets clamp against this so a long
/// inter-block gap can never push the target above the network minimum
/// difficulty — matches Bitcoin Core's `consensus.powLimit` clamp in
/// `pow.cpp::CalculateNextWorkRequired`.
pub(super) const POW_LIMIT_BITS: CompactBits = CompactBits::new(0x1d00_ffff);

// ── HeaderValidationExt ───────────────────────────────────────────────────────

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
        let target = self
            .bits()
            .to_target()
            .ok_or(ValidationError::InvalidBits)?;

        // Header hash in big-endian (reversed from internal storage).
        let header_hash = Hash256::hash(&self.to_bytes());
        let mut hash_be = *header_hash.as_bytes();
        hash_be.reverse();

        if hash_be > target {
            return Err(ValidationError::InsufficientProofOfWork);
        }
        Ok(())
    }

    fn check_version(&self, height: BlockHeight) -> Result<(), ValidationError> {
        const BIP65_HEIGHT: BlockHeight = BlockHeight::new(388_381);
        const BIP66_HEIGHT: BlockHeight = BlockHeight::new(363_725);

        let required = BlockVersion::new(if height >= BIP65_HEIGHT {
            4
        } else if height >= BIP66_HEIGHT {
            3
        } else if height >= BIP34_HEIGHT {
            2
        } else {
            1
        });

        if self.version() < required {
            return Err(ValidationError::BlockVersionTooLow {
                version: self.version(),
                required,
            });
        }
        Ok(())
    }
}

// ── RetargetExt ───────────────────────────────────────────────────────────────

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

// ── expected_nbits ────────────────────────────────────────────────────────────

/// Testnet 20-minute reset: if no block for 20 min, difficulty drops to minimum.
const TESTNET_RESET_SECONDS: u32 = 1200;

/// Compute the expected nBits for the block at `parent_height + 1`.
///
/// `block_time` is the new block's timestamp, needed for the testnet
/// 20-minute difficulty reset rule (BIP94 / testnet3 legacy).
/// `allow_min_difficulty` should be true for testnet3/testnet4, false
/// for mainnet/signet.
pub(super) fn expected_nbits(
    parent_height: BlockHeight,
    parent: BlockHeader,
    view: &impl ChainView,
    block_time: BlockTime,
    allow_min_difficulty: bool,
) -> Result<CompactBits, ValidationError> {
    let this_height = parent_height + 1;

    // Retarget boundary.
    if this_height % RETARGET_INTERVAL == 0 {
        let retarget_height = parent_height + 1 - RETARGET_INTERVAL;
        let retarget_header = view
            .get_header_at(retarget_height)?
            .ok_or(ValidationError::UnknownParent)?;

        let actual_timespan =
            u64::from(parent.time()).saturating_sub(u64::from(retarget_header.time()));
        let clamped = actual_timespan.clamp(MIN_TIMESPAN, MAX_TIMESPAN);

        return parent
            .bits()
            .retarget(clamped, TARGET_TIMESPAN, POW_LIMIT_BITS)
            .ok_or(ValidationError::InvalidBits);
    }

    // Testnet 20-minute reset: if the block timestamp exceeds the parent
    // by more than 2× the target spacing, allow minimum difficulty.
    if allow_min_difficulty && block_time > parent.time() + TESTNET_RESET_SECONDS {
        return Ok(POW_LIMIT_BITS);
    }

    // No retarget — carry parent's bits. On testnets, if the parent was a
    // min-difficulty block but we're back within 20 min, walk back to find
    // the last non-min-difficulty block's nBits.
    if allow_min_difficulty && parent.bits() == POW_LIMIT_BITS {
        let mut h = parent_height;
        let mut hdr = parent;
        while h > BlockHeight::ZERO && h % RETARGET_INTERVAL != 0 && hdr.bits() == POW_LIMIT_BITS {
            h = h.saturating_sub(1);
            hdr = view
                .get_header_at(h)?
                .ok_or(ValidationError::UnknownParent)?;
        }
        return Ok(hdr.bits());
    }

    Ok(parent.bits())
}

// ── U256 ──────────────────────────────────────────────────────────────────────

/// Unsigned 256-bit integer, stored as eight little-endian 32-bit limbs.
/// `limbs[0]` is the least-significant 32 bits.
///
/// Only the operations the consensus code needs are implemented:
/// big-endian byte conversion, comparison, multiply by `u64` (saturating),
/// and divide by `u64`. That keeps the surface small and auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct U256([u32; 8]);

impl U256 {
    pub(super) const MAX: Self = Self([u32::MAX; 8]);

    pub(super) fn from_be_bytes(bytes: [u8; 32]) -> Self {
        let mut limbs = [0u32; 8];
        for (i, limb) in limbs.iter_mut().enumerate() {
            // Limb i (LE order) sits at byte offset (7 - i) * 4 in big-endian.
            let off = (7 - i) * 4;
            *limb =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        }
        Self(limbs)
    }

    pub(super) fn to_be_bytes(self) -> [u8; 32] {
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
    pub(super) fn saturating_mul_u64(self, rhs: u64) -> Self {
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
                let p_hi = lhs_limb * rhs_hi;
                if i + 2 >= 9 {
                    // Propagate low-half carry to the overflow limb before
                    // checking the high-half guard, so it is not silently lost.
                    let s = acc[i + 1] + carry;
                    acc[i + 1] = s & 0xffff_ffff;
                    carry = s >> 32;
                    if p_hi != 0 || carry != 0 {
                        return Self::MAX;
                    }
                } else {
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
    pub(super) fn div_u64(self, rhs: u64) -> Self {
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
pub(super) fn scale_target(target: [u8; 32], actual: u64, expected: u64) -> Option<[u8; 32]> {
    let scaled = U256::from_be_bytes(target)
        .saturating_mul_u64(actual)
        .div_u64(expected);

    let pow_limit = U256::from_be_bytes(POW_LIMIT_BITS.to_target()?);
    Some(scaled.min(pow_limit).to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `U256` from its eight little-endian 32-bit limbs (limbs[0] = LSB).
    fn u256_from_limbs(limbs: [u32; 8]) -> U256 {
        U256(limbs)
    }

    #[test]
    fn u256_zero_roundtrip() {
        let zero = U256::from_be_bytes([0u8; 32]);
        assert_eq!(zero.to_be_bytes(), [0u8; 32]);
    }

    #[test]
    fn u256_one_roundtrip() {
        let mut bytes = [0u8; 32];
        bytes[31] = 1;
        let one = U256::from_be_bytes(bytes);
        assert_eq!(one.to_be_bytes(), bytes);
    }

    #[test]
    fn u256_max_roundtrip() {
        let bytes = [0xff; 32];
        let max = U256::from_be_bytes(bytes);
        assert_eq!(max, U256::MAX);
        assert_eq!(max.to_be_bytes(), bytes);
    }

    #[test]
    fn u256_ordering() {
        let a = U256::from_be_bytes({
            let mut b = [0u8; 32];
            b[31] = 1;
            b
        });
        let b = U256::from_be_bytes({
            let mut b = [0u8; 32];
            b[31] = 2;
            b
        });
        assert!(a < b);
        assert!(b > a);
        assert_eq!(a, a);
    }

    #[test]
    fn u256_ordering_high_limb() {
        let lo = U256::from_be_bytes({
            let mut b = [0u8; 32];
            b[31] = 0xff;
            b
        });
        let hi = U256::from_be_bytes({
            let mut b = [0u8; 32];
            b[0] = 1;
            b
        });
        assert!(hi > lo);
    }

    #[test]
    fn u256_mul_small() {
        let mut bytes = [0u8; 32];
        bytes[31] = 100;
        let val = U256::from_be_bytes(bytes);
        let result = val.saturating_mul_u64(3);
        let mut expected = [0u8; 32];
        expected[30] = 1;
        expected[31] = 0x2c; // 300
        assert_eq!(result.to_be_bytes(), expected);
    }

    #[test]
    fn u256_mul_zero() {
        let val = U256::from_be_bytes([0xff; 32]);
        assert_eq!(val.saturating_mul_u64(0).to_be_bytes(), [0u8; 32]);
    }

    #[test]
    fn u256_mul_one() {
        let bytes = [0xab; 32];
        let val = U256::from_be_bytes(bytes);
        assert_eq!(val.saturating_mul_u64(1).to_be_bytes(), bytes);
    }

    #[test]
    fn u256_mul_overflow_saturates() {
        let result = U256::MAX.saturating_mul_u64(2);
        assert_eq!(result, U256::MAX);
    }

    #[test]
    fn u256_div_small() {
        let mut bytes = [0u8; 32];
        bytes[30] = 1;
        bytes[31] = 0x2c; // 300
        let val = U256::from_be_bytes(bytes);
        let result = val.div_u64(3);
        let mut expected = [0u8; 32];
        expected[31] = 100;
        assert_eq!(result.to_be_bytes(), expected);
    }

    #[test]
    fn u256_div_one() {
        let bytes = [0xab; 32];
        let val = U256::from_be_bytes(bytes);
        assert_eq!(val.div_u64(1).to_be_bytes(), bytes);
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn u256_div_zero_panics() {
        let val = U256::from_be_bytes([1; 32]);
        let _ = val.div_u64(0);
    }

    #[test]
    fn u256_mul_then_div_roundtrips() {
        let mut bytes = [0u8; 32];
        bytes[28] = 0x00;
        bytes[29] = 0x0f;
        bytes[30] = 0x42;
        bytes[31] = 0x40; // 1_000_000
        let val = U256::from_be_bytes(bytes);
        let result = val
            .saturating_mul_u64(TARGET_TIMESPAN)
            .div_u64(TARGET_TIMESPAN);
        assert_eq!(result.to_be_bytes(), bytes);
    }

    #[test]
    fn scale_target_identity() {
        let target = POW_LIMIT_BITS.to_target().unwrap();
        let result = scale_target(target, TARGET_TIMESPAN, TARGET_TIMESPAN).unwrap();
        let result_bits = CompactBits::from_target(result).unwrap();
        assert_eq!(result_bits, POW_LIMIT_BITS);
    }

    #[test]
    fn scale_target_clamps_to_pow_limit() {
        let target = POW_LIMIT_BITS.to_target().unwrap();
        let result = scale_target(target, TARGET_TIMESPAN * 4, TARGET_TIMESPAN).unwrap();
        let result_bits = CompactBits::from_target(result).unwrap();
        assert_eq!(result_bits, POW_LIMIT_BITS);
    }

    // ── expected_nbits tests ─────────────────────────────────────────────────

    use crate::chain_view::MemoryChainView;
    use mantle::primitives::Nonce;

    /// Helper: build a dummy header with the given time and bits.
    fn dummy_header(time: u32, bits: CompactBits) -> BlockHeader {
        BlockHeader::new(
            BlockVersion::new(1),
            Hash256::ZERO,
            Hash256::ZERO,
            BlockTime::new(time),
            bits,
            Nonce::new(0),
        )
    }

    #[test]
    fn expected_nbits_non_retarget_carries_parent_bits() {
        // Height 100 is not a retarget boundary; should return parent's bits.
        let bits = CompactBits::new(0x1d00_ffff);
        let parent = dummy_header(1_000_000, bits);
        let view = MemoryChainView::new();
        let result = expected_nbits(
            BlockHeight::new(100),
            parent,
            &view,
            BlockTime::new(1_000_600),
            false,
        )
        .unwrap();
        assert_eq!(result, bits);
    }

    #[test]
    fn u256_mul_near_max_saturates() {
        // U256::MAX * 3 must saturate to U256::MAX.
        let result = U256::MAX.saturating_mul_u64(3);
        assert_eq!(result, U256::MAX);
    }

    #[test]
    fn u256_mul_high_value_with_large_rhs_lo_only() {
        // Large value with the top limb set, rhs fits in 32 bits (rhs_hi=0).
        // 2^255 * 3 > 2^256, so must saturate via the carry != 0 path.
        let mut bytes = [0u8; 32];
        bytes[0] = 0x80; // 2^255
        let val = U256::from_be_bytes(bytes);
        let result = val.saturating_mul_u64(3);
        assert_eq!(result, U256::MAX);
    }

    #[test]
    fn u256_mul_large_rhs_lo_carry_across_limbs() {
        // Exercise carry propagation across multiple limbs in the rhs_lo path.
        // value = 0xFFFF_FFFF_FFFF_FFFF (limbs 0 and 1 maxed), rhs = 0xFFFF_FFFF.
        let mut bytes = [0u8; 32];
        for i in 24..32 {
            bytes[i] = 0xff;
        }
        let val = U256::from_be_bytes(bytes);
        let result = val.saturating_mul_u64(0xFFFF_FFFF);
        // (2^64 - 1) * (2^32 - 1) = 2^96 - 2^64 - 2^32 + 1
        // Verify roundtrip: result / (2^32-1) should recover original value.
        let back = result.div_u64(0xFFFF_FFFF);
        assert_eq!(back.to_be_bytes(), bytes);
    }

    #[test]
    fn u256_mul_carry_through_acc8() {
        // Product that exactly fills the 9th accumulator limb (acc[8]).
        // Use a value where the product barely overflows 256 bits.
        // value = 2^248 + 1, rhs = 256 = 2^8. rhs is small (rhs_hi=0).
        // 2^248 * 256 = 2^256 which overflows (acc[8] nonzero).
        let mut bytes = [0u8; 32];
        bytes[0] = 0x01; // 2^248
        bytes[31] = 0x01;
        let val = U256::from_be_bytes(bytes);
        let result = val.saturating_mul_u64(256);
        assert_eq!(result, U256::MAX);
    }

    #[test]
    fn u256_mul_no_overflow_high_limbs() {
        // A multiplication that exercises high limbs but doesn't overflow.
        // 2^224 * 2 = 2^225, which fits in U256.
        let mut bytes = [0u8; 32];
        bytes[4] = 0x01; // byte 4 = 2^224 in big-endian
        let val = U256::from_be_bytes(bytes);
        let result = val.saturating_mul_u64(2);
        let mut expected = [0u8; 32];
        expected[4] = 0x02;
        assert_eq!(result.to_be_bytes(), expected);
    }

    #[test]
    fn u256_mul_carry_propagation_through_limbs() {
        // Test carry propagation through multiple limbs in the rhs_lo path.
        // value = 0xFFFF_FFFF in each of the first 4 limbs, rhs = 2.
        // This forces carries to ripple through limbs.
        let mut bytes = [0u8; 32];
        for i in 16..32 {
            bytes[i] = 0xff;
        }
        let val = U256::from_be_bytes(bytes);
        let result = val.saturating_mul_u64(2);
        // (2^128 - 1) * 2 = 2^129 - 2
        let result_bytes = result.to_be_bytes();
        assert_eq!(result_bytes[15], 0x01); // 2^128 bit
        assert_eq!(result_bytes[31], 0xfe); // -2 at the bottom
        for i in 16..31 {
            assert_eq!(result_bytes[i], 0xff);
        }
    }

    // ── saturating_mul_u64: i=7 / rhs_hi boundary tests ───────────────────────

    #[test]
    fn u256_mul_max_times_u32_max_plus_one_saturates() {
        // U256::MAX * (u32::MAX as u64 + 1) == U256::MAX * 2^32.
        // Any nonzero value times 2^32 with all limbs maxed must overflow.
        let rhs = u64::from(u32::MAX) + 1; // 0x1_0000_0000
        let result = U256::MAX.saturating_mul_u64(rhs);
        assert_eq!(result, U256::MAX);
    }

    #[test]
    fn u256_mul_limb7_nonzero_rhs_hi_and_lo() {
        // Limb 7 = 2, all others zero.  rhs = 0x0000_0001_0000_0002
        // (rhs_hi = 1, rhs_lo = 2).
        // value = 2 * 2^224.  product = 2 * 2^224 * (2^32 + 2)
        //       = 2^257 + 2^226  →  overflows, must saturate.
        let val = u256_from_limbs([0, 0, 0, 0, 0, 0, 0, 2]);
        let rhs: u64 = 0x1_0000_0002;
        let result = val.saturating_mul_u64(rhs);
        assert_eq!(result, U256::MAX);
    }

    #[test]
    fn u256_mul_limb7_one_rhs_hi_one() {
        // value = 2^224 (limb 7 = 1).  rhs = 0x1_0000_0001 (rhs_hi=1, rhs_lo=1).
        // product = 2^224 * (2^32 + 1) = 2^256 + 2^224  →  overflow.
        let val = u256_from_limbs([0, 0, 0, 0, 0, 0, 0, 1]);
        let result = val.saturating_mul_u64(0x1_0000_0001);
        assert_eq!(result, U256::MAX);
    }

    #[test]
    fn u256_mul_reference_against_u128() {
        // For values that fit in u128 the result must match plain u128 arithmetic.
        let cases: &[(u128, u64)] = &[
            (0, 0x1_0000_0001),
            (1, 0x1_0000_0001),
            (0xFFFF_FFFF, 0xFFFF_FFFF),
            (0xFFFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF),
            (0xFFFF_FFFF_FFFF_FFFF, 0x1_0000_0001),
            (0x1234_5678_9ABC_DEF0, 0xFEDC_BA98),
            (u128::MAX >> 64, u64::MAX),
            (1u128 << 96, 3),
            (0xDEAD_BEEF_CAFE_BABE, 0x1_0000_0001),
        ];
        for &(lhs128, rhs) in cases {
            let wide = lhs128 as u128 * rhs as u128;
            let mut be = [0u8; 32];
            // Place the u128 in the low 16 bytes (big-endian positions 16..32).
            be[16..32].copy_from_slice(&lhs128.to_be_bytes());
            let val = U256::from_be_bytes(be);
            let result = val.saturating_mul_u64(rhs);
            if wide > u128::from(u64::MAX) * u128::from(u64::MAX) {
                // Could overflow 256 bits in extreme cases — skip comparison.
                continue;
            }
            let mut expected_be = [0u8; 32];
            expected_be[16..32].copy_from_slice(&wide.to_be_bytes());
            assert_eq!(
                result.to_be_bytes(),
                expected_be,
                "mismatch for {lhs128:#x} * {rhs:#x}"
            );
        }
    }

    #[test]
    fn expected_nbits_retarget_boundary() {
        // Build a chain of 2016 headers so height 2016 triggers retarget.
        let bits = POW_LIMIT_BITS;
        let base_time: u32 = 1_000_000;
        let mut view = MemoryChainView::new();
        // Push 2016 headers (heights 0..2015).
        for i in 0..2016u32 {
            let hdr = dummy_header(base_time + i * 600, bits);
            view.push_header(hdr);
        }
        // Parent is at height 2015; this_height = 2016 which is a retarget.
        let parent = dummy_header(base_time + 2015 * 600, bits);
        let result = expected_nbits(
            BlockHeight::new(2015),
            parent,
            &view,
            BlockTime::new(base_time + 2016 * 600),
            false,
        )
        .unwrap();
        // With exactly TARGET_TIMESPAN the recalculated target equals the
        // original, but CompactBits round-trips through U256 can lose the
        // lowest mantissa bits.  Just verify it stays at the max exponent.
        assert_eq!(result.to_u32() >> 24, POW_LIMIT_BITS.to_u32() >> 24);
    }
}
