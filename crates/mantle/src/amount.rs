use std::fmt;
use std::io::{Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, Error};

/// A monetary value in satoshis (1 BTC = 100,000,000 satoshis).
///
/// Distinct from a plain `u64` so that values, fees, and counts cannot be
/// accidentally interchanged. Construct with [`Satoshi::new`]; extract with
/// [`Satoshi::to_u64`] or `u64::from`. The inner representation is hidden so
/// the type stays a single source of truth for "this u64 is money."
///
/// Note: any `u64` is a representable `Satoshi` — the consensus rule that
/// values must be at or below [`MAX_MONEY`](Self::MAX_MONEY) is enforced by
/// the validator, not the type. This matches Bitcoin Core's `CAmount`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Satoshi(u64);

impl Satoshi {
    pub const ZERO: Self = Self(0);

    /// Maximum money supply: 21,000,000 BTC expressed in satoshis.
    pub const MAX_MONEY: Self = Self(21_000_000 * 100_000_000);

    /// Construct from a raw `u64` count of satoshis.
    #[inline]
    #[must_use]
    pub const fn new(sats: u64) -> Self {
        Self(sats)
    }

    /// Extract the underlying `u64` count of satoshis.
    #[inline]
    #[must_use]
    pub const fn to_u64(self) -> u64 {
        self.0
    }

    /// `self + rhs`, returning `None` on overflow.
    #[must_use]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }
}

impl From<u64> for Satoshi {
    #[inline]
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<Satoshi> for u64 {
    #[inline]
    fn from(v: Satoshi) -> Self {
        v.0
    }
}

impl fmt::Display for Satoshi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Encode for Satoshi {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> Result<(), Error> {
        self.0.encode(w)
    }
}

impl Decode for Satoshi {
    #[inline]
    fn decode(r: &mut impl Read) -> Result<Self, Error> {
        u64::decode(r).map(Self)
    }
}

impl<'a> DecodeRef<'a> for Satoshi {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), Error> {
        u64::decode_ref(buf).map(|(v, rest)| (Self(v), rest))
    }
}
