use std::fmt;
use std::io::{Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, Error};

// ── BlockVersion ─────────────────────────────────────────────────────────────

/// Block header version field (i32, supports BIP9 version bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockVersion(i32);

impl BlockVersion {
    #[inline]
    #[must_use]
    pub const fn new(v: i32) -> Self {
        Self(v)
    }

    #[inline]
    #[must_use]
    pub const fn to_i32(self) -> i32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn to_le_bytes(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

impl Encode for BlockVersion {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
        self.0.encode(w)
    }
}

impl Decode for BlockVersion {
    #[inline]
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        i32::decode(r).map(Self)
    }
}

impl<'a> DecodeRef<'a> for BlockVersion {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), Error> {
        i32::decode_ref(buf).map(|(v, rest)| (Self(v), rest))
    }
}

// ── Nonce ────────────────────────────────────────────────────────────────────

/// Block header nonce field (u32, iterated during proof-of-work mining).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Nonce(u32);

impl Nonce {
    #[inline]
    #[must_use]
    pub const fn new(n: u32) -> Self {
        Self(n)
    }

    #[inline]
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn to_le_bytes(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

impl Encode for Nonce {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
        self.0.encode(w)
    }
}

impl Decode for Nonce {
    #[inline]
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        u32::decode(r).map(Self)
    }
}

impl<'a> DecodeRef<'a> for Nonce {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), Error> {
        u32::decode_ref(buf).map(|(v, rest)| (Self(v), rest))
    }
}

// ── CompactBits ──────────────────────────────────────────────────────────────

/// The `nBits` compact-format difficulty target stored in a block header.
///
/// Wraps a `u32` to make difficulty fields type-distinct from plain integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CompactBits(u32);

impl CompactBits {
    #[inline]
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[inline]
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn to_le_bytes(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

impl Encode for CompactBits {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
        self.0.encode(w)
    }
}

impl Decode for CompactBits {
    #[inline]
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        u32::decode(r).map(Self)
    }
}

impl<'a> DecodeRef<'a> for CompactBits {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), Error> {
        u32::decode_ref(buf).map(|(v, rest)| (Self(v), rest))
    }
}

impl CompactBits {
    /// Decode to a 256-bit target as a 32-byte big-endian array.
    ///
    /// Returns `None` if the value is zero, negative, or encodes an overflowed target.
    #[must_use]
    pub fn to_target(self) -> Option<[u8; 32]> {
        let raw = self.0;
        let exponent = (raw >> 24) as usize;
        let mantissa = raw & 0x007f_ffff;

        if raw & 0x0080_0000 != 0 {
            return None;
        } // negative mantissa
        if mantissa == 0 {
            return None;
        }
        if exponent > 34 || (exponent == 34 && mantissa > 0xff) {
            return None;
        }

        let mut target = [0u8; 32];
        if exponent >= 3 {
            let start = 32 - exponent;
            target[start] = ((mantissa >> 16) & 0xff) as u8;
            target[start + 1] = ((mantissa >> 8) & 0xff) as u8;
            target[start + 2] = (mantissa & 0xff) as u8;
        } else {
            let shift = (3 - exponent) * 8;
            let m = mantissa >> shift;
            if exponent > 0 {
                target[32 - exponent] = (m & 0xff) as u8;
            }
        }
        Some(target)
    }

    /// Encode a 256-bit big-endian target back into compact nBits format.
    ///
    /// Returns `None` if the target is all-zero.
    #[must_use]
    pub fn from_target(target: [u8; 32]) -> Option<Self> {
        let first_nonzero = target.iter().position(|&b| b != 0)?;
        let exponent = u32::try_from(32 - first_nonzero).ok()?;

        let mut mantissa = u32::from(target[first_nonzero]) << 16;
        if first_nonzero + 1 < 32 {
            mantissa |= u32::from(target[first_nonzero + 1]) << 8;
        }
        if first_nonzero + 2 < 32 {
            mantissa |= u32::from(target[first_nonzero + 2]);
        }

        if mantissa & 0x0080_0000 != 0 {
            mantissa >>= 8;
            Some(Self((exponent + 1) << 24 | mantissa))
        } else {
            Some(Self(exponent << 24 | mantissa))
        }
    }
}

/// A block height on the canonical chain.
///
/// Wraps a `u32` to make height values type-distinct from plain integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BlockHeight(u32);

impl BlockHeight {
    pub const ZERO: Self = Self(0);

    #[inline]
    #[must_use]
    pub const fn new(h: u32) -> Self {
        Self(h)
    }

    #[inline]
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        self.0
    }

    #[must_use]
    pub fn checked_add(self, rhs: u32) -> Option<Self> {
        self.0.checked_add(rhs).map(Self)
    }

    #[must_use]
    pub fn saturating_sub(self, rhs: u32) -> Self {
        Self(self.0.saturating_sub(rhs))
    }
}

impl fmt::Display for BlockHeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::ops::Add<u32> for BlockHeight {
    type Output = Self;
    fn add(self, rhs: u32) -> Self {
        Self(self.0.checked_add(rhs).expect("BlockHeight overflow"))
    }
}

impl std::ops::Sub<u32> for BlockHeight {
    type Output = Self;
    fn sub(self, rhs: u32) -> Self {
        Self(self.0.checked_sub(rhs).expect("BlockHeight underflow"))
    }
}

impl std::ops::Rem<u32> for BlockHeight {
    type Output = u32;
    fn rem(self, rhs: u32) -> u32 {
        self.0 % rhs
    }
}
