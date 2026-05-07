use mantle::hash::sha256d;
use mantle::primitives::{BlockHeader, BlockHeight, CompactBits};

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
pub(super) const POW_LIMIT_BITS: CompactBits = CompactBits(0x1d00_ffff);

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

/// Compute the expected nBits for the block at `parent_height + 1`.
pub(super) fn expected_nbits(
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
            *limb = u32::from_be_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]);
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
